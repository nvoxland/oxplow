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
//! A session is named by ref (`oxplow:oxplow/agent_session?id=ses3`), a thread by
//! `oxplow:oxplow/thread?id=thr1`. An agent acts only on sessions in its own stream, and its
//! close — destructive — waits for a person as a proposal.

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_config::OxplowConfig;
use oxplow_db::agent_session_store::{
    bind_tx, get_tx, insert_tx, reopen_tx, set_fork_next_tx, set_title_tx, unbind_tx,
};
use oxplow_domain::agent::registry::HarnessRegistry;
use oxplow_domain::agent_session::BindingOrigin;
use oxplow_domain::agent_session::{
    AgentSession, NewAgentSession, SessionCloseReason, SessionKind,
};
use oxplow_domain::uri::build::{stream_uri, thread_uri};
use oxplow_domain::{AgentSessionId, CommandCall, CommandError, Confirm, ThreadId, Timestamp};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::util::{bare_id, invalid, parse, schema, sql};
use super::{Handler, HandlerOutput, TxCtx};

pub const OPEN: &str = "oxplow.agent_session.open";
pub const RENAME: &str = "oxplow.agent_session.rename";
pub const CLOSE: &str = "oxplow.agent_session.close";
pub const REOPEN: &str = "oxplow.agent_session.reopen";
pub const START_FRESH: &str = "oxplow.agent_session.start_fresh";
pub const FORK_CONVERSATION: &str = "oxplow.agent_session.fork_conversation";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenInput {
    /// The thread (`thr1`).
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
    /// Also make this agent what new threads start with (the person's own
    /// `newThreadSession`). A person's choice; undoing the open leaves it.
    #[serde(default)]
    pub remember: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionInput {
    /// The session (`ses3`).
    pub session: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    /// The session (`ses3`).
    pub session: String,
    pub title: String,
    /// Also name the conversation in the harness's own UI (Codex's thread
    /// list): only where the harness declares `native_rename`, while its
    /// conversation is observed. Not undone with the rename.
    #[serde(default)]
    pub native: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForkInput {
    /// The session (`ses3`).
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

/// The session an `oxplow:oxplow/agent_session?id=<id>` ref names.
fn load(ctx: &TxCtx<'_>, value: &str) -> Result<AgentSession, CommandError> {
    let id: AgentSessionId = bare_id(
        value,
        oxplow_domain::uri::resources::AGENT_SESSION,
        "/session",
    )?;
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
                stream_uri(own)
            ),
        });
    }
    Ok(())
}

/// `thread` checked as one a session may open on: it exists, is open,
/// and is in the agent's own stream. Both `open` and `fork_conversation`
/// open their sessions through this.
fn open_thread(ctx: &TxCtx<'_>, thread: ThreadId) -> Result<(), CommandError> {
    let row = oxplow_db::thread_store::get_tx(ctx.conn, thread)
        .map_err(sql)?
        .filter(|t| t.archived_at.is_none())
        .ok_or_else(|| invalid("/thread", format!("no thread `{}`", thread_uri(thread))))?;
    if !row.is_open() {
        return Err(invalid(
            "/thread",
            format!("`{}` is closed; reopen it first", thread_uri(thread)),
        ));
    }
    in_own_stream(ctx, thread)
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
pub fn open_op(config: super::config_commands::ConfigTarget, harnesses: HarnessRegistry) -> Op {
    Op::new(
        "agent_sessions.write",
        "open",
        schema::<OpenInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: OpenInput = parse(input)?;
            let thread = bare_id(
                &input.thread,
                oxplow_domain::uri::resources::THREAD,
                "/thread",
            )?;
            open_thread(ctx, thread)?;
            let cfg = crate::config_service::read_config(&config.config);
            let (harness, kind) = choose_agent(&cfg, &harnesses, &input)?;
            let remembered = if input.remember {
                Some(super::config_commands::change(
                    &config,
                    ctx.actor(),
                    "newThreadSession",
                    Some(json!(harness)),
                    super::config_commands::Layer::Personal,
                )?)
            } else {
                None
            };
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
            let mut out = HandlerOutput {
                inverse: call(CLOSE, json!({ "session": session.id.to_string() })),
                ..result(&session)
            };
            // Remembering what new threads start with lands with the open
            // (its event, its file write after commit).
            if let Some(remembered) = remembered {
                out.events.extend(remembered.events);
                out.after_commit = remembered.after_commit;
            }
            Ok(out)
        })),
    )
}

/// `agent_session.rename { session, title, native? }`; undone by renaming
/// it back. With `native`, the harness names the conversation too, once
/// the rename commits (checked first: a harness that can't, or one not
/// observing the conversation now, refuses the whole rename).
pub fn rename_op(
    chat: Arc<crate::chat::manager::ChatManager>,
    capabilities: Arc<crate::capabilities::CapabilityRegistry>,
) -> Op {
    Op::new(
        "agent_sessions.write",
        "rename",
        schema::<RenameInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: RenameInput = parse(input)?;
            let session = load(ctx, &input.session)?;
            in_own_stream(ctx, session.thread_id)?;
            let native = input.title.trim().to_string();
            if input.native {
                let can = capabilities
                    .get("agent_harness", &session.harness)
                    .is_some_and(|i| {
                        i.features.get("native_rename").and_then(|v| v.as_bool()) == Some(true)
                    });
                if !can {
                    return Err(invalid(
                        "/native",
                        format!(
                            "`{}` can't rename its conversation; Rename sets oxplow's name for the session only",
                            session.harness
                        ),
                    ));
                }
                if native.is_empty() {
                    return Err(invalid("/title", "name the conversation"));
                }
                if !chat.is_open(&session.id) {
                    return Err(invalid(
                        "/native",
                        "the session's conversation isn't being observed right now; open its tab and try again",
                    ));
                }
            }
            let before = set_title_tx(ctx.conn, session.id, &input.title, Timestamp::now())?
                .unwrap_or_default();
            let after = get_tx(ctx.conn, session.id)?
                .ok_or_else(|| invalid("/session", "the session vanished"))?;
            let mut out = result(&after);
            out.result["native_renamed"] = json!(input.native);
            let after_commit: Option<Box<dyn FnOnce() + Send + Sync>> = if input.native {
                let chat = chat.clone();
                let id = session.id;
                Some(Box::new(move || {
                    tokio::spawn(async move {
                        if let Err(e) = chat.rename_native(&id, native).await {
                            tracing::warn!(session = %id, error = %e, "the harness didn't take the conversation's name");
                        }
                    });
                }))
            } else {
                None
            };
            Ok(HandlerOutput {
                inverse: call(RENAME, json!({ "session": input.session, "title": before })),
                after_commit,
                ..out
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
                        thread_uri(session.thread_id)
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

/// `agent_session.start_fresh { session }`: the one way a session gets a
/// new conversation. It forgets the saved harness conversation; the UI
/// then spawns the process. Destructive (the link
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
            let after = get_tx(ctx.conn, session.id)?
                .ok_or_else(|| invalid("/session", "the session vanished"))?;
            Ok(result(&after))
        })),
    )
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
                oxplow_domain::uri::build::agent_session_uri(other.id)
            ),
        ));
    }
    Ok(())
}

/// `agent_session.fork_conversation { session, conversation? }`: a NEW
/// session beside `session`, on its thread and harness, bound to the saved
/// conversation to fork — the one `session` is on, or the one named — with
/// `fork_next` set. Its first launch continues a copy and the hook ingest
/// binds the session to the copy's id once the harness names it. `session`
/// is untouched: its process keeps running and it keeps its conversation.
/// The result is the new session plus its `uri`; undone by closing it. A
/// person's only; refused for a harness without `conversation_fork`.
///
/// Between this and the new session's first launch it shares the source's
/// conversation id. Forking doesn't take the original (so nothing here
/// asks that it be free), and until the launch binds the copy a session
/// that tries to take that conversation is refused as usual.
pub fn fork_conversation_op(capabilities: Arc<crate::capabilities::CapabilityRegistry>) -> Op {
    Op::new(
        "agent_sessions.write",
        "fork_conversation",
        schema::<ForkInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: ForkInput = parse(input)?;
            let session = load(ctx, &input.session)?;
            in_own_stream(ctx, session.thread_id)?;
            if !session.is_open() {
                return Err(invalid("/session", "the session is closed"));
            }
            let can = capabilities
                .get("agent_harness", &session.harness)
                .is_some_and(|i| {
                    i.features
                        .get("conversation_fork")
                        .and_then(|v| v.as_bool())
                        == Some(true)
                });
            if !can {
                return Err(invalid(
                    "/session",
                    format!("`{}` can't fork a conversation", session.harness),
                ));
            }
            let conversation = match input.conversation.as_deref() {
                Some(other) if other.trim().is_empty() => {
                    return Err(invalid("/conversation", "name a saved conversation"));
                }
                Some(other) => other.to_string(),
                None if session.resume_session_id.is_empty() => {
                    return Err(invalid(
                        "/conversation",
                        "the session is on no saved conversation: name one to fork",
                    ));
                }
                None => session.resume_session_id.clone(),
            };
            open_thread(ctx, session.thread_id)?;
            let now = Timestamp::now();
            let new = insert_tx(
                ctx.conn,
                &NewAgentSession {
                    thread_id: session.thread_id,
                    kind: session.kind,
                    harness: session.harness.clone(),
                    title: format!("{} (fork)", named(&session)),
                },
                now,
            )?;
            bind_tx(ctx.conn, new.id, &conversation, BindingOrigin::Chosen, now)?;
            set_fork_next_tx(ctx.conn, new.id, true, now)?;
            let new = get_tx(ctx.conn, new.id)?
                .ok_or_else(|| invalid("/session", "the session vanished"))?;
            let mut out = HandlerOutput {
                inverse: call(CLOSE, json!({ "session": new.id.to_string() })),
                ..result(&new)
            };
            out.result["uri"] = json!(oxplow_domain::uri::build::agent_session_uri(new.id));
            Ok(out)
        })),
    )
}

/// The agent-session commands, for the bus.
pub fn ops(
    config: super::config_commands::ConfigTarget,
    harnesses: HarnessRegistry,
    processes: crate::agent_sessions::SessionProcesses,
    chat: Arc<crate::chat::manager::ChatManager>,
    capabilities: Arc<crate::capabilities::CapabilityRegistry>,
) -> Vec<Op> {
    vec![
        open_op(config, harnesses),
        rename_op(chat, capabilities.clone()),
        close_op(processes.clone()),
        reopen_op(),
        start_fresh_op(),
        fork_conversation_op(capabilities),
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
            json!({ "thread": fx.thread.to_string(), "title": "review" }),
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

    /// `remember` makes the chosen agent what new threads start with (the
    /// person's own `newThreadSession`), in the same call; it is a
    /// person's, so an agent that asks is refused and nothing opens.
    #[tokio::test]
    async fn remember_sets_what_new_threads_start_with() {
        let fx = services_with_effort().await;
        let s = open(
            &fx,
            &Actor::Human,
            json!({ "thread": fx.thread.to_string(), "harness": "claude", "remember": true }),
        )
        .await;
        assert_eq!(s.harness, "claude");
        assert_eq!(
            crate::config_service::read_config(&fx.svc.config)
                .personal_new_thread_session
                .as_deref(),
            Some("claude")
        );
        let before = fx
            .svc
            .agent_session_store
            .list_open_for_thread(&fx.thread)
            .await
            .unwrap()
            .len();
        // An agent may choose `remember` too.
        run(
            &fx,
            &agent(&fx),
            OPEN,
            json!({ "thread": fx.thread.to_string(), "remember": true }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            fx.svc
                .agent_session_store
                .list_open_for_thread(&fx.thread)
                .await
                .unwrap()
                .len(),
            before + 1
        );
    }

    #[tokio::test]
    async fn starting_fresh_forgets_the_conversation() {
        let fx = services_with_effort().await;
        let me = serde_json::json!({ "session": fx.session.to_string() });
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
                Ok(())
            })
            .await
            .unwrap();
        run(&fx, &Actor::Human, START_FRESH, me, true)
            .await
            .unwrap();
        let s = get(&fx, fx.session).await;
        assert_eq!(s.resume_session_id, "");
    }

    /// An agent may fork a session too.
    #[tokio::test]
    async fn an_agent_may_fork_a_session() {
        let fx = services_with_effort().await;
        let me = fx.session.to_string();
        bind(&fx, fx.session, "c5").await;
        run(
            &fx,
            &agent(&fx),
            FORK_CONVERSATION,
            serde_json::json!({ "session": me }),
            true,
        )
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    }

    /// A person opens a session on a saved conversation: it is bound
    /// `chosen` in the same transaction (undo closes the slot). One open in
    /// another session is refused, naming it. An agent may open a session
    /// and pick the conversation for it too.
    #[tokio::test]
    async fn a_person_opens_a_session_on_a_chosen_conversation() {
        let fx = services_with_effort().await;
        let (session, now) = (fx.session, Timestamp::now());
        fx.svc
            .db
            .transaction(move |tx| bind_tx(tx, session, "taken", BindingOrigin::Started, now))
            .await
            .unwrap();
        let thread = fx.thread.to_string();
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
            matches!(&err, CommandError::Invalid { field: Some(f), message } if f == "/resume" && message.contains(&oxplow_domain::uri::build::agent_session_uri(fx.session))),
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

        // An agent may choose the conversation too.
        run(
            &fx,
            &agent(&fx),
            OPEN,
            json!({ "thread": thread, "harness": "claude", "resume": "c2" }),
            true,
        )
        .await
        .unwrap();
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

    /// Bind session `id` to conversation `native`, as a hook would.
    async fn bind(fx: &EffortFixture, id: AgentSessionId, native: &'static str) {
        fx.svc
            .db
            .transaction(move |tx| {
                bind_tx(tx, id, native, BindingOrigin::Started, Timestamp::now())
            })
            .await
            .unwrap();
    }

    async fn fork(
        fx: &EffortFixture,
        input: serde_json::Value,
    ) -> Result<oxplow_domain::CommandOutcome, CommandError> {
        run(fx, &Actor::Human, FORK_CONVERSATION, input, false).await
    }

    /// A fork opens a NEW session beside the source, on the same thread
    /// and harness, bound to the conversation it forks (the source's, by
    /// default) with the next launch marked a fork. The result names the
    /// new session's URI. The source is untouched: still open, still on
    /// its conversation, its process (and bearer) not stopped.
    #[tokio::test]
    async fn a_fork_opens_a_new_session_and_leaves_the_source_alone() {
        use oxplow_domain::stores::ThreadStore as _;
        let fx = services_with_effort().await;
        let stream = fx
            .svc
            .thread_store
            .get(&fx.thread)
            .await
            .unwrap()
            .unwrap()
            .stream_id;
        bind(&fx, fx.session, "c5").await;
        let source = get(&fx, fx.session).await;
        let token = mint(&fx, &source, stream);

        let out = fork(&fx, json!({ "session": fx.session.to_string() }))
            .await
            .unwrap();
        let new: AgentSession = serde_json::from_value(out.result.clone()).unwrap();
        assert_ne!(new.id, fx.session);
        assert_eq!(
            out.result["uri"],
            oxplow_domain::uri::build::agent_session_uri(new.id)
        );
        let new = get(&fx, new.id).await;
        assert_eq!(
            (
                new.thread_id,
                new.harness.as_str(),
                new.kind,
                new.resume_session_id.as_str(),
                new.fork_next,
                new.is_open()
            ),
            (fx.thread, "claude", source.kind, "c5", true, true)
        );
        let after = get(&fx, fx.session).await;
        assert_eq!(
            (
                after.resume_session_id.as_str(),
                after.fork_next,
                after.is_open()
            ),
            ("c5", false, true)
        );
        assert!(
            fx.svc.session_auth.authenticate(&token).is_some(),
            "the source's process is not stopped"
        );
        // Undo closes the new session only.
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), true)
            .await
            .unwrap();
        assert!(!get(&fx, new.id).await.is_open());
        assert!(get(&fx, fx.session).await.is_open());
    }

    /// A named conversation is the one forked; the source keeps its own.
    /// A session on none must name one. A conversation another session
    /// is on may still be forked (the copy is a new one).
    #[tokio::test]
    async fn a_fork_takes_the_named_conversation_or_the_sources() {
        let fx = services_with_effort().await;
        let me = fx.session.to_string();
        let err = fork(&fx, json!({ "session": me })).await.unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/conversation"),
            "{err:?}"
        );
        let out = fork(&fx, json!({ "session": me, "conversation": "c7" }))
            .await
            .unwrap();
        let new: AgentSession = serde_json::from_value(out.result).unwrap();
        let new = get(&fx, new.id).await;
        assert_eq!(
            (new.resume_session_id.as_str(), new.fork_next),
            ("c7", true)
        );
        assert_eq!(get(&fx, fx.session).await.resume_session_id, "");
        bind(&fx, fx.session, "c5").await;
        let out = fork(&fx, json!({ "session": me, "conversation": "c5" }))
            .await
            .unwrap();
        let again: AgentSession = serde_json::from_value(out.result).unwrap();
        assert_eq!(get(&fx, again.id).await.resume_session_id, "c5");
    }

    /// A harness without `conversation_fork` (or a closed source) is
    /// refused, and nothing opens.
    #[tokio::test]
    async fn a_fork_needs_a_harness_that_can_and_an_open_session() {
        let fx = services_with_effort().await;
        let thread = fx.thread;
        let odd = fx
            .svc
            .db
            .transaction(move |tx| {
                insert_tx(
                    tx,
                    &NewAgentSession::of(thread, SessionKind::Terminal, "nothing"),
                    Timestamp::now(),
                )
            })
            .await
            .unwrap();
        let open_count = |fx: &EffortFixture| {
            let store = fx.svc.agent_session_store.clone();
            let thread = fx.thread;
            async move { store.list_open_for_thread(&thread).await.unwrap().len() }
        };
        let before = open_count(&fx).await;
        let err = fork(
            &fx,
            json!({ "session": odd.id.to_string(), "conversation": "c1" }),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("can't fork"), "{err}");
        run(
            &fx,
            &Actor::Human,
            CLOSE,
            json!({ "session": fx.session.to_string() }),
            true,
        )
        .await
        .unwrap();
        let err = fork(
            &fx,
            json!({ "session": fx.session.to_string(), "conversation": "c1" }),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("closed"), "{err}");
        assert_eq!(
            open_count(&fx).await,
            before - 1,
            "only the close changed it"
        );
    }

    /// The input is checked: an unknown field and a bad value name their
    /// field; an action session isn't built; a harness nothing registers,
    /// or the project hasn't enabled, or the retired `chat` kind or `acp`
    /// harness, is refused.
    #[tokio::test]
    async fn the_input_is_checked() {
        let fx = services_with_effort().await;
        let thread = fx.thread.to_string();
        fx.svc.config.write().unwrap().agents = vec!["claude".into()];
        for (input, field) in [
            (json!({ "thread": thread, "kind": "action" }), "/kind"),
            (json!({ "thread": thread, "kind": "chat" }), "/kind"),
            (json!({ "thread": thread, "harness": "codex" }), "/harness"),
            (json!({ "thread": thread, "harness": "nope" }), "/harness"),
            (json!({ "thread": thread, "harness": "acp" }), "/harness"),
            (json!({ "thread": "thr99" }), "/thread"),
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
        let mine = open(&fx, &agent(&fx), json!({ "thread": fx.thread.to_string() })).await;
        let err = run(
            &fx,
            &Actor::Agent {
                session_id: None,
                thread_id: Some(ThreadId::new(99)),
                stream_id: None,
            },
            OPEN,
            json!({ "thread": fx.thread.to_string() }),
            false,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let r = mine.id.to_string();
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
        let r = s.to_string();
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
            json!({ "thread": fx.thread.to_string() }),
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
        let r = fx.session.to_string();
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

    /// `native: true` also names the conversation in the harness's own UI,
    /// which only a harness declaring `native_rename` can, and only while
    /// its conversation is observed: otherwise the rename is refused and
    /// nothing changes. Without it, only oxplow's name changes.
    #[tokio::test]
    async fn renaming_natively_needs_a_harness_that_can() {
        let fx = services_with_effort().await;
        let r = fx.session.to_string();
        let err = run(
            &fx,
            &Actor::Human,
            RENAME,
            json!({ "session": r, "title": "tests", "native": true }),
            false,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("can't rename its conversation") || err.contains("isn't being observed"),
            "{err}"
        );
        assert_eq!(get(&fx, fx.session).await.title, "");
        let out = run(
            &fx,
            &Actor::Human,
            RENAME,
            json!({ "session": r, "title": "tests", "native": false }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(out.result["native_renamed"], false);
        assert_eq!(get(&fx, fx.session).await.title, "tests");
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
            json!({ "thread": second.id.to_string() }),
        )
        .await;
        run(
            &fx,
            &Actor::Human,
            crate::commands::thread::CLOSE,
            json!({ "thread": second.id.to_string() }),
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
            json!({ "session": session.id.to_string() }),
            true,
        )
        .await
        .unwrap();
        assert!(fx.svc.session_auth.authenticate(&token).is_none());

        let second = crate::test_fixtures::new_thread(&fx.svc, stream, "t").await;
        let s = open(
            &fx,
            &Actor::Human,
            json!({ "thread": second.id.to_string() }),
        )
        .await;
        let token = mint(&fx, &s, stream);
        run(
            &fx,
            &Actor::Human,
            crate::commands::thread::CLOSE,
            json!({ "thread": second.id.to_string() }),
            false,
        )
        .await
        .unwrap();
        assert!(fx.svc.session_auth.authenticate(&token).is_none());
    }
}
