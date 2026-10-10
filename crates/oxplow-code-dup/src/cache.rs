//! Per-file subtree records, cached by content so an unchanged file is
//! neither read nor parsed again between scans (`.context/code-quality.md`
//! "Scoped scans").
//!
//! An entry is a file's eligible subtrees ([`Record`]s: hash, size, lines,
//! enclosing function), independent of any scan's scope, so one entry
//! serves a file in scope (all its records) and out of it (those whose
//! hash the scope has). It is keyed on what decides the records: the
//! file's content id in its identity space, its language, the detector's
//! version ([`DETECTOR_VERSION`]) and the size floors it was built with —
//! an edit is a new id, so nothing here is ever stale.
//!
//! The cache is bounded in bytes ([`CACHE_BUDGET_BYTES`]) and evicts whole
//! files, least recently used first.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

/// Bumped on any change to how subtrees are hashed or chosen (the
/// tokenizer's normalization, skip rules, the tree-sitter grammars): a
/// golden test pins the hashes to it, so changing the hashing without
/// bumping this fails the suite.
pub const DETECTOR_VERSION: u32 = 1;

/// What the cache may hold, in bytes of records. Sized so the whole
/// source tree of a large repo fits: a record is [`RECORD_BYTES`] and a
/// source file has a few hundred (`.context/performance.md` "Duplicate
/// detection cache"). Not a setting until someone needs one.
pub const CACHE_BUDGET_BYTES: usize = 32 * 1024 * 1024;

/// One eligible subtree of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Record {
    pub hash: u64,
    pub node_count: u32,
    pub start_line: u32,
    pub end_line: u32,
    /// Byte range of the enclosing function.
    pub fn_byte_start: u32,
    pub fn_byte_end: u32,
}

/// What one record costs the budget.
pub const RECORD_BYTES: usize = std::mem::size_of::<Record>();

/// A file's content identity: the space its id is in (a snapshot's content
/// hash, the VCS's object id) and the id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileKey {
    pub space: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CacheKey {
    pub file: FileKey,
    pub lang: u32,
    pub min_nodes: u32,
    pub min_lines: u32,
    pub version: u32,
}

/// Hits and misses of one scan: a cold scan after a restart is all misses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanStats {
    pub hits: usize,
    pub misses: usize,
}

struct Entry {
    records: Arc<[Record]>,
    tick: u64,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<CacheKey, Entry>,
    /// Least recently used first.
    order: BTreeMap<u64, CacheKey>,
    tick: u64,
    bytes: usize,
}

/// The cache. Share one per process (a detection runs one at a time).
pub struct DupCache {
    budget: usize,
    inner: Mutex<Inner>,
}

impl Default for DupCache {
    fn default() -> Self {
        Self::with_budget(CACHE_BUDGET_BYTES)
    }
}

impl DupCache {
    pub fn with_budget(bytes: usize) -> Self {
        Self {
            budget: bytes,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn get(&self, key: &CacheKey) -> Option<Arc<[Record]>> {
        let mut inner = self.lock();
        inner.tick += 1;
        let tick = inner.tick;
        let entry = inner.entries.get_mut(key)?;
        let old = std::mem::replace(&mut entry.tick, tick);
        let records = Arc::clone(&entry.records);
        inner.order.remove(&old);
        inner.order.insert(tick, key.clone());
        Some(records)
    }

    /// Keep `records` for `key`, evicting the least recently used files
    /// until the cache is within its budget. A file bigger than the whole
    /// budget is not kept.
    pub(crate) fn put(&self, key: CacheKey, records: Arc<[Record]>) {
        let cost = records.len() * RECORD_BYTES;
        if cost > self.budget {
            return;
        }
        let mut inner = self.lock();
        if let Some(old) = inner.entries.remove(&key) {
            inner.order.remove(&old.tick);
            inner.bytes -= old.records.len() * RECORD_BYTES;
        }
        while inner.bytes + cost > self.budget {
            let Some((_, oldest)) = inner.order.pop_first() else {
                break;
            };
            if let Some(gone) = inner.entries.remove(&oldest) {
                inner.bytes -= gone.records.len() * RECORD_BYTES;
            }
        }
        inner.tick += 1;
        let tick = inner.tick;
        inner.order.insert(tick, key.clone());
        inner.bytes += cost;
        inner.entries.insert(key, Entry { records, tick });
    }

    /// Records held, and the bytes they count against the budget.
    pub fn held(&self) -> (usize, usize) {
        let inner = self.lock();
        let records = inner.entries.values().map(|e| e.records.len()).sum();
        (records, inner.bytes)
    }
}
