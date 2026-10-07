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
//! - **close** → `closed` (an ACP thread's session stops after commit);
//!   **reopen** → `queued`.
//!
//! A thread is named by ref (`thread:thr12`), a stream by `stream:str1`.
//! An agent acts only on its own stream, and closes only its own thread;
//! promoting a thread and setting a prompt steer an agent, so they are a
//! person's.

use std::str::FromStr;
use std::sync::{Arc, RwLock};

use oxplow_config::OxplowConfig;
use oxplow_db::thread_store::{get_tx, list_for_stream_tx, upsert_tx};
use oxplow_domain::refs::build::{stream_ref, thread_ref};
use oxplow_domain::{
    AgentKind, Atomicity, CommandCall, CommandEffect, CommandError, CommandSpec, Confirm, Invokers,
    Lifecycle, StreamId, Thread, ThreadId, ThreadStatus, Timestamp,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Command, Handler, HandlerOutput, TxCtx};

pub const CREATE: &str = "oxplow.thread.create";
pub const RENAME: &str = "oxplow.thread.rename";
pub const SET_PROMPT: &str = "oxplow.thread.set_prompt";
pub const PROMOTE: &str = "oxplow.thread.promote";
pub const DEMOTE: &str = "oxplow.thread.demote";
pub const CLOSE: &str = "oxplow.thread.close";
pub const REOPEN: &str = "oxplow.thread.reopen";
pub const REORDER: &str = "oxplow.thread.reorder";

/// A person, or a lens acting for one.
const PEOPLE: Invokers = Invokers {
    human: true,
    agent: false,
    lens: true,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    /// The stream (`stream:str1`).
    pub stream: String,
    pub title: String,
    /// Its agent harness (default: the project's first enabled agent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentKind>,
    /// For an `acp` thread, which ACP agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp_agent: Option<String>,
    /// Fork this thread (`thread:thr3`): the new one runs the same agent.
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

fn invalid(field: &str, message: String) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message,
    }
}

fn sql(e: rusqlite::Error) -> CommandError {
    CommandError::from(oxplow_db::map_sql_err(e))
}

fn parse<T: serde::de::DeserializeOwned>(input: serde_json::Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
}

/// The id a `<kind>:<id>` ref names.
fn id_of<T: FromStr>(value: &str, kind: &str, field: &str) -> Result<T, CommandError> {
    value
        .strip_prefix(&format!("{kind}:"))
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| invalid(field, format!("`{value}` isn't a {kind} ref ({kind}:<id>)")))
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
        Some(Some(own)) => {
            let thread =
                get_tx(ctx.conn, own)
                    .map_err(sql)?
                    .ok_or_else(|| CommandError::Denied {
                        reason: format!("the agent's thread `{}` doesn't exist", thread_ref(own)),
                    })?;
            Ok(Some((own, thread.stream_id)))
        }
    }
}

/// A `thread:<id>` ref named in an input.
pub(super) fn parse_thread_ref(value: &str) -> Result<ThreadId, CommandError> {
    value
        .strip_prefix("thread:")
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| {
            invalid(
                "/thread",
                format!("`{value}` isn't a thread ref (thread:<id>)"),
            )
        })
}

/// The thread an actor's record goes on — a note, a decision, a claim, a
/// test run: an agent's own (naming another, or having none, is refused),
/// else the one a person names. `own` is the agent's thread, when the
/// actor is one (`Actor::agent_thread`).
fn record_thread(
    own: Option<Option<ThreadId>>,
    named: Option<&str>,
) -> Result<ThreadId, CommandError> {
    let named = named.map(parse_thread_ref).transpose()?;
    match own {
        None => named.ok_or_else(|| invalid("/thread", "name the thread".into())),
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

fn call(name: &str, input: serde_json::Value) -> Option<CommandCall> {
    Some(CommandCall {
        name: name.into(),
        input,
    })
}

fn spec(
    name: &str,
    summary: &str,
    schema: serde_json::Value,
    invokers: Invokers,
    undoable: bool,
) -> CommandSpec {
    CommandSpec {
        id: name.into(),
        summary: summary.into(),
        input_schema: schema,
        invokers,
        confirm: Confirm::Never,
        undoable,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        effect: CommandEffect::Record,
        needs: Vec::new(),
    }
}

fn schema<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

/// `thread.create { stream, title, agent?, acp_agent?, from? }`.
pub fn create_command(config: Arc<RwLock<OxplowConfig>>) -> Command {
    Command::new(
        spec(
            CREATE,
            "Start a thread on a stream (`stream:str1`) — or fork one (`from: thread:thr3`), \
             running the same agent. It becomes the stream's writer when it has none, else \
             joins the queue.",
            schema::<CreateInput>(),
            Invokers::ALL,
            false,
        ),
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: CreateInput = parse(input)?;
            let stream: StreamId = id_of(&input.stream, "stream", "/stream")?;
            on_own_stream(ctx, stream)?;
            let config = crate::config_service::read_config(&config);
            let (agent, acp_agent) = match &input.from {
                Some(from) => {
                    if input.agent.is_some() || input.acp_agent.is_some() {
                        return Err(invalid(
                            "/from",
                            "a fork runs its source's agent; don't name one".into(),
                        ));
                    }
                    let source = load(ctx, id_of(from, "thread", "/from")?, "/from")?;
                    if source.stream_id != stream {
                        return Err(invalid("/from", format!("`{from}` is on another stream")));
                    }
                    (source.agent, source.acp_agent)
                }
                None => {
                    // Named or not, one rule for the default (tsk970).
                    let (default_agent, default_acp) = oxplow_config::default_thread_agent(&config);
                    let agent = input.agent.unwrap_or(default_agent);
                    let input_acp = input.acp_agent.or_else(|| {
                        (input.agent.is_none() && agent == AgentKind::Acp)
                            .then_some(default_acp)
                            .flatten()
                    });
                    if !config.agents.contains(&agent) {
                        return Err(invalid(
                            "/agent",
                            format!("agent `{}` isn't enabled for this project", agent.as_str()),
                        ));
                    }
                    let acp_agent = match (agent, input_acp) {
                        (AgentKind::Acp, Some(name)) => {
                            if crate::acp::agents::find(&config, &name).is_none() {
                                return Err(invalid(
                                    "/acp_agent",
                                    format!("no ACP agent named `{name}`"),
                                ));
                            }
                            Some(name)
                        }
                        (AgentKind::Acp, None) => {
                            return Err(invalid("/acp_agent", "an ACP thread needs one".into()))
                        }
                        (_, Some(_)) => {
                            return Err(invalid(
                                "/acp_agent",
                                "only an ACP thread names one".into(),
                            ))
                        }
                        (_, None) => None,
                    };
                    (agent, acp_agent)
                }
            };
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
                pane_target: "working".into(),
                agent,
                acp_agent,
                resume_session_id: String::new(),
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
    .expect("oxplow.thread.create is a valid command")
}

/// `thread.rename { thread, title }`; undone by renaming it back.
pub fn rename_command() -> Command {
    Command::new(
        spec(
            RENAME,
            "Rename a thread (`thread:thr12`).",
            schema::<RenameInput>(),
            Invokers::ALL,
            true,
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: RenameInput = parse(input)?;
            let mut thread = load(ctx, id_of(&input.thread, "thread", "/thread")?, "/thread")?;
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
    .expect("oxplow.thread.rename is a valid command")
}

/// `thread.set_prompt { thread, prompt? }` — a person's: it steers the
/// thread's agent. Undone by setting it back.
pub fn set_prompt_command() -> Command {
    Command::new(
        spec(
            SET_PROMPT,
            "Set (or clear) the text appended to a thread's agent prompt. A person's: it \
             steers the agent.",
            schema::<SetPromptInput>(),
            PEOPLE,
            true,
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: SetPromptInput = parse(input)?;
            let mut thread = load(ctx, id_of(&input.thread, "thread", "/thread")?, "/thread")?;
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
    .expect("oxplow.thread.set_prompt is a valid command")
}

/// `thread.promote { thread }` — a person's: the writer is who may change
/// the worktree. The current writer is demoted in the same transaction;
/// undone by promoting it back.
pub fn promote_command() -> Command {
    Command::new(
        spec(
            PROMOTE,
            "Make a thread its stream's writer (the one thread whose agent may change the \
             worktree); the current writer joins the queue. A person's.",
            schema::<ThreadInput>(),
            PEOPLE,
            true,
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ThreadInput = parse(input)?;
            let mut thread = load(ctx, id_of(&input.thread, "thread", "/thread")?, "/thread")?;
            match thread.status {
                ThreadStatus::Closed => {
                    return Err(invalid(
                        "/thread",
                        format!("`{}` is closed; reopen it first", input.thread),
                    ))
                }
                ThreadStatus::Active => return Ok(result(&thread)),
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
    .expect("oxplow.thread.promote is a valid command")
}

/// `thread.demote { thread }` — a person's: the stream's writer joins the
/// queue, leaving the stream with none. Undone by promoting it back; the
/// inverse of a promote onto a stream that had no writer.
pub fn demote_command() -> Command {
    Command::new(
        spec(
            DEMOTE,
            "Move a stream's writer back into the queue, leaving the stream with no writer. \
             A person's.",
            schema::<ThreadInput>(),
            PEOPLE,
            true,
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ThreadInput = parse(input)?;
            let mut thread = load(ctx, id_of(&input.thread, "thread", "/thread")?, "/thread")?;
            if thread.status != ThreadStatus::Active {
                return Ok(result(&thread));
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
    .expect("oxplow.thread.demote is a valid command")
}

/// `thread.close { thread }`: its open effort closes in the same
/// transaction (by `system`), and an ACP thread's session stops once the
/// close commits. An agent closes only its own thread. Undone by reopening
/// it (the effort stays closed).
pub fn close_command(acp: Arc<crate::acp::manager::AcpManager>) -> Command {
    Command::new(
        spec(
            CLOSE,
            "Close a thread (`thread:thr12`) — history, reopenable. Its open effort closes, \
             and an ACP thread's agent session stops with it. An agent closes only its own \
             thread.",
            schema::<ThreadInput>(),
            Invokers::ALL,
            true,
        ),
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: ThreadInput = parse(input)?;
            let id = id_of(&input.thread, "thread", "/thread")?;
            own_thread_only(ctx, id, "closes")?;
            let mut thread = load(ctx, id, "/thread")?;
            if thread.status == ThreadStatus::Closed {
                return Ok(result(&thread));
            }
            let now = Timestamp::now();
            thread.status = ThreadStatus::Closed;
            thread.closed_at = Some(now);
            thread.updated_at = now;
            save(ctx, &thread)?;
            // Its work ended with it: its open effort closes too.
            if let Some(effort) = oxplow_db::effort_store::open_for_thread_tx(ctx.conn, id)
                .map_err(|e| CommandError::Failed {
                    message: e.to_string(),
                })?
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
            let after_commit: Option<Box<dyn FnOnce() + Send + Sync>> =
                (thread.agent == AgentKind::Acp).then(|| {
                    let acp = acp.clone();
                    // Not open is fine: there's nothing to stop.
                    Box::new(move || {
                        let _ = acp.close(&id);
                    }) as Box<dyn FnOnce() + Send + Sync>
                });
            Ok(HandlerOutput {
                inverse: call(REOPEN, json!({ "thread": input.thread })),
                after_commit,
                ..result(&thread)
            })
        })),
    )
    .expect("oxplow.thread.close is a valid command")
}

/// `thread.reopen { thread }` → `queued`; undone by closing it.
pub fn reopen_command() -> Command {
    Command::new(
        spec(
            REOPEN,
            "Reopen a closed thread (`thread:thr12`); it joins its stream's queue. An agent \
             reopens only its own thread.",
            schema::<ThreadInput>(),
            Invokers::ALL,
            true,
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ThreadInput = parse(input)?;
            let id = id_of(&input.thread, "thread", "/thread")?;
            own_thread_only(ctx, id, "reopens")?;
            let mut thread = load(ctx, id, "/thread")?;
            if thread.status != ThreadStatus::Closed {
                return Ok(result(&thread));
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
    .expect("oxplow.thread.reopen is a valid command")
}

/// `thread.reorder { stream, order }`: the named threads take the positions
/// they're listed in. Undone by their previous order.
pub fn reorder_command() -> Command {
    Command::new(
        spec(
            REORDER,
            "Reorder a stream's threads (`order`: thread refs in their new order).",
            schema::<ReorderInput>(),
            PEOPLE,
            true,
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ReorderInput = parse(input)?;
            let stream: StreamId = id_of(&input.stream, "stream", "/stream")?;
            let mut threads = Vec::with_capacity(input.order.len());
            for (i, r) in input.order.iter().enumerate() {
                let field = format!("/order/{i}");
                let thread = load(ctx, id_of(r, "thread", &field)?, &field)?;
                if thread.stream_id != stream {
                    return Err(invalid(
                        &field,
                        format!("`{r}` isn't on `{}`", input.stream),
                    ));
                }
                threads.push(thread);
            }
            let mut before: Vec<&Thread> = threads.iter().collect();
            before.sort_by_key(|t| (t.sort_index, t.created_at));
            let previous: Vec<String> = before.iter().map(|t| thread_ref(t.id)).collect();
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
    .expect("oxplow.thread.reorder is a valid command")
}

/// The thread commands, for the bus.
pub fn commands(
    config: Arc<RwLock<OxplowConfig>>,
    acp: Arc<crate::acp::manager::AcpManager>,
) -> Vec<Command> {
    vec![
        create_command(config),
        rename_command(),
        set_prompt_command(),
        promote_command(),
        demote_command(),
        close_command(acp),
        reopen_command(),
        reorder_command(),
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

    /// A new thread joins the queue behind the stream's writer; a fork runs
    /// its source's agent.
    #[tokio::test]
    async fn a_new_thread_queues_behind_the_writer_and_a_fork_runs_its_sources_agent() {
        let fx = services_with_effort().await;
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
        assert_eq!(fork.agent, thread(&fx, fx.thread).await.agent);
        assert_eq!(fork.status, ThreadStatus::Queued);
    }

    /// Promoting demotes the writer in the same run; undoing it promotes
    /// the old writer back. An agent may not promote.
    #[tokio::test]
    async fn promote_is_one_run_undoable_and_a_persons() {
        let fx = services_with_effort().await;
        let second = create(&fx, "second").await;
        let denied = run(
            &fx,
            &agent(&fx),
            PROMOTE,
            json!({ "thread": thread_ref(second) }),
        )
        .await
        .unwrap_err();
        assert!(matches!(denied, CommandError::Denied { .. }), "{denied:?}");
        let out = run(
            &fx,
            &Actor::Human,
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

    /// tsk787: promoting onto a stream with no writer undoes by demoting
    /// it back; `oxplow.thread.demote` is a person's, and undoes by promoting.
    #[tokio::test]
    async fn promote_from_no_writer_undoes_by_demoting() {
        let fx = services_with_effort().await;
        let me = thread_ref(fx.thread);
        let denied = run(&fx, &agent(&fx), DEMOTE, json!({ "thread": me }))
            .await
            .unwrap_err();
        assert!(matches!(denied, CommandError::Denied { .. }), "{denied:?}");
        let demoted = run(&fx, &Actor::Human, DEMOTE, json!({ "thread": me }))
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
    }

    /// An ACP thread names a known ACP agent, and only an ACP thread names
    /// one; a fork of it runs the same ACP agent.
    #[tokio::test]
    async fn an_acp_thread_names_a_known_acp_agent_and_its_fork_keeps_it() {
        let fx = services_with_effort().await;
        fx.svc.config.write().unwrap().agents.push(AgentKind::Acp);
        let stream = stream_ref(thread(&fx, fx.thread).await.stream_id);
        for (input, field) in [
            (
                json!({ "stream": stream, "title": "x", "agent": "acp" }),
                "/acp_agent",
            ),
            (
                json!({ "stream": stream, "title": "x", "agent": "acp", "acp_agent": "nope" }),
                "/acp_agent",
            ),
            (
                json!({ "stream": stream, "title": "x", "agent": "claude", "acp_agent": "gemini" }),
                "/acp_agent",
            ),
        ] {
            let err = run(&fx, &Actor::Human, CREATE, input).await.unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == field),
                "{err:?}"
            );
        }
        let acp = run(
            &fx,
            &Actor::Human,
            CREATE,
            json!({ "stream": stream, "title": "g", "agent": "acp", "acp_agent": "gemini" }),
        )
        .await
        .unwrap();
        let acp: Thread = serde_json::from_value(acp.result).unwrap();
        let fork = run(
            &fx,
            &Actor::Human,
            CREATE,
            json!({ "stream": stream, "title": "fork", "from": thread_ref(acp.id) }),
        )
        .await
        .unwrap();
        let fork: Thread = serde_json::from_value(fork.result).unwrap();
        assert_eq!(
            (fork.agent, fork.acp_agent.as_deref()),
            (AgentKind::Acp, Some("gemini"))
        );
    }
}
