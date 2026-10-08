//! Cores for the `terminal` command module, including the agent-spawn
//! path: `open_terminal_session` takes the full [`RpcContext`] because
//! the agent runtime needs the control-plane coordinates
//! (`plugin_runtime`); both the Tauri shell and the daemon populate
//! them from their own control plane.

use oxplow_app::agent_prompt::assemble_system_prompt;
use oxplow_app::config_service::read_config;
use oxplow_app::terminal_sessions::{agent_pane_key, AttachResult, SpawnRequest};
use oxplow_app::Services;
use oxplow_domain::agent::harness::{
    AgentHarness, Endpoints, Launch, LaunchInput, LaunchSpec, SessionIds,
};
use oxplow_domain::agent_session::{AgentSession, SessionKind};
use oxplow_domain::stores::{AgentSessionStore, StreamStore, ThreadStore};
use oxplow_domain::{AgentSessionId, Stream, StreamId, Thread, ThreadId};

use crate::error::IpcError;
use crate::RpcContext;

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
                env_remove: oxplow_app::agent_path::not_inherited(&ctx.harnesses),
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
    let harness = ctx
        .harnesses
        .get(session.harness.as_str())
        .map_err(|e| IpcError::invalid(e.to_string()))?;
    let config = read_config(&ctx.config);
    let prompt = assemble_system_prompt(
        &ctx.layout.project_dir,
        harness.instruction_files(),
        &config,
        &stream,
        Some(&thread),
    );
    let harness_config = match config.agent_models.get(&session.harness) {
        Some(model) => serde_json::json!({ "model": model }),
        None => serde_json::json!({}),
    };
    let launch = launch_session(
        ctx,
        harness.as_ref(),
        &session,
        &thread,
        &stream,
        Some(&prompt),
        &harness_config,
    )
    .await?;
    let LaunchSpec::Pty { command } = launch.spec else {
        return Err(IpcError::invalid(format!(
            "agent session {session_id} runs no terminal"
        )));
    };
    let cwd = std::path::PathBuf::from(&stream.worktree_path);
    let env_remove = oxplow_app::agent_path::not_inherited(&ctx.harnesses);
    let result = ctx
        .terminal_sessions
        .attach_or_create_for_agent(
            agent_pane_key(session_id),
            Some(oxplow_app::terminal_sessions::AgentPane {
                thread: thread.id,
                session: Some(session.id),
            }),
            cols.max(20),
            rows.max(5),
            |c, r| SpawnRequest {
                command: "sh".into(),
                args: vec!["-lc".into(), command],
                cwd,
                env: oxplow_app::agent_path::base_pty_env(),
                env_remove,
                cols: c,
                rows: r,
            },
        )
        .await?;
    Ok(result)
}

/// Launch agent session `session` (its thread and stream given) through
/// its harness: identity, endpoints and what's offered now go in; how to
/// start its process comes out. A resume id the harness found stale is
/// forgotten here. Starting a session only spawns — nothing is typed.
pub(crate) async fn launch_session(
    ctx: &RpcContext,
    harness: &dyn AgentHarness,
    session: &AgentSession,
    thread: &Thread,
    stream: &Stream,
    system_prompt: Option<&str>,
    config: &serde_json::Value,
) -> Result<Launch, IpcError> {
    // Agent spawn needs the control-plane coordinates; a host that didn't
    // supply them can't wire hooks/MCP, so refuse cleanly.
    let rt = ctx.plugin_runtime.as_ref().ok_or_else(|| {
        IpcError::invalid("agent spawn unavailable: host supplied no plugin runtime")
    })?;
    let endpoints = Endpoints {
        hook_base_url: rt.hook_base_url.clone(),
        mcp_endpoint_url: rt.mcp_endpoint_url.clone(),
        otlp_base_url: rt.otlp_base_url.clone(),
        hook_token: rt.hook_token.clone(),
    };
    let identity = identity_env(rt, stream.id, thread.id, session.id);
    let text = oxplow_app::capabilities::agent_text(ctx);
    let executable = std::env::current_exe()
        .map_err(|e| IpcError::internal(format!("the oxplow binary: {e}")))?;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let resolve = |bin: &str| oxplow_app::agent_path::resolve_program(bin);
    let launch = harness
        .launch(&LaunchInput {
            session: SessionIds {
                stream: stream.id,
                thread: thread.id,
                session: session.id,
            },
            workspace: std::path::Path::new(&stream.worktree_path),
            project_dir: &ctx.layout.project_dir,
            endpoints: &endpoints,
            identity_env: &identity,
            system_prompt,
            resume: Some(session.resume_session_id.as_str()).filter(|r| !r.is_empty()),
            text: &text,
            config,
            oxplow_executable: &executable,
            home: home.as_deref(),
            resolve_program: &resolve,
        })
        .map_err(|e| IpcError::internal(e.to_string()))?;
    if launch.resume_dropped {
        if let Err(err) = oxplow_app::resume_check::forget_missing(
            &ctx.db,
            session.id,
            &session.resume_session_id,
        )
        .await
        {
            tracing::warn!(?err, "resume-check: clearing stale resume pointer failed");
        }
    }
    Ok(launch)
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
        .session_id_for_key(&agent_pane_key(session_id))
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

    use super::{identity_env, shell_session_key};
    use crate::error::IpcError;
    use crate::test_support::services;
    use oxplow_domain::AgentKind;

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

    /// An agent session opened, as a person does, on the primary stream's
    /// writer thread.
    async fn first_session(ctx: &crate::RpcContext) -> oxplow_domain::agent_session::AgentSession {
        let stream = ctx.streams.ensure_primary().await.unwrap();
        let thread = ctx
            .threads
            .selected_or_active(&stream.id)
            .await
            .unwrap()
            .expect("the primary stream has a writer thread");
        let out = crate::dispatch(
            "run_command",
            json!({ "id": "oxplow.agent_session.open", "input": {
                "thread": oxplow_domain::refs::build::thread_ref(thread),
            }, "confirmed": false }),
            ctx,
        )
        .await
        .unwrap();
        serde_json::from_value(out["result"].clone()).unwrap()
    }

    async fn open(ctx: &crate::RpcContext, pane: &str) -> Result<serde_json::Value, IpcError> {
        crate::dispatch(
            "open_terminal_session",
            json!({ "paneTarget": pane, "cols": 80, "rows": 24 }),
            ctx,
        )
        .await
    }

    /// A session whose harness nothing registers is refused, naming the
    /// ones that are.
    #[tokio::test]
    async fn an_unregistered_harness_is_refused_naming_the_registered() {
        let (mut ctx, _dir) = services();
        ctx.plugin_runtime = Some(runtime());
        let session = first_session(&ctx).await.id;
        ctx.harnesses.unregister("claude");
        let err = open(&ctx, &session.to_string()).await.unwrap_err();
        assert!(
            err.message.contains("claude") && err.message.contains("codex"),
            "msg: {}",
            err.message
        );
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
