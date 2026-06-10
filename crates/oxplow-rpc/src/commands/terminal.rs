//! Cores for the `terminal` command module.
//!
//! `open_terminal_session` is NOT here — it needs the Tauri-managed
//! `PluginRuntimeState` (control-plane URLs + hook token) and stays in
//! the shell.

use oxplow_app::Services;

use crate::error::IpcError;

/// Forward a JSON-encoded protocol message from the renderer to the
/// session backing `session_id`. See
/// `oxplow_app::terminal_sessions` for the message shapes.
pub async fn send_terminal_message(
    svc: &Services,
    session_id: String,
    message: String,
) -> Result<(), IpcError> {
    svc.terminal_sessions.send(&session_id, &message).await?;
    Ok(())
}

/// Detach the renderer from `session_id` without killing the PTY —
/// the agent keeps running in the background so the user can navigate
/// away and come back. Use `terminate_terminal_session` to actually
/// stop the agent.
pub async fn close_terminal_session(svc: &Services, session_id: String) -> Result<(), IpcError> {
    let _ = svc.terminal_sessions.detach(&session_id).await;
    Ok(())
}

/// Best-effort live working directory of a session's child process, as an
/// absolute path. `None` when it can't be determined (tmux-backed pane, dead
/// session, unsupported platform). The renderer uses it to resolve relative
/// terminal file-path links against the shell's real cwd, falling back to the
/// worktree root.
pub async fn terminal_session_cwd(
    svc: &Services,
    session_id: String,
) -> Result<Option<String>, IpcError> {
    Ok(svc
        .terminal_sessions
        .session_cwd(&session_id)
        .await
        .map(|p| p.to_string_lossy().into_owned()))
}

/// Permanently kill the PTY behind `session_id`. Used when a thread
/// is closed or the user explicitly terminates the agent.
pub async fn terminate_terminal_session(
    svc: &Services,
    session_id: String,
) -> Result<(), IpcError> {
    let _ = svc.terminal_sessions.close(&session_id).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::test_support::services;

    #[tokio::test]
    async fn terminal_session_cwd_returns_null_for_unknown_session() {
        let (svc, _dir) = services();
        let out = crate::dispatch("terminal_session_cwd", json!({ "sessionId": "nope" }), &svc)
            .await
            .unwrap();
        assert!(out.is_null());
    }

    #[tokio::test]
    async fn close_terminal_session_is_best_effort_on_unknown_session() {
        let (svc, _dir) = services();
        // Detach errors are swallowed — an unknown session still yields Ok.
        let out = crate::dispatch(
            "close_terminal_session",
            json!({ "sessionId": "nope" }),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_null());
    }
}
