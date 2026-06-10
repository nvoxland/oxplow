pub use oxplow_rpc::commands::config::WorkspaceContext;

use oxplow_config::{AgentKind, OxplowConfig};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_config(state: tauri::State<'_, AppState>) -> Result<OxplowConfig, IpcError> {
    oxplow_rpc::commands::config::get_config(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_agent_prompt_append(
    state: tauri::State<'_, AppState>,
    text: String,
) -> Result<OxplowConfig, IpcError> {
    oxplow_rpc::commands::config::set_agent_prompt_append(&state, text).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_agents(
    state: tauri::State<'_, AppState>,
    agents: Vec<AgentKind>,
) -> Result<OxplowConfig, IpcError> {
    oxplow_rpc::commands::config::set_agents(&state, agents).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_snapshot_retention_days(
    state: tauri::State<'_, AppState>,
    days: u32,
) -> Result<OxplowConfig, IpcError> {
    oxplow_rpc::commands::config::set_snapshot_retention_days(&state, days).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_snapshot_max_file_bytes(
    state: tauri::State<'_, AppState>,
    bytes: u64,
) -> Result<OxplowConfig, IpcError> {
    oxplow_rpc::commands::config::set_snapshot_max_file_bytes(&state, bytes).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_generated(
    state: tauri::State<'_, AppState>,
    entries: Vec<String>,
) -> Result<OxplowConfig, IpcError> {
    oxplow_rpc::commands::config::set_generated(&state, entries).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_workspace_context(
    state: tauri::State<'_, AppState>,
) -> Result<WorkspaceContext, IpcError> {
    oxplow_rpc::commands::config::get_workspace_context(&state).await
}
