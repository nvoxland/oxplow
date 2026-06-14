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

/// Forward a terminal-input protocol message from the renderer to the
/// PTY backing `session_id`. Plumbing for **human input only** (xterm
/// keystrokes / paste / scroll / resize from `TerminalPane.tsx`); not an
/// agent-messaging or automation API. See the no-automation invariant in
/// `.context/agent-model.md`. Message shapes live in
/// `oxplow_app::terminal_sessions`.
#[tauri::command]
#[specta::specta]
pub async fn forward_terminal_input(
    state: tauri::State<'_, AppState>,
    session_id: String,
    message: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::terminal::forward_terminal_input(&state, session_id, message).await
}

/// Detach the renderer from `session_id` without killing the PTY —
/// the agent keeps running in the background so the user can navigate
/// away and come back. Use `terminate_terminal_session` to actually
/// stop the agent.
#[tauri::command]
#[specta::specta]
pub async fn close_terminal_session(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::terminal::close_terminal_session(&state, session_id).await
}

/// Best-effort live working directory of a session's child process, as an
/// absolute path. `None` when it can't be determined (tmux-backed pane, dead
/// session, unsupported platform). The renderer uses it to resolve relative
/// terminal file-path links against the shell's real cwd, falling back to the
/// worktree root.
#[tauri::command]
#[specta::specta]
pub async fn terminal_session_cwd(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Option<String>, IpcError> {
    oxplow_rpc::commands::terminal::terminal_session_cwd(&state, session_id).await
}

/// Permanently kill the PTY behind `session_id`. Used when a thread
/// is closed or the user explicitly terminates the agent.
#[tauri::command]
#[specta::specta]
pub async fn terminate_terminal_session(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::terminal::terminate_terminal_session(&state, session_id).await
}
