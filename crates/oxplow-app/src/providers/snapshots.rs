//! An external provider's snapshots: [`ExternalSnapshots`] is the
//! [`SnapshotProvider`] registered under the instance's id while it runs
//! (`.context/providers.md` "A snapshots provider"). The process marks the
//! worktree; core keeps the record. Each mark is:
//!
//! - `mark { stream, worktree, trigger, thread?, turn?, effort?,
//!   budget_ms?, parent }` → `{ handle, unchanged, file_count, branch?,
//!   revision? }`, `parent` the handle on the stream's newest op (`null`
//!   when core's capture took it, or nothing has);
//! - unless it's unchanged from a parent it named, `changed { stream,
//!   from, to }` → `{ changes: [{ path, kind, identity?, size? }] }`;
//! - with `contents`, `read_at { handle, path }` → `{ bytes }` (base64)
//!   for each new identity the blob store doesn't hold, checked against
//!   the identity it reported;
//!
//! then one `SqliteSnapshotStore::record_take`, the one write path, with
//! the handle on its op. What changed and every read answer from that
//! record, so the rest of oxplow reads a provider's snapshots as it reads
//! its own. Without `contents` its rows are identities with no bytes, as
//! the hashes-only built-in's are. It emits nothing of its own.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use base64::Engine as _;
use oxplow_db::{FileSnapshot, SnapshotStorage, SnapshotTree, TakeRecord};
use oxplow_domain::capability::SNAPSHOTS;
use oxplow_domain::refs::build::{effort_ref, stream_ref, thread_ref};
use oxplow_domain::snapshot::{
    MarkRequest, Marked, SnapshotError, SnapshotProvider, SnapshotRegistry,
};
use oxplow_domain::tree_diff::FileChange;
use oxplow_domain::vcs::Revision;
use oxplow_domain::{InputValidator, StreamId, Timestamp};
use serde::Deserialize;
use serde_json::{json, Value};

use super::registry::{CapabilityHost, Instance};
use crate::blob_store::BlobStore;
use crate::snapshot_capture_registry::SnapshotCaptureRegistry;
use crate::snapshot_files::SnapshotFiles;

/// What a provider's marks are recorded into.
#[derive(Clone)]
pub struct Ledger {
    pub captures: SnapshotCaptureRegistry,
    pub blobs: BlobStore,
    pub files: SnapshotFiles,
}

/// The snapshots' side of the host: a started instance is a snapshot
/// implementation in `Services.snapshots`, under its id. Publishing it
/// logs `capability.switched` when it's the project's choice, and the
/// captures follow (`snapshots::SnapshotSwitch`).
pub struct SnapshotsHost {
    pub registry: Arc<SnapshotRegistry>,
    pub ledger: Ledger,
}

impl SnapshotsHost {
    pub const CAPABILITY: &'static str = "snapshots";
}

impl CapabilityHost for SnapshotsHost {
    fn capability(&self) -> &'static str {
        Self::CAPABILITY
    }

    fn has(&self, id: &str) -> bool {
        self.registry.has(id)
    }

    fn admit(&self, instance: &Arc<Instance>) -> Result<Value, String> {
        let declared = &instance.declared;
        let capability = declared
            .capabilities
            .iter()
            .find(|c| c.capability == Self::CAPABILITY)
            .ok_or("it declares no `snapshots` capability")?;
        let features = Some(capability.features.clone())
            .filter(|f| !f.is_null())
            .unwrap_or_else(|| json!({}));
        let contents = features.get("contents").and_then(Value::as_bool) == Some(true);
        let mut verbs = BTreeMap::new();
        for verb in SNAPSHOTS.verbs {
            if let Some(c) = declared.commands.iter().find(|c| c.name == verb.name) {
                let input = InputValidator::compile(&c.input_schema)
                    .map_err(|e| format!("verb `{}`: {e}", verb.name))?;
                verbs.insert(verb.name, input);
            }
        }
        for required in SNAPSHOTS.required(&features) {
            if !verbs.contains_key(required) {
                return Err(format!("it declares no `{required}` verb"));
            }
        }
        self.registry.register(Arc::new(ExternalSnapshots {
            instance: instance.clone(),
            contents,
            verbs,
            ledger: self.ledger.clone(),
            marking: tokio::sync::Mutex::new(()),
        }));
        Ok(features)
    }

    fn retire(&self, id: &str) {
        self.registry.unregister(id);
    }
}

pub struct ExternalSnapshots {
    instance: Arc<Instance>,
    contents: bool,
    /// Each declared contract verb's compiled input schema.
    verbs: BTreeMap<&'static str, InputValidator>,
    ledger: Ledger,
    /// One mark at a time: each reads the handle the last one recorded.
    marking: tokio::sync::Mutex<()>,
}

/// `mark`'s answer.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkAnswer {
    handle: String,
    /// The files it saw; the record counts what it records.
    file_count: u64,
    unchanged: bool,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    revision: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangedAnswer {
    changes: Vec<Change>,
}

/// One entry of `changed`'s answer.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Change {
    pub path: String,
    pub kind: ChangeKind,
    #[serde(default)]
    pub identity: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BytesAnswer {
    bytes: String,
}

/// The rows a `changed` answer records, and the files whose bytes to pull.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Planned {
    pub rows: Vec<FileSnapshot>,
    /// `(path, identity)` of each file to keep the bytes of.
    pub pull: Vec<(String, String)>,
}

/// Turn `changes` into ledger rows for `stream`. `whole` is the ledger's
/// current tree when the listing is the provider's whole tree (it marked
/// with no parent of its own): files it lists unchanged are left out, and
/// files it doesn't list are tombstoned. A file over `max_bytes` is
/// `oversize`; with `contents` every other new file's bytes are pulled.
pub(crate) fn plan(
    stream: StreamId,
    changes: Vec<Change>,
    max_bytes: u64,
    contents: bool,
    whole: Option<&SnapshotTree>,
) -> Result<Planned, String> {
    let now = Timestamp::now();
    let row = |path: &str, storage, hash: Option<&str>, size: u64| FileSnapshot {
        id: 0,
        stream_id: stream,
        path: path.to_string(),
        blob_hash: hash.map(str::to_string),
        size_bytes: size as i64,
        captured_at: now,
        storage,
        snapshot_id: None,
        mtime_ms: None,
        content_hash: hash.map(str::to_string),
    };
    let mut planned = Planned::default();
    let mut seen = BTreeSet::new();
    for change in changes {
        let path = workspace_path(&change.path)?;
        if !seen.insert(path.clone()) {
            return Err(format!("it lists `{path}` twice"));
        }
        if change.kind == ChangeKind::Deleted {
            if whole.is_some() {
                return Err(format!("it lists `{path}` as deleted from the empty tree"));
            }
            planned
                .rows
                .push(row(&path, SnapshotStorage::Deleted, None, 0));
            continue;
        }
        let identity = change
            .identity
            .as_deref()
            .filter(|i| is_identity(i))
            .ok_or_else(|| {
                format!("`{path}` has no identity (32 lower-case hex digits of its xxh3-128)")
            })?;
        let size = change.size.ok_or_else(|| format!("`{path}` has no size"))?;
        let same = whole
            .and_then(|t| t.get(&path))
            .is_some_and(|e| e.content_hash.as_deref() == Some(identity));
        if same {
            continue;
        }
        if size > max_bytes {
            planned
                .rows
                .push(row(&path, SnapshotStorage::Oversize, None, size));
            continue;
        }
        planned
            .rows
            .push(row(&path, SnapshotStorage::Oxplow, Some(identity), size));
        if contents {
            planned.pull.push((path, identity.to_string()));
        }
    }
    if let Some(tree) = whole {
        for (path, entry) in tree {
            if entry.storage != SnapshotStorage::Deleted && !seen.contains(path) {
                planned
                    .rows
                    .push(row(path, SnapshotStorage::Deleted, None, 0));
            }
        }
    }
    Ok(planned)
}

/// A provider's path as the ledger keeps it: relative to the worktree,
/// `/`-separated, with no `..`, `.` or root.
fn workspace_path(path: &str) -> Result<String, String> {
    let ok = !path.contains('\\')
        && path
            .split('/')
            .all(|segment| !matches!(segment, "" | "." | ".."));
    if ok {
        Ok(path.to_string())
    } else {
        Err(format!("`{path}` isn't a path inside the worktree"))
    }
}

fn is_identity(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl ExternalSnapshots {
    fn failed(&self, message: impl std::fmt::Display) -> SnapshotError {
        SnapshotError::Storage(format!(
            "snapshot provider `{}`: {message}",
            self.instance.name
        ))
    }

    /// Invoke `verb` with `input`, checked against what it declared, and
    /// read its answer as `T`.
    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        verb: &'static str,
        input: Value,
    ) -> Result<T, SnapshotError> {
        let schema = self
            .verbs
            .get(verb)
            .ok_or_else(|| self.failed(format!("it declares no `{verb}` verb")))?;
        schema
            .check(&input)
            .map_err(|e| self.failed(format!("its `{verb}` schema refuses the call: {e}")))?;
        let key = format!("{verb}:{}", uuid::Uuid::new_v4().simple());
        let out = self
            .instance
            .invoke(verb, input, Some(key))
            .await
            .map_err(|e| self.failed(format!("`{verb}`: {e}")))?;
        if !out.events.is_empty() {
            return Err(self.failed(format!(
                "`{verb}` returned events, and a snapshots provider emits none"
            )));
        }
        serde_json::from_value(out.result)
            .map_err(|e| self.failed(format!("`{verb}` answered something else: {e}")))
    }

    /// Keep the bytes of each `(path, identity)` the blob store lacks.
    async fn pull(&self, handle: &str, pull: Vec<(String, String)>) -> Result<(), SnapshotError> {
        for (path, identity) in pull {
            if self.ledger.blobs.has(&identity) {
                continue;
            }
            let answer: BytesAnswer = self
                .call("read_at", json!({ "handle": handle, "path": path }))
                .await?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(answer.bytes)
                .map_err(|e| self.failed(format!("`read_at` of `{path}`: {e}")))?;
            if BlobStore::hash(&bytes) != identity {
                return Err(self.failed(format!(
                    "the bytes of `{path}` aren't the identity it reported"
                )));
            }
            let blobs = self.ledger.blobs.clone();
            tokio::task::spawn_blocking(move || blobs.write_hashed(&identity, &bytes))
                .await
                .map_err(|e| self.failed(e))?
                .map_err(|e| self.failed(e))?;
        }
        Ok(())
    }
}

#[async_trait]
impl SnapshotProvider for ExternalSnapshots {
    fn id(&self) -> &str {
        &self.instance.id
    }

    fn contents(&self) -> bool {
        self.contents
    }

    async fn mark(&self, request: &MarkRequest) -> Result<Option<Marked>, SnapshotError> {
        // A stream with no worktree on this machine has nothing to mark.
        let Some(capture) = self.ledger.captures.get(&request.stream) else {
            return Ok(None);
        };
        let _one = self.marking.lock().await;
        let started = Instant::now();
        let store = &self.ledger.files.snapshots;
        let storage = |e: oxplow_domain::DomainError| SnapshotError::Storage(e.to_string());
        let newest = store
            .list_ops(request.stream, 1)
            .await
            .map_err(storage)?
            .into_iter()
            .next();
        let parent = newest.as_ref().and_then(|op| op.handle.clone());
        let mut input = json!({
            "stream": stream_ref(request.stream),
            "worktree": capture.project_dir().to_string_lossy(),
            "trigger": request.trigger.as_db_str(),
            "parent": parent,
        });
        if let Some(t) = request.thread {
            input["thread"] = json!(thread_ref(t));
        }
        if let Some(t) = request.turn {
            input["turn"] = json!(t);
        }
        if let Some(e) = request.effort {
            input["effort"] = json!(effort_ref(e));
        }
        if let Some(b) = request.budget {
            input["budget_ms"] = json!(b.as_millis() as u64);
        }
        let marked: MarkAnswer = self.call("mark", input).await?;
        tracing::debug!(
            provider = %self.instance.name,
            handle = %marked.handle,
            files = marked.file_count,
            unchanged = marked.unchanged,
            "snapshot provider marked",
        );
        let revision = match marked.revision.as_deref() {
            None => None,
            Some(r) => match r.parse::<Revision>() {
                Ok(rev @ Revision::Vcs { .. }) => Some(rev),
                _ => return Err(self.failed(format!("`{r}` isn't a VCS revision"))),
            },
        };
        let planned = if marked.unchanged && parent.is_some() {
            Planned::default()
        } else {
            let changed: ChangedAnswer = self
                .call(
                    "changed",
                    json!({
                        "stream": stream_ref(request.stream),
                        "from": parent,
                        "to": marked.handle,
                    }),
                )
                .await?;
            // A listing from the empty tree is its whole tree: what the
            // record holds now (core's chain, before a switch) reconciles.
            let whole = match (&parent, &newest) {
                (Some(_), _) => None,
                (None, Some(op)) => Some(store.tree_at(op.snapshot_id).await.map_err(storage)?),
                (None, None) => Some(SnapshotTree::new()),
            };
            plan(
                request.stream,
                changed.changes,
                self.ledger.captures.max_file_bytes(),
                self.contents,
                whole.as_ref(),
            )
            .map_err(|e| self.failed(e))?
        };
        self.pull(&marked.handle, planned.pull).await?;
        let outcome = store
            .record_take(TakeRecord {
                stream_id: request.stream,
                rows: planned.rows,
                trigger: request.trigger,
                thread_id: request.thread,
                turn_id: request.turn,
                effort_id: request.effort,
                branch: marked.branch,
                revision,
                elapsed_ms: started.elapsed().as_millis() as u64,
                budget_ms: request.budget.map(|b| b.as_millis() as u64),
                source: format!("provider:{}", self.instance.name),
                provider: Some(self.instance.id.clone()),
                contents: self.contents,
                handle: Some(marked.handle),
            })
            .await
            .map_err(storage)?;
        Ok(outcome.map(|o| Marked {
            snapshot: o.snapshot_id,
            parent: o.parent_snapshot_id,
            unchanged: o.unchanged,
            file_count: u64::from(o.file_count),
            over_budget: o.over_budget,
        }))
    }

    async fn changed(
        &self,
        _stream: StreamId,
        from: Option<i64>,
        to: i64,
    ) -> Result<Vec<FileChange>, SnapshotError> {
        crate::snapshots::changed_in(&self.ledger.files, from, to).await
    }

    async fn read_at(&self, snapshot: i64, path: &str) -> Result<Vec<u8>, SnapshotError> {
        crate::snapshots::read_recorded(&self.ledger.files, snapshot, path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::TreeEntry;

    const A: &str = "0123456789abcdef0123456789abcdef";
    const B: &str = "fedcba9876543210fedcba9876543210";

    fn change(path: &str, kind: ChangeKind, identity: Option<&str>, size: u64) -> Change {
        Change {
            path: path.into(),
            kind,
            identity: identity.map(str::to_string),
            size: (kind != ChangeKind::Deleted).then_some(size),
        }
    }

    fn shape(planned: &Planned) -> Vec<(String, SnapshotStorage, Option<String>)> {
        planned
            .rows
            .iter()
            .map(|r| (r.path.clone(), r.storage, r.content_hash.clone()))
            .collect()
    }

    /// Added and modified files are identities (bytes pulled with
    /// `contents`), deletions tombstones, a file over the cap `oversize`.
    #[test]
    fn a_listing_becomes_rows_and_the_bytes_to_pull() {
        let planned = plan(
            StreamId::new(1),
            vec![
                change("src/a.rs", ChangeKind::Added, Some(A), 5),
                change("b.rs", ChangeKind::Modified, Some(B), 5),
                change("gone.rs", ChangeKind::Deleted, None, 0),
                change("big.bin", ChangeKind::Added, Some(A), 100),
            ],
            10,
            true,
            None,
        )
        .unwrap();
        assert_eq!(
            shape(&planned),
            [
                ("src/a.rs".into(), SnapshotStorage::Oxplow, Some(A.into())),
                ("b.rs".into(), SnapshotStorage::Oxplow, Some(B.into())),
                ("gone.rs".into(), SnapshotStorage::Deleted, None),
                ("big.bin".into(), SnapshotStorage::Oversize, None),
            ]
        );
        assert_eq!(
            planned.pull,
            [("src/a.rs".into(), A.into()), ("b.rs".into(), B.into())]
        );
        // Without `contents` the same rows, nothing pulled.
        let identities = plan(
            StreamId::new(1),
            vec![change("src/a.rs", ChangeKind::Added, Some(A), 5)],
            10,
            false,
            None,
        )
        .unwrap();
        assert_eq!(identities.rows[0].blob_hash.as_deref(), Some(A));
        assert!(identities.pull.is_empty());
    }

    /// A whole-tree listing (no parent of its own) is reconciled with the
    /// record: what it lists unchanged is left out, what it doesn't list
    /// is deleted.
    #[test]
    fn a_whole_tree_listing_reconciles_with_the_record() {
        let entry = |hash: &str| TreeEntry {
            storage: SnapshotStorage::Oxplow,
            address: Some(hash.into()),
            content_hash: Some(hash.into()),
            size_bytes: 5,
            mtime_ms: None,
        };
        let tree: SnapshotTree = [
            ("same.rs".to_string(), entry(A)),
            ("edited.rs".to_string(), entry(A)),
            ("removed.rs".to_string(), entry(A)),
        ]
        .into_iter()
        .collect();
        let planned = plan(
            StreamId::new(1),
            vec![
                change("same.rs", ChangeKind::Added, Some(A), 5),
                change("edited.rs", ChangeKind::Added, Some(B), 5),
            ],
            10,
            true,
            Some(&tree),
        )
        .unwrap();
        assert_eq!(
            shape(&planned),
            [
                ("edited.rs".into(), SnapshotStorage::Oxplow, Some(B.into())),
                ("removed.rs".into(), SnapshotStorage::Deleted, None),
            ]
        );
        assert_eq!(planned.pull, [("edited.rs".into(), B.into())]);
    }

    /// Provider output is data: a path outside the worktree, a listing that
    /// names a file twice, or a file with no identity is refused.
    #[test]
    fn a_listing_that_isnt_the_worktree_is_refused() {
        for bad in ["../etc/passwd", "/abs", "a/./b", "a\\b", ""] {
            let err = plan(
                StreamId::new(1),
                vec![change(bad, ChangeKind::Added, Some(A), 1)],
                10,
                true,
                None,
            )
            .unwrap_err();
            assert!(
                err.contains("isn't a path inside the worktree"),
                "{bad}: {err}"
            );
        }
        let twice = plan(
            StreamId::new(1),
            vec![
                change("a", ChangeKind::Added, Some(A), 1),
                change("a", ChangeKind::Deleted, None, 0),
            ],
            10,
            true,
            None,
        )
        .unwrap_err();
        assert!(twice.contains("twice"), "{twice}");
        let unnamed = plan(
            StreamId::new(1),
            vec![change("a", ChangeKind::Added, Some("ABC"), 1)],
            10,
            true,
            None,
        )
        .unwrap_err();
        assert!(unnamed.contains("no identity"), "{unnamed}");
    }
}
