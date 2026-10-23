use oxplow_app::terminal_sessions::AttachResult;

use crate::error::IpcError;
use crate::state::{AppState, PluginRuntimeState};

/// Open a renderer-attached terminal session: the agent CLI or a shell,
/// run directly in a PTY, named by its URI (`oxplow_rpc::commands::terminal`).
#[tauri::command]
#[specta::specta]
pub async fn open_terminal_session(
    state: tauri::State<'_, AppState>,
    plugin_runtime: tauri::State<'_, PluginRuntimeState>,
    uri: String,
    cols: u16,
    rows: u16,
) -> Result<AttachResult, IpcError> {
    let ctx = oxplow_rpc::RpcContext {
        services: state.inner().clone(),
        plugin_runtime: Some(plugin_runtime.inner().as_ref().clone()),
    };
    oxplow_rpc::commands::terminal::open_terminal_session(&ctx, uri, cols, rows).await
}
