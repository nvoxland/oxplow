//! Cores for the `snapshot` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

pub use oxplow_app::endpoint_diff::{DiffEndpoint, DiffEntry};
use oxplow_app::Services;
use oxplow_db::{FileSnapshot, Snapshot, SnapshotStats};
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

/// Diff two endpoints. `start = None` diffs `end` against the empty
/// tree (everything added).
///
/// The unified resolver builds a `path -> content cell` tree for each
/// endpoint, then compares. Same-identity-space pairs (snapshot↔snapshot,
/// commit↔commit, commit↔working, …) compare raw — no byte reads.
/// Cross-space pairs (snapshot↔commit, snapshot↔working) **normalize**
/// the snapshot side into git-oid space (read each oxplow-stored blob's
/// bytes, recompute its git blob oid) so it compares like-for-like
/// against the git tree. Oversize / pruned content keeps a best-effort
/// opaque identity. `additions`/`deletions` are per-file line counts via
/// `similar`, computed for the changed set only.
pub async fn diff_endpoints(
    svc: &Services,
    start: Option<DiffEndpoint>,
    end: DiffEndpoint,
) -> Result<Vec<DiffEntry>, IpcError> {
    // Snapshot trees come off the DB (async); prefetch them, then do the
    // git / fs / hashing / diff work on the blocking pool.
    let mut start_snap = match &start {
        Some(DiffEndpoint::Snapshot { snapshot_id }) => {
            Some(svc.snapshot_store.tree_at(*snapshot_id).await?)
        }
        _ => None,
    };
    let mut end_snap = match &end {
        DiffEndpoint::Snapshot { snapshot_id } => {
            Some(svc.snapshot_store.tree_at(*snapshot_id).await?)
        }
        _ => None,
    };
    if let (Some(a), Some(b)) = (start_snap.as_mut(), end_snap.as_mut()) {
        svc.snapshot_store.resolve_for_compare(a, b).await?;
    }
    let project_dir = svc.layout.project_dir.clone();
    let blobs = svc.blobs.clone();
    let filter = current_filter(svc);
    tokio::task::spawn_blocking(move || {
        oxplow_app::endpoint_diff::compute_diff(
            start,
            end,
            start_snap,
            end_snap,
            &project_dir,
            &blobs,
            &filter,
        )
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
    .map_err(|e| match e {
        // The routine miss since tsk105: records are permanent, but the
        // on-disk bytes age out of the retention window. Refuse plainly —
        // never a half-restore, never an opaque internal error.
        oxplow_app::snapshot_content::SnapshotReadError::Blob(_) => IpcError::invalid(
            "this snapshot's content has expired from Local History — the record is \
             permanent, but file bytes are only kept for the retention window",
        ),
        other => IpcError::internal(other.to_string()),
    })?;
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
            // A row with no bytes is a deletion tombstone.
            storage: if hash.is_some() {
                oxplow_db::SnapshotStorage::Oxplow
            } else {
                oxplow_db::SnapshotStorage::Deleted
            },
            snapshot_id: Some(snap),
            mtime_ms: None,
            content_hash: None,
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
                content_hash: None,
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
    async fn diff_endpoints_mixed_snapshot_vs_commit_normalizes_oxplow_blob() {
        let (svc, dir) = crate::test_support::services();
        let p = dir.path();
        let stream = svc.streams.list_streams().await.unwrap()[0].id;
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(p)
                .status()
                .unwrap()
                .success());
        };
        let rev = || {
            let out = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(p)
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };

        // Commit a file, then capture an oxplow-storage snapshot of the
        // SAME bytes (blob in the oxplow store, keyed by xxh3). The two
        // identities differ raw (xxh3 vs git oid) but must compare equal
        // after the snapshot side is normalized into git-oid space.
        let content = "line one\nline two\n";
        std::fs::write(p.join("a.txt"), content).unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c1"]);
        let c1 = rev();

        let hash = svc.blobs.write(content.as_bytes()).unwrap();
        let p1 = svc.snapshot_store.create_snapshot(stream).await.unwrap();
        svc.snapshot_store
            .capture(oxplow_db::FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "a.txt".into(),
                blob_hash: Some(hash),
                size_bytes: content.len() as i64,
                captured_at: oxplow_domain::Timestamp::now(),
                storage: oxplow_db::SnapshotStorage::Oxplow,
                snapshot_id: Some(p1),
                mtime_ms: None,
                content_hash: None,
            })
            .await
            .unwrap();

        let same = super::diff_endpoints(
            &svc,
            Some(super::DiffEndpoint::Snapshot { snapshot_id: p1 }),
            super::DiffEndpoint::Commit { sha: c1 },
        )
        .await
        .unwrap();
        assert!(
            !same.iter().any(|e| e.path == "a.txt"),
            "identical content across stores must not be a change: {same:?}"
        );

        // Now change the committed bytes → a.txt is modified.
        std::fs::write(p.join("a.txt"), "line one\nCHANGED\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c2"]);
        let c2 = rev();
        let changed = super::diff_endpoints(
            &svc,
            Some(super::DiffEndpoint::Snapshot { snapshot_id: p1 }),
            super::DiffEndpoint::Commit { sha: c2 },
        )
        .await
        .unwrap();
        assert!(
            changed
                .iter()
                .any(|e| e.path == "a.txt" && e.status == "modified"),
            "differing content must be modified: {changed:?}"
        );
    }

    #[tokio::test]
    async fn diff_endpoints_working_tree_endpoint_detects_new_file() {
        let (svc, dir) = crate::test_support::services();
        std::fs::write(dir.path().join("w.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let entries = super::diff_endpoints(&svc, None, super::DiffEndpoint::Working)
            .await
            .unwrap();
        let w = entries
            .iter()
            .find(|e| e.path == "w.txt")
            .expect("new working-tree file should appear as added");
        assert_eq!(w.status, "added");
        assert_eq!(w.additions, 3, "three added lines: {w:?}");
        assert_eq!(w.deletions, 0);
    }

    #[tokio::test]
    async fn diff_endpoints_populates_line_counts_for_commits() {
        let (svc, dir) = crate::test_support::services();
        let p = dir.path();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(p)
                .status()
                .unwrap()
                .success());
        };
        let rev = || {
            let out = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(p)
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        std::fs::write(p.join("m.txt"), "a\nb\nc\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c1"]);
        let c1 = rev();
        // line 2 b→B (1 del + 1 add), line 4 d appended (1 add).
        std::fs::write(p.join("m.txt"), "a\nB\nc\nd\n").unwrap();
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
        let m = entries
            .iter()
            .find(|e| e.path == "m.txt")
            .expect("m.txt changed");
        assert_eq!(m.status, "modified");
        assert_eq!((m.additions, m.deletions), (2, 1), "line counts: {m:?}");
    }

    #[tokio::test]
    async fn diff_endpoints_does_not_report_filtered_files_as_deleted() {
        // tsk177: the ignore-filter was applied to the WORKING endpoint only, so
        // any path present in the other endpoint but hidden from the worktree
        // scan classified as "deleted" with its full line count. A filtered path
        // isn't deleted — it's out of scope, and must be absent from BOTH sides.
        //
        // `.oxplow/project.yaml` is the live case: tracked in git (oxplow writes
        // `.oxplow/.gitignore` with `!project.yaml` precisely so it can be
        // committed), while WorkspaceFilter drops `.oxplow` unconditionally.
        let (svc, dir) = crate::test_support::services();
        let p = dir.path();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(p)
                .status()
                .unwrap()
                .success());
        };
        std::fs::create_dir_all(p.join(".oxplow")).unwrap();
        std::fs::write(
            p.join(".oxplow/.gitignore"),
            "*\n!.gitignore\n!project.yaml\n",
        )
        .unwrap();
        std::fs::write(p.join(".oxplow/project.yaml"), "collection:\n  a: 1\n").unwrap();
        std::fs::write(p.join("normal.txt"), "x\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c1"]);
        let out = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(p)
            .output()
            .unwrap();
        let c1 = String::from_utf8(out.stdout).unwrap().trim().to_string();

        // Nothing was removed from the worktree — both files are still there.
        let entries = super::diff_endpoints(
            &svc,
            Some(super::DiffEndpoint::Commit { sha: c1 }),
            super::DiffEndpoint::Working,
        )
        .await
        .unwrap();

        let phantom: Vec<_> = entries
            .iter()
            .filter(|e| e.status == "deleted")
            .map(|e| e.path.as_str())
            .collect();
        assert!(
            phantom.is_empty(),
            "nothing was deleted, but the diff claims: {phantom:?}"
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
