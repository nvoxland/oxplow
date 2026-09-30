//! Cores for the `snapshot` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use std::path::Path;

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

/// Every captured row of one file path, newest first (`file_snapshot` rows).
pub async fn list_file_snapshots(
    svc: &Services,
    path: String,
) -> Result<Vec<FileSnapshot>, IpcError> {
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

/// The stream's snapshot operation log, newest first: every take — a new
/// snapshot or one that found the tree unchanged — with its trigger,
/// anchors, timing and budget (P2.11).
pub async fn list_snapshot_ops(
    svc: &Services,
    stream_id: StreamId,
    limit: Option<usize>,
) -> Result<Vec<oxplow_db::SnapshotOp>, IpcError> {
    Ok(svc
        .snapshot_store
        .list_ops(stream_id, limit.unwrap_or(200))
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

/// One captured file row by its `file_snapshot` id.
pub async fn get_file_snapshot(
    svc: &Services,
    file_snapshot_id: i64,
) -> Result<Option<FileSnapshot>, IpcError> {
    Ok(svc.snapshot_store.get(file_snapshot_id).await?)
}

/// Restore a captured file (`file_snapshot` id) into its stream's
/// worktree (`oxplow_app::snapshot_files`).
pub async fn restore_file_snapshot(svc: &Services, file_snapshot_id: i64) -> Result<(), IpcError> {
    use oxplow_app::snapshot_files::{restore_file_snapshot, SnapshotFileError as E};
    restore_file_snapshot(svc, file_snapshot_id)
        .await
        .map(|_| ())
        .map_err(|e| match e {
            E::NotFound => IpcError::not_found(),
            E::NoContent | E::Expired => IpcError::invalid(e.to_string()),
            E::Other(m) => IpcError::internal(m),
        })
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_file_snapshots_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_file_snapshots",
            serde_json::json!({ "path": "src/main.rs" }),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array(), "expected a JSON array, got {out}");
    }
}
