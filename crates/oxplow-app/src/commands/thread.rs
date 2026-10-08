//! Thread commands (P8.A3, `.context/commands.md`): a thread's lifecycle
//! — create (or fork), rename, its custom prompt, promote to its stream's
//! writer, close and reopen, the queue's order — as `Tx` commands, so
//! every surface (the desktop, an agent, a lens) goes through the same
//! validation, policy, audit and undo. The state machine:
//!
//! - **create** → `active` when the stream has no writer, else `queued`
//!   at the end of the queue;
//! - **promote** → `active`, demoting the current writer to `queued` in
//!   the same transaction (the partial unique index never trips);
//! - **close** → `closed` (its agent sessions close too, and their
//!   processes stop after commit);
//!   **reopen** → `queued`.
//!
//! A thread is named by ref (`thread:thr12`), a stream by `stream:str1`.
//! An agent acts only on its own stream, and closes only its own thread;
//! promoting a thread and setting a prompt steer an agent, so they are a
//! person's.

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_db::thread_store::{get_tx, list_for_stream_tx, upsert_tx};
use oxplow_domain::refs::build::{stream_ref, thread_ref};
use oxplow_domain::{
    CommandCall, CommandError, Invokers, StreamId, Thread, ThreadId, ThreadStatus, Timestamp,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::util::{invalid, parse, ref_id, schema, sql};
use super::{Handler, HandlerOutput, TxCtx};

pub const CREATE: &str = "oxplow.thread.create";
pub const RENAME: &str = "oxplow.thread.rename";
pub const SET_PROMPT: &str = "oxplow.thread.set_prompt";
pub const PROMOTE: &str = "oxplow.thread.promote";
pub const DEMOTE: &str = "oxplow.thread.demote";
pub const CLOSE: &str = "oxplow.thread.close";
pub const REOPEN: &str = "oxplow.thread.reopen";
pub const REORDER: &str = "oxplow.thread.reorder";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    /// The stream (`stream:str1`).
    pub stream: String,
    pub title: String,
    /// Fork this thread (`thread:thr3`): the new one joins its stream.
    /// Nothing agent-ish is copied — a thread's agents are its sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    /// The thread (`thread:thr12`).
    pub thread: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPromptInput {
    /// The thread (`thread:thr12`).
    pub thread: String,
    /// Appended to the agent's prompt; empty or absent clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadInput {
    /// The thread (`thread:thr12`).
    pub thread: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReorderInput {
    /// The stream (`stream:str1`).
    pub stream: String,
    /// Its threads in their new order (`thread:thr12`, …).
    pub order: Vec<String>,
}

fn load(ctx: &TxCtx<'_>, id: ThreadId, field: &str) -> Result<Thread, CommandError> {
    get_tx(ctx.conn, id)
        .map_err(sql)?
        .filter(|t| t.archived_at.is_none())
        .ok_or_else(|| invalid(field, format!("no thread `{}`", thread_ref(id))))
}

fn save(ctx: &TxCtx<'_>, thread: &Thread) -> Result<ThreadId, CommandError> {
    upsert_tx(ctx.conn, thread).map_err(sql)
}

/// The agent's own thread and stream, when the run is an agent's.
pub(super) fn agent_scope(ctx: &TxCtx<'_>) -> Result<Option<(ThreadId, StreamId)>, CommandError> {
    match ctx.actor.agent_thread() {
        None => Ok(None),
        Some(None) => Err(CommandError::Denied {
            reason: "an agent without a thread can't change threads".into(),
        }),
        // The bus resolved the agent's stream from its thread.
        Some(Some(own)) => match ctx.actor.stream_id() {
            Some(stream) => Ok(Some((own, stream))),
            None => Err(CommandError::Denied {
                reason: format!("the agent's thread `{}` has no stream", thread_ref(own)),
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
    let named = named.map(|t| ref_id(t, "thread", "/thread")).transpose()?;
    match own {
        None => named.ok_or_else(|| invalid("/thread", "name the thread")),
        Some(None) => Err(CommandError::Denied {
            reason: "an agent without a thread can't record on one".into(),
        }),
        Some(Some(own)) => match named {
            Some(t) if t != own => Err(CommandError::Denied {
                reason: format!(
                    "an agent writes only in its own thread (`{}`)",
                    thread_ref(own)
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
                thread_ref(own)
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
                stream_ref(own)
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
            let stream: StreamId = ref_id(&input.stream, "stream", "/stream")?;
            on_own_stream(ctx, stream)?;
            // Only a stream that's there — not archived — takes a thread.
            oxplow_db::stream_store::get_tx(ctx.conn, stream)
                .map_err(sql)?
                .filter(|s| s.archived_at.is_none())
                .ok_or_else(|| invalid("/stream", format!("no stream `{}`", input.stream)))?;
            if let Some(from) = &input.from {
                let source = load(ctx, ref_id(from, "thread", "/from")?, "/from")?;
                if source.stream_id != stream {
                    return Err(invalid("/from", format!("`{from}` is on another stream")));
                }
            }
            let existing = list_for_stream_tx(ctx.conn, stream).map_err(sql)?;
            let has_writer = existing.iter().any(|t| t.status == ThreadStatus::Active);
            let next_sort = existing
                .iter()
                .filter(|t| t.status != ThreadStatus::Closed)
                .map(|t| t.sort_index)
                .max()
                .unwrap_or(-1)
                + 1;
            let now = Timestamp::now();
            let mut thread = Thread {
                id: ThreadId::placeholder(),
                stream_id: stream,
                title: input.title,
                status: if has_writer {
                    ThreadStatus::Queued
                } else {
                    ThreadStatus::Active
                },
                sort_index: next_sort,
                summary: String::new(),
                summary_updated_at: None,
                closed_at: None,
                custom_prompt: None,
                created_at: now,
                updated_at: now,
                archived_at: None,
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
            let mut thread = load(ctx, ref_id(&input.thread, "thread", "/thread")?, "/thread")?;
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
            let mut thread = load(ctx, ref_id(&input.thread, "thread", "/thread")?, "/thread")?;
            let before = std::mem::replace(
                &mut thread.custom_prompt,
                input.prompt.filter(|p| !p.is_empty()),
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

/// `thread.promote { thread }` — a person's or an agent's: the writer is
/// who may change the worktree, and handing it to the thread that should
/// write next is part of an agent's work. The current writer is demoted in
/// the same transaction; undone by promoting it back.
pub fn promote_op() -> Op {
    Op::new(
        "threads.write",
        "promote",
        schema::<ThreadInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ThreadInput = parse(input)?;
            let mut thread = load(ctx, ref_id(&input.thread, "thread", "/thread")?, "/thread")?;
            match thread.status {
                ThreadStatus::Closed => {
                    return Err(invalid(
                        "/thread",
                        format!("`{}` is closed; reopen it first", input.thread),
                    ))
                }
                ThreadStatus::Active => return Ok(unchanged(&thread)),
                ThreadStatus::Queued => {}
            }
            let now = Timestamp::now();
            let mut before = None;
            for mut writer in list_for_stream_tx(ctx.conn, thread.stream_id)
                .map_err(sql)?
                .into_iter()
                .filter(|t| t.status == ThreadStatus::Active)
            {
                before = Some(writer.id);
                writer.status = ThreadStatus::Queued;
                writer.updated_at = now;
                save(ctx, &writer)?;
            }
            thread.status = ThreadStatus::Active;
            thread.updated_at = now;
            save(ctx, &thread)?;
            // Undone by promoting the demoted writer back — or, when the
            // stream had none, by demoting this one (tsk787).
            let inverse = match before {
                Some(w) => call(PROMOTE, json!({ "thread": thread_ref(w) })),
                None => call(DEMOTE, json!({ "thread": input.thread })),
            };
            Ok(HandlerOutput {
                inverse,
                ..result(&thread)
            })
        })),
    )
}

/// `thread.demote { thread }` — a person's or an agent's (handing the
/// worktree back): the stream's writer joins the queue, leaving the stream
/// with none. Undone by promoting it back; the
/// inverse of a promote onto a stream that had no writer.
pub fn demote_op() -> Op {
    Op::new(
        "threads.write",
        "demote",
        schema::<ThreadInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ThreadInput = parse(input)?;
            let mut thread = load(ctx, ref_id(&input.thread, "thread", "/thread")?, "/thread")?;
            if thread.status != ThreadStatus::Active {
                return Ok(unchanged(&thread));
            }
            thread.status = ThreadStatus::Queued;
            thread.updated_at = Timestamp::now();
            save(ctx, &thread)?;
            Ok(HandlerOutput {
                inverse: call(PROMOTE, json!({ "thread": input.thread })),
                ..result(&thread)
            })
        })),
    )
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
            let id = ref_id(&input.thread, "thread", "/thread")?;
            own_thread_only(ctx, id, "closes")?;
            let mut thread = load(ctx, id, "/thread")?;
            if thread.status == ThreadStatus::Closed {
                return Ok(unchanged(&thread));
            }
            let now = Timestamp::now();
            thread.status = ThreadStatus::Closed;
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

/// `thread.reopen { thread }` → `queued`; undone by closing it.
pub fn reopen_op() -> Op {
    Op::new(
        "threads.write",
        "reopen",
        schema::<ThreadInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ThreadInput = parse(input)?;
            let id = ref_id(&input.thread, "thread", "/thread")?;
            own_thread_only(ctx, id, "reopens")?;
            let mut thread = load(ctx, id, "/thread")?;
            if thread.status != ThreadStatus::Closed {
                return Ok(unchanged(&thread));
            }
            thread.status = ThreadStatus::Queued;
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

/// `thread.reorder { stream, order }`: the stream's threads take the
/// positions `order` lists them in — every one of them, once. Undone by
/// their previous order.
pub fn reorder_op() -> Op {
    Op::new(
        "threads.write",
        "reorder",
        schema::<ReorderInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ReorderInput = parse(input)?;
            let stream: StreamId = ref_id(&input.stream, "stream", "/stream")?;
            let mut threads = Vec::with_capacity(input.order.len());
            for (i, r) in input.order.iter().enumerate() {
                let field = format!("/order/{i}");
                let thread = load(ctx, ref_id(r, "thread", &field)?, &field)?;
                if thread.stream_id != stream {
                    return Err(invalid(
                        &field,
                        format!("`{r}` isn't on `{}`", input.stream),
                    ));
                }
                if threads.iter().any(|t: &Thread| t.id == thread.id) {
                    return Err(invalid(
                        "/order",
                        format!("`order` names `{r}` twice; it names each thread once"),
                    ));
                }
                threads.push(thread);
            }
            // The stream's whole order, so every thread has its place and
            // the undo puts each back where it was.
            let listed = list_for_stream_tx(ctx.conn, stream).map_err(sql)?;
            let missing: Vec<String> = listed
                .iter()
                .filter(|t| !threads.iter().any(|n| n.id == t.id))
                .map(|t| format!("`{}`", thread_ref(t.id)))
                .collect();
            if !missing.is_empty() {
                return Err(invalid(
                    "/order",
                    format!(
                        "`order` leaves out {}: it names every thread of `{}` once",
                        missing.join(", "),
                        input.stream
                    ),
                ));
            }
            let previous: Vec<String> = listed.iter().map(|t| thread_ref(t.id)).collect();
            let now = Timestamp::now();
            for (i, mut thread) in threads.clone().into_iter().enumerate() {
                thread.sort_index = i as i64;
                thread.updated_at = now;
                save(ctx, &thread)?;
            }
            Ok(HandlerOutput {
                result: json!({ "stream": input.stream, "order": input.order }),
                inverse: call(
                    REORDER,
                    json!({ "stream": input.stream, "order": previous }),
                ),
                ..HandlerOutput::default()
            })
        })),
    )
    .open_to(Invokers::NO_AGENT)
}

/// The thread commands, for the bus.
pub fn ops(processes: crate::agent_sessions::SessionProcesses) -> Vec<Op> {
    vec![
        create_op(),
        rename_op(),
        set_prompt_op(),
        promote_op(),
        demote_op(),
        close_op(processes),
        reopen_op(),
        reorder_op(),
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
                json!({ "thread": oxplow_domain::refs::build::thread_ref(fx.thread) }),
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
            json!({ "stream": stream_ref(stream), "title": title }),
        )
        .await
        .unwrap();
        serde_json::from_value::<Thread>(out.result).unwrap().id
    }

    /// A new thread joins the queue behind the stream's writer, with no
    /// agent session; a fork copies none of its source's.
    #[tokio::test]
    async fn a_new_thread_queues_behind_the_writer_and_a_fork_copies_no_session() {
        use oxplow_domain::stores::AgentSessionStore as _;
        let fx = services_with_effort().await;
        // Only a stream that's there takes a thread.
        for stream in ["stream:str99"] {
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
        assert_eq!((t.status, t.sort_index), (ThreadStatus::Queued, 1));
        let fork = run(
            &fx,
            &agent(&fx),
            CREATE,
            json!({ "stream": stream_ref(t.stream_id), "title": "fork",
                    "from": thread_ref(fx.thread) }),
        )
        .await
        .unwrap();
        let fork: Thread = serde_json::from_value(fork.result).unwrap();
        assert_eq!(fork.status, ThreadStatus::Queued);
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
        let stream = stream_ref(thread(&fx, fx.thread).await.stream_id);
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

    /// Promoting demotes the writer in the same run; undoing it promotes
    /// the old writer back. An agent may promote too: handing the worktree
    /// to the thread that should write next is part of its work.
    #[tokio::test]
    async fn promote_is_one_run_and_undoable_by_a_person_or_an_agent() {
        let fx = services_with_effort().await;
        let second = create(&fx, "second").await;
        let out = run(
            &fx,
            &agent(&fx),
            PROMOTE,
            json!({ "thread": thread_ref(second) }),
        )
        .await
        .unwrap();
        assert_eq!(thread(&fx, second).await.status, ThreadStatus::Active);
        assert_eq!(thread(&fx, fx.thread).await.status, ThreadStatus::Queued);
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(thread(&fx, fx.thread).await.status, ThreadStatus::Active);
        assert_eq!(thread(&fx, second).await.status, ThreadStatus::Queued);
    }

    /// Moving a thread to the state it's in changes nothing, so it leaves
    /// no record — no audit row, nothing to undo — like any such call.
    #[tokio::test]
    async fn a_move_to_the_state_a_thread_is_in_leaves_no_record() {
        let fx = services_with_effort().await;
        let me = json!({ "thread": thread_ref(fx.thread) });
        let second = create(&fx, "second").await;
        let queued = json!({ "thread": thread_ref(second) });
        // The writer promoted, a queued thread demoted or reopened.
        for (name, input) in [(PROMOTE, &me), (DEMOTE, &queued), (REOPEN, &queued)] {
            let out = run(&fx, &Actor::Human, name, input.clone()).await.unwrap();
            assert_eq!(out.audit_id, None, "{name}");
            assert!(out.inverse.is_none(), "{name}");
        }
        run(&fx, &Actor::Human, CLOSE, queued.clone())
            .await
            .unwrap();
        let again = run(&fx, &Actor::Human, CLOSE, queued).await.unwrap();
        assert_eq!(again.audit_id, None, "closing a closed thread");
    }

    /// tsk787: promoting onto a stream with no writer undoes by demoting
    /// it back; `oxplow.thread.demote` (anyone's: an agent hands the
    /// worktree back when it's done) undoes by promoting.
    #[tokio::test]
    async fn promote_from_no_writer_undoes_by_demoting() {
        let fx = services_with_effort().await;
        let me = thread_ref(fx.thread);
        let demoted = run(&fx, &agent(&fx), DEMOTE, json!({ "thread": me }))
            .await
            .unwrap();
        assert_eq!(thread(&fx, fx.thread).await.status, ThreadStatus::Queued);
        let promoted = run(&fx, &Actor::Human, PROMOTE, json!({ "thread": me }))
            .await
            .unwrap();
        assert_eq!(thread(&fx, fx.thread).await.status, ThreadStatus::Active);
        fx.svc
            .commands
            .undo(&Actor::Human, promoted.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(thread(&fx, fx.thread).await.status, ThreadStatus::Queued);
        let _ = demoted;
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
            json!({ "thread": thread_ref(second) }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let err = run(
            &fx,
            &agent(&fx),
            CREATE,
            json!({ "stream": "stream:str99", "title": "x" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");

        let closed = run(
            &fx,
            &Actor::Human,
            CLOSE,
            json!({ "thread": thread_ref(second) }),
        )
        .await
        .unwrap();
        let t = thread(&fx, second).await;
        assert_eq!(t.status, ThreadStatus::Closed);
        assert!(t.closed_at.is_some());
        fx.svc
            .commands
            .undo(&Actor::Human, closed.audit_id.unwrap(), false)
            .await
            .unwrap();
        let t = thread(&fx, second).await;
        assert_eq!((t.status, t.closed_at), (ThreadStatus::Queued, None));

        // tsk788: and reopens only its own thread, as it closes only its own.
        run(
            &fx,
            &Actor::Human,
            CLOSE,
            json!({ "thread": thread_ref(second) }),
        )
        .await
        .unwrap();
        let err = run(
            &fx,
            &agent(&fx),
            REOPEN,
            json!({ "thread": thread_ref(second) }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        assert_eq!(thread(&fx, second).await.status, ThreadStatus::Closed);
    }

    /// Rename and prompt undo to what they were; a prompt is a person's.
    #[tokio::test]
    async fn rename_and_prompt_undo_to_what_they_were() {
        let fx = services_with_effort().await;
        let r = thread_ref(fx.thread);
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
        assert_eq!(thread(&fx, fx.thread).await.custom_prompt, None);
    }

    /// Reorder rewrites the named threads' positions and undoes to the old
    /// order; a thread of another stream is refused at its position.
    #[tokio::test]
    async fn reorder_rewrites_positions_and_undoes() {
        let fx = services_with_effort().await;
        let b = create(&fx, "b").await;
        let c = create(&fx, "c").await;
        let stream = stream_ref(thread(&fx, fx.thread).await.stream_id);
        let order = [c, b, fx.thread].map(thread_ref);
        let out = run(
            &fx,
            &Actor::Human,
            REORDER,
            json!({ "stream": stream, "order": order }),
        )
        .await
        .unwrap();
        let listed: Vec<ThreadId> = fx
            .svc
            .threads
            .list_for_stream(&thread(&fx, fx.thread).await.stream_id)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(listed, [c, b, fx.thread]);
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        let listed: Vec<ThreadId> = fx
            .svc
            .threads
            .list_for_stream(&thread(&fx, fx.thread).await.stream_id)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(listed, [fx.thread, b, c]);
        let err = run(
            &fx,
            &Actor::Human,
            REORDER,
            json!({ "stream": "stream:str99", "order": [thread_ref(b)] }),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/order/0"),
            "{err:?}"
        );
        // The order is the stream's whole order: each of its threads once,
        // so the undo puts every one back where it was.
        for (order, says) in [
            (vec![thread_ref(c), thread_ref(b)], "leaves out"),
            (
                vec![
                    thread_ref(c),
                    thread_ref(c),
                    thread_ref(b),
                    thread_ref(fx.thread),
                ],
                "names `thread:thr",
            ),
        ] {
            let err = run(
                &fx,
                &Actor::Human,
                REORDER,
                json!({ "stream": stream, "order": order }),
            )
            .await
            .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), message }
                    if f == "/order" && message.contains(says)),
                "{err:?}"
            );
        }
    }
}
