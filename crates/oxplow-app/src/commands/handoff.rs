//! Handoff commands: work handed from one thread to another, as a record
//! oxplow keeps — never a message it delivers (`.context/agent-model.md`
//! "Handoffs and native messaging"). `Tx` over `oxplow_db::handoff_store`.
//!
//! - `oxplow.handoff.send { to_thread, body, work_item?, from_thread? }`:
//!   from the actor's thread (an agent's own, an effect's event's, the one
//!   a person names) to another open thread, any stream's. Logs
//!   `handoff.sent@1`. The result carries `link_warnings`.
//! - `oxplow.handoff.settle { handoff, state, note? }`: the record of how
//!   one ended. `taken` / `declined` are the recipient's, `withdrawn` the
//!   sender's, each only while open; `open` again undoes either. Logs
//!   `handoff.settled@1`; undoes to the state it replaced.
//!
//! - `oxplow.handoff.resolve { handoff, outcome, note? }`: how the
//!   recipient takes or declines one. A composite: taking a handoff that
//!   hands over a work item moves the item onto the recipient thread
//!   (`oxplow.work_item.move`, when the work list keeps per-thread lists)
//!   and then settles it. Who may is checked before anything is composed,
//!   so a stranger's run moves nothing. A work list's verbs run outside
//!   the transaction, so a take that moves an item is steps and has no
//!   undo; the recipient reopens it with `settle` instead.
//!
//! Nothing here reaches an agent. The recipient thread's agent is given
//! an open handoff at its next prompt (`agent_context`), and the person
//! sees it at once.

use std::sync::Arc;

use oxplow_db::handoff_store::{handoff_tx, send_tx, settle_tx, Handoff, HandoffState, NewHandoff};
use oxplow_domain::run::RunContext;
use oxplow_domain::uri::build::{handoff_uri, thread_uri};
use oxplow_domain::work_items::WorkItemsRegistry;
use oxplow_domain::{Actor, CommandCall, CommandError, HandoffId, ThreadId};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::comment::author_of;
use super::compose::{Compose, Composer, Composing, Composition};
use super::ops::Op;
use super::thread::acting_thread;
use super::util::{bare_id, invalid, parse, schema, sql};
use super::work_item::with_loose_refs;
use super::{Handler, HandlerOutput, TxCtx};
use crate::link_check::LinkDeps;

pub const SEND: &str = "oxplow.handoff.send";
pub const SETTLE: &str = "oxplow.handoff.settle";
pub const RESOLVE: &str = "oxplow.handoff.resolve";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendInput {
    /// The thread it is for (`thr3`): an open thread, not the
    /// sender's own (a note is for that). Any stream's.
    pub to_thread: String,
    /// What is handed over and what the recipient needs to know (markdown;
    /// `[[…]]` links are checked).
    #[schemars(extend("contentMediaType" = "text/markdown"))]
    pub body: String,
    /// The work item it hands over (`oxplow:oxplow/work_item?id=tsk12&provider=oxplow`, or a loose
    /// id): taking the handoff moves it onto the recipient thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item: Option<String>,
    /// The sending thread; an agent's is always its own, so only a person
    /// names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_thread: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SettleState {
    Open,
    Taken,
    Declined,
    Withdrawn,
}

impl From<SettleState> for HandoffState {
    fn from(s: SettleState) -> Self {
        match s {
            SettleState::Open => HandoffState::Open,
            SettleState::Taken => HandoffState::Taken,
            SettleState::Declined => HandoffState::Declined,
            SettleState::Withdrawn => HandoffState::Withdrawn,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SettleInput {
    /// The handoff (`hnd3`).
    pub handoff: String,
    /// `taken` or `declined` (its recipient), `withdrawn` (its sender),
    /// `open` (undoing either).
    pub state: SettleState,
    /// Why, for the other side to read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Taken,
    Declined,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolveInput {
    /// The handoff to your thread (`hnd3`).
    pub handoff: String,
    /// `taken` moves its work item onto your thread; `declined` leaves it.
    pub outcome: Outcome,
    /// Why, for its sender to read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The thread a handoff is sent from: an agent's own (naming another is
/// refused), an effect's event's, else the one a person names.
fn sending_thread(ctx: &TxCtx<'_>, named: Option<&str>) -> Result<ThreadId, CommandError> {
    if let Actor::Effect { thread_id, .. } = ctx.actor() {
        let own = thread_id.ok_or_else(|| CommandError::Denied {
            reason: "an effect whose event has no thread can't hand work off".into(),
        })?;
        if let Some(named) = named {
            if bare_id::<ThreadId>(named, oxplow_domain::uri::resources::THREAD, "/from_thread")?
                != own
            {
                return Err(CommandError::Denied {
                    reason: format!(
                        "an effect hands off only from its event's thread (`{}`)",
                        thread_uri(own)
                    ),
                });
            }
        }
        return Ok(own);
    }
    acting_thread(ctx, named).map_err(|e| match e {
        CommandError::Invalid { message, .. } => invalid("/from_thread", message),
        e => e,
    })
}

/// Whether thread `id` exists and is open.
fn open_thread(ctx: &TxCtx<'_>, id: ThreadId) -> Result<bool, CommandError> {
    Ok(ctx
        .conn
        .query_row(
            "SELECT closed_at IS NULL FROM threads WHERE id = ?1",
            [id.value()],
            |r| r.get::<_, bool>(0),
        )
        .optional()
        .map_err(sql)?
        .unwrap_or(false))
}

/// The thread an agent-driven actor (or an effect) acts for; `None` for a
/// person, who may act on either side.
fn acting_for(actor: &Actor) -> Option<Option<ThreadId>> {
    match actor {
        Actor::Effect { thread_id, .. } => Some(*thread_id),
        other => other.agent_thread(),
    }
}

/// `handoff.send { to_thread, body, work_item?, from_thread? }`.
pub fn send_op(deps: LinkDeps, work_items: WorkItemsRegistry) -> Op {
    Op::new(
        "handoffs.write",
        "send",
        schema::<SendInput>(),
        false,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: SendInput = parse(with_loose_refs(&work_items, input)?)?;
            let from = sending_thread(ctx, input.from_thread.as_deref())?;
            let to: ThreadId = bare_id(
                &input.to_thread,
                oxplow_domain::uri::resources::THREAD,
                "/to_thread",
            )?;
            if to == from {
                return Err(invalid(
                    "/to_thread",
                    "that's the sending thread: a note (`oxplow.knowledge.add_note`) is for that",
                ));
            }
            if !open_thread(ctx, to)? {
                return Err(invalid(
                    "/to_thread",
                    format!("`{}` isn't an open thread", input.to_thread),
                ));
            }
            if input.body.trim().is_empty() {
                return Err(invalid("/body", "say what is handed over"));
            }
            if let Some(item) = &input.work_item {
                let known = ctx
                    .conn
                    .query_row("SELECT 1 FROM v_work_item WHERE ref = ?1", [item], |_| {
                        Ok(())
                    })
                    .optional()
                    .map_err(sql)?
                    .is_some();
                if !known {
                    return Err(invalid(
                        "/work_item",
                        format!("`{item}` isn't an item on the active work list"),
                    ));
                }
            }
            let (handoff, event) = send_tx(
                ctx.conn,
                &ctx.events.vocabulary.resources,
                NewHandoff {
                    from_thread: from,
                    from_session: ctx.actor().session_id(),
                    from_actor: author_of(ctx.actor()),
                    to_thread: to,
                    work_item: input.work_item.as_deref(),
                    body: &input.body,
                },
            )?;
            Ok(HandlerOutput {
                result: json!({
                    "handoff": shown(&handoff),
                    "link_warnings": deps.warnings(ctx, &input.body, Some(from)),
                }),
                events: vec![event],
                ..HandlerOutput::default()
            })
        })),
    )
}

/// Whether `actor` may move a handoff from its state to `to`: the
/// recipient takes or declines, the sender withdraws, each only while it
/// is open; whoever settled it may open it again. A person may do any.
fn may_settle(actor: &Actor, h: &Handoff, to: HandoffState) -> Result<(), CommandError> {
    use HandoffState::*;
    let recipient = (h.to_thread_id, "its recipient thread");
    let sender = (h.from_thread_id, "its sending thread");
    let (side, verb) = match (h.state, to) {
        (Open, Taken | Declined) => (recipient, "takes or declines"),
        (Open, Withdrawn) => (sender, "withdraws"),
        (Taken | Declined, Open) => (recipient, "reopens"),
        (Withdrawn, Open) => (sender, "reopens"),
        (from, to) => {
            return Err(invalid(
                "/state",
                format!(
                    "`{}` is {}: it can't become {}",
                    handoff_uri(h.id),
                    from.as_str(),
                    to.as_str()
                ),
            ))
        }
    };
    match acting_for(actor) {
        None => Ok(()),
        Some(Some(own)) if own == side.0 => Ok(()),
        Some(_) => Err(CommandError::Denied {
            reason: format!(
                "only {} (`{}`) {verb} `{}`",
                side.1,
                thread_uri(side.0),
                handoff_uri(h.id)
            ),
        }),
    }
}

/// `handoff.settle { handoff, state, note? }`; undone by the state and
/// note it replaced.
pub fn settle_op() -> Op {
    Op::new(
        "handoffs.write",
        "settle",
        schema::<SettleInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: SettleInput = parse(input)?;
            let id: HandoffId = bare_id(
                &input.handoff,
                oxplow_domain::uri::resources::HANDOFF,
                "/handoff",
            )?;
            let before = handoff_tx(ctx.conn, id)?
                .ok_or_else(|| invalid("/handoff", format!("no handoff `{}`", input.handoff)))?;
            let to = HandoffState::from(input.state);
            may_settle(ctx.actor(), &before, to)?;
            let event = settle_tx(
                ctx.conn,
                id,
                to,
                ctx.actor().session_id(),
                input.note.as_deref(),
            )?;
            let after = handoff_tx(ctx.conn, id)?
                .ok_or_else(|| invalid("/handoff", format!("no handoff `{}`", input.handoff)))?;
            let mut undo = json!({ "handoff": id.to_string(), "state": before.state.as_str() });
            if let Some(note) = &before.note {
                undo["note"] = Value::String(note.clone());
            }
            Ok(HandlerOutput {
                result: json!({ "handoff": shown(&after) }),
                inverse: Some(CommandCall {
                    name: SETTLE.into(),
                    input: undo,
                }),
                events: vec![event],
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `handoff.resolve { handoff, outcome, note? }`: the move (when taking
/// hands over an item that isn't on the recipient thread yet, and the
/// work list keeps per-thread lists), then the settle.
pub fn resolve_op() -> Op {
    let compose: Arc<Composer> = Arc::new(
        |composing: &Composing<'_>, run: &RunContext, input: &Value| {
            let input: ResolveInput = parse(input.clone())?;
            let id: HandoffId = bare_id(
                &input.handoff,
                oxplow_domain::uri::resources::HANDOFF,
                "/handoff",
            )?;
            let (handoff, item) = composing
                .runtime
                .block_on(composing.db.read(move |tx| {
                    let Some(h) = handoff_tx(tx, id)? else {
                        return Ok((None, None));
                    };
                    let item = match &h.work_item {
                        Some(item) => tx
                            .query_row(
                                "SELECT w.thread_id,
                                        EXISTS (SELECT 1 FROM v_capability_provider p
                                                 WHERE p.capability = 'work_items' AND p.active = 1
                                                   AND instr(p.features, '\"lists\"') > 0)
                                   FROM v_work_item w WHERE w.ref = ?1",
                                [item],
                                |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, bool>(1)?)),
                            )
                            .optional()
                            .map_err(oxplow_db::map_sql_err)?,
                        None => None,
                    };
                    Ok((Some(h), item))
                }))
                .map_err(CommandError::from)?;
            let handoff = handoff
                .ok_or_else(|| invalid("/handoff", format!("no handoff `{}`", input.handoff)))?;
            let to = match input.outcome {
                Outcome::Taken => HandoffState::Taken,
                Outcome::Declined => HandoffState::Declined,
            };
            may_settle(run.actor(), &handoff, to)?;
            let mut calls = Vec::new();
            if let (HandoffState::Taken, Some(item), Some((thread, true))) =
                (to, &handoff.work_item, item)
            {
                if thread != Some(handoff.to_thread_id.value()) {
                    calls.push(CommandCall {
                        name: super::work_item::MOVE.into(),
                        input: json!({ "ref": item, "to": { "thread": handoff.to_thread_id.to_string() } }),
                    });
                }
            }
            let mut settle = json!({ "handoff": id.to_string(), "state": to.as_str() });
            if let Some(note) = &input.note {
                settle["note"] = Value::String(note.clone());
            }
            calls.push(CommandCall {
                name: SETTLE.into(),
                input: settle,
            });
            Ok(Composition {
                calls,
                result: Some(json!({
                    "handoff": id.to_string(),
                    "state": to.as_str(),
                    "work_item": handoff.work_item,
                })),
                events: Vec::new(),
            })
        },
    );
    Op::new(
        "handoffs.write",
        "resolve",
        schema::<ResolveInput>(),
        true,
        Compose::handler(compose),
    )
}

/// A handoff as a result shows it: its id and ref, its threads' ids.
fn shown(h: &Handoff) -> Value {
    json!({
        "id": h.id.to_string(),
        "ref": handoff_uri(h.id),
        "from_thread": h.from_thread_id.to_string(),
        "to_thread": h.to_thread_id.to_string(),
        "work_item": h.work_item,
        "state": h.state.as_str(),
        "sent_at": h.sent_at,
        "delivered_at": h.delivered_at,
        "note": h.note,
    })
}

/// The handoff commands, for the bus. A loose id in `work_item` is the
/// active work list's.
pub fn ops(deps: LinkDeps, work_items: WorkItemsRegistry) -> Vec<Op> {
    vec![send_op(deps, work_items), settle_op(), resolve_op()]
}

#[cfg(test)]
mod tests {
    /// The bare id (`hnd1`) of the handoff a result shows by its URI.
    fn handoff_id(result: &Value) -> String {
        result["handoff"]["id"].as_str().unwrap().to_string()
    }

    use super::*;
    use crate::test_fixtures::{new_thread, services_with_effort, services_with_task_effort};
    use oxplow_domain::{StreamId, Thread};
    use oxplow_tasks::{work_item_ref, TaskStore as _};

    fn agent_on(thread: ThreadId) -> Actor {
        Actor::Agent {
            session_id: None,
            thread_id: Some(thread),
            stream_id: None,
        }
    }

    async fn second_thread(svc: &crate::Services) -> Thread {
        new_thread(svc, StreamId::new(1), "receiver").await
    }

    async fn run(
        svc: &crate::Services,
        actor: &Actor,
        name: &str,
        input: Value,
    ) -> Result<oxplow_domain::CommandOutcome, CommandError> {
        svc.commands.run(actor, name, input, false).await
    }

    /// An agent hands work from its own thread to another; the run logs
    /// `handoff.sent` caused by it, and the record names both threads and
    /// the agent as its sender.
    #[tokio::test]
    async fn an_agent_hands_work_from_its_own_thread() {
        let fx = services_with_effort().await;
        let to = second_thread(&fx.svc).await;
        let out = run(
            &fx.svc,
            &agent_on(fx.thread),
            SEND,
            json!({ "to_thread": to.id.to_string(), "body": "see [[tsk999]]" }),
        )
        .await
        .unwrap();
        assert_eq!(out.result["handoff"]["from_thread"], fx.thread.to_string());
        assert_eq!(out.result["handoff"]["to_thread"], to.id.to_string());
        assert_eq!(out.result["handoff"]["state"], "open");
        assert_eq!(out.result["link_warnings"][0]["target"], "tsk999");
        let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
        assert!(events
            .iter()
            .any(|e| e.envelope.event_type == "handoff.sent"
                && e.envelope.cause.as_ref() == out.event_id.as_ref()));
        let actor: String = fx
            .svc
            .db
            .read(|tx| {
                tx.query_row("SELECT from_actor FROM handoff", [], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(actor, "agent");
    }

    /// What a send refuses: an agent naming another thread as its own, its
    /// own thread as the recipient, a closed thread, an empty body, an item
    /// the work list doesn't have, and a person who names no sender.
    #[tokio::test]
    async fn a_send_is_refused_for_what_isnt_a_handoff() {
        let fx = services_with_effort().await;
        let to = second_thread(&fx.svc).await;
        let agent = agent_on(fx.thread);
        let to_ref = to.id.to_string();

        let err = run(
            &fx.svc,
            &agent,
            SEND,
            json!({ "to_thread": to_ref, "body": "x", "from_thread": to_ref }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");

        let err = run(
            &fx.svc,
            &agent,
            SEND,
            json!({ "to_thread": fx.thread.to_string(), "body": "x" }),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/to_thread"),
            "{err:?}"
        );

        let err = run(
            &fx.svc,
            &agent,
            SEND,
            json!({ "to_thread": to_ref, "body": "  " }),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/body"),
            "{err:?}"
        );

        let err = run(
            &fx.svc,
            &agent,
            SEND,
            json!({ "to_thread": to_ref, "body": "x", "work_item": "oxplow:oxplow/work_item?id=tsk999&provider=oxplow" }),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/work_item"),
            "{err:?}"
        );

        let err = run(
            &fx.svc,
            &Actor::Human,
            SEND,
            json!({ "to_thread": to_ref, "body": "x" }),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/from_thread"),
            "{err:?}"
        );

        run(
            &fx.svc,
            &Actor::Human,
            "oxplow.thread.close",
            json!({ "thread": to_ref }),
        )
        .await
        .unwrap();
        let err = run(
            &fx.svc,
            &agent,
            SEND,
            json!({ "to_thread": to_ref, "body": "x" }),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/to_thread"),
            "{err:?}"
        );
    }

    /// The recipient takes or declines, the sender withdraws, each only
    /// while open; the other side is refused, and a settled handoff can't
    /// be settled again. Undo opens it again.
    #[tokio::test]
    async fn each_side_settles_only_what_is_its_own() {
        let fx = services_with_effort().await;
        let to = second_thread(&fx.svc).await;
        let (sender, recipient) = (agent_on(fx.thread), agent_on(to.id));
        let sent = run(
            &fx.svc,
            &sender,
            SEND,
            json!({ "to_thread": to.id.to_string(), "body": "yours" }),
        )
        .await
        .unwrap();
        let h = handoff_id(&sent.result);

        let err = run(
            &fx.svc,
            &sender,
            SETTLE,
            json!({ "handoff": h, "state": "taken" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let err = run(
            &fx.svc,
            &recipient,
            SETTLE,
            json!({ "handoff": h, "state": "withdrawn" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");

        let declined = run(
            &fx.svc,
            &recipient,
            SETTLE,
            json!({ "handoff": h, "state": "declined", "note": "not mine" }),
        )
        .await
        .unwrap();
        assert_eq!(declined.result["handoff"]["state"], "declined");
        assert_eq!(declined.result["handoff"]["note"], "not mine");
        let err = run(
            &fx.svc,
            &sender,
            SETTLE,
            json!({ "handoff": h, "state": "withdrawn" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");

        fx.svc
            .commands
            .undo(&Actor::Human, declined.audit_id.unwrap(), false)
            .await
            .unwrap();
        let withdrawn = run(
            &fx.svc,
            &sender,
            SETTLE,
            json!({ "handoff": h, "state": "withdrawn" }),
        )
        .await
        .unwrap();
        assert_eq!(withdrawn.result["handoff"]["state"], "withdrawn");
        assert_eq!(withdrawn.result["handoff"]["note"], Value::Null);
    }

    /// Taking a handoff that hands over a work item moves the item onto
    /// the recipient's thread in the same run. A work list's verbs run
    /// outside the bus's transaction, so that run is steps and has no undo;
    /// the recipient reopens the handoff instead. Declining moves nothing
    /// and undoes.
    #[tokio::test]
    async fn taking_a_handoff_moves_its_work_item() {
        let fx = services_with_task_effort().await;
        let to = second_thread(&fx.svc).await;
        let recipient = agent_on(to.id);
        let send = |body: &str| {
            json!({
                "to_thread": to.id.to_string(),
                "work_item": work_item_ref(fx.task),
                "body": body,
            })
        };
        let state = |svc: Arc<crate::Services>, h: String| async move {
            svc.db
                .read(move |tx| {
                    tx.query_row(
                        "SELECT state FROM v_handoff WHERE ref = uri('oxplow/handoff', 'id', ?1)",
                        [&h],
                        |r| r.get::<_, String>(0),
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap()
        };
        let first = run(&fx.svc, &agent_on(fx.thread), SEND, send("decline me"))
            .await
            .unwrap();
        let declined = handoff_id(&first.result);
        let out = run(
            &fx.svc,
            &recipient,
            RESOLVE,
            json!({ "handoff": declined, "outcome": "declined" }),
        )
        .await
        .unwrap();
        let task = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(task.thread_id, Some(fx.thread), "declining moves nothing");
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(state(fx.svc.clone(), declined).await, "open");

        let second = run(&fx.svc, &agent_on(fx.thread), SEND, send("take me"))
            .await
            .unwrap();
        let h = handoff_id(&second.result);
        let err = run(
            &fx.svc,
            &agent_on(fx.thread),
            RESOLVE,
            json!({ "handoff": h, "outcome": "taken" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let task = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(
            task.thread_id,
            Some(fx.thread),
            "a stranger's resolve moves nothing"
        );

        let taken = run(
            &fx.svc,
            &recipient,
            RESOLVE,
            json!({ "handoff": h, "outcome": "taken", "note": "on it" }),
        )
        .await
        .unwrap();
        let task = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(task.thread_id, Some(to.id));
        assert_eq!(state(fx.svc.clone(), h.clone()).await, "taken");
        assert!(fx
            .svc
            .commands
            .undo(&Actor::Human, taken.audit_id.unwrap(), false)
            .await
            .is_err());
        run(
            &fx.svc,
            &recipient,
            SETTLE,
            json!({ "handoff": h, "state": "open" }),
        )
        .await
        .unwrap();
        assert_eq!(state(fx.svc.clone(), h.clone()).await, "open");
    }
}
