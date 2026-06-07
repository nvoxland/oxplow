//! Agent pane lifecycle commands.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::agent_command::{AgentCommandOptions, PaneKind};
use oxplow_app::agent_pane::EnsurePaneOutcome;
use oxplow_app::agent_prompt::assemble_system_prompt;
use oxplow_app::config_service::read_config;
use oxplow_domain::stores::{StreamStore, ThreadStore};
use oxplow_domain::AgentKind;
use oxplow_domain::{StreamId, ThreadId};

use crate::error::IpcError;
use crate::state::AppState;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum PaneKindArg {
    Working,
    Talking,
}

impl From<PaneKindArg> for PaneKind {
    fn from(p: PaneKindArg) -> Self {
        match p {
            PaneKindArg::Working => PaneKind::Working,
            PaneKindArg::Talking => PaneKind::Talking,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct EnsureAgentPaneRequest {
    pub stream_id: StreamId,
    pub pane: PaneKindArg,
    /// Optionally force a specific thread to drive the system prompt;
    /// otherwise the stream's currently-selected thread is used.
    pub thread_id: Option<ThreadId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct EnsureAgentPaneResponse {
    pub session: String,
    pub target: String,
    pub created: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn ensure_agent_pane(
    state: tauri::State<'_, AppState>,
    req: EnsureAgentPaneRequest,
) -> Result<EnsureAgentPaneResponse, IpcError> {
    use oxplow_db::SqliteStreamStore;
    let stream_store = SqliteStreamStore::new(state.db.clone());
    let stream = stream_store
        .get(&req.stream_id)
        .await?
        .ok_or_else(IpcError::not_found)?;

    // Resolve which thread's system prompt to use: caller override,
    // selected thread, or the stream's active thread.
    let thread_id = match req.thread_id {
        Some(t) => Some(t),
        None => state.threads.selected_or_active(&req.stream_id).await?,
    };
    let thread = match &thread_id {
        Some(id) => state.thread_store.get(id).await?,
        None => None,
    };

    let config = read_config(&state.config);
    let agent = thread
        .as_ref()
        .map(|t| t.agent)
        .unwrap_or_else(|| config.agents.first().copied().unwrap_or(AgentKind::Claude));
    let prompt =
        assemble_system_prompt(&state.layout.project_dir, &config, &stream, thread.as_ref());

    let opts = AgentCommandOptions {
        append_system_prompt: if prompt.is_empty() {
            None
        } else {
            Some(prompt)
        },
        ..Default::default()
    };

    let EnsurePaneOutcome {
        session,
        target,
        created,
    } = state
        .agent_panes
        .ensure_pane(&stream, req.pane.into(), agent, opts)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(EnsureAgentPaneResponse {
        session: session.0,
        target: target.as_str().to_string(),
        created,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn teardown_agent_panes(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<(), IpcError> {
    use oxplow_db::SqliteStreamStore;
    let stream_store = SqliteStreamStore::new(state.db.clone());
    let stream = stream_store
        .get(&stream_id)
        .await?
        .ok_or_else(IpcError::not_found)?;
    state.agent_panes.teardown_stream(&stream).await;
    Ok(())
}
