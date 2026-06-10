use oxplow_app::BacklogState;
use oxplow_domain::Task;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_backlog(state: tauri::State<'_, AppState>) -> Result<Vec<Task>, IpcError> {
    oxplow_rpc::commands::backlog::list_backlog(&state).await
}

/// Bucketed backlog view: ready/blocked/in_progress/done.
#[tauri::command]
#[specta::specta]
pub async fn get_backlog_state(
    state: tauri::State<'_, AppState>,
) -> Result<BacklogState, IpcError> {
    oxplow_rpc::commands::backlog::get_backlog_state(&state).await
}
