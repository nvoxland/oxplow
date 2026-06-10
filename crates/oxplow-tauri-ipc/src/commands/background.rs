use oxplow_app::{BackgroundTask, BackgroundTaskKind};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_background_tasks(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<BackgroundTask>, IpcError> {
    oxplow_rpc::commands::background::list_background_tasks(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_background_task(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Option<BackgroundTask>, IpcError> {
    oxplow_rpc::commands::background::get_background_task(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn start_background_task(
    state: tauri::State<'_, AppState>,
    kind: BackgroundTaskKind,
    label: String,
    detail: Option<String>,
) -> Result<BackgroundTask, IpcError> {
    oxplow_rpc::commands::background::start_background_task(&state, kind, label, detail).await
}

#[tauri::command]
#[specta::specta]
pub async fn complete_background_task(
    state: tauri::State<'_, AppState>,
    id: String,
    result_json: Option<String>,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::background::complete_background_task(&state, id, result_json).await
}

#[tauri::command]
#[specta::specta]
pub async fn fail_background_task(
    state: tauri::State<'_, AppState>,
    id: String,
    error: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::background::fail_background_task(&state, id, error).await
}

#[tauri::command]
#[specta::specta]
pub async fn update_background_task(
    state: tauri::State<'_, AppState>,
    id: String,
    label: Option<String>,
    detail: Option<Option<String>>,
    progress: Option<Option<f64>>,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::background::update_background_task(&state, id, label, detail, progress)
        .await
}
