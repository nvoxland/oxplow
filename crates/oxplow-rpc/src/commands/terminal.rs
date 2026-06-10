//! Cores for the `terminal` command module, including the agent-spawn
//! path: `open_terminal_session` takes the full [`RpcContext`] because
//! the agent runtime needs the control-plane coordinates
//! (`plugin_runtime`); both the Tauri shell and the daemon populate
//! them from their own control plane.

use oxplow_app::agent_command::{build_agent_command_for_session, AgentCommandOptions, PaneKind};
use oxplow_app::agent_prompt::assemble_system_prompt;
use oxplow_app::config_service::read_config;
use oxplow_app::terminal_sessions::{AttachResult, SpawnRequest};
use oxplow_app::Services;
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::AgentKind;

use crate::error::IpcError;
use crate::RpcContext;

fn codex_config_overrides(
    paths: &oxplow_plugin::CodexRuntimePaths,
    mcp_endpoint_url: &str,
) -> Vec<String> {
    let mut out = vec![
        format!(
            "mcp_servers.oxplow.url={}",
            toml_cli_string(mcp_endpoint_url)
        ),
        "mcp_servers.oxplow.bearer_token_env_var=\"OXPLOW_HOOK_TOKEN\"".into(),
    ];
    for event in [
        "PreToolUse",
        "PermissionRequest",
        "PostToolUse",
        "UserPromptSubmit",
        "SessionStart",
        "Stop",
    ] {
        let command = codex_hook_command(&paths.oxplow_executable, event);
        let group = if matches!(event, "PreToolUse" | "PermissionRequest" | "PostToolUse") {
            format!(
                "hooks.{event}=[{{matcher=\"*\",hooks=[{{type=\"command\",command={},timeout=30,statusMessage=\"Syncing oxplow runtime\"}}]}}]",
                toml_cli_string(&command)
            )
        } else {
            format!(
                "hooks.{event}=[{{hooks=[{{type=\"command\",command={},timeout=30,statusMessage=\"Syncing oxplow runtime\"}}]}}]",
                toml_cli_string(&command)
            )
        };
        out.push(group);
    }
    out
}

fn codex_hook_command(oxplow_executable: &std::path::Path, event: &str) -> String {
    format!(
        "{} hook {}",
        shell_command_arg(&oxplow_executable.to_string_lossy()),
        shell_command_arg(event)
    )
}

fn shell_command_arg(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn toml_cli_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Build the PTY session key for a shell terminal, or `None` for a
/// non-shell `pane_target`.
///
/// `pane_target` is either the bare `"shell"` (the default Terminal-page
/// terminal — kept verbatim so it reattaches the existing persistent
/// shell after this upgrade) or `"shell:<id>"` for an additional
/// terminal. The full `pane_target` rides inside the key so each terminal
/// id resolves to its own PTY; the bare-shell case reproduces the legacy
/// `{stream}|shell|{mode}` key exactly.
fn shell_session_key(stream_id: &str, pane_target: &str, transport_mode: &str) -> Option<String> {
    if pane_target == "shell" || pane_target.starts_with("shell:") {
        Some(format!("{stream_id}|{pane_target}|{transport_mode}"))
    } else {
        None
    }
}

/// Open a renderer-attached terminal session.
///
/// Two transports, mirroring the main-branch design:
/// - `transport_mode == "direct"` — spawn the agent CLI directly via
///   `sh -lc <build_agent_command>` in a PTY; no tmux. The default.
/// - `transport_mode == "tmux"` — `ensure_pane` to create/reuse a
///   tmux session+window running the agent command, then
///   `tmux attach-session -t <resolved-target>`. The target is the
///   `oxplow-<stream-id>:working|talking` form, not the bare slot.
pub async fn open_terminal_session(
    ctx: &RpcContext,
    pane_target: String,
    cols: u16,
    rows: u16,
    transport_mode: String,
) -> Result<AttachResult, IpcError> {
    // The "shell" pane is a plain interactive terminal (the Terminal
    // page), not the agent: spawn the user's $SHELL rooted at the
    // worktree dir with no agent command, plugin, or system prompt.
    // `shell:<id>` is an additional terminal in the same page — each id
    // gets its own PTY. No plugin runtime needed on this path.
    if pane_target == "shell" || pane_target.starts_with("shell:") {
        let stream = match ctx.streams.current().await? {
            Some(s) => s,
            None => ctx.streams.ensure_primary().await?,
        };
        let cols = cols.max(20);
        let rows = rows.max(5);
        // One persistent shell per (stream, terminal id); re-attach resumes it.
        let session_key = shell_session_key(&stream.id.to_string(), &pane_target, &transport_mode)
            .expect("pane_target was verified to be a shell target above");
        let cwd = std::path::PathBuf::from(&stream.worktree_path);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let result = ctx
            .terminal_sessions
            .attach_or_create(session_key, "shell".to_string(), cols, rows, |c, r| {
                SpawnRequest {
                    command: shell,
                    args: vec!["-l".into()],
                    cwd,
                    env: vec![
                        ("TERM".into(), "xterm-256color".into()),
                        ("COLORTERM".into(), "truecolor".into()),
                    ],
                    cols: c,
                    rows: r,
                }
            })
            .await?;
        return Ok(result);
    }

    let pane_kind = match pane_target.as_str() {
        "working" => PaneKind::Working,
        "talking" => PaneKind::Talking,
        other => return Err(IpcError::invalid(format!("unknown pane target: {other}"))),
    };

    // Agent spawn needs the control-plane coordinates; a host that
    // didn't supply them can't wire hooks/MCP, so refuse cleanly.
    let plugin_runtime = ctx.plugin_runtime.as_ref().ok_or_else(|| {
        IpcError::invalid("agent spawn unavailable: host supplied no plugin runtime")
    })?;

    // Resolve the stream the user is currently driving. Falls back to
    // the primary so a brand-new project that hasn't called
    // switch_stream still gets a working pane.
    let stream = match ctx.streams.current().await? {
        Some(s) => s,
        None => ctx.streams.ensure_primary().await?,
    };

    // Pull the selected thread so the system prompt the agent sees
    // matches what the renderer is showing. Fall back to the stream's
    // active writer thread when no explicit selection has been made,
    // so the agent always knows its thread id (it shows up in the
    // `<session-context>` block + OXPLOW_THREAD_ID env).
    let thread_id = ctx.threads.selected_or_active(&stream.id).await?;
    let thread = match thread_id {
        Some(id) => ctx.thread_store.get(&id).await?,
        None => None,
    };

    let config = read_config(&ctx.config);
    let agent = thread
        .as_ref()
        .map(|t| t.agent)
        .unwrap_or_else(|| config.agents.first().copied().unwrap_or(AgentKind::Claude));
    let cols = cols.max(20);
    let rows = rows.max(5);

    // Identity used to deduplicate sessions so re-attaches resume the
    // same PTY instead of spawning a new one. Includes the thread id
    // when known so per-thread state is isolated.
    let session_key = format!(
        "{}|{}|{}|{}|{}",
        stream.id,
        thread_id
            .as_ref()
            .map(|t| t.to_string())
            .unwrap_or_default(),
        agent.as_str(),
        pane_target,
        transport_mode,
    );

    // Materialize the agent-specific runtime on every spawn. Claude
    // uses its plugin directory and MCP JSON; Codex uses command-hook
    // and MCP config overrides. Per-spawn identity rides env vars below.
    let agent_runtime = oxplow_plugin::write_agent_runtime(
        agent,
        &ctx.layout.project_dir,
        &plugin_runtime.hook_base_url,
        &plugin_runtime.mcp_endpoint_url,
        &plugin_runtime.hook_token,
    )
    .map_err(|e| IpcError::internal(format!("plugin write failed: {e}")))?;

    let plugin_env = vec![
        (
            "OXPLOW_HOOK_TOKEN".to_string(),
            plugin_runtime.hook_token.clone(),
        ),
        (
            "OXPLOW_HOOK_BASE_URL".to_string(),
            plugin_runtime.hook_base_url.clone(),
        ),
        ("OXPLOW_STREAM_ID".to_string(), stream.id.to_string()),
        (
            "OXPLOW_THREAD_ID".to_string(),
            thread_id
                .as_ref()
                .map(|t| t.to_string())
                .unwrap_or_default(),
        ),
        ("OXPLOW_PANE".to_string(), pane_target.clone()),
    ];

    let prompt = assemble_system_prompt(&ctx.layout.project_dir, &config, &stream, thread.as_ref());
    let mut opts = AgentCommandOptions {
        env: plugin_env.clone(),
        append_system_prompt: if prompt.is_empty() {
            None
        } else {
            Some(prompt)
        },
        ..Default::default()
    };
    match &agent_runtime {
        oxplow_plugin::AgentRuntimePaths::Claude(paths) => {
            opts.plugin_dir = Some(paths.plugin_dir.to_string_lossy().into_owned());
            opts.mcp_config = Some(paths.mcp_config.to_string_lossy().into_owned());
        }
        oxplow_plugin::AgentRuntimePaths::Codex(paths) => {
            opts.codex_config_overrides =
                codex_config_overrides(paths, &plugin_runtime.mcp_endpoint_url);
        }
    }

    let result = match transport_mode.as_str() {
        "tmux" => {
            let outcome = ctx
                .agent_panes
                .ensure_pane(&stream, pane_kind, agent, opts.clone())
                .await
                .map_err(|e| IpcError::internal(e.to_string()))?;
            let target_label = outcome.target.as_str().to_string();
            ctx.terminal_sessions
                .attach_or_create(session_key, target_label.clone(), cols, rows, |c, r| {
                    SpawnRequest {
                        command: "tmux".into(),
                        args: vec!["attach-session".into(), "-t".into(), target_label.clone()],
                        cwd: std::env::current_dir()
                            .unwrap_or_else(|_| std::path::PathBuf::from(".")),
                        env: vec![
                            ("TERM".into(), "xterm-256color".into()),
                            ("COLORTERM".into(), "truecolor".into()),
                        ],
                        cols: c,
                        rows: r,
                    }
                })
                .await?
        }
        // Default to direct.
        _ => {
            // Resume from the THREAD's resume_session_id (populated by
            // the resume-tracker in the control plane), not the
            // stream's working_session_id. Each thread runs an
            // independent Claude session even though they share the
            // working pane slot.
            let resume_session_id = thread
                .as_ref()
                .map(|t| t.resume_session_id.as_str())
                .unwrap_or("");
            let command = build_agent_command_for_session(
                agent,
                &stream.worktree_path,
                resume_session_id,
                &opts,
            );
            let cwd = std::path::PathBuf::from(&stream.worktree_path);
            ctx.terminal_sessions
                .attach_or_create(session_key, pane_target.clone(), cols, rows, |c, r| {
                    SpawnRequest {
                        command: "sh".into(),
                        args: vec!["-lc".into(), command],
                        cwd,
                        env: vec![
                            ("TERM".into(), "xterm-256color".into()),
                            ("COLORTERM".into(), "truecolor".into()),
                        ],
                        cols: c,
                        rows: r,
                    }
                })
                .await?
        }
    };
    Ok(result)
}

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
    use std::path::Path;

    use super::{codex_hook_command, shell_session_key};
    use crate::test_support::services;

    #[test]
    fn codex_hook_command_is_stable_and_url_independent() {
        let command =
            codex_hook_command(Path::new("/Applications/Oxplow App/oxplow"), "PreToolUse");
        assert_eq!(
            command,
            "'/Applications/Oxplow App/oxplow' hook 'PreToolUse'"
        );
        assert!(!command.contains("python"));
        assert!(!command.contains("http://"));
        assert!(!command.contains("https://"));
    }

    #[test]
    fn bare_shell_keeps_legacy_key() {
        // The default Terminal-page terminal must reattach the existing
        // persistent shell after the upgrade, so its key is unchanged
        // from the old `{stream}|shell|{mode}` form.
        assert_eq!(
            shell_session_key("s-1", "shell", "direct").as_deref(),
            Some("s-1|shell|direct"),
        );
    }

    #[test]
    fn additional_shell_gets_its_own_key() {
        assert_eq!(
            shell_session_key("s-1", "shell:t2", "direct").as_deref(),
            Some("s-1|shell:t2|direct"),
        );
        // Distinct from the default terminal's key.
        assert_ne!(
            shell_session_key("s-1", "shell:t2", "direct"),
            shell_session_key("s-1", "shell", "direct"),
        );
    }

    #[test]
    fn non_shell_target_is_none() {
        assert_eq!(shell_session_key("s-1", "working", "direct"), None);
        assert_eq!(shell_session_key("s-1", "talking", "tmux"), None);
    }

    #[tokio::test]
    async fn open_terminal_session_agent_path_requires_plugin_runtime() {
        // test_support builds a context with plugin_runtime: None — the
        // agent path must refuse cleanly instead of panicking, so a
        // mis-configured host degrades to plain terminals only.
        let (svc, _dir) = services();
        let err = crate::dispatch(
            "open_terminal_session",
            json!({ "paneTarget": "working", "cols": 80, "rows": 24, "transportMode": "direct" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
        assert!(
            err.message.contains("plugin runtime"),
            "msg: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn open_terminal_session_rejects_unknown_pane_target() {
        let (svc, _dir) = services();
        let err = crate::dispatch(
            "open_terminal_session",
            json!({ "paneTarget": "bogus", "cols": 80, "rows": 24, "transportMode": "direct" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
        assert!(
            err.message.contains("unknown pane target"),
            "msg: {}",
            err.message
        );
    }

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
