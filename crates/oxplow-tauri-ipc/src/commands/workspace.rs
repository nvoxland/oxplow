use oxplow_git::{
    GitFileStatus, WorkspaceEntry, WorkspaceFile, WorkspaceIndexedFile, WorkspaceStatusSummary,
};
use oxplow_tree_source::TreeVersion;

use crate::error::IpcError;
use crate::state::AppState;

/// Versioned file read. Dispatches on `version`:
/// - `Disk` → `read_workspace_file` (working tree, possibly dirty).
/// - `Ref { ref }` → `read_file_at_ref` (committed blob).
/// - `Snapshot { id }` → `snapshot_store.blob_hash_for_path` + blob read.
///
/// Returns `Ok(None)` if the path doesn't exist at that version.
/// Callers MUST pass an explicit version — there is no implicit
/// "current working tree" default. This is the chokepoint that makes
/// it impossible to forget which version you're reading, the way the
/// duplication-scan bug did against `readWorkspaceFile`.
#[tauri::command]
#[specta::specta]
pub async fn read_file(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
    version: TreeVersion,
) -> Result<Option<String>, IpcError> {
    oxplow_rpc::commands::workspace::read_file(&state, stream_id, relative_path, version).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_workspace_entries(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<Vec<WorkspaceEntry>, IpcError> {
    oxplow_rpc::commands::workspace::list_workspace_entries(&state, stream_id, relative_path).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_workspace_files(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<Vec<WorkspaceIndexedFile>, IpcError> {
    oxplow_rpc::commands::workspace::list_workspace_files(&state, stream_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn read_workspace_file(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<WorkspaceFile, IpcError> {
    oxplow_rpc::commands::workspace::read_workspace_file(&state, stream_id, relative_path).await
}

#[tauri::command]
#[specta::specta]
pub async fn write_workspace_file(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
    content: String,
) -> Result<WorkspaceFile, IpcError> {
    oxplow_rpc::commands::workspace::write_workspace_file(&state, stream_id, relative_path, content)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn create_workspace_file(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
    content: String,
) -> Result<WorkspaceFile, IpcError> {
    oxplow_rpc::commands::workspace::create_workspace_file(
        &state,
        stream_id,
        relative_path,
        content,
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn create_workspace_directory(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<String, IpcError> {
    oxplow_rpc::commands::workspace::create_workspace_directory(&state, stream_id, relative_path)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn rename_workspace_path(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    from_path: String,
    to_path: String,
) -> Result<(String, String), IpcError> {
    oxplow_rpc::commands::workspace::rename_workspace_path(&state, stream_id, from_path, to_path)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_workspace_path(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<String, IpcError> {
    oxplow_rpc::commands::workspace::delete_workspace_path(&state, stream_id, relative_path).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_workspace_status_summary(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<WorkspaceStatusSummary, IpcError> {
    oxplow_rpc::commands::workspace::get_workspace_status_summary(&state, stream_id).await
}

/// Re-export so the binding for GitFileStatus is generated.
pub fn _capture_git_file_status() -> GitFileStatus {
    GitFileStatus::Modified
}
