use oxplow_db::{FileSnapshot, Snapshot, SnapshotChangeEntry, SnapshotStats};
use oxplow_domain::StreamId;

use crate::error::IpcError;
use crate::state::AppState;

pub use oxplow_rpc::commands::snapshot::{
    DiffEndpoint, DiffEntry, SnapshotEntry, SnapshotFileRow, SnapshotPairDiff, SnapshotSummary,
    SnapshotSummaryCounts,
};

#[tauri::command]
#[specta::specta]
pub async fn list_snapshots(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<Vec<FileSnapshot>, IpcError> {
    oxplow_rpc::commands::snapshot::list_snapshots(&state, path).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_file_snapshots_for_stream(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
    limit: Option<usize>,
) -> Result<Vec<FileSnapshot>, IpcError> {
    oxplow_rpc::commands::snapshot::list_file_snapshots_for_stream(&state, stream_id, limit).await
}

/// `snapshot` rows for a stream — one entry per `request_snapshot()`
/// call that captured anything. Newest first.
#[tauri::command]
#[specta::specta]
pub async fn list_snapshots_for_stream(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
    limit: Option<usize>,
) -> Result<Vec<Snapshot>, IpcError> {
    oxplow_rpc::commands::snapshot::list_snapshots_for_stream(&state, stream_id, limit).await
}

/// Created/modified/deleted counts for a snapshot. Powers the Local
/// History dashboard's per-snapshot stats column.
#[tauri::command]
#[specta::specta]
pub async fn get_snapshot_stats(
    state: tauri::State<'_, AppState>,
    snapshot_id: i64,
) -> Result<SnapshotStats, IpcError> {
    oxplow_rpc::commands::snapshot::get_snapshot_stats(&state, snapshot_id).await
}

/// Per-file change entries for one snapshot, in the shape the
/// renderer's `useSnapshotChangeAnalysis` hook expects so it can
/// feed the same SummaryCard / ChangeAnalysisPanel components the
/// Git pages use.
#[tauri::command]
#[specta::specta]
pub async fn list_snapshot_change_entries(
    state: tauri::State<'_, AppState>,
    snapshot_id: i64,
) -> Result<Vec<SnapshotChangeEntry>, IpcError> {
    oxplow_rpc::commands::snapshot::list_snapshot_change_entries(&state, snapshot_id).await
}

/// Read a `file_snapshot` row's blob content as a UTF-8 string.
/// Returns `None` when:
/// - the row id doesn't exist,
/// - the row has no blob hash (deletion row or oversize-tracked),
/// - the blob has been pruned from disk.
///
/// Binary bytes pass through as UTF-8 lossy — the renderer's diff /
/// function-analysis pipeline treats the result as text either way.
#[tauri::command]
#[specta::specta]
pub async fn read_snapshot_file_content(
    state: tauri::State<'_, AppState>,
    file_snapshot_id: i64,
) -> Result<Option<String>, IpcError> {
    oxplow_rpc::commands::snapshot::read_snapshot_file_content(&state, file_snapshot_id).await
}

/// Total on-disk size of every blob in the content-addressed store.
/// Used by the Local History dashboard's Storage card.
#[tauri::command]
#[specta::specta]
pub async fn get_blob_storage_bytes(state: tauri::State<'_, AppState>) -> Result<i64, IpcError> {
    oxplow_rpc::commands::snapshot::get_blob_storage_bytes(&state).await
}

/// For each snapshot id in the input list, the wiki slugs whose
/// body changed in that snapshot. Drives the Local History
/// dashboard's wiki badges. Cheaper than fetching the full
/// `file_snapshot` rows per snapshot.
#[tauri::command]
#[specta::specta]
pub async fn list_wiki_slugs_for_snapshots(
    state: tauri::State<'_, AppState>,
    snapshot_ids: Vec<i64>,
) -> Result<Vec<(i64, String)>, IpcError> {
    oxplow_rpc::commands::snapshot::list_wiki_slugs_for_snapshots(&state, snapshot_ids).await
}

/// Every `file_snapshot` row captured under a single parent
/// snapshot id (i.e. one batch of `request_snapshot()`).
#[tauri::command]
#[specta::specta]
pub async fn list_files_for_snapshot(
    state: tauri::State<'_, AppState>,
    snapshot_id: i64,
) -> Result<Vec<FileSnapshot>, IpcError> {
    oxplow_rpc::commands::snapshot::list_files_for_snapshot(&state, snapshot_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_snapshot(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<Option<FileSnapshot>, IpcError> {
    oxplow_rpc::commands::snapshot::get_snapshot(&state, id).await
}

/// Compare two captures of the same path. The renderer surfaces this
/// in the snapshots panel as "what changed between then and now".
#[tauri::command]
#[specta::specta]
pub async fn get_snapshot_pair_diff(
    state: tauri::State<'_, AppState>,
    before_id: Option<i64>,
    after_id: Option<i64>,
) -> Result<SnapshotPairDiff, IpcError> {
    oxplow_rpc::commands::snapshot::get_snapshot_pair_diff(&state, before_id, after_id).await
}

/// Diff two endpoints, each a snapshot id or a git commit (the live
/// working tree is reserved for an in-progress effort's open end).
/// `start = null` diffs `end` against the empty tree. Powers the
/// effort / local-history diff view.
#[tauri::command]
#[specta::specta]
pub async fn diff_endpoints(
    state: tauri::State<'_, AppState>,
    start: Option<DiffEndpoint>,
    end: DiffEndpoint,
) -> Result<Vec<DiffEntry>, IpcError> {
    oxplow_rpc::commands::snapshot::diff_endpoints(&state, start, end).await
}

/// Read each `path`'s UTF-8 content as of `endpoint` (base + head for
/// the diff view's function-level analysis). `None` per path that's
/// absent / binary / oversize at that endpoint.
#[tauri::command]
#[specta::specta]
pub async fn read_endpoint_files_content(
    state: tauri::State<'_, AppState>,
    endpoint: DiffEndpoint,
    paths: Vec<String>,
) -> Result<Vec<Option<String>>, IpcError> {
    oxplow_rpc::commands::snapshot::read_endpoint_files_content(&state, endpoint, paths).await
}

/// Build a per-snapshot summary: the FileSnapshot row, the id of the
/// prior capture of the same path (if any), and a one-row diff
/// describing how the captured file relates to its predecessor
/// (created / updated / deleted). The renderer's local-history pane
/// keys off this shape.
#[tauri::command]
#[specta::specta]
pub async fn get_snapshot_summary(
    state: tauri::State<'_, AppState>,
    snapshot_id: i64,
) -> Result<Option<SnapshotSummary>, IpcError> {
    oxplow_rpc::commands::snapshot::get_snapshot_summary(&state, snapshot_id).await
}

/// Restore a file's contents from a snapshot. Reads the bytes from
/// the content-addressed blob store using the snapshot's `blob_hash`
/// and writes them back to the snapshot's path inside the workspace.
/// Errors with NOT_FOUND if the snapshot row is gone or its blob
/// was pruned.
#[tauri::command]
#[specta::specta]
pub async fn restore_file_from_snapshot(
    state: tauri::State<'_, AppState>,
    snapshot_id: i64,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::snapshot::restore_file_from_snapshot(&state, snapshot_id).await
}
