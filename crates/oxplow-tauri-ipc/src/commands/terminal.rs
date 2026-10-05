use oxplow_app::terminal_sessions::AttachResult;

use crate::error::IpcError;
use crate::state::{AppState, PluginRuntimeState};

/// Open a renderer-attached terminal session: the agent CLI or a shell,
/// run directly in a PTY (`oxplow_rpc::commands::terminal`).
#[tauri::command]
#[specta::specta]
pub async fn open_terminal_session(
    state: tauri::State<'_, AppState>,
    plugin_runtime: tauri::State<'_, PluginRuntimeState>,
    pane_target: String,
    cols: u16,
    rows: u16,
) -> Result<AttachResult, IpcError> {
    let ctx = oxplow_rpc::RpcContext {
        services: state.inner().clone(),
        plugin_runtime: Some(plugin_runtime.inner().as_ref().clone()),
    };
    oxplow_rpc::commands::terminal::open_terminal_session(&ctx, pane_target, cols, rows).await
}

/// Open (or reattach to) an ACP thread's agent session. Hand-written like
/// `open_terminal_session`: the MCP endpoint comes from the plugin runtime.
#[tauri::command]
#[specta::specta]
pub async fn acp_open_session(
    state: tauri::State<'_, AppState>,
    plugin_runtime: tauri::State<'_, PluginRuntimeState>,
    thread_id: oxplow_domain::ThreadId,
) -> Result<oxplow_app::acp::manager::AcpSnapshot, IpcError> {
    let ctx = oxplow_rpc::RpcContext {
        services: state.inner().clone(),
        plugin_runtime: Some(plugin_runtime.inner().as_ref().clone()),
    };
    oxplow_rpc::commands::acp::acp_open_session(&ctx, thread_id).await
}
