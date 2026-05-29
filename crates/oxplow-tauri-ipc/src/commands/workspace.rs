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
    match version {
        TreeVersion::Disk => match state
            .git
            .read_workspace_file(stream_id.as_deref(), relative_path)
            .await
        {
            Ok(file) => Ok(Some(file.content)),
            Err(e) => {
                // The git facade returns NotFound as an error; surface
                // that as Ok(None) so the IPC contract matches the
                // ref-reader's None semantics.
                if e.to_string().to_lowercase().contains("not found") {
                    Ok(None)
                } else {
                    Err(IpcError::internal(e.to_string()))
                }
            }
        },
        TreeVersion::Ref { r#ref } => Ok(state.git.read_file_at_ref(r#ref, relative_path).await),
        TreeVersion::Snapshot { id } => {
            let snapshot_id: i64 = id
                .parse()
                .map_err(|_| IpcError::invalid(format!("invalid snapshot id: {id}")))?;
            let Some(hash) = state
                .snapshot_store
                .blob_hash_for_path(snapshot_id, &relative_path)
                .await
                .map_err(|e| IpcError::internal(e.to_string()))?
            else {
                return Ok(None);
            };
            let blobs = state.blobs.clone();
            let bytes = tokio::task::spawn_blocking(move || blobs.read(&hash))
                .await
                .map_err(|e| IpcError::internal(e.to_string()))?;
            match bytes {
                Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
                Err(_) => Ok(None),
            }
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_workspace_entries(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<Vec<WorkspaceEntry>, IpcError> {
    state
        .git
        .list_workspace_entries(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn list_workspace_files(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<Vec<WorkspaceIndexedFile>, IpcError> {
    state
        .git
        .list_workspace_files(stream_id.as_deref())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn read_workspace_file(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<WorkspaceFile, IpcError> {
    state
        .git
        .read_workspace_file(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn write_workspace_file(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
    content: String,
) -> Result<WorkspaceFile, IpcError> {
    state
        .git
        .write_workspace_file(stream_id.as_deref(), relative_path, content)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn create_workspace_file(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
    content: String,
) -> Result<WorkspaceFile, IpcError> {
    state
        .git
        .create_workspace_file(stream_id.as_deref(), relative_path, content)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn create_workspace_directory(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<String, IpcError> {
    state
        .git
        .create_workspace_directory(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn rename_workspace_path(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    from_path: String,
    to_path: String,
) -> Result<(String, String), IpcError> {
    state
        .git
        .rename_workspace_path(stream_id.as_deref(), from_path, to_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn delete_workspace_path(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<String, IpcError> {
    state
        .git
        .delete_workspace_path(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn get_workspace_status_summary(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<WorkspaceStatusSummary, IpcError> {
    Ok(state.git.status_summary(stream_id.as_deref()).await)
}

/// Re-export so the binding for GitFileStatus is generated.
pub fn _capture_git_file_status() -> GitFileStatus {
    GitFileStatus::Modified
}
