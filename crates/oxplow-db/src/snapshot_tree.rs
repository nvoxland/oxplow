//! A snapshot's reconstructed tree, typed (P2.1, tsk423;
//! `.context/data-model.md` "snapshot + file_snapshot").
//!
//! A snapshot row is a delta; the tree at snapshot `S` is the latest
//! `file_snapshot` row per path with `snapshot_id <= S`
//! ([`crate::SqliteSnapshotStore::tree_at`]). Each entry keeps its storage
//! class and address (where the bytes live) apart from its content
//! identity (what the bytes are), so a diff never compares a git blob OID
//! with an xxh3 — the bug that made the same bytes look changed whenever
//! one side was git-backed and the other copied into the blob store.

use std::collections::BTreeMap;

use crate::{SnapshotContentRef, SnapshotStorage};

/// One path in a reconstructed tree. Deletion tombstones never appear:
/// a deleted path is absent from the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub storage: SnapshotStorage,
    /// Where the bytes live: an xxh3 for `oxplow`, a git blob OID for
    /// `git`, `None` for `oversize`.
    pub address: Option<String>,
    /// The xxh3-128 of the bytes. Always known for `oxplow`; filled
    /// lazily for `git` (see `SqliteSnapshotStore::with_content_hasher`);
    /// `None` for `oversize`.
    pub content_hash: Option<String>,
    pub size_bytes: i64,
    pub mtime_ms: Option<i64>,
}

/// A reconstructed tree: path → entry.
pub type SnapshotTree = BTreeMap<String, TreeEntry>;

/// Hashes a git blob, given its OID, to the xxh3-128 content hash — the
/// lazy half of the content identity for git-backed rows. Supplied by the
/// app layer (it owns the git odb and the blob-store hash); blocking.
/// `None` when the object is gone (e.g. GC'd after a history rewrite).
pub type ContentHasher = std::sync::Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

impl TreeEntry {
    /// The comparison identity. Two entries with equal identities hold
    /// the same bytes; unequal identities almost always mean different
    /// bytes. The one exception is an un-hashed `git` entry (`git:<oid>`)
    /// against a hashed entry — call
    /// `SqliteSnapshotStore::resolve_for_compare` first, which hashes
    /// exactly those. Oversize files have no content hash, so their
    /// identity is `oversize:<size>:<mtime>` (a change to a too-big file
    /// is still noticed).
    pub fn identity(&self) -> String {
        if let Some(h) = &self.content_hash {
            return h.clone();
        }
        match (self.storage, &self.address) {
            // An oxplow address IS the xxh3 of the bytes (rows written
            // before V96 or by raw SQL may lack the column).
            (SnapshotStorage::Oxplow, Some(a)) => a.clone(),
            (SnapshotStorage::Git, Some(oid)) => format!("git:{oid}"),
            _ => format!(
                "oversize:{}:{}",
                self.size_bytes,
                self.mtime_ms.unwrap_or(0)
            ),
        }
    }

    /// True for a git-backed entry whose content hash isn't known yet.
    pub fn needs_content_hash(&self) -> bool {
        self.storage == SnapshotStorage::Git
            && self.content_hash.is_none()
            && self.address.is_some()
    }

    /// Where to read the bytes, when there are bytes.
    pub fn content_ref(&self) -> Option<SnapshotContentRef> {
        match (self.storage.has_bytes(), &self.address) {
            (true, Some(a)) => Some(SnapshotContentRef {
                storage: self.storage,
                hash: a.clone(),
            }),
            _ => None,
        }
    }
}

/// `path → identity` for [`oxplow_domain::diff_trees`].
pub fn identities(tree: &SnapshotTree) -> BTreeMap<String, String> {
    tree.iter()
        .map(|(p, e)| (p.clone(), e.identity()))
        .collect()
}

/// Whole-tree identity: the xxh3-128 (lowercase hex) of the manifest
/// `path \0 identity \n`, sorted by path (a `BTreeMap` iterates sorted).
/// Equal trees hash equal. It is conservative for un-hashed git entries
/// (`git:<oid>` differs from the xxh3 of the same bytes), so an equal
/// tree can occasionally hash differently — which costs an extra
/// snapshot, never a missed change.
pub fn manifest_hash(tree: &SnapshotTree) -> String {
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    for (path, entry) in tree {
        h.update(path.as_bytes());
        h.update(b"\0");
        h.update(entry.identity().as_bytes());
        h.update(b"\n");
    }
    format!("{:032x}", h.digest128())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(storage: SnapshotStorage, address: Option<&str>, content: Option<&str>) -> TreeEntry {
        TreeEntry {
            storage,
            address: address.map(str::to_string),
            content_hash: content.map(str::to_string),
            size_bytes: 3,
            mtime_ms: Some(7),
        }
    }

    #[test]
    fn identity_prefers_content_and_never_mixes_address_spaces() {
        let oid = "a".repeat(40);
        assert_eq!(
            entry(SnapshotStorage::Oxplow, Some("x1"), Some("x1")).identity(),
            "x1"
        );
        // Pre-V96 oxplow row: the address is the content hash.
        assert_eq!(
            entry(SnapshotStorage::Oxplow, Some("x1"), None).identity(),
            "x1"
        );
        // Git row: its OID never stands in for a content hash.
        assert_eq!(
            entry(SnapshotStorage::Git, Some(&oid), None).identity(),
            format!("git:{oid}")
        );
        assert!(entry(SnapshotStorage::Git, Some(&oid), None).needs_content_hash());
        assert_eq!(
            entry(SnapshotStorage::Git, Some(&oid), Some("x1")).identity(),
            "x1"
        );
        assert!(!entry(SnapshotStorage::Git, Some(&oid), Some("x1")).needs_content_hash());
        assert_eq!(
            entry(SnapshotStorage::Oversize, None, None).identity(),
            "oversize:3:7"
        );
        assert_eq!(
            entry(SnapshotStorage::Oversize, None, None).content_ref(),
            None
        );
        assert_eq!(
            entry(SnapshotStorage::Git, Some(&oid), Some("x1")).content_ref(),
            Some(SnapshotContentRef {
                storage: SnapshotStorage::Git,
                hash: oid
            })
        );
    }

    #[test]
    fn manifest_hash_is_the_xxh3_of_the_sorted_manifest() {
        let mut t = SnapshotTree::new();
        t.insert(
            "b.txt".into(),
            entry(SnapshotStorage::Oxplow, Some("hb"), Some("hb")),
        );
        t.insert(
            "a.txt".into(),
            entry(SnapshotStorage::Oxplow, Some("ha"), Some("ha")),
        );
        let expected = format!(
            "{:032x}",
            xxhash_rust::xxh3::xxh3_128(b"a.txt\0ha\nb.txt\0hb\n")
        );
        assert_eq!(manifest_hash(&t), expected);
        assert_eq!(manifest_hash(&t).len(), 32);
        // Same content through a different storage class hashes the same.
        let mut g = t.clone();
        g.insert(
            "a.txt".into(),
            entry(SnapshotStorage::Git, Some(&"c".repeat(40)), Some("ha")),
        );
        assert_eq!(manifest_hash(&g), manifest_hash(&t));
        // A change changes it; the empty tree has a hash too.
        let mut m = t.clone();
        m.insert(
            "a.txt".into(),
            entry(SnapshotStorage::Oxplow, Some("hz"), Some("hz")),
        );
        assert_ne!(manifest_hash(&m), manifest_hash(&t));
        assert_eq!(
            manifest_hash(&SnapshotTree::new()),
            format!("{:032x}", xxhash_rust::xxh3::xxh3_128(b""))
        );
    }
}
