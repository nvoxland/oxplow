//! Agent pane lifecycle commands.

use oxplow_domain::StreamId;

use crate::error::IpcError;
use crate::state::AppState;

pub use oxplow_rpc::commands::agent_panes::{
    EnsureAgentPaneRequest, EnsureAgentPaneResponse, PaneKindArg,
};

#[tauri::command]
#[specta::specta]
pub async fn ensure_agent_pane(
    state: tauri::State<'_, AppState>,
    req: EnsureAgentPaneRequest,
) -> Result<EnsureAgentPaneResponse, IpcError> {
    oxplow_rpc::commands::agent_panes::ensure_agent_pane(&state, req).await
}

#[tauri::command]
#[specta::specta]
pub async fn teardown_agent_panes(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::agent_panes::teardown_agent_panes(&state, stream_id).await
}
