//! Cores for the `snapshot` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::Services;
use oxplow_db::{FileSnapshot, Snapshot, SnapshotChangeEntry, SnapshotStats};
use oxplow_domain::StreamId;
use oxplow_fs_watch::WorkspaceFilter;

use crate::error::IpcError;

/// Build a `WorkspaceFilter` from the project's currently-live
/// `generated` config. Used by the snapshot-list IPCs so that paths
/// matching the user's current ignore list are stripped from the
/// returned list — even if those paths were captured under an older
/// config (or before they were marked generated). The capture
/// pipeline already filters going forward via this same struct; this
/// is the read-side complement.
fn current_filter(svc: &Services) -> WorkspaceFilter {
    let cfg = svc.config.read();
    cfg.as_ref()
        .map(|c| {
            WorkspaceFilter::for_project(
                &svc.layout.project_dir,
                &c.generated.exclude,
                &c.generated.include,
            )
        })
        .unwrap_or_default()
}

pub async fn list_snapshots(svc: &Services, path: String) -> Result<Vec<FileSnapshot>, IpcError> {
    // Whole-path filter: if the queried path itself is currently
    // marked generated, return an empty history rather than the
    // pre-config captures. The UI shouldn't surface a "history" view
    // for a path the user has declared they don't care about.
    let filter = current_filter(svc);
    if filter.ignore(Path::new(&path), false) {
        return Ok(Vec::new());
    }
    Ok(svc.snapshot_store.list_for_path(&path).await?)
}

pub async fn list_file_snapshots_for_stream(
    svc: &Services,
    stream_id: StreamId,
    limit: Option<usize>,
) -> Result<Vec<FileSnapshot>, IpcError> {
    let filter = current_filter(svc);
    let rows = svc
        .snapshot_store
        .list_for_stream(stream_id, limit.unwrap_or(200))
        .await?;
    Ok(rows
        .into_iter()
        .filter(|r| !filter.ignore(Path::new(&r.path), false))
        .collect())
}

/// `snapshot` rows for a stream — one entry per `request_snapshot()`
/// call that captured anything. Newest first.
pub async fn list_snapshots_for_stream(
    svc: &Services,
    stream_id: StreamId,
    limit: Option<usize>,
) -> Result<Vec<Snapshot>, IpcError> {
    Ok(svc
        .snapshot_store
        .list_snapshots_for_stream(stream_id, limit.unwrap_or(200))
        .await?)
}

/// Created/modified/deleted counts for a snapshot. Powers the Local
/// History dashboard's per-snapshot stats column.
pub async fn get_snapshot_stats(
    svc: &Services,
    snapshot_id: i64,
) -> Result<SnapshotStats, IpcError> {
    Ok(svc.snapshot_store.stats_for_snapshot(snapshot_id).await?)
}

/// Per-file change entries for one snapshot, in the shape the
/// renderer's `useSnapshotChangeAnalysis` hook expects so it can
/// feed the same SummaryCard / ChangeAnalysisPanel components the
/// Git pages use.
pub async fn list_snapshot_change_entries(
    svc: &Services,
    snapshot_id: i64,
) -> Result<Vec<SnapshotChangeEntry>, IpcError> {
    let filter = current_filter(svc);
    let rows = svc
        .snapshot_store
        .list_changes_for_snapshot(snapshot_id)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|r| !filter.ignore(Path::new(&r.path), false))
        .collect())
}

/// Read a `file_snapshot` row's blob content as a UTF-8 string.
/// Returns `None` when:
/// - the row id doesn't exist,
/// - the row has no blob hash (deletion row or oversize-tracked),
/// - the blob has been pruned from disk.
///
/// Binary bytes pass through as UTF-8 lossy — the renderer's diff /
/// function-analysis pipeline treats the result as text either way.
pub async fn read_snapshot_file_content(
    svc: &Services,
    file_snapshot_id: i64,
) -> Result<Option<String>, IpcError> {
    let Some(snap) = svc.snapshot_store.get(file_snapshot_id).await? else {
        return Ok(None);
    };
    let Some(hash) = snap.blob_hash.clone() else {
        return Ok(None);
    };
    let blobs = svc.blobs.clone();
    let project_dir = svc.layout.project_dir.clone();
    let storage = snap.storage;
    let bytes = tokio::task::spawn_blocking(move || {
        oxplow_app::snapshot_content::read_snapshot_content(storage, &hash, &project_dir, &blobs)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?;
    match bytes {
        Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
        Err(_) => Ok(None),
    }
}

/// Total on-disk size of every blob in the content-addressed store.
/// Used by the Local History dashboard's Storage card.
pub async fn get_blob_storage_bytes(svc: &Services) -> Result<i64, IpcError> {
    let blobs = svc.blobs.clone();
    let total = tokio::task::spawn_blocking(move || blobs.total_bytes())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(total as i64)
}

/// For each snapshot id in the input list, the wiki slugs whose
/// body changed in that snapshot. Drives the Local History
/// dashboard's wiki badges. Cheaper than fetching the full
/// `file_snapshot` rows per snapshot.
pub async fn list_wiki_slugs_for_snapshots(
    svc: &Services,
    snapshot_ids: Vec<i64>,
) -> Result<Vec<(i64, String)>, IpcError> {
    Ok(svc
        .snapshot_store
        .list_wiki_slugs_for_snapshots(snapshot_ids)
        .await?)
}

/// Every `file_snapshot` row captured under a single parent
/// snapshot id (i.e. one batch of `request_snapshot()`).
pub async fn list_files_for_snapshot(
    svc: &Services,
    snapshot_id: i64,
) -> Result<Vec<FileSnapshot>, IpcError> {
    let filter = current_filter(svc);
    let rows = svc
        .snapshot_store
        .list_files_for_snapshot(snapshot_id)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|r| !filter.ignore(Path::new(&r.path), false))
        .collect())
}

pub async fn get_snapshot(svc: &Services, id: i64) -> Result<Option<FileSnapshot>, IpcError> {
    Ok(svc.snapshot_store.get(id).await?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SnapshotPairDiff {
    pub before: Option<FileSnapshot>,
    pub after: Option<FileSnapshot>,
    /// True when the two captures hash differently (i.e. content
    /// changed between them). Always false when either side is None.
    pub changed: bool,
}

/// Compare two captures of the same path. The renderer surfaces this
/// in the snapshots panel as "what changed between then and now".
pub async fn get_snapshot_pair_diff(
    svc: &Services,
    before_id: Option<i64>,
    after_id: Option<i64>,
) -> Result<SnapshotPairDiff, IpcError> {
    let before = match before_id {
        Some(id) => svc.snapshot_store.get(id).await?,
        None => None,
    };
    let after = match after_id {
        Some(id) => svc.snapshot_store.get(id).await?,
        None => None,
    };
    let changed = match (&before, &after) {
        (Some(b), Some(a)) => b.blob_hash != a.blob_hash,
        _ => false,
    };
    Ok(SnapshotPairDiff {
        before,
        after,
        changed,
    })
}

/// One endpoint of a diff: a captured local-history snapshot, a git
/// commit (any revspec libgit2 resolves), or the live working tree
/// (reserved for an in-progress effort's open end).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DiffEndpoint {
    Snapshot { snapshot_id: i64 },
    Commit { sha: String },
    Working,
}

/// One changed path between two [`DiffEndpoint`]s. `status` is
/// `"added" | "modified" | "deleted"`, matching the renderer's
/// `BranchChangeEntry`. Line counts are 0 until the per-file content
/// pass lands (tracked separately) — the SummaryCard renders 0s, the
/// same as the previous snapshot-mode behavior.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiffEntry {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

fn change_status_str(s: oxplow_domain::ChangeStatus) -> &'static str {
    match s {
        oxplow_domain::ChangeStatus::Added => "added",
        oxplow_domain::ChangeStatus::Modified => "modified",
        oxplow_domain::ChangeStatus::Deleted => "deleted",
    }
}

/// Diff two endpoints. `start = None` diffs `end` against the empty
/// tree (everything added). Homogeneous pairs use the native fast
/// paths — snapshot↔snapshot via `SqliteSnapshotStore::diff_snapshots`,
/// commit↔commit via [`oxplow_git::tree_at_commit`] + the shared
/// [`oxplow_domain::diff_trees`]. Mixed snapshot/commit and
/// working-tree endpoints are not yet supported: their content-identity
/// spaces differ (snapshot xxh3 vs git blob oid), so a faithful diff
/// needs a normalization pass (tracked separately).
pub async fn diff_endpoints(
    svc: &Services,
    start: Option<DiffEndpoint>,
    end: DiffEndpoint,
) -> Result<Vec<DiffEntry>, IpcError> {
    let changes = endpoint_changes(svc, start.as_ref(), &end).await?;
    Ok(changes
        .into_iter()
        .map(|c| DiffEntry {
            path: c.path,
            status: change_status_str(c.status).to_string(),
            additions: 0,
            deletions: 0,
        })
        .collect())
}

async fn endpoint_changes(
    svc: &Services,
    start: Option<&DiffEndpoint>,
    end: &DiffEndpoint,
) -> Result<Vec<oxplow_domain::FileChange>, IpcError> {
    use DiffEndpoint::{Commit, Snapshot};
    match (start, end) {
        (None, Snapshot { snapshot_id }) => Ok(svc
            .snapshot_store
            .diff_snapshots(None, *snapshot_id)
            .await?),
        (Some(Snapshot { snapshot_id: from }), Snapshot { snapshot_id: to }) => Ok(svc
            .snapshot_store
            .diff_snapshots(Some(*from), *to)
            .await?),
        (None, Commit { sha }) => commit_changes(svc, None, sha).await,
        (Some(Commit { sha: from }), Commit { sha: to }) => {
            commit_changes(svc, Some(from), to).await
        }
        _ => Err(IpcError::invalid(
            "diff_endpoints: mixed snapshot/commit and working-tree endpoints are not yet supported",
        )),
    }
}

/// commit↔commit (or empty↔commit) via libgit2 trees, off the async
/// runtime. `from = None` ⇒ everything in `to` is added.
async fn commit_changes(
    svc: &Services,
    from: Option<&str>,
    to: &str,
) -> Result<Vec<oxplow_domain::FileChange>, IpcError> {
    let dir = svc.layout.project_dir.clone();
    let from = from.map(str::to_string);
    let to = to.to_string();
    tokio::task::spawn_blocking(move || -> Result<_, String> {
        let after = oxplow_git::tree_at_commit(&dir, &to).map_err(|e| e.to_string())?;
        let before = match from {
            Some(f) => oxplow_git::tree_at_commit(&dir, &f).map_err(|e| e.to_string())?,
            None => std::collections::BTreeMap::new(),
        };
        Ok(oxplow_domain::diff_trees(&before, &after))
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(IpcError::internal)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotEntry {
    pub hash: String,
    pub mtime_ms: i64,
    pub size: i64,
    /// "present" for normal captures, "oversize" for files that
    /// exceeded the configured cap (no blob written).
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SnapshotFileRow {
    pub entry: SnapshotEntry,
    /// "created" when this is the first capture of `path`,
    /// "updated" when the prior capture had a different blob,
    /// "deleted" when the current capture has no blob (file gone).
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Default)]
pub struct SnapshotSummaryCounts {
    pub created: i64,
    pub updated: i64,
    pub deleted: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSummary {
    pub snapshot: FileSnapshot,
    pub previous_snapshot_id: Option<String>,
    pub files: std::collections::HashMap<String, SnapshotFileRow>,
    pub counts: SnapshotSummaryCounts,
}

/// Build a per-snapshot summary: the FileSnapshot row, the id of the
/// prior capture of the same path (if any), and a one-row diff
/// describing how the captured file relates to its predecessor
/// (created / updated / deleted). The renderer's local-history pane
/// keys off this shape.
pub async fn get_snapshot_summary(
    svc: &Services,
    snapshot_id: i64,
) -> Result<Option<SnapshotSummary>, IpcError> {
    let Some(snap) = svc.snapshot_store.get(snapshot_id).await? else {
        return Ok(None);
    };
    // Order is DESC by captured_at; find the row immediately after
    // ours (i.e. older). Equal-timestamp ties fall back to id order
    // implicitly via SQLite's row order.
    let history = svc.snapshot_store.list_for_path(&snap.path).await?;
    let mut prev: Option<&FileSnapshot> = None;
    let mut found_self = false;
    for row in &history {
        if found_self {
            prev = Some(row);
            break;
        }
        if row.id == snap.id {
            found_self = true;
        }
    }
    let kind = match (&snap.blob_hash, prev.and_then(|p| p.blob_hash.clone())) {
        (None, _) => "deleted",
        (Some(_), None) => "created",
        (Some(cur), Some(prev_hash)) if *cur == prev_hash => "updated",
        (Some(_), Some(_)) => "updated",
    };
    let state_label = if snap.storage.is_oversize() {
        "oversize"
    } else {
        "present"
    };
    let entry = SnapshotEntry {
        hash: snap.blob_hash.clone().unwrap_or_default(),
        mtime_ms: 0,
        size: snap.size_bytes,
        state: state_label.to_string(),
    };
    let mut files = std::collections::HashMap::new();
    files.insert(
        snap.path.clone(),
        SnapshotFileRow {
            entry,
            kind: kind.to_string(),
        },
    );
    let counts = SnapshotSummaryCounts {
        created: if kind == "created" { 1 } else { 0 },
        updated: if kind == "updated" { 1 } else { 0 },
        deleted: if kind == "deleted" { 1 } else { 0 },
    };
    Ok(Some(SnapshotSummary {
        snapshot: snap,
        previous_snapshot_id: prev.map(|p| p.id.to_string()),
        files,
        counts,
    }))
}

/// Restore a file's contents from a snapshot. Reads the bytes from
/// the content-addressed blob store using the snapshot's `blob_hash`
/// and writes them back to the snapshot's path inside the workspace.
/// Errors with NOT_FOUND if the snapshot row is gone or its blob
/// was pruned.
pub async fn restore_file_from_snapshot(svc: &Services, snapshot_id: i64) -> Result<(), IpcError> {
    let snap = svc
        .snapshot_store
        .get(snapshot_id)
        .await?
        .ok_or_else(IpcError::not_found)?;
    let hash = snap
        .blob_hash
        .clone()
        .ok_or_else(|| IpcError::invalid("snapshot has no blob (oversize or deleted)"))?;
    // Route through the read seam so a git-backed row recovers its bytes
    // from the git odb instead of the (absent) blob store.
    let bytes = oxplow_app::snapshot_content::read_snapshot_content(
        snap.storage,
        &hash,
        &svc.layout.project_dir,
        &svc.blobs,
    )
    .map_err(|e| IpcError::internal(e.to_string()))?;
    let target = svc.layout.project_dir.join(&snap.path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| IpcError::internal(e.to_string()))?;
    }
    std::fs::write(&target, &bytes).map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_snapshots_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_snapshots",
            serde_json::json!({ "path": "src/main.rs" }),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array(), "expected a JSON array, got {out}");
    }

    #[tokio::test]
    async fn get_snapshot_pair_diff_accepts_optional_ids() {
        let (svc, _dir) = crate::test_support::services();
        // Both ids missing → None/None → changed: false.
        let out = crate::dispatch("get_snapshot_pair_diff", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(out["changed"], serde_json::json!(false));
    }

    #[tokio::test]
    async fn diff_endpoints_snapshot_vs_snapshot_classifies_changes() {
        let (svc, _dir) = crate::test_support::services();
        let stream = svc.streams.list_streams().await.unwrap()[0].id;
        let store = &svc.snapshot_store;
        let mk = |path: &str, hash: Option<&str>, snap: i64| oxplow_db::FileSnapshot {
            id: 0,
            stream_id: stream,
            path: path.into(),
            blob_hash: hash.map(|h| h.into()),
            size_bytes: 1,
            captured_at: oxplow_domain::Timestamp::now(),
            storage: oxplow_db::SnapshotStorage::Oxplow,
            snapshot_id: Some(snap),
            mtime_ms: None,
        };
        // p1: a + b baselined.
        let p1 = store.create_snapshot(stream).await.unwrap();
        store.capture(mk("a.txt", Some("h-a-1"), p1)).await.unwrap();
        store.capture(mk("b.txt", Some("h-b-1"), p1)).await.unwrap();
        // p2: a modified, c added, b deleted.
        let p2 = store.create_snapshot(stream).await.unwrap();
        store.capture(mk("a.txt", Some("h-a-2"), p2)).await.unwrap();
        store.capture(mk("c.txt", Some("h-c-1"), p2)).await.unwrap();
        store.capture(mk("b.txt", None, p2)).await.unwrap();

        let entries = super::diff_endpoints(
            &svc,
            Some(super::DiffEndpoint::Snapshot { snapshot_id: p1 }),
            super::DiffEndpoint::Snapshot { snapshot_id: p2 },
        )
        .await
        .unwrap();
        let by: std::collections::HashMap<_, _> = entries
            .iter()
            .map(|e| (e.path.as_str(), e.status.as_str()))
            .collect();
        assert_eq!(by.get("a.txt"), Some(&"modified"));
        assert_eq!(by.get("c.txt"), Some(&"added"));
        assert_eq!(by.get("b.txt"), Some(&"deleted"));
        // unchanged paths are omitted.
        assert_eq!(entries.len(), 3);
    }

    #[tokio::test]
    async fn diff_endpoints_none_start_is_all_added() {
        let (svc, _dir) = crate::test_support::services();
        let stream = svc.streams.list_streams().await.unwrap()[0].id;
        let store = &svc.snapshot_store;
        let p1 = store.create_snapshot(stream).await.unwrap();
        store
            .capture(oxplow_db::FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "only.txt".into(),
                blob_hash: Some("h".into()),
                size_bytes: 1,
                captured_at: oxplow_domain::Timestamp::now(),
                storage: oxplow_db::SnapshotStorage::Oxplow,
                snapshot_id: Some(p1),
                mtime_ms: None,
            })
            .await
            .unwrap();
        let entries = super::diff_endpoints(
            &svc,
            None,
            super::DiffEndpoint::Snapshot { snapshot_id: p1 },
        )
        .await
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "added");
    }

    #[tokio::test]
    async fn diff_endpoints_commit_vs_commit_classifies_changes() {
        let (svc, dir) = crate::test_support::services();
        let p = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&p)
                .status()
                .unwrap()
                .success());
        };
        let rev = || {
            let out = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&p)
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        std::fs::write(p.join("keep.txt"), "k").unwrap();
        std::fs::write(p.join("mod.txt"), "v1").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c1"]);
        let c1 = rev();
        std::fs::write(p.join("mod.txt"), "v2").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c2"]);
        let c2 = rev();

        let entries = super::diff_endpoints(
            &svc,
            Some(super::DiffEndpoint::Commit { sha: c1 }),
            super::DiffEndpoint::Commit { sha: c2 },
        )
        .await
        .unwrap();
        assert!(entries
            .iter()
            .any(|e| e.path == "mod.txt" && e.status == "modified"));
        assert!(!entries.iter().any(|e| e.path == "keep.txt"));
    }

    #[tokio::test]
    async fn diff_endpoints_rejects_mixed_endpoints() {
        let (svc, _dir) = crate::test_support::services();
        let out = super::diff_endpoints(
            &svc,
            Some(super::DiffEndpoint::Snapshot { snapshot_id: 1 }),
            super::DiffEndpoint::Commit { sha: "HEAD".into() },
        )
        .await;
        assert!(
            out.is_err(),
            "mixed snapshot/commit endpoints not yet supported"
        );
    }

    #[tokio::test]
    async fn diff_endpoints_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "diff_endpoints",
            serde_json::json!({ "start": null, "end": { "kind": "commit", "sha": "HEAD" } }),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array(), "expected a JSON array, got {out}");
    }
}
