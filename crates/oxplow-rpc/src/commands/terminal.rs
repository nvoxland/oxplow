//! Cores for the `terminal` command module, including the agent-spawn
//! path: `open_terminal_session` takes the full [`RpcContext`] because
//! the agent runtime needs the control-plane coordinates
//! (`plugin_runtime`); both the Tauri shell and the daemon populate
//! them from their own control plane.

use std::sync::Arc;

use oxplow_app::agent_prompt::assemble_system_prompt;
use oxplow_app::chat::codex::CodexOpen;
use oxplow_app::chat::host::ServicesChatHost;
use oxplow_app::chat::session::SessionSpec;
use oxplow_app::config_service::read_config;
use oxplow_app::terminal_sessions::{agent_pane_key, terminal_key, AttachResult, SpawnRequest};
use oxplow_app::Services;
use oxplow_domain::agent::harness::{
    AgentHarness, Endpoints, Launch, LaunchInput, LaunchSpec, SessionIds,
};
use oxplow_domain::agent_session::{AgentSession, SessionKind};
use oxplow_domain::stores::{AgentSessionStore, StreamStore, ThreadStore};
use oxplow_domain::{AgentSessionId, Stream, StreamId, Thread, ThreadId};

use crate::error::IpcError;
use crate::RpcContext;

/// Whether `session` runs in a terminal oxplow may start: it is open, and
/// a terminal one.
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

/// The error code `open_terminal_session` answers for a session the
/// person stopped: the pane shows Start Again instead of spawning.
pub const STOPPED: &str = "STOPPED";

/// A session the person stopped is not spawned again until they start it.
fn not_stopped(session: &AgentSession) -> Result<(), IpcError> {
    if session.stopped_by_user_at.is_some() {
        return Err(IpcError {
            code: STOPPED.into(),
            message: format!("agent session {} was stopped", session.id),
            cause: None,
            field: None,
            details: Vec::new(),
        });
    }
    Ok(())
}

/// What every process oxplow starts for agent session `session` is told
/// about itself: the hook endpoint, its session's bearer, and its stream,
/// thread and session. The bearer alone tells the control plane who is
/// calling; the ids are the agent's own knowledge of where it is.
/// `OXPLOW_BIN` is this process's own binary, which runs the extension SDK
/// (`$OXPLOW_BIN extension test <name>`) wherever the agent runs.
fn identity_env(
    endpoints: &Endpoints,
    stream: StreamId,
    thread: ThreadId,
    session: AgentSessionId,
) -> Vec<(String, String)> {
    vec![
        (
            "OXPLOW_HOOK_TOKEN".to_string(),
            endpoints.hook_token.clone(),
        ),
        (
            "OXPLOW_HOOK_BASE_URL".to_string(),
            endpoints.hook_base_url.clone(),
        ),
        ("OXPLOW_STREAM_ID".to_string(), stream.to_string()),
        ("OXPLOW_THREAD_ID".to_string(), thread.to_string()),
        ("OXPLOW_SESSION".to_string(), session.to_string()),
        (
            "OXPLOW_BIN".to_string(),
            std::env::current_exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "oxplow".to_string()),
        ),
    ]
}

/// Where agent session `session` reaches oxplow, with the session's bearer
/// (`SessionAuth::issue`: the one it holds, so a launch that attaches to
/// the running process leaves that process's bearer working).
pub(crate) fn session_endpoints(
    ctx: &RpcContext,
    session: &AgentSession,
    thread: &Thread,
) -> Result<Endpoints, IpcError> {
    // Agent spawn needs the control-plane coordinates; a host that didn't
    // supply them can't wire hooks/MCP, so refuse cleanly.
    let rt = ctx.plugin_runtime.as_ref().ok_or_else(|| {
        IpcError::invalid("agent spawn unavailable: host supplied no plugin runtime")
    })?;
    let hook_token = ctx.session_auth.issue(oxplow_app::session_auth::Principal {
        session: session.id,
        thread: thread.id,
        stream: thread.stream_id,
        harness: session.harness.clone(),
    });
    Ok(Endpoints {
        hook_base_url: rt.hook_base_url.clone(),
        mcp_endpoint_url: rt.mcp_endpoint_url.clone(),
        otlp_base_url: rt.otlp_base_url.clone(),
        hook_token,
    })
}

/// What a URI names a PTY for: a terminal (a shell in its stream's
/// worktree) or an agent session (its agent CLI). The URI is the key the
/// registry keeps the process under; nothing else names one.
enum Target {
    Terminal(oxplow_domain::TerminalId),
    AgentSession(AgentSessionId),
}

fn target_of(uri: &str) -> Result<Target, IpcError> {
    use oxplow_domain::uri::{build::parse_as, resources};
    if let Ok(id) = parse_as(uri, resources::TERMINAL) {
        return Ok(Target::Terminal(id));
    }
    if let Ok(id) = parse_as(uri, resources::AGENT_SESSION) {
        return Ok(Target::AgentSession(id));
    }
    Err(IpcError::invalid(format!(
        "`{uri}` names neither a terminal nor an agent session"
    )))
}

/// Open a renderer-attached terminal session for `uri`: a terminal's shell
/// or an agent session's agent CLI, run directly in a PTY. There is no
/// terminal multiplexer (tsk1018): the process lives as long as the daemon,
/// and a re-attach replays its buffer. The URI is the key — opening it
/// again, from another window or a browser client, resumes the one process.
pub async fn open_terminal_session(
    ctx: &RpcContext,
    uri: String,
    cols: u16,
    rows: u16,
) -> Result<AttachResult, IpcError> {
    let session_id = match target_of(&uri)? {
        // A plain interactive shell: the user's $SHELL rooted at the
        // terminal's stream's worktree, with no agent command, plugin or
        // system prompt. No plugin runtime needed on this path.
        Target::Terminal(id) => {
            let row = ctx
                .terminal_store
                .get(id)
                .await?
                .ok_or_else(|| IpcError::invalid(format!("no terminal {id}")))?;
            let stream = ctx
                .stream_store
                .get(&row.stream_id)
                .await?
                .filter(|s| s.archived_at.is_none())
                .ok_or_else(|| IpcError::invalid(format!("terminal {id}'s stream is gone")))?;
            let cwd = std::path::PathBuf::from(&stream.worktree_path);
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
            return Ok(ctx
                .terminal_sessions
                .attach_or_create(terminal_key(id), cols.max(20), rows.max(5), |c, r| {
                    SpawnRequest {
                        command: shell,
                        args: vec!["-l".into()],
                        cwd,
                        env: oxplow_app::agent_path::base_pty_env(),
                        env_remove: oxplow_app::agent_path::not_inherited(&ctx.harnesses),
                        cols: c,
                        rows: r,
                    }
                })
                .await?);
        }
        // An agent session's pane. The session names its thread, the thread
        // its stream — what the person has selected doesn't matter.
        Target::AgentSession(id) => id,
    };

    let session = ctx
        .agent_session_store
        .get(&session_id)
        .await?
        .ok_or_else(|| IpcError::invalid(format!("no agent session {session_id}")))?;
    has_a_terminal(&session)?;
    not_stopped(&session)?;
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
    let declared = ctx
        .capabilities
        .get("agent_harness", &session.harness)
        .ok_or_else(|| {
            IpcError::invalid(format!(
                "no agent harness `{}` is declared",
                session.harness
            ))
        })?;
    // A harness without a prompt channel hears who else is in the worktree,
    // and how many handoffs wait for its thread, only here, as the launch
    // saw them.
    let around =
        oxplow_app::agent_context::surroundings(ctx, stream.id, thread.id, Some(session.id)).await;
    let prompt = assemble_system_prompt(
        &config,
        &stream,
        Some(&thread),
        &around,
        &declared,
        &oxplow_app::capabilities::agent_text(ctx),
    );
    let harness_config = config
        .agent_config
        .get(&session.harness)
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let endpoints = session_endpoints(ctx, &session, &thread)?;
    let launch = launch_session(
        ctx,
        harness.as_ref(),
        &endpoints,
        &session,
        &thread,
        &stream,
        Some(&prompt),
        &harness_config,
    )
    .await?;
    let cwd = std::path::PathBuf::from(&stream.worktree_path);
    let (program, args, launch_env) = match launch.spec {
        LaunchSpec::Pty { command, env } => ("sh".to_string(), vec!["-lc".into(), command], env),
        // Codex's TUI on the thread oxplow opens and observes first, so the
        // thread has oxplow's MCP server and system prompt before the TUI
        // attaches (one already observed is kept).
        LaunchSpec::CodexAppServer {
            socket,
            program,
            method,
            params,
            attach,
        } => {
            let thread_id = ctx
                .chat
                .open_codex(
                    Arc::new(ServicesChatHost::new(
                        &ctx.services,
                        Some(stream.id),
                        session.id,
                    )),
                    SessionSpec {
                        session_id,
                        thread_id: thread.id,
                        agent: harness.title().to_string(),
                        cwd: cwd.clone(),
                    },
                    CodexOpen {
                        socket,
                        program,
                        method,
                        params,
                    },
                )
                .await
                .map_err(|e| IpcError::invalid(e.to_string()))?;
            let (program, args) = tui_argv(attach, thread_id)?;
            (program, args, Vec::new())
        }
    };
    let env_remove = oxplow_app::agent_path::not_inherited(&ctx.harnesses);
    let result = ctx
        .terminal_sessions
        .attach_or_create_for_agent(
            agent_pane_key(session_id),
            Some(oxplow_app::terminal_sessions::AgentPane {
                thread: thread.id,
                session: Some(session.id),
                launch: oxplow_app::session_outcome::LaunchInfo {
                    resumed: !session.resume_session_id.is_empty(),
                    transcript_missing: launch.transcript_missing,
                },
            }),
            cols.max(20),
            rows.max(5),
            |c, r| SpawnRequest {
                command: program,
                args,
                cwd,
                env: oxplow_app::agent_path::base_pty_env()
                    .into_iter()
                    .chain(launch_env)
                    .collect(),
                env_remove,
                cols: c,
                rows: r,
            },
        )
        .await?;
    Ok(result)
}

/// The TUI's program and arguments: its argv with the thread it attaches
/// to appended.
fn tui_argv(attach: Vec<String>, thread: String) -> Result<(String, Vec<String>), IpcError> {
    let mut argv = attach.into_iter();
    let program = argv
        .next()
        .ok_or_else(|| IpcError::internal("the harness gave no program to attach with"))?;
    Ok((program, argv.chain([thread]).collect()))
}

/// Launch agent session `session` (its thread and stream given) through
/// its harness: identity, endpoints and what's offered now go in; how to
/// start its process comes out. A resume id the harness found stale is
/// forgotten here. Starting a session only spawns — nothing is typed.
#[allow(
    clippy::too_many_arguments,
    reason = "everything a launch is told, gathered by its two callers"
)]
pub(crate) async fn launch_session(
    ctx: &RpcContext,
    harness: &dyn AgentHarness,
    endpoints: &Endpoints,
    session: &AgentSession,
    thread: &Thread,
    stream: &Stream,
    system_prompt: Option<&str>,
    config: &serde_json::Value,
) -> Result<Launch, IpcError> {
    let identity = identity_env(endpoints, stream.id, thread.id, session.id);
    let text = oxplow_app::capabilities::agent_text(ctx);
    // The harness looks for its saved conversations under the home of the
    // machine the agent runs on: this one's only when the stream is local.
    let home = (stream.host == oxplow_domain::HostId::LOCAL)
        .then(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
        .flatten();
    let launch = harness
        .launch(&LaunchInput {
            session: SessionIds {
                stream: stream.id,
                thread: thread.id,
                session: session.id,
            },
            workspace: std::path::PathBuf::from(&stream.worktree_path),
            project_dir: ctx.layout.project_dir.clone(),
            endpoints: endpoints.clone(),
            identity_env: identity,
            system_prompt: system_prompt.map(str::to_string),
            resume: Some(session.resume_session_id.clone()).filter(|r| !r.is_empty()),
            fork: session.fork_next,
            text,
            config: config.clone(),
            home,
            search_path: oxplow_app::agent_path::search_path(),
        })
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(launch)
}

/// Queue a terminal-input protocol message for the PTY of `uri` (a
/// terminal or an agent session): the person's keystrokes, paste, scroll
/// and resize, in the order the window wrote them. The window writes them
/// on the daemon's `/events` socket, one frame each; this is called for
/// each frame before the next is read. Synchronous: it only queues, so the
/// frames' order is the PTY's, and keys typed before
/// the PTY is up wait for it in the registry
/// (`TerminalSessionRegistry::enqueue_input`). This is **plumbing for human
/// input only**: it is NOT an agent-messaging or automation API — nothing
/// in oxplow may synthesize `{type:"input"}` here to "type at" the agent.
/// See the no-automation invariant in `.context/agent-model.md`. Message
/// shapes live in `oxplow_app::terminal_sessions`.
pub fn terminal_input(svc: &Services, uri: String, message: String) -> Result<(), IpcError> {
    svc.terminal_sessions.enqueue_input(key_of(&uri)?, message);
    Ok(())
}

/// The registry key of `uri`'s PTY.
fn key_of(uri: &str) -> Result<String, IpcError> {
    Ok(match target_of(uri)? {
        Target::Terminal(id) => terminal_key(id),
        Target::AgentSession(id) => agent_pane_key(id),
    })
}

/// Read-only lookup of the live PTY of `uri` (a terminal or an agent session),
/// **without any spawn side effect**: the registry index under the key
/// `open_terminal_session` registers, `None` when no live PTY exists (never
/// opened, or terminated; an unknown session likewise).
///
/// This is the spawn-free path a second client uses to find a session's
/// agent PTY, instead of the spawn-capable `open_terminal_session`.
pub async fn lookup_terminal_session(
    svc: &Services,
    uri: String,
) -> Result<Option<String>, IpcError> {
    Ok(svc
        .terminal_sessions
        .session_id_for_key(&key_of(&uri)?)
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

    use super::{identity_env, session_endpoints};
    use crate::error::IpcError;
    use crate::test_support::services;
    use oxplow_domain::uri::build::agent_session_uri;

    #[test]
    fn the_identity_env_names_the_session() {
        let endpoints = oxplow_domain::agent::harness::Endpoints {
            hook_base_url: "http://127.0.0.1:9/hook".into(),
            mcp_endpoint_url: "http://127.0.0.1:9/mcp".into(),
            otlp_base_url: "http://127.0.0.1:9".into(),
            hook_token: "test-token".into(),
        };
        let env: std::collections::HashMap<String, String> = identity_env(
            &endpoints,
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
        // The binary that runs `extension test`: this process's own.
        assert_eq!(
            env["OXPLOW_BIN"],
            std::env::current_exe().unwrap().to_string_lossy()
        );
    }

    fn runtime() -> crate::PluginRuntime {
        crate::PluginRuntime {
            hook_base_url: "http://127.0.0.1:9/hook".into(),
            mcp_endpoint_url: "http://127.0.0.1:9/mcp".into(),
            otlp_base_url: "http://127.0.0.1:9".into(),
        }
    }

    /// A launch's endpoints carry its session's bearer: it authenticates
    /// as that session, thread and stream, and a second launch (the tab
    /// attaching again to the running process) hands back the same one, so
    /// the process already holding it keeps working.
    #[tokio::test]
    async fn a_launch_carries_its_sessions_one_bearer() {
        let (mut ctx, _dir) = services().await;
        ctx.plugin_runtime = Some(runtime());
        let session = first_session(&ctx).await;
        let thread =
            oxplow_domain::stores::ThreadStore::get(&*ctx.thread_store, &session.thread_id)
                .await
                .unwrap()
                .unwrap();
        let first = session_endpoints(&ctx, &session, &thread).unwrap();
        let principal = ctx.session_auth.authenticate(&first.hook_token).unwrap();
        assert_eq!(principal.session, session.id);
        assert_eq!(principal.thread, thread.id);
        assert_eq!(principal.stream, thread.stream_id);
        assert_eq!(principal.harness, session.harness);
        let second = session_endpoints(&ctx, &session, &thread).unwrap();
        assert_eq!(second.hook_token, first.hook_token);
        assert!(ctx.session_auth.authenticate(&first.hook_token).is_some());
    }

    /// An agent session opened, as a person does, on the primary stream's
    /// first thread.
    async fn first_session(ctx: &crate::RpcContext) -> oxplow_domain::agent_session::AgentSession {
        let stream = ctx.streams.ensure_primary().await.unwrap();
        let thread = ctx
            .threads
            .selected_or_first(&stream.id)
            .await
            .unwrap()
            .expect("the primary stream has a thread");
        let out = crate::dispatch(
            "run_command",
            json!({ "id": "oxplow.agent_session.open", "input": {
                "thread": thread.to_string(),
            }, "confirmed": false }),
            ctx,
        )
        .await
        .unwrap();
        serde_json::from_value(out["result"].clone()).unwrap()
    }

    /// Open the PTY of agent session `session` (`ses3`).
    async fn open(ctx: &crate::RpcContext, session: &str) -> Result<serde_json::Value, IpcError> {
        open_uri(ctx, &agent_session_uri(session.parse().unwrap())).await
    }

    async fn open_uri(ctx: &crate::RpcContext, uri: &str) -> Result<serde_json::Value, IpcError> {
        crate::dispatch(
            "open_terminal_session",
            json!({ "uri": uri, "cols": 80, "rows": 24 }),
            ctx,
        )
        .await
    }

    /// A session whose harness nothing registers is refused, naming the
    /// ones that are.
    #[tokio::test]
    async fn an_unregistered_harness_is_refused_naming_the_registered() {
        let (mut ctx, _dir) = services().await;
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

    /// A closed session has no terminal, and neither does an action.
    #[test]
    fn a_closed_or_action_session_has_no_terminal() {
        use oxplow_domain::agent_session::{AgentSession, SessionCloseReason, SessionKind};
        let now = oxplow_domain::Timestamp::from_unix_ms(1);
        let open = AgentSession {
            id: oxplow_domain::AgentSessionId::new(3),
            thread_id: oxplow_domain::ThreadId::new(1),
            kind: SessionKind::Terminal,
            harness: "claude".into(),
            title: String::new(),
            resume_session_id: String::new(),
            host: None,
            opened_at: now,
            closed_at: None,
            closed_reason: None,
            updated_at: now,
            pid: None,
            last_seen_at: None,
            stopped_by_user_at: None,
            native_title: String::new(),
            fork_next: false,
        };
        assert!(super::has_a_terminal(&open).is_ok());
        let action = AgentSession {
            kind: SessionKind::Action,
            ..open.clone()
        };
        let err = super::has_a_terminal(&action).unwrap_err();
        assert!(err.message.contains("action"), "msg: {}", err.message);
        let closed = AgentSession {
            closed_at: Some(now),
            closed_reason: Some(SessionCloseReason::Closed),
            ..open
        };
        let err = super::has_a_terminal(&closed).unwrap_err();
        assert!(err.message.contains("closed"), "msg: {}", err.message);
    }

    #[test]
    fn a_session_the_person_stopped_is_not_spawned() {
        use oxplow_domain::agent_session::{AgentSession, SessionKind};
        let now = oxplow_domain::Timestamp::from_unix_ms(1);
        let mut s = AgentSession {
            id: oxplow_domain::AgentSessionId::new(3),
            thread_id: oxplow_domain::ThreadId::new(1),
            kind: SessionKind::Terminal,
            harness: "claude".into(),
            title: String::new(),
            resume_session_id: String::new(),
            host: None,
            opened_at: now,
            closed_at: None,
            closed_reason: None,
            updated_at: now,
            pid: None,
            last_seen_at: None,
            stopped_by_user_at: None,
            native_title: String::new(),
            fork_next: false,
        };
        assert!(super::not_stopped(&s).is_ok());
        s.stopped_by_user_at = Some(now);
        assert_eq!(super::not_stopped(&s).unwrap_err().code, super::STOPPED);
    }

    /// A terminal's URI opens its shell in its stream's worktree, keyed by
    /// the URI: opening it again resumes the same PTY, another terminal is
    /// another PTY, and a closed terminal's URI is refused.
    #[tokio::test]
    async fn a_terminal_uri_opens_its_one_shell() {
        let (ctx, _dir) = services().await;
        let stream = ctx.streams.ensure_primary().await.unwrap();
        let open_terminal = |title: &'static str| {
            let ctx = &ctx;
            let stream = stream.id.to_string();
            async move {
                let out = crate::dispatch(
                    "run_command",
                    json!({ "id": "oxplow.terminal.open", "input": { "stream": stream, "title": title }, "confirmed": false }),
                    ctx,
                )
                .await
                .unwrap();
                out["result"]["uri"].as_str().unwrap().to_string()
            }
        };
        let (one, two) = (open_terminal("one").await, open_terminal("two").await);
        let a = open_uri(&ctx, &one).await.unwrap();
        let again = open_uri(&ctx, &one).await.unwrap();
        let b = open_uri(&ctx, &two).await.unwrap();
        assert_eq!(
            a["sessionId"], again["sessionId"],
            "the same URI is the same shell"
        );
        assert_ne!(
            a["sessionId"], b["sessionId"],
            "another terminal, another shell"
        );
        // A second client finds it without spawning.
        let found = crate::dispatch("lookup_terminal_session", json!({ "uri": one }), &ctx)
            .await
            .unwrap();
        assert_eq!(found, a["sessionId"]);
        for session in [&a, &b] {
            let _ = ctx
                .terminal_sessions
                .close(session["sessionId"].as_str().unwrap())
                .await;
        }
        // A terminal that was never opened is refused at its id.
        let missing = open_uri(&ctx, "oxplow:oxplow/terminal?id=trm9999")
            .await
            .unwrap_err();
        assert_eq!(missing.code, "INVALID");
        assert!(missing.message.contains("trm9999"), "{}", missing.message);
    }

    #[tokio::test]
    async fn open_terminal_session_agent_path_requires_plugin_runtime() {
        // test_support builds a context with plugin_runtime: None — the
        // agent path must refuse cleanly instead of panicking, so a
        // mis-configured host degrades to plain terminals only.
        let (svc, _dir) = services().await;
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
        let (mut ctx, _dir) = services().await;
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

    /// The TUI runs its argv with the thread it attaches to last.
    #[test]
    fn the_tui_attaches_with_the_thread_last() {
        let (program, args) = super::tui_argv(
            vec![
                "/bin/codex".into(),
                "resume".into(),
                "--cd".into(),
                "/w".into(),
            ],
            "thr-1".into(),
        )
        .unwrap();
        assert_eq!(program, "/bin/codex");
        assert_eq!(args, ["resume", "--cd", "/w", "thr-1"]);
        assert!(super::tui_argv(Vec::new(), "thr-1".into()).is_err());
    }

    /// A Codex session end to end, then a backend restart: opening it runs
    /// the harness's launch, opens its thread on the daemon (a new one,
    /// writable) and spawns the TUI attached to it; after the backend
    /// restarts over the same database, opening it again resumes the bound
    /// thread — it doesn't start another — and the PTY attaches again.
    #[expect(
        clippy::disallowed_methods,
        reason = "test setup: reopening the database file is how a restart is simulated"
    )]
    #[tokio::test]
    async fn a_codex_session_resumes_its_thread_after_a_backend_restart() {
        // A failure shows as a failure, not a hang.
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            use oxplow_harnesses::codex_app_server::fake::FakeAppServer;
            // Short paths: a unix socket's path has a length limit.
            let machine = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
            let codex_home = machine.path().join("cx");
            let bin = machine.path().join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            // A `codex` the TUI can be: it only has to exist and idle.
            let program = bin.join("codex");
            std::fs::write(&program, "#!/bin/sh\nexec sleep 600\n").unwrap();
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let path = format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            );
            // One process per test (nextest): the machine this one stands for.
            std::env::set_var("PATH", path);
            std::env::set_var("CODEX_HOME", &codex_home);
            std::env::set_var("HOME", machine.path());
            let daemon = FakeAppServer::start(&codex_home, |method, _| match method {
                "thread/start" | "thread/resume" => {
                    Ok(json!({"thread": {"id": "thr-restart", "name": null}}))
                }
                "thread/goal/set" | "thread/goal/clear" | "thread/settings/update" => Ok(json!({})),
                other => Err(format!("no {other}")),
            });

            let project = tempfile::tempdir().unwrap();
            let git = |args: &[&str]| {
                assert!(std::process::Command::new("git")
                    .args(args)
                    .current_dir(project.path())
                    .status()
                    .unwrap()
                    .success());
            };
            git(&["init", "-q"]);
            git(&["config", "user.name", "test"]);
            git(&["config", "user.email", "test@example.com"]);
            git(&["commit", "-q", "--allow-empty", "-m", "init"]);
            let db_file = machine.path().join("oxplow.sqlite");
            let boot = || async {
                let services = oxplow_app::Services::on_machine_with_database(
                    project.path(),
                    project.path().join(".oxplow/global-config"),
                    std::sync::Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
                    oxplow_app::providers::host::process_env(),
                    oxplow_db::Database::open(&db_file).unwrap(),
                )
                .await
                .unwrap();
                crate::RpcContext {
                    services,
                    plugin_runtime: Some(runtime()),
                }
            };

            let ctx = boot().await;
            let stream = ctx.streams.ensure_primary().await.unwrap();
            let thread = ctx
                .threads
                .selected_or_first(&stream.id)
                .await
                .unwrap()
                .expect("the primary stream has a thread");
            let out = crate::dispatch(
                "run_command",
                json!({ "id": "oxplow.agent_session.open", "input": {
                "thread": thread.to_string(),
                "harness": "codex",
            }, "confirmed": false }),
                &ctx,
            )
            .await
            .unwrap();
            let session: oxplow_domain::agent_session::AgentSession =
                serde_json::from_value(out["result"].clone()).unwrap();
            let methods = |daemon: &FakeAppServer| -> Vec<String> {
                daemon
                    .received()
                    .iter()
                    .filter_map(|v| v["method"].as_str().map(str::to_string))
                    .collect()
            };

            let first = open(&ctx, &session.id.to_string()).await.unwrap();
            let start = daemon
                .received()
                .into_iter()
                .find(|v| v["method"] == "thread/start")
                .expect("the thread was opened on the daemon");
            assert_eq!(start["params"]["sandbox"], "workspace-write");
            // The slot is bound to the thread the daemon gave it.
            let bound = || async {
                oxplow_domain::stores::AgentSessionStore::get(
                    &*ctx.agent_session_store,
                    &session.id,
                )
                .await
                .unwrap()
                .unwrap()
                .resume_session_id
            };
            for _ in 0..100 {
                if bound().await == "thr-restart" {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            assert_eq!(bound().await, "thr-restart");
            let _ = ctx
                .terminal_sessions
                .close(first["sessionId"].as_str().unwrap())
                .await;

            // The backend restarts: everything in memory is gone; the database,
            // the daemon and the program are not.
            drop(ctx);
            let ctx = boot().await;
            let again = open(&ctx, &session.id.to_string()).await.unwrap();
            assert!(again["sessionId"].is_string());
            let sent = methods(&daemon);
            assert_eq!(
                sent.iter().filter(|m| *m == "thread/start").count(),
                1,
                "the bound thread is resumed, never replaced: {sent:?}"
            );
            let resume = daemon
                .received()
                .into_iter()
                .find(|v| v["method"] == "thread/resume")
                .expect("the bound thread was resumed");
            assert_eq!(resume["params"]["threadId"], "thr-restart");
            assert_eq!(resume["params"]["sandbox"], "workspace-write");
            let _ = ctx
                .terminal_sessions
                .close(again["sessionId"].as_str().unwrap())
                .await;
        })
        .await
        .expect("the restart scenario finishes within a minute");
    }

    /// A URI of neither a terminal nor an agent session is refused, and so
    /// are the old pane-target strings.
    #[tokio::test]
    async fn open_terminal_session_refuses_what_is_no_terminal_or_session() {
        let (svc, _dir) = services().await;
        for uri in [
            "bogus",
            "shell",
            "ses3",
            "oxplow:oxplow/stream?id=str1",
            "oxplow:oxplow/file?path=src/a.rs",
        ] {
            let err = open_uri(&svc, uri).await.unwrap_err();
            assert_eq!(err.code, "INVALID", "{uri}");
            assert!(
                err.message
                    .contains("neither a terminal nor an agent session"),
                "{uri}: {}",
                err.message
            );
        }
    }

    #[tokio::test]
    async fn terminal_session_cwd_returns_null_for_unknown_session() {
        let (svc, _dir) = services().await;
        let out = crate::dispatch("terminal_session_cwd", json!({ "sessionId": "nope" }), &svc)
            .await
            .unwrap();
        assert!(out.is_null());
    }

    #[test]
    fn terminal_input_has_single_production_caller() {
        // No-automation guard (see .context/agent-model.md → "No
        // synthesized agent terminal input"). The registry's input queue
        // `terminal_sessions.enqueue_input(` is the only way to put bytes
        // in front of the agent (its drain task is the only caller of
        // `send`). It may have EXACTLY ONE production caller:
        // `terminal_input` in this file, which carries the human's own
        // keystrokes/paste. Any other crate-source caller would be a way
        // for oxplow to synthesize agent input and must fail the build.
        // (The registry's own tests call `reg.enqueue_input(` and don't
        // match.)
        let crates_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates dir");
        let allowed = "oxplow-rpc/src/commands/terminal.rs";
        let needle = "terminal_sessions.enqueue_input(";
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
             — agent terminal input must flow only through terminal_input"
        );
    }

    #[tokio::test]
    async fn terminal_input_refuses_a_uri_of_no_terminal() {
        let (ctx, _dir) = services().await;
        let err = super::terminal_input(
            &ctx.services,
            "oxplow:oxplow/thread?id=thr1".into(),
            r#"{"type":"input","bytes":"eA=="}"#.into(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("thread"), "{err}");
    }

    #[tokio::test]
    async fn lookup_terminal_session_returns_none_without_spawning() {
        // Read-only: a session with no live PTY resolves to null, and the
        // lookup never spawns (no plugin runtime: a spawn is impossible).
        let (ctx, _dir) = services().await;
        let session = first_session(&ctx).await.id;
        let out = crate::dispatch(
            "lookup_terminal_session",
            json!({ "uri": agent_session_uri(session) }),
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
        let (mut ctx, _dir) = services().await;
        ctx.plugin_runtime = Some(runtime());
        let session = first_session(&ctx).await.id.to_string();
        let opened = open(&ctx, &session).await.unwrap();
        let opened_id = opened["sessionId"].as_str().expect("opened sessionId");
        let out = crate::dispatch(
            "lookup_terminal_session",
            json!({ "uri": agent_session_uri(session.parse().unwrap()) }),
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
        let (svc, _dir) = services().await;
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
