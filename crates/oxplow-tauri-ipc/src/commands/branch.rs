use oxplow_git::BranchRef;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_branches(state: tauri::State<'_, AppState>) -> Result<Vec<BranchRef>, IpcError> {
    oxplow_rpc::commands::branch::list_branches(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_default_branch(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, IpcError> {
    oxplow_rpc::commands::branch::get_default_branch(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn rename_branch(
    state: tauri::State<'_, AppState>,
    from: String,
    to: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::branch::rename_branch(&state, from, to).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_branch(
    state: tauri::State<'_, AppState>,
    branch: String,
    force: bool,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::branch::delete_branch(&state, branch, force).await
}

/// Filter helper for the UI that wants only locals or only remotes.
#[tauri::command]
#[specta::specta]
pub async fn list_local_branches(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<BranchRef>, IpcError> {
    oxplow_rpc::commands::branch::list_local_branches(&state).await
}
