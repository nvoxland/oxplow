//! Agent-session commands (`.context/commands.md`): the agent slots a
//! person opens on a thread — open, rename, close — as `Tx` commands.
//!
//! **The command is the slot's lifecycle; the RPC is the process's.**
//! Opening a session only inserts its row: the UI sees it and starts its
//! process (a PTY or an ACP agent), and nothing types into it — the
//! no-automation guards keep that true for a session an agent opened too.
//! Closing one closes its slot and, once that commits, stops its process
//! (`SessionProcesses`). Stopping a process alone (`terminate_terminal_session`,
//! `chat_close_session`) leaves the slot open.
//!
//! A session is named by ref (`agent_session:ses3`), a thread by
//! `thread:thr1`. An agent acts only on sessions in its own stream, and its
//! close — destructive — waits for a person as a proposal.

use crate::commands::ops::Op;
use std::sync::{Arc, RwLock};

use oxplow_config::OxplowConfig;
use oxplow_db::agent_session_store::{
    bind_tx, get_tx, insert_tx, reopen_tx, set_fork_next_tx, set_stopped_tx, set_title_tx,
    unbind_tx,
};
use oxplow_domain::agent::registry::HarnessRegistry;
use oxplow_domain::agent_session::BindingOrigin;
use oxplow_domain::agent_session::{
    AgentSession, NewAgentSession, SessionCloseReason, SessionKind,
};
use oxplow_domain::refs::build::{agent_session_ref, stream_ref, thread_ref};
use oxplow_domain::{
    AgentSessionId, CommandCall, CommandError, Confirm, Invokers, ThreadId, Timestamp,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::util::{invalid, parse, ref_id, schema, sql};
use super::{Handler, HandlerOutput, TxCtx};

pub const OPEN: &str = "oxplow.agent_session.open";
pub const RENAME: &str = "oxplow.agent_session.rename";
pub const CLOSE: &str = "oxplow.agent_session.close";
pub const REOPEN: &str = "oxplow.agent_session.reopen";
pub const STOP: &str = "oxplow.agent_session.stop";
pub const START: &str = "oxplow.agent_session.start";
pub const START_FRESH: &str = "oxplow.agent_session.start_fresh";
pub const RESUME_CONVERSATION: &str = "oxplow.agent_session.resume_conversation";
pub const FORK_CONVERSATION: &str = "oxplow.agent_session.fork_conversation";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenInput {
    /// The thread (`thread:thr1`).
    pub thread: String,
    /// `terminal` (the default); an `action` session isn't built yet.
    #[serde(default)]
    pub kind: Option<SessionKind>,
    /// What runs in it: a registered harness's key (default: the project's
    /// first enabled agent).
    #[serde(default)]
    pub harness: Option<String>,
    /// What to call it.
    #[serde(default)]
    pub title: Option<String>,
    /// A saved conversation of its harness (its native id) to start on,
    /// bound `chosen`. A person's choice: an agent may open a session but
    /// not pick the conversation it resumes.
    #[serde(default)]
    pub resume: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionInput {
    /// The session (`agent_session:ses3`).
    pub session: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    /// The session (`agent_session:ses3`).
    pub session: String,
    pub title: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConversationInput {
    /// The session (`agent_session:ses3`).
    pub session: String,
    /// One of its harness's saved conversations (its id).
    pub conversation: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForkInput {
    /// The session (`agent_session:ses3`).
    pub session: String,
    /// The saved conversation to fork (its id); default the one the
    /// session is on.
    #[serde(default)]
    pub conversation: Option<String>,
}

fn call(name: &str, input: serde_json::Value) -> Option<CommandCall> {
    Some(CommandCall {
        name: name.into(),
        input,
    })
}

fn result(session: &AgentSession) -> HandlerOutput {
    HandlerOutput {
        result: serde_json::to_value(session).expect("a session serializes"),
        ..HandlerOutput::default()
    }
}

/// The session an `agent_session:<id>` ref names.
fn load(ctx: &TxCtx<'_>, value: &str) -> Result<AgentSession, CommandError> {
    let id: AgentSessionId = ref_id(value, "agent_session", "/session")?;
    get_tx(ctx.conn, id)?.ok_or_else(|| invalid("/session", format!("no agent session `{value}`")))
}

/// An agent acts only on sessions of threads in its own stream.
fn in_own_stream(ctx: &TxCtx<'_>, thread: ThreadId) -> Result<(), CommandError> {
    let Some((_, own)) = super::thread::agent_scope(ctx)? else {
        return Ok(());
    };
    let stream = oxplow_db::thread_store::get_tx(ctx.conn, thread)
        .map_err(sql)?
        .map(|t| t.stream_id);
    if stream != Some(own) {
        return Err(CommandError::Denied {
            reason: format!(
                "an agent changes agent sessions only in its own stream (`{}`)",
                stream_ref(own)
            ),
        });
    }
    Ok(())
}

/// The harness and kind a session runs: what was named, checked against
/// the registered harnesses and the project's enabled ones, else the
/// project's default. Every harness runs in a terminal.
fn choose_agent(
    config: &OxplowConfig,
    harnesses: &HarnessRegistry,
    input: &OpenInput,
) -> Result<(String, SessionKind), CommandError> {
    let harness = match &input.harness {
        Some(key) => harnesses.get(key),
        None => harnesses.default(),
    }
    .map_err(|e| invalid("/harness", e.to_string()))?;
    let key = harness.id().to_string();
    if !config.agents.is_empty() && !config.agents.contains(&key) {
        return Err(invalid(
            "/harness",
            format!("agent `{key}` isn't enabled for this project"),
        ));
    }
    match input.kind.unwrap_or(SessionKind::Terminal) {
        SessionKind::Action => Err(invalid(
            "/kind",
            "an action session has no implementation yet",
        )),
        kind => Ok((key, kind)),
    }
}

/// `agent_session.open { thread, kind?, harness?, title?, resume? }`:
/// a new slot on the thread. It only inserts the row — it never starts a
/// process and never sends a prompt. Undone by closing it.
pub fn open_op(config: Arc<RwLock<OxplowConfig>>, harnesses: HarnessRegistry) -> Op {
    Op::new(
        "agent_sessions.write",
        "open",
        schema::<OpenInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: OpenInput = parse(input)?;
            let thread = ref_id(&input.thread, "thread", "/thread")?;
            let row = oxplow_db::thread_store::get_tx(ctx.conn, thread)
                .map_err(sql)?
                .filter(|t| t.archived_at.is_none())
                .ok_or_else(|| invalid("/thread", format!("no thread `{}`", thread_ref(thread))))?;
            if !row.is_open() {
                return Err(invalid(
                    "/thread",
                    format!("`{}` is closed; reopen it first", thread_ref(thread)),
                ));
            }
            in_own_stream(ctx, thread)?;
            let config = crate::config_service::read_config(&config);
            let (harness, kind) = choose_agent(&config, &harnesses, &input)?;
            if input.resume.is_some() && ctx.actor().is_agent_driven() {
                return Err(CommandError::Denied {
                    reason:
                        "an agent may open a session but not choose the conversation it resumes"
                            .into(),
                });
            }
            let now = Timestamp::now();
            let session = insert_tx(
                ctx.conn,
                &NewAgentSession {
                    thread_id: thread,
                    kind,
                    harness,
                    title: input.title.unwrap_or_default(),
                },
                now,
            )?;
            let session = match input.resume.as_deref() {
                None => session,
                Some(conversation) => {
                    free_conversation(ctx, &session, conversation).map_err(|e| match e {
                        CommandError::Invalid { message, .. } => invalid("/resume", message),
                        other => other,
                    })?;
                    bind_tx(
                        ctx.conn,
                        session.id,
                        conversation,
                        BindingOrigin::Chosen,
                        now,
                    )?;
                    get_tx(ctx.conn, session.id)?
                        .ok_or_else(|| invalid("/resume", "the session vanished"))?
                }
            };
            Ok(HandlerOutput {
                inverse: call(CLOSE, json!({ "session": agent_session_ref(session.id) })),
                ..result(&session)
            })
        })),
    )
}

/// `agent_session.rename { session, title }`; undone by renaming it back.
pub fn rename_op() -> Op {
    Op::new(
        "agent_sessions.write",
        "rename",
        schema::<RenameInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: RenameInput = parse(input)?;
            let session = load(ctx, &input.session)?;
            in_own_stream(ctx, session.thread_id)?;
            let before = set_title_tx(ctx.conn, session.id, &input.title, Timestamp::now())?
                .unwrap_or_default();
            let after = get_tx(ctx.conn, session.id)?
                .ok_or_else(|| invalid("/session", "the session vanished"))?;
            Ok(HandlerOutput {
                inverse: call(RENAME, json!({ "session": input.session, "title": before })),
                ..result(&after)
            })
        })),
    )
}

/// `agent_session.close { session }`: its slot closes, its open turns end
/// and it logs `stopped`, in one transaction; its process stops once that
/// commits. Undone by `reopen`: the conversation is saved, and the
/// reopened slot's process resumes it when its tab mounts — so it asks no
/// one first.
pub fn close_op(processes: crate::agent_sessions::SessionProcesses) -> Op {
    Op::new(
        "agent_sessions.write",
        "close",
        schema::<SessionInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: SessionInput = parse(input)?;
            let session = load(ctx, &input.session)?;
            in_own_stream(ctx, session.thread_id)?;
            if !session.is_open() {
                return Ok(result(&session));
            }
            oxplow_db::agent_stores::close_session_tx(
                ctx.conn,
                &ctx.events,
                session.id,
                SessionCloseReason::Closed,
                Timestamp::now(),
            )?;
            let after = get_tx(ctx.conn, session.id)?
                .ok_or_else(|| invalid("/session", "the session vanished"))?;
            let processes = processes.clone();
            let id = session.id;
            Ok(HandlerOutput {
                inverse: call(REOPEN, json!({ "session": input.session })),
                after_commit: Some(Box::new(move || processes.kill(id))),
                ..result(&after)
            })
        })),
    )
}

/// `agent_session.reopen { session }`: a slot a person closed opens again,
/// its tab with it; its process resumes the saved conversation when the
/// tab mounts. One its thread's or stream's close took is refused (reopen
/// that instead), as is one on a closed thread. Undone by `close`.
pub fn reopen_op() -> Op {
    Op::new(
        "agent_sessions.write",
        "reopen",
        schema::<SessionInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: SessionInput = parse(input)?;
            let session = load(ctx, &input.session)?;
            in_own_stream(ctx, session.thread_id)?;
            if session.is_open() {
                return Err(invalid("/session", "the session is open"));
            }
            if session.closed_reason != Some(SessionCloseReason::Closed) {
                return Err(invalid(
                    "/session",
                    "its thread or stream closed it: reopen that instead",
                ));
            }
            let thread_open: bool = ctx
                .conn
                .query_row(
                    "SELECT closed_at IS NULL FROM threads WHERE id = ?1",
                    [session.thread_id.value()],
                    |r| r.get(0),
                )
                .map_err(sql)?;
            if !thread_open {
                return Err(invalid(
                    "/session",
                    format!(
                        "`{}` is closed; reopen it first",
                        thread_ref(session.thread_id)
                    ),
                ));
            }
            reopen_tx(ctx.conn, session.id, Timestamp::now())?;
            let after = get_tx(ctx.conn, session.id)?
                .ok_or_else(|| invalid("/session", "the session vanished"))?;
            Ok(HandlerOutput {
                inverse: call(CLOSE, json!({ "session": input.session })),
                ..result(&after)
            })
        })),
    )
}

/// `agent_session.stop { session }`: the person stopped the agent. The
/// slot stays open and keeps its conversation, `stopped_by_user_at` is
/// set — so the process is not spawned again until a person starts it —
/// and the process stops after commit. Undone by `start`. A person's only:
/// a crash or restart is not a stop and leaves the column alone.
pub fn stop_op(processes: crate::agent_sessions::SessionProcesses) -> Op {
    Op::new(
        "agent_sessions.write",
        "stop",
        schema::<SessionInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: SessionInput = parse(input)?;
            let session = load(ctx, &input.session)?;
            in_own_stream(ctx, session.thread_id)?;
            if !session.is_open() {
                return Err(invalid("/session", "the session is closed"));
            }
            let now = Timestamp::now();
            set_stopped_tx(ctx.conn, session.id, Some(now), now)?;
            let after = get_tx(ctx.conn, session.id)?
                .ok_or_else(|| invalid("/session", "the session vanished"))?;
            let processes = processes.clone();
            let id = session.id;
            Ok(HandlerOutput {
                inverse: (session.stopped_by_user_at.is_none())
                    .then(|| call(START, json!({ "session": input.session })))
                    .flatten(),
                after_commit: Some(Box::new(move || processes.kill(id))),
                ..result(&after)
            })
        })),
    )
    .open_to(Invokers::HUMAN_ONLY)
}

/// `agent_session.start { session }`: a person starts a stopped agent
/// again. It clears the mark only — the UI spawns the process, as it does
/// for any session (`open_terminal_session`), resuming the saved
/// conversation. Undone by `stop`.
pub fn start_op() -> Op {
    Op::new(
        "agent_sessions.write",
        "start",
        schema::<SessionInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: SessionInput = parse(input)?;
            let session = load(ctx, &input.session)?;
            in_own_stream(ctx, session.thread_id)?;
            if !session.is_open() {
                return Err(invalid("/session", "the session is closed"));
            }
            set_stopped_tx(ctx.conn, session.id, None, Timestamp::now())?;
            let after = get_tx(ctx.conn, session.id)?
                .ok_or_else(|| invalid("/session", "the session vanished"))?;
            Ok(HandlerOutput {
                inverse: session
                    .stopped_by_user_at
                    .is_some()
                    .then(|| call(STOP, json!({ "session": input.session })))
                    .flatten(),
                ..result(&after)
            })
        })),
    )
    .open_to(Invokers::HUMAN_ONLY)
}

/// `agent_session.start_fresh { session }`: the one way a session gets a
/// new conversation. It forgets the saved harness conversation and clears
/// the stop mark; the UI then spawns the process. Destructive (the link
/// to the saved conversation is dropped) so it confirms, and not undoable.
/// A person's only.
pub fn start_fresh_op() -> Op {
    Op::new(
        "agent_sessions.write",
        "start_fresh",
        schema::<SessionInput>(),
        false,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: SessionInput = parse(input)?;
            let session = load(ctx, &input.session)?;
            in_own_stream(ctx, session.thread_id)?;
            if !session.is_open() {
                return Err(invalid("/session", "the session is closed"));
            }
            let now = Timestamp::now();
            unbind_tx(ctx.conn, session.id, None, now)?;
            set_stopped_tx(ctx.conn, session.id, None, now)?;
            let after = get_tx(ctx.conn, session.id)?
                .ok_or_else(|| invalid("/session", "the session vanished"))?;
            Ok(result(&after))
        })),
    )
    .open_to(Invokers::HUMAN_ONLY)
    .confirm_at_least(Confirm::Destructive)
}

/// What a person calls `session` in a message.
fn named(session: &AgentSession) -> String {
    [&session.title, &session.native_title]
        .into_iter()
        .map(|t| t.trim())
        .find(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| session.harness.clone())
}

/// `conversation` checked as one the slot may take: not empty, and not
/// open in another slot on its host (one conversation runs in one).
fn free_conversation(
    ctx: &TxCtx<'_>,
    session: &AgentSession,
    conversation: &str,
) -> Result<(), CommandError> {
    if conversation.trim().is_empty() {
        return Err(invalid("/conversation", "name a saved conversation"));
    }
    if let Some(other) = oxplow_db::agent_session_store::bound_elsewhere_tx(
        ctx.conn,
        conversation,
        session.host.as_deref(),
        session.id,
    )? {
        return Err(invalid(
            "/conversation",
            format!(
                "it is open in the session `{}` ({}): switch to that one, or fork it",
                named(&other),
                oxplow_domain::refs::build::agent_session_ref(other.id)
            ),
        ));
    }
    Ok(())
}

/// `agent_session.resume_conversation { session, conversation }`: the
/// session takes one of its harness's saved conversations (the person
/// chose it). The process it runs stops; the UI spawns the next, which
/// resumes the chosen one. The conversation it was on stays saved, and
/// undoing goes back to it. A person's only.
pub fn resume_conversation_op(processes: crate::agent_sessions::SessionProcesses) -> Op {
    Op::new(
        "agent_sessions.write",
        "resume_conversation",
        schema::<ConversationInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: ConversationInput = parse(input)?;
            let session = load(ctx, &input.session)?;
            in_own_stream(ctx, session.thread_id)?;
            if !session.is_open() {
                return Err(invalid("/session", "the session is closed"));
            }
            free_conversation(ctx, &session, &input.conversation)?;
            let now = Timestamp::now();
            bind_tx(
                ctx.conn,
                session.id,
                &input.conversation,
                BindingOrigin::Chosen,
                now,
            )?;
            set_stopped_tx(ctx.conn, session.id, None, now)?;
            let after = get_tx(ctx.conn, session.id)?
                .ok_or_else(|| invalid("/session", "the session vanished"))?;
            let previous = session.resume_session_id.clone();
            let processes = processes.clone();
            let id = session.id;
            Ok(HandlerOutput {
                inverse: (!previous.is_empty() && previous != input.conversation)
                    .then(|| {
                        call(
                            RESUME_CONVERSATION,
                            json!({ "session": input.session, "conversation": previous }),
                        )
                    })
                    .flatten(),
                after_commit: Some(Box::new(move || processes.kill(id))),
                ..result(&after)
            })
        })),
    )
    .open_to(Invokers::HUMAN_ONLY)
}

/// `agent_session.fork_conversation { session, conversation? }`: the
/// session's next launch forks a saved conversation — the one it is on,
/// or the one named — into a new one, which it is bound to once the
/// harness names it. The process it runs stops; the original stays
/// saved. A person's only.
pub fn fork_conversation_op(processes: crate::agent_sessions::SessionProcesses) -> Op {
    Op::new(
        "agent_sessions.write",
        "fork_conversation",
        schema::<ForkInput>(),
        false,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: ForkInput = parse(input)?;
            let session = load(ctx, &input.session)?;
            in_own_stream(ctx, session.thread_id)?;
            if !session.is_open() {
                return Err(invalid("/session", "the session is closed"));
            }
            let now = Timestamp::now();
            match input.conversation.as_deref() {
                Some(other) if other != session.resume_session_id => {
                    if other.trim().is_empty() {
                        return Err(invalid("/conversation", "name a saved conversation"));
                    }
                    // Forking doesn't take the original: another slot may
                    // be on it.
                    bind_tx(ctx.conn, session.id, other, BindingOrigin::Chosen, now)?;
                }
                _ if session.resume_session_id.is_empty() => {
                    return Err(invalid(
                        "/conversation",
                        "the session is on no saved conversation: name one to fork",
                    ));
                }
                _ => {}
            }
            set_fork_next_tx(ctx.conn, session.id, true, now)?;
            set_stopped_tx(ctx.conn, session.id, None, now)?;
            let after = get_tx(ctx.conn, session.id)?
                .ok_or_else(|| invalid("/session", "the session vanished"))?;
            let processes = processes.clone();
            let id = session.id;
            Ok(HandlerOutput {
                after_commit: Some(Box::new(move || processes.kill(id))),
                ..result(&after)
            })
        })),
    )
    .open_to(Invokers::HUMAN_ONLY)
}

/// The agent-session commands, for the bus.
pub fn ops(
    config: Arc<RwLock<OxplowConfig>>,
    harnesses: HarnessRegistry,
    processes: crate::agent_sessions::SessionProcesses,
) -> Vec<Op> {
    vec![
        open_op(config, harnesses),
        rename_op(),
        close_op(processes.clone()),
        reopen_op(),
        stop_op(processes.clone()),
        start_op(),
        start_fresh_op(),
        resume_conversation_op(processes.clone()),
        fork_conversation_op(processes),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{services_with_effort, EffortFixture};
    use oxplow_domain::stores::AgentSessionStore as _;
    use oxplow_domain::Actor;

    fn agent(fx: &EffortFixture) -> Actor {
        Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    async fn run(
        fx: &EffortFixture,
        actor: &Actor,
        name: &str,
        input: serde_json::Value,
        confirmed: bool,
    ) -> Result<oxplow_domain::CommandOutcome, CommandError> {
        fx.svc.commands.run(actor, name, input, confirmed).await
    }

    async fn open(fx: &EffortFixture, actor: &Actor, input: serde_json::Value) -> AgentSession {
        let out = run(fx, actor, OPEN, input, false).await.unwrap();
        serde_json::from_value(out.result).unwrap()
    }

    async fn get(fx: &EffortFixture, id: AgentSessionId) -> AgentSession {
        fx.svc.agent_session_store.get(&id).await.unwrap().unwrap()
    }

    /// Opening adds a slot of the default agent and starts nothing; undo
    /// closes it.
    #[tokio::test]
    async fn opening_adds_a_slot_and_undo_closes_it() {
        let fx = services_with_effort().await;
        let out = run(
            &fx,
            &Actor::Human,
            OPEN,
            json!({ "thread": thread_ref(fx.thread), "title": "review" }),
            false,
        )
        .await
        .unwrap();
        let s: AgentSession = serde_json::from_value(out.result).unwrap();
        assert_eq!(
            (s.kind, s.harness, s.title.as_str()),
            (SessionKind::Terminal, "claude".to_string(), "review"),
            "the first declared harness, with no agents: named"
        );
        assert!(!fx
            .svc
            .terminal_sessions
            .session_id_for_key(&crate::terminal_sessions::agent_pane_key(s.id))
            .await
            .is_some());
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), true)
            .await
            .unwrap();
        assert!(!get(&fx, s.id).await.is_open());
    }

    /// A person's stop marks the session and keeps the slot; start clears
    /// it; start_fresh also forgets the saved conversation. An agent may
    /// do none of them.
    #[tokio::test]
    async fn stopping_marks_the_session_and_starting_clears_it() {
        let fx = services_with_effort().await;
        let me = serde_json::json!({ "session": agent_session_ref(fx.session) });
        let stopped = run(&fx, &Actor::Human, STOP, me.clone(), false)
            .await
            .unwrap();
        let s = get(&fx, fx.session).await;
        assert!(s.is_open() && s.stopped_by_user_at.is_some());
        // Undo of the stop is a start.
        fx.svc
            .commands
            .undo(&Actor::Human, stopped.audit_id.unwrap(), true)
            .await
            .unwrap();
        assert!(get(&fx, fx.session).await.stopped_by_user_at.is_none());

        run(&fx, &Actor::Human, STOP, me.clone(), false)
            .await
            .unwrap();
        run(&fx, &Actor::Human, START, me.clone(), false)
            .await
            .unwrap();
        assert!(get(&fx, fx.session).await.stopped_by_user_at.is_none());

        for name in [STOP, START, START_FRESH] {
            let err = run(&fx, &agent(&fx), name, me.clone(), true)
                .await
                .unwrap_err();
            assert!(
                matches!(err, CommandError::Denied { .. }),
                "{name}: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn starting_fresh_forgets_the_conversation_and_the_stop() {
        let fx = services_with_effort().await;
        let me = serde_json::json!({ "session": agent_session_ref(fx.session) });
        let session = fx.session;
        fx.svc
            .db
            .transaction(move |tx| {
                oxplow_db::agent_session_store::bind_tx(
                    tx,
                    session,
                    "conv-1",
                    oxplow_domain::agent_session::BindingOrigin::Started,
                    Timestamp::now(),
                )?;
                set_stopped_tx(tx, session, Some(Timestamp::now()), Timestamp::now())?;
                Ok(())
            })
            .await
            .unwrap();
        run(&fx, &Actor::Human, START_FRESH, me, true)
            .await
            .unwrap();
        let s = get(&fx, fx.session).await;
        assert_eq!(s.resume_session_id, "");
        assert!(s.stopped_by_user_at.is_none());
    }

    /// A person moves a session to a saved conversation of their choosing:
    /// it is bound `chosen`, its stop clears, and undoing goes back to the
    /// one it was on. One open in another session is refused, naming it.
    /// An agent may do neither this nor a fork.
    #[tokio::test]
    async fn a_person_resumes_a_chosen_conversation() {
        let fx = services_with_effort().await;
        let me = agent_session_ref(fx.session);
        let other = open(
            &fx,
            &Actor::Human,
            serde_json::json!({ "thread": thread_ref(fx.thread), "harness": "claude" }),
        )
        .await;
        let (session, other_id) = (fx.session, other.id);
        fx.svc
            .db
            .transaction(move |tx| {
                bind_tx(tx, session, "c1", BindingOrigin::Started, Timestamp::now())?;
                bind_tx(
                    tx,
                    other_id,
                    "taken",
                    BindingOrigin::Started,
                    Timestamp::now(),
                )
            })
            .await
            .unwrap();
        let chosen = run(
            &fx,
            &Actor::Human,
            RESUME_CONVERSATION,
            serde_json::json!({ "session": me, "conversation": "c9" }),
            false,
        )
        .await
        .unwrap();
        let s = get(&fx, fx.session).await;
        assert_eq!(s.resume_session_id, "c9");
        fx.svc
            .commands
            .undo(&Actor::Human, chosen.audit_id.unwrap(), true)
            .await
            .unwrap();
        assert_eq!(get(&fx, fx.session).await.resume_session_id, "c1");

        let err = run(
            &fx,
            &Actor::Human,
            RESUME_CONVERSATION,
            serde_json::json!({ "session": me, "conversation": "taken" }),
            false,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message } if f == "/conversation" && message.contains(&agent_session_ref(other.id))),
            "{err:?}"
        );
        for (name, input) in [
            (
                RESUME_CONVERSATION,
                serde_json::json!({ "session": me, "conversation": "c2" }),
            ),
            (FORK_CONVERSATION, serde_json::json!({ "session": me })),
        ] {
            let err = run(&fx, &agent(&fx), name, input, true).await.unwrap_err();
            assert!(
                matches!(err, CommandError::Denied { .. }),
                "{name}: {err:?}"
            );
        }
    }

    /// A person opens a session on a saved conversation: it is bound
    /// `chosen` in the same transaction (undo closes the slot). One open in
    /// another session is refused, naming it. An agent may open a session
    /// but not pick the conversation for it.
    #[tokio::test]
    async fn a_person_opens_a_session_on_a_chosen_conversation() {
        let fx = services_with_effort().await;
        let (session, now) = (fx.session, Timestamp::now());
        fx.svc
            .db
            .transaction(move |tx| bind_tx(tx, session, "taken", BindingOrigin::Started, now))
            .await
            .unwrap();
        let thread = thread_ref(fx.thread);
        let opened = open(
            &fx,
            &Actor::Human,
            json!({ "thread": thread, "harness": "claude", "resume": "c9" }),
        )
        .await;
        let row = get(&fx, opened.id).await;
        assert_eq!(row.resume_session_id, "c9");
        let origin: String = fx
            .svc
            .db
            .read(move |c| {
                c.query_row(
                    "SELECT origin FROM agent_session_binding WHERE agent_session_id = ?1 AND unbound_at IS NULL",
                    [opened.id.value()],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(origin, "chosen");

        let err = run(
            &fx,
            &Actor::Human,
            OPEN,
            json!({ "thread": thread, "harness": "claude", "resume": "taken" }),
            false,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message } if f == "/resume" && message.contains(&agent_session_ref(fx.session))),
            "{err:?}"
        );
        // Nothing was left behind by the refusal.
        assert_eq!(
            fx.svc
                .agent_session_store
                .list_open_for_thread(&fx.thread)
                .await
                .unwrap()
                .len(),
            2
        );

        // An agent may open a session, not choose its conversation.
        let err = run(
            &fx,
            &agent(&fx),
            OPEN,
            json!({ "thread": thread, "harness": "claude", "resume": "c2" }),
            true,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                &err,
                CommandError::Denied { .. } | CommandError::Invalid { .. }
            ),
            "{err:?}"
        );
        run(
            &fx,
            &agent(&fx),
            OPEN,
            json!({ "thread": thread, "harness": "claude" }),
            true,
        )
        .await
        .unwrap();
    }

    /// A fork marks the next launch to fork the conversation the session
    /// is on, or the one named; a session on none must name one.
    #[tokio::test]
    async fn a_fork_marks_the_next_launch() {
        let fx = services_with_effort().await;
        let me = agent_session_ref(fx.session);
        let err = run(
            &fx,
            &Actor::Human,
            FORK_CONVERSATION,
            serde_json::json!({ "session": me }),
            false,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/conversation"),
            "{err:?}"
        );
        run(
            &fx,
            &Actor::Human,
            FORK_CONVERSATION,
            serde_json::json!({ "session": me, "conversation": "c5" }),
            false,
        )
        .await
        .unwrap();
        let s = get(&fx, fx.session).await;
        assert_eq!((s.resume_session_id.as_str(), s.fork_next), ("c5", true));
    }

    /// The input is checked: an unknown field and a bad value name their
    /// field; an action session isn't built; a harness nothing registers,
    /// or the project hasn't enabled, or the retired `chat` kind or `acp`
    /// harness, is refused.
    #[tokio::test]
    async fn the_input_is_checked() {
        let fx = services_with_effort().await;
        let thread = thread_ref(fx.thread);
        fx.svc.config.write().unwrap().agents = vec!["claude".into()];
        for (input, field) in [
            (json!({ "thread": thread, "kind": "action" }), "/kind"),
            (json!({ "thread": thread, "kind": "chat" }), "/kind"),
            (json!({ "thread": thread, "harness": "codex" }), "/harness"),
            (json!({ "thread": thread, "harness": "nope" }), "/harness"),
            (json!({ "thread": thread, "harness": "acp" }), "/harness"),
            (json!({ "thread": "thread:thr99" }), "/thread"),
        ] {
            let err = run(&fx, &Actor::Human, OPEN, input.clone(), false)
                .await
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == field),
                "{input}: {err:?}"
            );
        }
        let err = run(
            &fx,
            &Actor::Human,
            OPEN,
            json!({ "thread": thread, "agent": "claude" }),
            false,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("agent"), "{err}");
    }

    /// An agent opens and closes sessions only in its own stream; a close
    /// is undoable, so it runs without waiting for a person.
    #[tokio::test]
    async fn an_agent_opens_and_closes_in_its_own_stream() {
        let fx = services_with_effort().await;
        let mine = open(&fx, &agent(&fx), json!({ "thread": thread_ref(fx.thread) })).await;
        let err = run(
            &fx,
            &Actor::Agent {
                session_id: None,
                thread_id: Some(ThreadId::new(99)),
                stream_id: None,
            },
            OPEN,
            json!({ "thread": thread_ref(fx.thread) }),
            false,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let r = agent_session_ref(mine.id);
        run(&fx, &agent(&fx), CLOSE, json!({ "session": r }), false)
            .await
            .unwrap();
        assert!(!get(&fx, mine.id).await.is_open());
    }

    /// Closing needs no confirmation: it closes the slot and its open turn
    /// in one run, audited with its `command.executed`, and undoing it
    /// reopens the slot (its process resumes the saved conversation when
    /// its tab mounts). Reopening an open session, or one its thread's
    /// close took, is refused.
    #[tokio::test]
    async fn closing_ends_the_open_turn_and_undo_reopens_it() {
        use oxplow_domain::stores::AgentTurnStore as _;
        let fx = services_with_effort().await;
        let s = fx.session;
        fx.svc
            .hook_ingest
            .ingest(crate::hook_ingest::HookEnvelope {
                session_title: None,
                turn_run: None,
                kind: oxplow_domain::HookKind::UserPromptSubmit,
                thread_id: Some(fx.thread),
                stream_id: None,
                agent_session_id: Some(s),
                session_id: Some("h1".into()),
                payload_json: "{}".into(),
                prompt: Some("go".into()),
                decision: None,
                tool: None,
                subagent: None,
                peer: None,
            })
            .await
            .unwrap();
        let r = agent_session_ref(s);
        let out = run(&fx, &Actor::Human, CLOSE, json!({ "session": r }), false)
            .await
            .unwrap();
        let closed = get(&fx, s).await;
        assert_eq!(closed.closed_reason, Some(SessionCloseReason::Closed));
        assert!(fx
            .svc
            .agent_turn_store
            .list_open(&fx.thread)
            .await
            .unwrap()
            .is_empty());
        let audit = fx
            .svc
            .commands
            .audit_store()
            .list_recent(1)
            .await
            .unwrap()
            .remove(0);
        assert_eq!(Some(audit.id), out.audit_id);
        let events = fx.svc.event_log_store.read_after(0, 1000).await.unwrap();
        assert!(events
            .iter()
            .any(|e| e.envelope.event_type == "command.executed"
                && e.envelope.payload["audit_id"] == audit.id));
        assert!(events
            .iter()
            .any(|e| e.envelope.event_type == "agent.status.changed"
                && e.envelope.anchors.agent_session_id == Some(s)
                && e.envelope.payload["state"] == "stopped"));

        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        let reopened = get(&fx, s).await;
        assert!(reopened.is_open());
        assert_eq!(reopened.closed_reason, None);
        let err = run(&fx, &Actor::Human, REOPEN, json!({ "session": r }), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");

        run(
            &fx,
            &Actor::Human,
            "oxplow.thread.close",
            json!({ "thread": thread_ref(fx.thread) }),
            true,
        )
        .await
        .unwrap();
        let err = run(&fx, &Actor::Human, REOPEN, json!({ "session": r }), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn renaming_undoes_to_the_old_title() {
        let fx = services_with_effort().await;
        let r = agent_session_ref(fx.session);
        let out = run(
            &fx,
            &Actor::Human,
            RENAME,
            json!({ "session": r, "title": "tests" }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(get(&fx, fx.session).await.title, "tests");
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), true)
            .await
            .unwrap();
        assert_eq!(get(&fx, fx.session).await.title, "");
    }

    /// Closing a thread closes its sessions (`thread_closed`).
    #[tokio::test]
    async fn closing_a_thread_closes_its_sessions() {
        let fx = services_with_effort().await;
        use oxplow_domain::stores::ThreadStore as _;
        let stream = fx
            .svc
            .thread_store
            .get(&fx.thread)
            .await
            .unwrap()
            .unwrap()
            .stream_id;
        let second = crate::test_fixtures::new_thread(&fx.svc, stream, "t").await;
        let s = open(
            &fx,
            &Actor::Human,
            json!({ "thread": thread_ref(second.id) }),
        )
        .await;
        run(
            &fx,
            &Actor::Human,
            crate::commands::thread::CLOSE,
            json!({ "thread": thread_ref(second.id) }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            get(&fx, s.id).await.closed_reason,
            Some(SessionCloseReason::ThreadClosed)
        );
    }

    fn mint(fx: &EffortFixture, session: &AgentSession, stream: oxplow_domain::StreamId) -> String {
        fx.svc.session_auth.issue(crate::session_auth::Principal {
            session: session.id,
            thread: session.thread_id,
            stream,
            harness: session.harness.clone(),
        })
    }

    /// A closed session's process stops, and so does its bearer: nothing
    /// can post or call as it after.
    #[tokio::test]
    async fn closing_a_session_or_its_thread_retires_its_bearer() {
        let fx = services_with_effort().await;
        use oxplow_domain::stores::ThreadStore as _;
        let stream = fx
            .svc
            .thread_store
            .get(&fx.thread)
            .await
            .unwrap()
            .unwrap()
            .stream_id;
        let session = get(&fx, fx.session).await;
        let token = mint(&fx, &session, stream);
        run(
            &fx,
            &Actor::Human,
            CLOSE,
            json!({ "session": agent_session_ref(session.id) }),
            true,
        )
        .await
        .unwrap();
        assert!(fx.svc.session_auth.authenticate(&token).is_none());

        let second = crate::test_fixtures::new_thread(&fx.svc, stream, "t").await;
        let s = open(
            &fx,
            &Actor::Human,
            json!({ "thread": thread_ref(second.id) }),
        )
        .await;
        let token = mint(&fx, &s, stream);
        run(
            &fx,
            &Actor::Human,
            crate::commands::thread::CLOSE,
            json!({ "thread": thread_ref(second.id) }),
            false,
        )
        .await
        .unwrap();
        assert!(fx.svc.session_auth.authenticate(&token).is_none());
    }
}
