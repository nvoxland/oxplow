//! Cores for the `terminal` command module, including the agent-spawn
//! path: `open_terminal_session` takes the full [`RpcContext`] because
//! the agent runtime needs the control-plane coordinates
//! (`plugin_runtime`); both the Tauri shell and the daemon populate
//! them from their own control plane.

use oxplow_app::agent_command::{build_agent_command_for_session, AgentCommandOptions};
use oxplow_app::agent_prompt::assemble_system_prompt;
use oxplow_app::config_service::read_config;
use oxplow_app::terminal_sessions::{AttachResult, SpawnRequest};
use oxplow_app::Services;
use oxplow_domain::agent_session::{AgentSession, SessionKind};
use oxplow_domain::stores::{AgentSessionStore, StreamStore, ThreadStore};
use oxplow_domain::{AgentKind, AgentSessionId, StreamId, ThreadId};

use crate::error::IpcError;
use crate::RpcContext;

/// The MCP endpoint with the calling thread's identity in the query
/// string, for a harness whose MCP config has no per-session headers
/// (Codex). The control plane reads `?thread=…&stream=…` like the
/// `X-Oxplow-*` headers.
fn mcp_url_with_identity(
    mcp_endpoint_url: &str,
    thread_id: Option<&str>,
    stream_id: &str,
) -> String {
    let sep = if mcp_endpoint_url.contains('?') {
        '&'
    } else {
        '?'
    };
    match thread_id {
        Some(t) => format!("{mcp_endpoint_url}{sep}thread={t}&stream={stream_id}"),
        None => format!("{mcp_endpoint_url}{sep}stream={stream_id}"),
    }
}

fn codex_config_overrides(
    paths: &oxplow_plugin::CodexRuntimePaths,
    mcp_endpoint_url: &str,
    thread_id: Option<&str>,
    stream_id: &str,
) -> Vec<String> {
    let mut out = vec![
        format!(
            "mcp_servers.oxplow.url={}",
            toml_cli_string(&mcp_url_with_identity(
                mcp_endpoint_url,
                thread_id,
                stream_id
            ))
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

/// OTEL env that points Claude Code's OTLP metrics exporter at oxplow's
/// control-plane receiver (epic tsk22). Only metrics are exported (no logs/
/// traces). The owning thread and agent session ride custom OTLP headers so
/// the receiver attributes token facts to the session's turn — one agent
/// process per session, so the headers are constant for its lifetime. Temporality is
/// left at the SDK default (delta): each export is the per-interval increment,
/// which maps straight onto the additive `oxplow.tokens` facts.
fn claude_otel_env(
    plugin_runtime: &crate::PluginRuntime,
    thread_id: &str,
    session_id: &str,
) -> Vec<(String, String)> {
    vec![
        ("CLAUDE_CODE_ENABLE_TELEMETRY".to_string(), "1".to_string()),
        ("OTEL_METRICS_EXPORTER".to_string(), "otlp".to_string()),
        (
            "OTEL_EXPORTER_OTLP_PROTOCOL".to_string(),
            "http/protobuf".to_string(),
        ),
        (
            // Base URL; the SDK appends the `/v1/metrics` signal path.
            "OTEL_EXPORTER_OTLP_ENDPOINT".to_string(),
            plugin_runtime.otlp_base_url.clone(),
        ),
        (
            "OTEL_EXPORTER_OTLP_HEADERS".to_string(),
            format!(
                "Authorization=Bearer {},X-Oxplow-Thread={},X-Oxplow-Session={}",
                plugin_runtime.hook_token, thread_id, session_id
            ),
        ),
        // 10s (default is 60s) — snappier token updates in the UI.
        (
            "OTEL_METRIC_EXPORT_INTERVAL".to_string(),
            "10000".to_string(),
        ),
    ]
}

/// Codex OTEL config (`--config otel.*`) pointing its OTLP metrics exporter at
/// oxplow's receiver (tsk24). Codex has NO OTEL env vars — config only. The
/// http exporter is protobuf ("binary"); the endpoint is the FULL signal URL
/// (`<base>/v1/metrics` — Codex uses it as-is, unlike Claude which appends the
/// signal path). Attribution + auth ride the `X-Oxplow-Thread`,
/// `X-Oxplow-Session` and bearer headers the receiver reads (the thread names
/// its stream). The tagged-union `otel.exporter.otlp-http.*` keys select
/// the http exporter (default is `none`).
fn codex_otel_overrides(
    otlp_base_url: &str,
    hook_token: &str,
    thread_id: &str,
    session_id: &str,
) -> Vec<String> {
    let endpoint = format!("{otlp_base_url}/v1/metrics");
    vec![
        format!(
            "otel.exporter.otlp-http.endpoint={}",
            toml_cli_string(&endpoint)
        ),
        "otel.exporter.otlp-http.protocol=\"binary\"".to_string(),
        format!(
            "otel.exporter.otlp-http.headers.authorization={}",
            toml_cli_string(&format!("Bearer {hook_token}"))
        ),
        format!(
            "otel.exporter.otlp-http.headers.x-oxplow-thread={}",
            toml_cli_string(thread_id)
        ),
        format!(
            "otel.exporter.otlp-http.headers.x-oxplow-session={}",
            toml_cli_string(session_id)
        ),
    ]
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
    text: &oxplow_plugin::AgentText,
) -> String {
    serde_json::json!({
        "mcp": {
            "oxplow": {
                "type": "remote",
                "url": mcp_endpoint_url,
                "enabled": true,
                // opencode substitutes `{env:…}` in headers, so the
                // per-spawn identity env vars ride along with the token.
                "headers": {
                    "Authorization": "Bearer {env:OXPLOW_HOOK_TOKEN}",
                    "X-Oxplow-Thread": "{env:OXPLOW_THREAD_ID}",
                    "X-Oxplow-Stream": "{env:OXPLOW_STREAM_ID}",
                },
            },
        },
        "plugin": [hooks_plugin.to_string_lossy()],
        "instructions": instructions,
        "command": oxplow_plugin::opencode_command_definitions(text),
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
/// terminal) or `"shell:<id>"` for an additional terminal. The full
/// `pane_target` rides inside the key (`{stream}|{pane_target}`), so each
/// terminal id resolves to its own PTY.
fn shell_session_key(stream_id: &str, pane_target: &str) -> Option<String> {
    if pane_target == "shell" || pane_target.starts_with("shell:") {
        Some(format!("{stream_id}|{pane_target}"))
    } else {
        None
    }
}

/// The dedup key for an agent session's PTY, so a re-attach — another
/// window, a browser client — resumes the one live agent rather than
/// spawning a duplicate in the same worktree.
fn agent_session_key(session: AgentSessionId) -> String {
    format!("session|{session}")
}

/// Whether `session` runs in a terminal oxplow may start: it is open, and
/// a terminal one (an ACP chat speaks the protocol on its stdio).
fn has_a_terminal(session: &AgentSession) -> Result<(), IpcError> {
    if !session.is_open() {
        return Err(IpcError::invalid(format!(
            "agent session {} is closed",
            session.id
        )));
    }
    if session.kind != SessionKind::Terminal {
        return Err(IpcError::invalid(format!(
            "agent session {} is a {}, which has no terminal",
            session.id,
            session.kind.as_str()
        )));
    }
    Ok(())
}

/// What every process oxplow starts for agent session `session` is told
/// about itself: the hook endpoint and token, and its stream, thread and
/// session, which its hooks and exports send back (`X-Oxplow-*`).
fn identity_env(
    plugin_runtime: &crate::PluginRuntime,
    stream: StreamId,
    thread: ThreadId,
    session: AgentSessionId,
) -> Vec<(String, String)> {
    vec![
        (
            "OXPLOW_HOOK_TOKEN".to_string(),
            plugin_runtime.hook_token.clone(),
        ),
        (
            "OXPLOW_HOOK_BASE_URL".to_string(),
            plugin_runtime.hook_base_url.clone(),
        ),
        ("OXPLOW_STREAM_ID".to_string(), stream.to_string()),
        ("OXPLOW_THREAD_ID".to_string(), thread.to_string()),
        ("OXPLOW_SESSION".to_string(), session.to_string()),
    ]
}

/// Open a renderer-attached terminal session: the agent CLI, or a shell,
/// run directly in a PTY (`sh -lc <command>`). There is no terminal
/// multiplexer (tsk1018): the session lives as long as the daemon, and a
/// re-attach replays its buffer.
pub async fn open_terminal_session(
    ctx: &RpcContext,
    pane_target: String,
    cols: u16,
    rows: u16,
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
        let session_key = shell_session_key(&stream.id.to_string(), &pane_target)
            .expect("pane_target was verified to be a shell target above");
        let cwd = std::path::PathBuf::from(&stream.worktree_path);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let result = ctx
            .terminal_sessions
            .attach_or_create(session_key, cols, rows, |c, r| SpawnRequest {
                command: shell,
                args: vec!["-l".into()],
                cwd,
                env: oxplow_app::agent_path::base_pty_env(),
                env_remove: oxplow_app::agent_path::not_inherited(),
                cols: c,
                rows: r,
            })
            .await?;
        return Ok(result);
    }

    // Anything else is an agent session's pane: its id (`ses3`). The
    // session names its thread, the thread its stream — what the person
    // has selected doesn't matter.
    let Some(session_id) = AgentSessionId::try_from_str(&pane_target) else {
        return Err(IpcError::invalid(format!(
            "unknown pane target: {pane_target}"
        )));
    };

    // Agent spawn needs the control-plane coordinates; a host that
    // didn't supply them can't wire hooks/MCP, so refuse cleanly.
    let plugin_runtime = ctx.plugin_runtime.as_ref().ok_or_else(|| {
        IpcError::invalid("agent spawn unavailable: host supplied no plugin runtime")
    })?;

    let session = ctx
        .agent_session_store
        .get(&session_id)
        .await?
        .ok_or_else(|| IpcError::invalid(format!("no agent session {session_id}")))?;
    has_a_terminal(&session)?;
    let thread = ctx
        .thread_store
        .get(&session.thread_id)
        .await?
        .ok_or_else(IpcError::not_found)?;
    let stream = ctx
        .stream_store
        .get(&thread.stream_id)
        .await?
        .ok_or_else(IpcError::not_found)?;
    let config = read_config(&ctx.config);
    let agent = session.harness;
    let cols = cols.max(20);
    let rows = rows.max(5);
    let session_key = agent_session_key(session_id);
    let thread_id_str = thread.id.to_string();

    // Materialize the agent-specific runtime on every spawn. Claude
    // uses its plugin directory and MCP JSON; Codex uses command-hook
    // and MCP config overrides. Per-spawn identity rides env vars below.
    // Its skills and commands are what's offered now
    // (`capabilities::agent_text`).
    let agent_text = oxplow_app::capabilities::agent_text(ctx);
    let agent_runtime = oxplow_plugin::write_agent_runtime(
        agent,
        &ctx.layout.project_dir,
        &plugin_runtime.hook_base_url,
        &plugin_runtime.mcp_endpoint_url,
        &plugin_runtime.hook_token,
        &agent_text,
    )
    .map_err(|e| IpcError::internal(format!("plugin write failed: {e}")))?;

    let session_id = session_id.to_string();
    let mut plugin_env = identity_env(plugin_runtime, stream.id, thread.id, session.id);
    // Claude Code exports token-usage metrics via OTEL to the control-plane
    // OTLP receiver (epic tsk22); the owning thread/stream ride custom OTLP
    // headers so the receiver attributes the facts without a session→thread
    // lookup. Codex is wired via `--config` (phase 2, tsk24); opencode is not
    // auto-instrumented (a user's own OTEL plugin still reaches the receiver).
    if agent == AgentKind::Claude {
        plugin_env.extend(claude_otel_env(plugin_runtime, &thread_id_str, &session_id));
    }

    let prompt = assemble_system_prompt(&ctx.layout.project_dir, &config, &stream, Some(&thread));
    let mut opts = AgentCommandOptions {
        env: plugin_env.clone(),
        append_system_prompt: if prompt.is_empty() {
            None
        } else {
            Some(prompt)
        },
        opencode_model: config.agent_models.get(&AgentKind::Opencode).cloned(),
        // Spawn by absolute path: a GUI-launched oxplow has macOS's minimal
        // PATH, and `sh -l` won't recover a zsh user's (tsk245).
        program: oxplow_app::agent_path::resolve_agent_program(agent),
        ..Default::default()
    };
    match &agent_runtime {
        oxplow_plugin::AgentRuntimePaths::Claude(paths) => {
            opts.plugin_dir = Some(paths.plugin_dir.to_string_lossy().into_owned());
            // Claude's MCP config can't read env vars, so the session's
            // identity is baked into a per-session file (like the token).
            let mcp_config = oxplow_plugin::write_claude_mcp_config(
                &paths.plugin_dir,
                &plugin_runtime.mcp_endpoint_url,
                &plugin_runtime.hook_token,
                oxplow_plugin::McpIdentity {
                    thread_id: &thread_id_str,
                    stream_id: &stream.id.to_string(),
                    session_id: Some(&session_id),
                },
            )
            .map_err(|e| IpcError::internal(format!("mcp config write failed: {e}")))?;
            opts.mcp_config = Some(mcp_config.to_string_lossy().into_owned());
        }
        oxplow_plugin::AgentRuntimePaths::Codex(paths) => {
            opts.codex_config_overrides = codex_config_overrides(
                paths,
                &plugin_runtime.mcp_endpoint_url,
                Some(&thread_id_str),
                &stream.id.to_string(),
            );
            // Codex exports token-usage metrics via OTEL to the same OTLP
            // receiver as Claude (tsk24); Codex has NO OTEL env vars, so this
            // rides `--config otel.*`. Attribution + auth via the same headers.
            opts.codex_config_overrides.extend(codex_otel_overrides(
                &plugin_runtime.otlp_base_url,
                &plugin_runtime.hook_token,
                &thread_id_str,
                &session_id,
            ));
        }
        oxplow_plugin::AgentRuntimePaths::Opencode(paths) => {
            // opencode has no --append-system-prompt; the assembled
            // prompt lands in a per-thread instructions file referenced
            // from the inline config. Hooks + MCP ride the same config
            // via OPENCODE_CONFIG_CONTENT (merged last by opencode).
            let mut instructions = Vec::new();
            if let Some(prompt_text) = opts.append_system_prompt.take() {
                // One per session: two sessions of a thread run apart.
                let file_name = format!("{session_id}.md");
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
                    &agent_text,
                ),
            ));
        }
    }

    let result = {
        // Resume from the agent session's resume_session_id (populated
        // by the resume-tracker in the hook ingest), not the stream's
        // working_session_id.
        let mut resume_session_id = session.resume_session_id.clone();

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
                    if let Err(err) = oxplow_app::resume_check::forget_missing(
                        &ctx.db,
                        session.id,
                        &resume_session_id,
                    )
                    .await
                    {
                        tracing::warn!(?err, "resume-check: clearing stale resume pointer failed");
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
            .attach_or_create_for_agent(
                session_key,
                Some(oxplow_app::terminal_sessions::AgentPane {
                    thread: thread.id,
                    session: Some(session.id),
                }),
                cols,
                rows,
                |c, r| SpawnRequest {
                    command: "sh".into(),
                    args: vec!["-lc".into(), command],
                    cwd,
                    env: oxplow_app::agent_path::base_pty_env(),
                    env_remove: oxplow_app::agent_path::not_inherited(),
                    cols: c,
                    rows: r,
                },
            )
            .await?
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

/// Read-only lookup of the live PTY of agent session `session_id`,
/// **without any spawn side effect**: the registry index under the key
/// `open_terminal_session` registers, `None` when no live PTY exists (never
/// opened, or terminated; an unknown session likewise).
///
/// This is the spawn-free path a second client uses to resolve a
/// session's agent PTY before `forward_terminal_input` (delivering the
/// human's keystrokes), instead of the spawn-capable
/// `open_terminal_session`.
pub async fn lookup_terminal_session(
    svc: &Services,
    session_id: AgentSessionId,
) -> Result<Option<String>, IpcError> {
    Ok(svc
        .terminal_sessions
        .session_id_for_key(&agent_session_key(session_id))
        .await)
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
/// absolute path. `None` when it can't be determined (dead session,
/// unsupported platform). The renderer uses it to resolve relative
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
        agent_session_key, claude_otel_env, codex_hook_command, codex_otel_overrides, identity_env,
        opencode_config_content, shell_session_key,
    };
    use crate::error::IpcError;
    use crate::test_support::services;
    use oxplow_domain::AgentKind;

    #[test]
    fn claude_otel_env_points_exporter_at_receiver_with_attribution_headers() {
        // tsk22: these exact env-var names/values are the contract with Claude
        // Code's OTEL exporter — a typo silently breaks token capture, so pin
        // them. The endpoint is the base (SDK appends /v1/metrics); the headers
        // carry the bearer + attribution spine the receiver reads.
        let pr = crate::PluginRuntime {
            hook_base_url: "http://127.0.0.1:9/hook".into(),
            mcp_endpoint_url: "http://127.0.0.1:9/mcp".into(),
            otlp_base_url: "http://127.0.0.1:9".into(),
            hook_token: "tok123".into(),
        };
        let env: std::collections::HashMap<String, String> =
            claude_otel_env(&pr, "thr1", "ses3").into_iter().collect();
        assert_eq!(env["CLAUDE_CODE_ENABLE_TELEMETRY"], "1");
        assert_eq!(env["OTEL_METRICS_EXPORTER"], "otlp");
        assert_eq!(env["OTEL_EXPORTER_OTLP_PROTOCOL"], "http/protobuf");
        assert_eq!(env["OTEL_EXPORTER_OTLP_ENDPOINT"], "http://127.0.0.1:9");
        assert_eq!(
            env["OTEL_EXPORTER_OTLP_HEADERS"],
            "Authorization=Bearer tok123,X-Oxplow-Thread=thr1,X-Oxplow-Session=ses3"
        );
        assert_eq!(env["OTEL_METRIC_EXPORT_INTERVAL"], "10000");
    }

    #[test]
    fn codex_otel_overrides_configure_the_http_exporter_with_headers() {
        // tsk24: Codex OTEL is config-only (`--config`). Pin the keys — the
        // endpoint is the full /v1/metrics signal URL, protobuf ("binary"),
        // and the bearer + attribution headers ride the exporter's headers map.
        let ov = codex_otel_overrides("http://127.0.0.1:9", "tok123", "thr1", "ses3");
        assert!(ov.contains(
            &"otel.exporter.otlp-http.endpoint=\"http://127.0.0.1:9/v1/metrics\"".to_string()
        ));
        assert!(ov.contains(&"otel.exporter.otlp-http.protocol=\"binary\"".to_string()));
        assert!(ov.contains(
            &"otel.exporter.otlp-http.headers.authorization=\"Bearer tok123\"".to_string()
        ));
        assert!(
            ov.contains(&"otel.exporter.otlp-http.headers.x-oxplow-thread=\"thr1\"".to_string())
        );
        assert!(
            ov.contains(&"otel.exporter.otlp-http.headers.x-oxplow-session=\"ses3\"".to_string())
        );
        assert!(!ov.iter().any(|o| o.contains("x-oxplow-stream")));
    }

    #[test]
    fn an_agent_pane_is_keyed_by_its_session() {
        assert_eq!(
            agent_session_key(oxplow_domain::AgentSessionId::new(3)),
            "session|ses3"
        );
    }

    #[test]
    fn the_identity_env_names_the_session() {
        let env: std::collections::HashMap<String, String> = identity_env(
            &runtime(),
            oxplow_domain::StreamId::new(1),
            oxplow_domain::ThreadId::new(2),
            oxplow_domain::AgentSessionId::new(3),
        )
        .into_iter()
        .collect();
        assert_eq!(env["OXPLOW_SESSION"], "ses3");
        assert_eq!(env["OXPLOW_THREAD_ID"], "thr2");
        assert_eq!(env["OXPLOW_STREAM_ID"], "str1");
        assert_eq!(env["OXPLOW_HOOK_TOKEN"], "test-token");
    }

    fn runtime() -> crate::PluginRuntime {
        crate::PluginRuntime {
            hook_base_url: "http://127.0.0.1:9/hook".into(),
            mcp_endpoint_url: "http://127.0.0.1:9/mcp".into(),
            otlp_base_url: "http://127.0.0.1:9".into(),
            hook_token: "test-token".into(),
        }
    }

    /// The primary stream's writer thread's agent session.
    async fn first_session(ctx: &crate::RpcContext) -> oxplow_domain::agent_session::AgentSession {
        let stream = ctx.streams.ensure_primary().await.unwrap();
        let thread = ctx
            .threads
            .selected_or_active(&stream.id)
            .await
            .unwrap()
            .expect("the primary stream has a writer thread");
        ctx.agent_session_store
            .newest_for_thread(thread)
            .await
            .unwrap()
            .expect("its thread has a session")
    }

    async fn open(ctx: &crate::RpcContext, pane: &str) -> Result<serde_json::Value, IpcError> {
        crate::dispatch(
            "open_terminal_session",
            json!({ "paneTarget": pane, "cols": 80, "rows": 24 }),
            ctx,
        )
        .await
    }

    /// A closed session has no terminal, and neither does a chat.
    #[test]
    fn a_closed_or_chat_session_has_no_terminal() {
        use oxplow_domain::agent_session::{AgentSession, SessionCloseReason, SessionKind};
        let now = oxplow_domain::Timestamp::from_unix_ms(1);
        let open = AgentSession {
            id: oxplow_domain::AgentSessionId::new(3),
            thread_id: oxplow_domain::ThreadId::new(1),
            kind: SessionKind::Terminal,
            harness: AgentKind::Claude,
            acp_agent: None,
            title: String::new(),
            resume_session_id: String::new(),
            host: None,
            opened_at: now,
            closed_at: None,
            closed_reason: None,
            updated_at: now,
        };
        assert!(super::has_a_terminal(&open).is_ok());
        let chat = AgentSession {
            kind: SessionKind::Chat,
            harness: AgentKind::Acp,
            ..open.clone()
        };
        let err = super::has_a_terminal(&chat).unwrap_err();
        assert!(err.message.contains("chat"), "msg: {}", err.message);
        let closed = AgentSession {
            closed_at: Some(now),
            closed_reason: Some(SessionCloseReason::Closed),
            ..open
        };
        let err = super::has_a_terminal(&closed).unwrap_err();
        assert!(err.message.contains("closed"), "msg: {}", err.message);
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
            &oxplow_plugin::AgentText::core(),
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
            v["command"]["oxplow-review-comments"]["template"]
                .as_str()
                .map(|t| !t.is_empty())
                .unwrap_or(false),
            "oxplow-review-comments command must be defined inline"
        );
        assert_eq!(
            v["instructions"][0],
            "/proj/.oxplow/runtime/opencode-plugin/prompts/thr1.md"
        );
    }

    #[test]
    fn each_shell_terminal_has_its_own_key() {
        assert_eq!(
            shell_session_key("s-1", "shell").as_deref(),
            Some("s-1|shell")
        );
        assert_eq!(
            shell_session_key("s-1", "shell:t2").as_deref(),
            Some("s-1|shell:t2"),
        );
    }

    #[test]
    fn non_shell_target_is_none() {
        assert_eq!(shell_session_key("s-1", "working"), None);
        assert_eq!(shell_session_key("s-1", "talking"), None);
    }

    #[tokio::test]
    async fn open_terminal_session_agent_path_requires_plugin_runtime() {
        // test_support builds a context with plugin_runtime: None — the
        // agent path must refuse cleanly instead of panicking, so a
        // mis-configured host degrades to plain terminals only.
        let (svc, _dir) = services();
        let session = first_session(&svc).await.id;
        let err = open(&svc, &session.to_string()).await.unwrap_err();
        assert_eq!(err.code, "INVALID");
        assert!(
            err.message.contains("plugin runtime"),
            "msg: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn an_agent_session_is_reattached_not_spawned_again() {
        // Opening the same agent session twice — another window, a
        // browser client — reattaches the ONE existing PTY, not a second
        // agent.
        let (mut ctx, _dir) = services();
        ctx.plugin_runtime = Some(runtime());
        let session = first_session(&ctx).await.id.to_string();
        let first = open(&ctx, &session).await.unwrap();
        let second = open(&ctx, &session).await.unwrap();

        let first_id = first["sessionId"].as_str().expect("first sessionId");
        let second_id = second["sessionId"].as_str().expect("second sessionId");
        assert_eq!(
            first_id, second_id,
            "a second open must reattach the same agent PTY, not spawn a duplicate"
        );

        // Clean up the spawned PTY so the test doesn't leak a child.
        let _ = ctx.terminal_sessions.close(first_id).await;
    }

    #[tokio::test]
    async fn open_terminal_session_rejects_unknown_pane_target() {
        let (svc, _dir) = services();
        let err = crate::dispatch(
            "open_terminal_session",
            json!({ "paneTarget": "bogus", "cols": 80, "rows": 24 }),
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
        // Read-only: a session with no live PTY resolves to null, and the
        // lookup never spawns (no plugin runtime: a spawn is impossible).
        let (ctx, _dir) = services();
        let session = first_session(&ctx).await.id;
        let out = crate::dispatch(
            "lookup_terminal_session",
            json!({ "sessionId": session.to_string() }),
            &ctx,
        )
        .await
        .unwrap();
        assert!(out.is_null(), "expected null, got {out}");
    }

    #[tokio::test]
    async fn lookup_terminal_session_finds_live_agent_session() {
        // Open a session's PTY, then resolve it through the read-only
        // lookup — the id must match.
        let (mut ctx, _dir) = services();
        ctx.plugin_runtime = Some(runtime());
        let session = first_session(&ctx).await.id.to_string();
        let opened = open(&ctx, &session).await.unwrap();
        let opened_id = opened["sessionId"].as_str().expect("opened sessionId");
        let out = crate::dispatch(
            "lookup_terminal_session",
            json!({ "sessionId": session }),
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
