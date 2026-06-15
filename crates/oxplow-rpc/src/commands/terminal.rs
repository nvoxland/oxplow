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

/// Inline opencode config carried per-spawn in the
/// `OPENCODE_CONFIG_CONTENT` env var (opencode merges it on top of the
/// user's global/project config). Wires the oxplow MCP server (bearer
/// token interpolated from env by opencode itself), the hook-bridge
/// plugin, the per-thread system-prompt file as an instruction, and
/// the oxplow slash commands (inline `command` defs — opencode's
/// markdown-command dirs are fixed locations, but config commands ride
/// this env var with no disk footprint). Skills can't ride the config
/// (no key exists) — `write_opencode_runtime` materializes them into
/// `.opencode/skills/` instead.
fn opencode_config_content(
    mcp_endpoint_url: &str,
    hooks_plugin: &std::path::Path,
    instructions: &[String],
) -> String {
    serde_json::json!({
        "mcp": {
            "oxplow": {
                "type": "remote",
                "url": mcp_endpoint_url,
                "enabled": true,
                "headers": {
                    "Authorization": "Bearer {env:OXPLOW_HOOK_TOKEN}",
                },
            },
        },
        "plugin": [hooks_plugin.to_string_lossy()],
        "instructions": instructions,
        "command": oxplow_plugin::opencode_command_definitions(),
    })
    .to_string()
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

/// Build the dedup key for an *agent* PTY session.
///
/// Keyed on (stream, thread, agent, pane) ONLY — deliberately **not**
/// the transport mode. A re-attach that negotiated a different transport
/// (e.g. a second daemon/browser client) must resume the one live agent
/// PTY for this (stream, thread, pane), not spawn a duplicate agent in
/// the same worktree (tsk138). The shell path keeps transport in its key
/// (`shell_session_key`) because shell sessions may legitimately differ
/// by transport.
fn agent_session_key(
    stream_id: &str,
    thread_id: Option<&str>,
    agent: AgentKind,
    pane_target: &str,
) -> String {
    format!(
        "{}|{}|{}|{}",
        stream_id,
        thread_id.unwrap_or_default(),
        agent.as_str(),
        pane_target,
    )
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
    // when known so per-thread state is isolated. Transport mode is
    // intentionally excluded so a re-attach over a different transport
    // resumes the one live agent rather than spawning a duplicate
    // (tsk138).
    let thread_id_str = thread_id.as_ref().map(|t| t.to_string());
    let session_key = agent_session_key(
        &stream.id.to_string(),
        thread_id_str.as_deref(),
        agent,
        &pane_target,
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
        opencode_model: config.agent_models.get(&AgentKind::Opencode).cloned(),
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
        oxplow_plugin::AgentRuntimePaths::Opencode(paths) => {
            // opencode has no --append-system-prompt; the assembled
            // prompt lands in a per-thread instructions file referenced
            // from the inline config. Hooks + MCP ride the same config
            // via OPENCODE_CONFIG_CONTENT (merged last by opencode).
            let mut instructions = Vec::new();
            if let Some(prompt_text) = opts.append_system_prompt.take() {
                let file_name = format!(
                    "{}.md",
                    thread_id
                        .as_ref()
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "default".into())
                );
                let prompt_path = paths.prompts_dir.join(file_name);
                std::fs::write(&prompt_path, prompt_text)
                    .map_err(|e| IpcError::internal(format!("prompt write failed: {e}")))?;
                instructions.push(prompt_path.to_string_lossy().into_owned());
            }
            opts.env.push((
                "OPENCODE_CONFIG_CONTENT".to_string(),
                opencode_config_content(
                    &plugin_runtime.mcp_endpoint_url,
                    &paths.hooks_plugin,
                    &instructions,
                ),
            ));
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
                .attach_or_create_for_thread(
                    session_key,
                    target_label.clone(),
                    thread_id,
                    cols,
                    rows,
                    |c, r| SpawnRequest {
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
                    },
                )
                .await?
        }
        // Default to direct.
        _ => {
            // Resume from the THREAD's resume_session_id (populated by
            // the resume-tracker in the control plane), not the
            // stream's working_session_id. Each thread runs an
            // independent Claude session even though they share the
            // working pane slot.
            let mut resume_session_id = thread
                .as_ref()
                .map(|t| t.resume_session_id.clone())
                .unwrap_or_default();

            // Proactively drop a stale Claude resume pointer. If the
            // session transcript is gone, `claude --resume <id>` prints a
            // raw "No conversation found" error before the shell `||` net
            // falls back to fresh, and the dead id lingers in the DB until
            // the next prompt self-heals it (Claude Code drops HTTP hooks
            // for SessionStart, so nothing fires sooner). Clearing it here
            // launches fresh with no `--resume` and no raw error. Claude-
            // only: codex/opencode use different on-disk session schemes
            // and keep the shell net. See `.context/agent-model.md`.
            if matches!(agent, AgentKind::Claude) && !resume_session_id.is_empty() {
                if let Ok(home) = std::env::var("HOME") {
                    let state = oxplow_app::resume_check::claude_resume_state(
                        std::path::Path::new(&home),
                        &stream.worktree_path,
                        &resume_session_id,
                    );
                    if state == oxplow_app::resume_check::ResumeState::Missing {
                        if let Some(t) = thread.as_ref() {
                            let mut updated = t.clone();
                            updated.resume_session_id.clear();
                            if let Err(err) = ctx.thread_store.upsert(&updated).await {
                                tracing::warn!(
                                    ?err,
                                    "resume-check: clearing stale resume pointer failed"
                                );
                            }
                        }
                        resume_session_id.clear();
                    }
                }
            }

            let command = build_agent_command_for_session(
                agent,
                &stream.worktree_path,
                &resume_session_id,
                &opts,
            );
            let cwd = std::path::PathBuf::from(&stream.worktree_path);
            ctx.terminal_sessions
                .attach_or_create_for_thread(
                    session_key,
                    pane_target.clone(),
                    thread_id,
                    cols,
                    rows,
                    |c, r| SpawnRequest {
                        command: "sh".into(),
                        args: vec!["-lc".into(), command],
                        cwd,
                        env: vec![
                            ("TERM".into(), "xterm-256color".into()),
                            ("COLORTERM".into(), "truecolor".into()),
                        ],
                        cols: c,
                        rows: r,
                    },
                )
                .await?
        }
    };
    Ok(result)
}

/// Forward a terminal-input protocol message from the renderer to the
/// PTY backing `session_id`. This is **plumbing for human input only**:
/// the renderer's xterm pipes the user's own keystrokes / paste / scroll
/// / resize through here (see `TerminalPane.tsx`). It is NOT an
/// agent-messaging or automation API — nothing in oxplow may synthesize
/// `{type:"input"}` here to "type at" the agent. See the no-automation
/// invariant in `.context/agent-model.md`. Message shapes live in
/// `oxplow_app::terminal_sessions`.
pub async fn forward_terminal_input(
    svc: &Services,
    session_id: String,
    message: String,
) -> Result<(), IpcError> {
    svc.terminal_sessions.send(&session_id, &message).await?;
    Ok(())
}

/// Read-only lookup of the live agent session id for `thread_id`'s
/// pane, **without any spawn side effect**. Rebuilds the same
/// `(stream, thread, agent, pane)` key `open_terminal_session` uses for
/// an agent PTY, then reads the registry index — returning `None` when
/// no live session exists (the thread was never opened, or its PTY was
/// terminated; an unknown `thread_id` likewise reads as `None`).
///
/// This is the spawn-free path a second client / automation uses to
/// resolve a thread's agent PTY before `forward_terminal_input`
/// (delivering the human's keystrokes), instead of going through the
/// spawn-capable `open_terminal_session` (tsk139). `pane` defaults to
/// `"working"`; only the agent panes (`working` / `talking`) are valid.
pub async fn lookup_terminal_session(
    svc: &Services,
    thread_id: oxplow_domain::ThreadId,
    pane: Option<String>,
) -> Result<Option<String>, IpcError> {
    let pane_target = pane.unwrap_or_else(|| "working".to_string());
    // Agent panes only — shells aren't agent sessions and key
    // differently (transport is in the shell key, not the agent key).
    match pane_target.as_str() {
        "working" | "talking" => {}
        other => return Err(IpcError::invalid(format!("unknown pane target: {other}"))),
    }

    // Resolve the thread's stream + agent so the rebuilt key matches the
    // one the spawn path registered. A missing thread has no session.
    let thread = match svc.thread_store.get(&thread_id).await? {
        Some(t) => t,
        None => return Ok(None),
    };
    let key = agent_session_key(
        &thread.stream_id.to_string(),
        Some(&thread_id.to_string()),
        thread.agent,
        &pane_target,
    );
    Ok(svc.terminal_sessions.session_id_for_key(&key).await)
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

    use super::{
        agent_session_key, codex_hook_command, opencode_config_content, shell_session_key,
    };
    use crate::test_support::services;
    use oxplow_domain::AgentKind;

    #[test]
    fn agent_key_ignores_transport_mode() {
        // The agent key is (stream, thread, agent, pane) only — the same
        // tuple must produce the same key regardless of the transport a
        // client negotiated, so a re-attach resumes the one live PTY
        // instead of spawning a duplicate (tsk138).
        let key = agent_session_key("s-1", Some("thr3"), AgentKind::Claude, "working");
        assert_eq!(key, "s-1|thr3|claude|working");
        // No transport segment appears anywhere in the key.
        assert!(!key.contains("direct"));
        assert!(!key.contains("tmux"));
    }

    #[test]
    fn agent_key_distinguishes_thread_agent_and_pane() {
        let base = agent_session_key("s-1", Some("thr3"), AgentKind::Claude, "working");
        assert_ne!(
            base,
            agent_session_key("s-1", Some("thr4"), AgentKind::Claude, "working")
        );
        assert_ne!(
            base,
            agent_session_key("s-1", Some("thr3"), AgentKind::Codex, "working")
        );
        assert_ne!(
            base,
            agent_session_key("s-1", Some("thr3"), AgentKind::Claude, "talking")
        );
        // Missing thread id falls back to an empty segment.
        assert_eq!(
            agent_session_key("s-1", None, AgentKind::Claude, "working"),
            "s-1||claude|working"
        );
    }

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
    fn opencode_config_content_wires_mcp_plugin_and_instructions() {
        let content = opencode_config_content(
            "http://127.0.0.1:9/mcp",
            Path::new("/proj/.oxplow/runtime/opencode-plugin/plugin/oxplow-hooks.js"),
            &["/proj/.oxplow/runtime/opencode-plugin/prompts/thr1.md".to_string()],
        );
        let v: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
        assert_eq!(v["mcp"]["oxplow"]["type"], "remote");
        assert_eq!(v["mcp"]["oxplow"]["url"], "http://127.0.0.1:9/mcp");
        // opencode interpolates {env:VAR} itself — the literal token
        // must NOT be baked into the env var value.
        assert_eq!(
            v["mcp"]["oxplow"]["headers"]["Authorization"],
            "Bearer {env:OXPLOW_HOOK_TOKEN}"
        );
        assert_eq!(
            v["plugin"][0],
            "/proj/.oxplow/runtime/opencode-plugin/plugin/oxplow-hooks.js"
        );
        // Slash commands ride the inline `command` key — markdown
        // command dirs are fixed locations opencode controls, but
        // config commands have no disk footprint.
        assert!(
            v["command"]["oxplow-work-next"]["template"]
                .as_str()
                .map(|t| !t.is_empty())
                .unwrap_or(false),
            "oxplow-work-next command must be defined inline"
        );
        assert_eq!(
            v["instructions"][0],
            "/proj/.oxplow/runtime/opencode-plugin/prompts/thr1.md"
        );
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
    async fn agent_session_dedupes_across_transport_modes() {
        // Regression for tsk138: transport_mode must NOT be part of the
        // agent session key. Opening the same (stream, thread, pane)
        // twice with two different transports must reattach the ONE
        // existing PTY, not spawn a second agent. Both "direct" and
        // "pipe" land on the non-tmux spawn branch, so this exercises the
        // real dedup path without requiring tmux.
        let (mut ctx, _dir) = services();
        ctx.plugin_runtime = Some(crate::PluginRuntime {
            hook_base_url: "http://127.0.0.1:9/hook".into(),
            mcp_endpoint_url: "http://127.0.0.1:9/mcp".into(),
            hook_token: "test-token".into(),
        });

        let first = crate::dispatch(
            "open_terminal_session",
            json!({ "paneTarget": "working", "cols": 80, "rows": 24, "transportMode": "direct" }),
            &ctx,
        )
        .await
        .unwrap();
        let second = crate::dispatch(
            "open_terminal_session",
            json!({ "paneTarget": "working", "cols": 80, "rows": 24, "transportMode": "pipe" }),
            &ctx,
        )
        .await
        .unwrap();

        let first_id = first["sessionId"].as_str().expect("first sessionId");
        let second_id = second["sessionId"].as_str().expect("second sessionId");
        assert_eq!(
            first_id, second_id,
            "different transports must reattach the same agent PTY, not spawn a duplicate"
        );

        // Clean up the spawned PTY so the test doesn't leak a child.
        let _ = ctx.terminal_sessions.close(first_id).await;
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

    #[test]
    fn terminal_sessions_send_has_single_production_caller() {
        // No-automation guard (see .context/agent-model.md → "No
        // synthesized agent terminal input"). The terminal-input registry
        // method `terminal_sessions.send(` is the path that turns a
        // protocol message into a PTY write — i.e. the only way to put
        // bytes in front of the agent. It may have EXACTLY ONE production
        // caller: `forward_terminal_input` in this file, which carries
        // the human's own keystrokes/paste. Any other crate-source caller
        // would be a way for oxplow to synthesize agent input and must
        // fail the build. (Test/`reg.send(` call sites in
        // oxplow-app use a different receiver and don't match.)
        let crates_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates dir");
        let allowed = "oxplow-rpc/src/commands/terminal.rs";
        let needle = "terminal_sessions.send(";
        let mut offenders: Vec<String> = Vec::new();
        let mut stack = vec![crates_dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read crates dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                        continue;
                    }
                    stack.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    let text = std::fs::read_to_string(&path).expect("read rs file");
                    if text.contains(needle) {
                        let rel = path
                            .strip_prefix(crates_dir)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .replace('\\', "/");
                        if rel != allowed {
                            offenders.push(rel);
                        }
                    }
                }
            }
        }
        offenders.sort();
        assert!(
            offenders.is_empty(),
            "unexpected callers of the terminal-input registry method: {offenders:?} \
             — agent terminal input must flow only through forward_terminal_input"
        );
    }

    #[tokio::test]
    async fn lookup_terminal_session_returns_none_without_spawning() {
        // Read-only: a thread with no open agent PTY resolves to null,
        // and the lookup never spawns (it runs on the svc path with no
        // plugin runtime — an agent spawn would be impossible anyway).
        let (ctx, _dir) = services();
        let stream = ctx.streams.ensure_primary().await.unwrap();
        let thread_id = ctx
            .threads
            .selected_or_active(&stream.id)
            .await
            .unwrap()
            .expect("primary stream has a writer thread");
        let out = crate::dispatch(
            "lookup_terminal_session",
            json!({ "threadId": thread_id.to_string(), "pane": "working" }),
            &ctx,
        )
        .await
        .unwrap();
        assert!(out.is_null(), "expected null, got {out}");
    }

    #[tokio::test]
    async fn lookup_terminal_session_finds_live_agent_session() {
        // Open an agent PTY, then resolve its session id by thread id +
        // pane through the read-only lookup — the id must match.
        let (mut ctx, _dir) = services();
        ctx.plugin_runtime = Some(crate::PluginRuntime {
            hook_base_url: "http://127.0.0.1:9/hook".into(),
            mcp_endpoint_url: "http://127.0.0.1:9/mcp".into(),
            hook_token: "test-token".into(),
        });
        let opened = crate::dispatch(
            "open_terminal_session",
            json!({ "paneTarget": "working", "cols": 80, "rows": 24, "transportMode": "direct" }),
            &ctx,
        )
        .await
        .unwrap();
        let opened_id = opened["sessionId"].as_str().expect("opened sessionId");

        // The open path resolves the stream via current()-or-ensure_primary;
        // with no switch_stream it took the ensure_primary fallback.
        let stream = ctx.streams.ensure_primary().await.unwrap();
        let thread_id = ctx
            .threads
            .selected_or_active(&stream.id)
            .await
            .unwrap()
            .expect("a thread backs the opened session");

        // `pane` omitted → defaults to "working", the pane we opened.
        let out = crate::dispatch(
            "lookup_terminal_session",
            json!({ "threadId": thread_id.to_string() }),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(
            out.as_str(),
            Some(opened_id),
            "lookup must find the live agent PTY"
        );

        let _ = ctx.terminal_sessions.close(opened_id).await;
    }

    #[tokio::test]
    async fn lookup_terminal_session_rejects_non_agent_pane() {
        let (ctx, _dir) = services();
        let stream = ctx.streams.ensure_primary().await.unwrap();
        let thread_id = ctx
            .threads
            .selected_or_active(&stream.id)
            .await
            .unwrap()
            .expect("primary stream has a writer thread");
        let err = crate::dispatch(
            "lookup_terminal_session",
            json!({ "threadId": thread_id.to_string(), "pane": "shell" }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
        assert!(err.message.contains("pane"), "msg: {}", err.message);
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
