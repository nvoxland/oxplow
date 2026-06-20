use oxplow_app::Followup;
use oxplow_domain::ThreadId;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_followups(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<Vec<Followup>, IpcError> {
    oxplow_rpc::commands::followup::list_followups(&state, thread_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn add_followup(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
    body: String,
) -> Result<Followup, IpcError> {
    oxplow_rpc::commands::followup::add_followup(&state, thread_id, body).await
}

#[tauri::command]
#[specta::specta]
pub async fn remove_followup(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::followup::remove_followup(&state, id).await
}
