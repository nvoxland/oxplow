//! Thread commands (P8.A3, `.context/commands.md`): a thread's lifecycle
//! — create (or fork), rename, its custom prompt, its mark, close and
//! reopen — as `Tx` commands, so every surface (the desktop, an agent, a
//! lens) goes through the same validation, policy, audit and undo. A
//! thread is open or closed, nothing more: every open thread of a stream
//! shares its worktree, and any of them may change it
//! (`.context/agent-model.md` "Why there is no writer thread").
//!
//! - **create** → open, at the end of its stream's order;
//! - **close** → closed (its agent sessions close too, and their
//!   processes stop after commit); **reopen** → open again.
//!
//! A thread is named by ref (`oxplow:oxplow/thread?id=thr12`), a stream by `oxplow:oxplow/stream?id=str1`.
//! An agent acts only on its own stream, and closes only its own thread;
//! setting a prompt steers an agent and marking a thread is the person's
//! own grouping, so both are a person's.

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_db::thread_store::{get_tx, list_for_stream_tx, upsert_tx};
use oxplow_domain::uri::build::{stream_uri, thread_uri};
use oxplow_domain::{
    CommandCall, CommandError, Invokers, StreamId, Thread, ThreadId, ThreadMark, Timestamp,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::util::{invalid, parse, ref_id, schema, sql};
use super::{Handler, HandlerOutput, TxCtx};

pub const CREATE: &str = "oxplow.thread.create";
pub const RENAME: &str = "oxplow.thread.rename";
pub const SET_PROMPT: &str = "oxplow.thread.set_prompt";
pub const MARK: &str = "oxplow.thread.mark";
pub const CLOSE: &str = "oxplow.thread.close";
pub const REOPEN: &str = "oxplow.thread.reopen";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    /// The stream (`oxplow:oxplow/stream?id=str1`).
    pub stream: String,
    pub title: String,
    /// Fork this thread (`oxplow:oxplow/thread?id=thr3`): the new one joins its stream.
    /// Nothing agent-ish is copied — a thread's agents are its sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    /// The thread (`oxplow:oxplow/thread?id=thr12`).
    pub thread: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPromptInput {
    /// The thread (`oxplow:oxplow/thread?id=thr12`).
    pub thread: String,
    /// Appended to the agent's prompt; empty (or only whitespace) or absent
    /// clears it.
    #[schemars(extend("contentMediaType" = "text/markdown"))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkInput {
    /// The thread (`oxplow:oxplow/thread?id=thr12`).
    pub thread: String,
    /// The colour for its tab: `amber`, `red`, `green`, `blue` or
    /// `violet`; absent clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<ThreadMark>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadInput {
    /// The thread (`oxplow:oxplow/thread?id=thr12`).
    pub thread: String,
}

fn load(ctx: &TxCtx<'_>, id: ThreadId, field: &str) -> Result<Thread, CommandError> {
    get_tx(ctx.conn, id)
        .map_err(sql)?
        .filter(|t| t.archived_at.is_none())
        .ok_or_else(|| invalid(field, format!("no thread `{}`", thread_uri(id))))
}

fn save(ctx: &TxCtx<'_>, thread: &Thread) -> Result<ThreadId, CommandError> {
    upsert_tx(ctx.conn, thread).map_err(sql)
}

/// The agent's own thread and stream, when the run is an agent's.
pub(super) fn agent_scope(ctx: &TxCtx<'_>) -> Result<Option<(ThreadId, StreamId)>, CommandError> {
    match ctx.actor().agent_thread() {
        None => Ok(None),
        Some(None) => Err(CommandError::Denied {
            reason: "an agent without a thread can't change threads".into(),
        }),
        // The bus resolved the agent's stream from its thread.
        Some(Some(own)) => match ctx.actor().stream_id() {
            Some(stream) => Ok(Some((own, stream))),
            None => Err(CommandError::Denied {
                reason: format!("the agent's thread `{}` has no stream", thread_uri(own)),
            }),
        },
    }
}

/// The thread an actor's record goes on — a note, a decision, a claim, a
/// test run: an agent's own (naming another, or having none, is refused),
/// else the one a person names. `own` is the agent's thread, when the
/// actor is one (`Actor::agent_thread`).
fn record_thread(
    own: Option<Option<ThreadId>>,
    named: Option<&str>,
) -> Result<ThreadId, CommandError> {
    let named = named
        .map(|t| ref_id(t, oxplow_domain::uri::resources::THREAD, "/thread"))
        .transpose()?;
    match own {
        None => named.ok_or_else(|| invalid("/thread", "name the thread")),
        Some(None) => Err(CommandError::Denied {
            reason: "an agent without a thread can't record on one".into(),
        }),
        Some(Some(own)) => match named {
            Some(t) if t != own => Err(CommandError::Denied {
                reason: format!(
                    "an agent writes only in its own thread (`{}`)",
                    thread_uri(own)
                ),
            }),
            _ => Ok(own),
        },
    }
}

/// [`record_thread`] in the bus's transaction: the agent's thread must
/// exist.
pub(super) fn acting_thread(
    ctx: &TxCtx<'_>,
    named: Option<&str>,
) -> Result<ThreadId, CommandError> {
    let own = agent_scope(ctx)?.map(|(own, _)| Some(own));
    record_thread(own, named)
}

/// [`record_thread`] for a command that runs outside the transaction.
pub(crate) fn acting_thread_of(
    actor: &oxplow_domain::Actor,
    named: Option<&str>,
) -> Result<ThreadId, CommandError> {
    record_thread(actor.agent_thread(), named)
}

/// An agent acts only on its own stream.
/// An agent closes and reopens only its own thread (`verb`: what it does).
fn own_thread_only(ctx: &TxCtx<'_>, id: ThreadId, verb: &str) -> Result<(), CommandError> {
    match agent_scope(ctx)? {
        Some((own, _)) if own != id => Err(CommandError::Denied {
            reason: format!(
                "an agent {verb} only its own thread (`{}`)",
                thread_uri(own)
            ),
        }),
        _ => Ok(()),
    }
}

fn on_own_stream(ctx: &TxCtx<'_>, stream: StreamId) -> Result<(), CommandError> {
    match agent_scope(ctx)? {
        Some((_, own)) if own != stream => Err(CommandError::Denied {
            reason: format!(
                "an agent changes threads only on its own stream (`{}`)",
                stream_uri(own)
            ),
        }),
        _ => Ok(()),
    }
}

fn result(thread: &Thread) -> HandlerOutput {
    HandlerOutput {
        result: serde_json::to_value(thread).expect("a thread serializes"),
        ..HandlerOutput::default()
    }
}

/// `thread`, already where the call would move it: nothing to record or
/// undo.
fn unchanged(thread: &Thread) -> HandlerOutput {
    HandlerOutput {
        unchanged: true,
        ..result(thread)
    }
}

fn call(name: &str, input: serde_json::Value) -> Option<CommandCall> {
    Some(CommandCall {
        name: name.into(),
        input,
    })
}

/// `thread.create { stream, title, from? }`: a thread opens with no agent
/// session (`oxplow.agent_session.open` adds them).
pub fn create_op() -> Op {
    Op::new(
        "threads.write",
        "create",
        schema::<CreateInput>(),
        false,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: CreateInput = parse(input)?;
            let stream: StreamId = ref_id(
                &input.stream,
                oxplow_domain::uri::resources::STREAM,
                "/stream",
            )?;
            on_own_stream(ctx, stream)?;
            // Only a stream that's there — not archived — takes a thread.
            oxplow_db::stream_store::get_tx(ctx.conn, stream)
                .map_err(sql)?
                .filter(|s| s.archived_at.is_none())
                .ok_or_else(|| invalid("/stream", format!("no stream `{}`", input.stream)))?;
            if let Some(from) = &input.from {
                let source = load(
                    ctx,
                    ref_id(from, oxplow_domain::uri::resources::THREAD, "/from")?,
                    "/from",
                )?;
                if source.stream_id != stream {
                    return Err(invalid("/from", format!("`{from}` is on another stream")));
                }
            }
            let existing = list_for_stream_tx(ctx.conn, stream).map_err(sql)?;
            let next_sort = existing
                .iter()
                .filter(|t| t.is_open())
                .map(|t| t.sort_index)
                .max()
                .unwrap_or(-1)
                + 1;
            let now = Timestamp::now();
            let mut thread = Thread {
                id: ThreadId::placeholder(),
                stream_id: stream,
                title: input.title,
                sort_index: next_sort,
                summary: String::new(),
                summary_updated_at: None,
                closed_at: None,
                custom_prompt: None,
                created_at: now,
                updated_at: now,
                archived_at: None,
                mark: None,
            };
            thread.id = save(ctx, &thread)?;
            Ok(result(&thread))
        })),
    )
}

/// `thread.rename { thread, title }`; undone by renaming it back.
pub fn rename_op() -> Op {
    Op::new(
        "threads.write",
        "rename",
        schema::<RenameInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: RenameInput = parse(input)?;
            let mut thread = load(
                ctx,
                ref_id(
                    &input.thread,
                    oxplow_domain::uri::resources::THREAD,
                    "/thread",
                )?,
                "/thread",
            )?;
            on_own_stream(ctx, thread.stream_id)?;
            let before = std::mem::replace(&mut thread.title, input.title);
            thread.updated_at = Timestamp::now();
            save(ctx, &thread)?;
            Ok(HandlerOutput {
                inverse: call(RENAME, json!({ "thread": input.thread, "title": before })),
                ..result(&thread)
            })
        })),
    )
}

/// `thread.set_prompt { thread, prompt? }` — a person's: it steers the
/// thread's agent. Undone by setting it back.
pub fn set_prompt_op() -> Op {
    Op::new(
        "threads.write",
        "set_prompt",
        schema::<SetPromptInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: SetPromptInput = parse(input)?;
            let mut thread = load(
                ctx,
                ref_id(
                    &input.thread,
                    oxplow_domain::uri::resources::THREAD,
                    "/thread",
                )?,
                "/thread",
            )?;
            let before = std::mem::replace(
                &mut thread.custom_prompt,
                input.prompt.filter(|p| !p.trim().is_empty()),
            );
            thread.updated_at = Timestamp::now();
            save(ctx, &thread)?;
            Ok(HandlerOutput {
                inverse: call(
                    SET_PROMPT,
                    json!({ "thread": input.thread, "prompt": before }),
                ),
                ..result(&thread)
            })
        })),
    )
    .open_to(Invokers::NO_AGENT)
}

/// `thread.mark { thread, mark? }` — a person's: a colour on the thread's
/// tab, for grouping threads their own way. It changes nothing else.
/// Undone by setting the previous mark back; marking it what it already
/// is leaves no record.
pub fn mark_op() -> Op {
    Op::new(
        "threads.write",
        "mark",
        schema::<MarkInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: MarkInput = parse(input)?;
            let mut thread = load(
                ctx,
                ref_id(
                    &input.thread,
                    oxplow_domain::uri::resources::THREAD,
                    "/thread",
                )?,
                "/thread",
            )?;
            if thread.mark == input.mark {
                return Ok(unchanged(&thread));
            }
            let before = std::mem::replace(&mut thread.mark, input.mark);
            thread.updated_at = Timestamp::now();
            save(ctx, &thread)?;
            Ok(HandlerOutput {
                inverse: call(MARK, json!({ "thread": input.thread, "mark": before })),
                ..result(&thread)
            })
        })),
    )
    .open_to(Invokers::NO_AGENT)
}

/// `thread.close { thread }`: its open effort and its agent sessions close
/// in the same transaction (the effort by `system`, the sessions
/// `thread_closed`), and the sessions' processes stop once it commits. An agent closes only its own thread. Undone by reopening
/// it (the effort stays closed).
pub fn close_op(processes: crate::agent_sessions::SessionProcesses) -> Op {
    Op::new(
        "threads.write",
        "close",
        schema::<ThreadInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: ThreadInput = parse(input)?;
            let id = ref_id(
                &input.thread,
                oxplow_domain::uri::resources::THREAD,
                "/thread",
            )?;
            own_thread_only(ctx, id, "closes")?;
            let mut thread = load(ctx, id, "/thread")?;
            if !thread.is_open() {
                return Ok(unchanged(&thread));
            }
            let now = Timestamp::now();
            thread.closed_at = Some(now);
            thread.updated_at = now;
            save(ctx, &thread)?;
            // Its work ended with it: its open effort closes too.
            if let Some(effort) =
                oxplow_db::effort_store::open_for_thread_tx(ctx.conn, id).map_err(sql)?
            {
                oxplow_db::effort_store::finish_tx(
                    ctx.conn,
                    &ctx.events,
                    effort,
                    &oxplow_db::effort_store::EffortEnd::at(
                        now,
                        oxplow_db::effort_store::ClosedBy::System,
                    ),
                )
                .map_err(CommandError::from)?;
            }
            // Its sessions close with it, and their processes stop once the
            // close commits.
            let sessions: Vec<_> =
                oxplow_db::agent_session_store::list_open_for_thread_tx(ctx.conn, id)?
                    .into_iter()
                    .map(|s| s.id)
                    .collect();
            for session in &sessions {
                oxplow_db::agent_stores::close_session_tx(
                    ctx.conn,
                    &ctx.events,
                    *session,
                    oxplow_domain::agent_session::SessionCloseReason::ThreadClosed,
                    now,
                )?;
            }
            let processes = processes.clone();
            let after_commit: Option<Box<dyn FnOnce() + Send + Sync>> = Some(Box::new(move || {
                for session in sessions {
                    processes.kill(session);
                }
            }));
            Ok(HandlerOutput {
                inverse: call(REOPEN, json!({ "thread": input.thread })),
                after_commit,
                ..result(&thread)
            })
        })),
    )
}

/// `thread.reopen { thread }` → open again; undone by closing it.
pub fn reopen_op() -> Op {
    Op::new(
        "threads.write",
        "reopen",
        schema::<ThreadInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ThreadInput = parse(input)?;
            let id = ref_id(
                &input.thread,
                oxplow_domain::uri::resources::THREAD,
                "/thread",
            )?;
            own_thread_only(ctx, id, "reopens")?;
            let mut thread = load(ctx, id, "/thread")?;
            if thread.is_open() {
                return Ok(unchanged(&thread));
            }
            thread.closed_at = None;
            thread.updated_at = Timestamp::now();
            save(ctx, &thread)?;
            Ok(HandlerOutput {
                inverse: call(CLOSE, json!({ "thread": input.thread })),
                ..result(&thread)
            })
        })),
    )
}

/// The thread commands, for the bus.
pub fn ops(processes: crate::agent_sessions::SessionProcesses) -> Vec<Op> {
    vec![
        create_op(),
        rename_op(),
        set_prompt_op(),
        mark_op(),
        close_op(processes),
        reopen_op(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{services_with_effort, EffortFixture};
    use oxplow_domain::stores::ThreadStore as _;
    use oxplow_domain::Actor;

    /// Closing a thread closes its open effort: the thread's work ended.
    #[tokio::test]
    async fn closing_a_thread_closes_its_effort() {
        use oxplow_db::EffortStore as _;
        let fx = services_with_effort().await;
        fx.svc
            .commands
            .run(
                &Actor::Human,
                CLOSE,
                json!({ "thread": oxplow_domain::uri::build::thread_uri(fx.thread) }),
                false,
            )
            .await
            .unwrap();
        let effort = fx
            .svc
            .effort_store
            .get_effort(&fx.effort)
            .await
            .unwrap()
            .unwrap();
        assert!(effort.ended_at.is_some());
        assert_eq!(effort.closed_by.as_deref(), Some("system"));
    }

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
    ) -> Result<oxplow_domain::CommandOutcome, CommandError> {
        fx.svc.commands.run(actor, name, input, false).await
    }

    async fn thread(fx: &EffortFixture, id: ThreadId) -> Thread {
        fx.svc.thread_store.get(&id).await.unwrap().unwrap()
    }

    async fn create(fx: &EffortFixture, title: &str) -> ThreadId {
        let stream = thread(fx, fx.thread).await.stream_id;
        let out = run(
            fx,
            &Actor::Human,
            CREATE,
            json!({ "stream": stream_uri(stream), "title": title }),
        )
        .await
        .unwrap();
        serde_json::from_value::<Thread>(out.result).unwrap().id
    }

    /// A new thread opens at the end of its stream's order with no agent
    /// session; a fork copies none of its source's.
    #[tokio::test]
    async fn a_new_thread_opens_last_and_a_fork_copies_no_session() {
        use oxplow_domain::stores::AgentSessionStore as _;
        let fx = services_with_effort().await;
        // Only a stream that's there takes a thread.
        for stream in ["oxplow:oxplow/stream?id=str99"] {
            let err = run(
                &fx,
                &Actor::Human,
                CREATE,
                json!({ "stream": stream, "title": "x" }),
            )
            .await
            .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/stream"),
                "{stream}: {err:?}"
            );
        }
        let second = create(&fx, "second").await;
        let t = thread(&fx, second).await;
        assert_eq!((t.is_open(), t.sort_index), (true, 1));
        let fork = run(
            &fx,
            &agent(&fx),
            CREATE,
            json!({ "stream": stream_uri(t.stream_id), "title": "fork",
                    "from": thread_uri(fx.thread) }),
        )
        .await
        .unwrap();
        let fork: Thread = serde_json::from_value(fork.result).unwrap();
        assert_eq!((fork.is_open(), fork.sort_index), (true, 2));
        for id in [second, fork.id] {
            assert!(fx
                .svc
                .agent_session_store
                .list_open_for_thread(&id)
                .await
                .unwrap()
                .is_empty());
        }
    }

    /// A thread names no agent: its sessions do.
    #[tokio::test]
    async fn a_thread_names_no_agent() {
        let fx = services_with_effort().await;
        let stream = stream_uri(thread(&fx, fx.thread).await.stream_id);
        let err = run(
            &fx,
            &Actor::Human,
            CREATE,
            json!({ "stream": stream, "title": "x", "agent": "claude" }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("agent"), "{err}");
    }

    /// A mark is the person's colour for a thread's tab: set, changed,
    /// cleared, and undone to what it was. An agent may not mark.
    #[tokio::test]
    async fn a_mark_is_a_persons_and_undoes_to_what_it_was() {
        let fx = services_with_effort().await;
        let r = thread_uri(fx.thread);
        assert_eq!(thread(&fx, fx.thread).await.mark, None);
        run(
            &fx,
            &Actor::Human,
            MARK,
            json!({ "thread": r, "mark": "red" }),
        )
        .await
        .unwrap();
        let out = run(
            &fx,
            &Actor::Human,
            MARK,
            json!({ "thread": r, "mark": "violet" }),
        )
        .await
        .unwrap();
        assert_eq!(thread(&fx, fx.thread).await.mark, Some(ThreadMark::Violet));
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(thread(&fx, fx.thread).await.mark, Some(ThreadMark::Red));
        run(&fx, &Actor::Human, MARK, json!({ "thread": r }))
            .await
            .unwrap();
        assert_eq!(thread(&fx, fx.thread).await.mark, None);
        let err = run(
            &fx,
            &Actor::Human,
            MARK,
            json!({ "thread": r, "mark": "plaid" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
        let denied = run(
            &fx,
            &agent(&fx),
            MARK,
            json!({ "thread": r, "mark": "red" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(denied, CommandError::Denied { .. }), "{denied:?}");
    }

    /// Any open thread's agent may change the workspace — the stream's
    /// threads share it — and a closed thread's may not, until it reopens.
    #[tokio::test]
    async fn an_open_threads_agent_writes_and_a_closed_ones_is_refused() {
        let fx = services_with_effort().await;
        let second = create(&fx, "second").await;
        let as_second = Actor::Agent {
            session_id: None,
            thread_id: Some(second),
            stream_id: None,
        };
        let set = |v: i64| json!({ "key": "metricRetentionDays", "value": v });
        run(&fx, &agent(&fx), "oxplow.config.set", set(30))
            .await
            .unwrap();
        run(&fx, &as_second, "oxplow.config.set", set(31))
            .await
            .unwrap();
        run(
            &fx,
            &Actor::Human,
            CLOSE,
            json!({ "thread": thread_uri(second) }),
        )
        .await
        .unwrap();
        let err = run(&fx, &as_second, "oxplow.config.set", set(32))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Denied { reason } if reason.contains("is closed")),
            "{err:?}"
        );
    }

    /// Moving a thread to the state it's in changes nothing, so it leaves
    /// no record — no audit row, nothing to undo — like any such call.
    #[tokio::test]
    async fn a_move_to_the_state_a_thread_is_in_leaves_no_record() {
        let fx = services_with_effort().await;
        let second = create(&fx, "second").await;
        let open = json!({ "thread": thread_uri(second) });
        // An open thread reopened, an unmarked one unmarked.
        for (name, input) in [(REOPEN, &open), (MARK, &open)] {
            let out = run(&fx, &Actor::Human, name, input.clone()).await.unwrap();
            assert_eq!(out.audit_id, None, "{name}");
            assert!(out.inverse.is_none(), "{name}");
        }
        run(&fx, &Actor::Human, CLOSE, open.clone()).await.unwrap();
        let again = run(&fx, &Actor::Human, CLOSE, open).await.unwrap();
        assert_eq!(again.audit_id, None, "closing a closed thread");
    }

    /// An agent acts on its own stream only, and closes only its own
    /// thread; close and reopen undo each other.
    #[tokio::test]
    async fn an_agent_stays_on_its_own_stream_and_closes_and_reopens_only_its_own_thread() {
        let fx = services_with_effort().await;
        let second = create(&fx, "second").await;
        let err = run(
            &fx,
            &agent(&fx),
            CLOSE,
            json!({ "thread": thread_uri(second) }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let err = run(
            &fx,
            &agent(&fx),
            CREATE,
            json!({ "stream": "oxplow:oxplow/stream?id=str99", "title": "x" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");

        let closed = run(
            &fx,
            &Actor::Human,
            CLOSE,
            json!({ "thread": thread_uri(second) }),
        )
        .await
        .unwrap();
        let t = thread(&fx, second).await;
        assert!(!t.is_open());
        fx.svc
            .commands
            .undo(&Actor::Human, closed.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert!(thread(&fx, second).await.is_open());

        // tsk788: and reopens only its own thread, as it closes only its own.
        run(
            &fx,
            &Actor::Human,
            CLOSE,
            json!({ "thread": thread_uri(second) }),
        )
        .await
        .unwrap();
        let err = run(
            &fx,
            &agent(&fx),
            REOPEN,
            json!({ "thread": thread_uri(second) }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        assert!(!thread(&fx, second).await.is_open());
    }

    /// Rename and prompt undo to what they were; a prompt is a person's.
    #[tokio::test]
    async fn rename_and_prompt_undo_to_what_they_were() {
        let fx = services_with_effort().await;
        let r = thread_uri(fx.thread);
        let out = run(
            &fx,
            &agent(&fx),
            RENAME,
            json!({ "thread": r, "title": "Mine" }),
        )
        .await
        .unwrap();
        assert_eq!(thread(&fx, fx.thread).await.title, "Mine");
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(thread(&fx, fx.thread).await.title, "Thread");

        let denied = run(
            &fx,
            &agent(&fx),
            SET_PROMPT,
            json!({ "thread": r, "prompt": "x" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(denied, CommandError::Denied { .. }), "{denied:?}");
        run(
            &fx,
            &Actor::Human,
            SET_PROMPT,
            json!({ "thread": r, "prompt": "be brief" }),
        )
        .await
        .unwrap();
        assert_eq!(
            thread(&fx, fx.thread).await.custom_prompt.as_deref(),
            Some("be brief")
        );
        run(
            &fx,
            &Actor::Human,
            SET_PROMPT,
            json!({ "thread": r, "prompt": "" }),
        )
        .await
        .unwrap();
        assert_eq!(thread(&fx, fx.thread).await.custom_prompt, None); // A prompt that is only whitespace clears it, from any caller.
        run(
            &fx,
            &Actor::Human,
            SET_PROMPT,
            json!({ "thread": r, "prompt": "be brief" }),
        )
        .await
        .unwrap();
        run(
            &fx,
            &Actor::Human,
            SET_PROMPT,
            json!({ "thread": r, "prompt": " \n\t " }),
        )
        .await
        .unwrap();
        assert_eq!(thread(&fx, fx.thread).await.custom_prompt, None);
    }
}
