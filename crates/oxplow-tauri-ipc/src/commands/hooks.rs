use oxplow_app::HookEnvelope;
use oxplow_domain::{AgentStatus, AgentTurn, HookEvent, ThreadId};

use crate::error::IpcError;
use crate::state::AppState;

/// Land an envelope from the hook subprocess. Drives the agent_turn /
/// agent_status state machine inside HookIngestService.
#[tauri::command]
#[specta::specta]
pub async fn ingest_hook_event(
    state: tauri::State<'_, AppState>,
    envelope: HookEnvelope,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::hooks::ingest_hook_event(&state, envelope).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_hook_events(
    state: tauri::State<'_, AppState>,
    thread_id: Option<ThreadId>,
    limit: Option<usize>,
) -> Result<Vec<HookEvent>, IpcError> {
    oxplow_rpc::commands::hooks::list_hook_events(&state, thread_id, limit).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_agent_statuses(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AgentStatus>, IpcError> {
    oxplow_rpc::commands::hooks::list_agent_statuses(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_open_agent_turns(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<Vec<AgentTurn>, IpcError> {
    oxplow_rpc::commands::hooks::list_open_agent_turns(&state, thread_id).await
}
