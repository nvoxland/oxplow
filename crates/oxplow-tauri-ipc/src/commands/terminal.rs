use oxplow_app::terminal_sessions::AttachResult;

use crate::error::IpcError;
use crate::state::{AppState, PluginRuntimeState};

/// Open a renderer-attached terminal session.
///
/// Two transports, mirroring the main-branch design:
/// - `transport_mode == "direct"` — spawn the agent CLI directly via
///   `sh -lc <build_agent_command>` in a PTY; no tmux. The default.
/// - `transport_mode == "tmux"` — `ensure_pane` to create/reuse a
///   tmux session+window running the agent command, then
///   `tmux attach-session -t <resolved-target>`. The target is the
///   `oxplow-<stream-id>:working|talking` form, not the bare slot.
#[tauri::command]
#[specta::specta]
pub async fn open_terminal_session(
    state: tauri::State<'_, AppState>,
    plugin_runtime: tauri::State<'_, PluginRuntimeState>,
    pane_target: String,
    cols: u16,
    rows: u16,
    transport_mode: String,
) -> Result<AttachResult, IpcError> {
    let ctx = oxplow_rpc::RpcContext {
        services: state.inner().clone(),
        plugin_runtime: Some(plugin_runtime.inner().as_ref().clone()),
    };
    oxplow_rpc::commands::terminal::open_terminal_session(
        &ctx,
        pane_target,
        cols,
        rows,
        transport_mode,
    )
    .await
}
