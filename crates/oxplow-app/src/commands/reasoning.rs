//! The agent's reasoning as records (P8.A7): `oxplow.effort.record_decision` (a
//! fork it took without asking) and `oxplow.effort.record_claim` (what it says
//! about its work — "tests pass"). `Tx` over `oxplow_db::reasoning_store::
//! {record_decision_tx, record_claim_tx}`; they land in `v_decision` /
//! `v_claim` for the review. Both are `Record` and never ask. An agent's
//! go on its own thread whatever thread it names; the effort is the one
//! open for `work_item` (which must be its stream's work), else its
//! thread's open effort.

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_db::effort_store::find_open_for_work_item_tx;
use oxplow_db::reasoning_store::{record_claim_tx, record_decision_tx, NewClaim, NewDecision};
use oxplow_db::thread_store::get_tx as thread_tx;
use oxplow_domain::refs::build::validate_work_item_ref;
use oxplow_domain::{CommandError, ThreadId};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::thread::acting_thread;
use super::{Handler, HandlerOutput, TxCtx};

pub const RECORD_DECISION: &str = "oxplow.effort.record_decision";
pub const RECORD_CLAIM: &str = "oxplow.effort.record_claim";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionInput {
    /// The thread (`thread:thr3`); an agent's is always its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// The work item it was made on (`work_item:oxplow:tsk42`); absent,
    /// the thread's open effort's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item: Option<String>,
    /// The fork: what had to be decided.
    pub question: String,
    /// What was chosen.
    pub choice: String,
    /// The options not taken.
    #[serde(default)]
    pub alternatives: Vec<String>,
    /// `low`, `medium` (the default) or `high`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    #[serde(default)]
    pub why: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimInput {
    /// The thread (`thread:thr3`); an agent's is always its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// The work item it's about (`work_item:oxplow:tsk42`); absent, the
    /// thread's open effort's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item: Option<String>,
    pub statement: String,
    /// `tests_pass`, `no_behavior_change`, `handles_case` or `other`.
    pub kind: String,
    /// What backs it (`run:<id>`, a test name, a file); absent, unbacked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_ref: Option<String>,
}

fn invalid(field: &str, message: String) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message,
    }
}

fn parse<T: serde::de::DeserializeOwned>(input: Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
}

fn sql(e: rusqlite::Error) -> CommandError {
    CommandError::from(oxplow_db::map_sql_err(e))
}

/// Where a record lands: its thread, work item and effort.
struct Place {
    thread: ThreadId,
    work_item: Option<String>,
    effort: Option<i64>,
}

/// The acting thread, and the effort the record attaches to: `work_item`'s
/// open one (the item must be the thread's stream's work), else the
/// thread's newest open effort.
fn place(
    ctx: &TxCtx<'_>,
    thread: Option<&str>,
    work_item: Option<&str>,
) -> Result<Place, CommandError> {
    let thread = acting_thread(ctx, thread)?;
    let stream_of = |t: ThreadId| -> Result<Option<oxplow_domain::StreamId>, CommandError> {
        Ok(thread_tx(ctx.conn, t).map_err(sql)?.map(|t| t.stream_id))
    };
    let Some(work_item) = work_item else {
        let effort = ctx
            .conn
            .query_row(
                "SELECT id, work_item FROM effort WHERE thread_id = ?1 AND ended_at IS NULL
                 ORDER BY started_at DESC LIMIT 1",
                [thread.value()],
                // An effort linked to nothing has no work item.
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(sql)?;
        return Ok(Place {
            thread,
            work_item: effort.as_ref().and_then(|(_, item)| item.clone()),
            effort: effort.map(|(id, _)| id),
        });
    };
    validate_work_item_ref(work_item).map_err(|e| invalid("/work_item", e.to_string()))?;
    let open = find_open_for_work_item_tx(ctx.conn, work_item).map_err(sql)?;
    // Whose work it is: the thread of its open effort, else the list it's
    // on (the work-item interface's `thread_id`).
    let owner = match &open {
        Some(e) => Some(e.thread_id),
        None => ctx
            .conn
            .query_row(
                "SELECT thread_id FROM v_work_item WHERE ref = ?1",
                [work_item],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()
            .map_err(sql)?
            .flatten()
            .map(ThreadId::new),
    };
    if let Some(owner) = owner {
        if stream_of(owner)? != stream_of(thread)? {
            return Err(CommandError::Denied {
                reason: format!("`{work_item}` is another stream's work"),
            });
        }
    }
    Ok(Place {
        thread,
        work_item: Some(work_item.to_string()),
        effort: open.map(|e| e.id.value()),
    })
}

fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

/// `effort.record_decision { thread?, work_item?, question, choice, … }`.
pub fn record_decision_op() -> Op {
    Op::new(
        "efforts.write",
        "record_decision",
        schema::<DecisionInput>(),
        false,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: DecisionInput = parse(input)?;
            let at = place(ctx, input.thread.as_deref(), input.work_item.as_deref())?;
            let id = record_decision_tx(
                ctx.conn,
                &NewDecision {
                    thread_id: at.thread.value(),
                    work_item: at.work_item.clone(),
                    effort_id: at.effort,
                    question: input.question,
                    choice: input.choice,
                    alternatives: input.alternatives,
                    confidence: input.confidence.unwrap_or_else(|| "medium".into()),
                    why: input.why,
                },
            )
            .map_err(CommandError::from)?;
            Ok(HandlerOutput {
                result: json!({ "id": id, "effort": at.effort }),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `effort.record_claim { thread?, work_item?, statement, kind, evidence_ref? }`.
pub fn record_claim_op() -> Op {
    Op::new(
        "efforts.write",
        "record_claim",
        schema::<ClaimInput>(),
        false,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ClaimInput = parse(input)?;
            let at = place(ctx, input.thread.as_deref(), input.work_item.as_deref())?;
            let id = record_claim_tx(
                ctx.conn,
                &NewClaim {
                    thread_id: at.thread.value(),
                    work_item: at.work_item.clone(),
                    effort_id: at.effort,
                    statement: input.statement,
                    kind: input.kind,
                    evidence_ref: input.evidence_ref,
                },
            )
            .map_err(CommandError::from)?;
            Ok(HandlerOutput {
                result: json!({ "id": id, "effort": at.effort }),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// The reasoning commands, for the bus.
pub fn ops() -> Vec<Op> {
    vec![record_decision_op(), record_claim_op()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{new_thread, services_with_effort};
    use oxplow_domain::refs::build::thread_ref;
    use oxplow_domain::{Actor, StreamId};
    use oxplow_tasks::work_item_ref;

    async fn claims(svc: &crate::Services) -> Vec<(i64, Option<i64>, String)> {
        svc.db
            .read(|c| {
                let mut s = c
                    .prepare("SELECT thread_id, effort_id, statement FROM claim ORDER BY id")
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = s
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(rows)
            })
            .await
            .unwrap()
    }

    /// A claim lands on the agent's own thread and its open effort; naming
    /// another thread is refused, and nothing lands.
    #[tokio::test]
    async fn a_claim_lands_on_the_agents_own_thread() {
        let fx = services_with_effort().await;
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let out = fx
            .svc
            .commands
            .run(
                &agent,
                RECORD_CLAIM,
                json!({ "statement": "tests pass", "kind": "tests_pass", "evidence_ref": "run:7" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["effort"], json!(fx.effort.value()));
        assert!(out.audit_id.is_some(), "the claim is audited");

        let other = new_thread(&fx.svc, StreamId::new(1), "other").await;
        let err = fx
            .svc
            .commands
            .run(
                &agent,
                RECORD_CLAIM,
                json!({ "thread": thread_ref(other.id), "statement": "x", "kind": "other" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        assert_eq!(
            claims(&fx.svc).await,
            vec![(
                fx.thread.value(),
                Some(fx.effort.value()),
                "tests pass".into()
            )]
        );
    }

    /// A decision names its work item; a bad confidence is the caller's
    /// error, and a person must name the thread.
    #[tokio::test]
    async fn a_decision_attaches_to_its_work_items_effort() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let input = |confidence: &str| {
            json!({
                "work_item": work_item_ref(fx.task),
                "question": "Where does export live?",
                "choice": "src/export",
                "confidence": confidence,
            })
        };
        let out = fx
            .svc
            .commands
            .run(&agent, RECORD_DECISION, input("high"), false)
            .await
            .unwrap();
        assert_eq!(out.result["effort"], json!(fx.effort.value()));
        let err = fx
            .svc
            .commands
            .run(&agent, RECORD_DECISION, input("certain"), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
        let err = fx
            .svc
            .commands
            .run(&Actor::Human, RECORD_DECISION, input("low"), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
    }

    /// A claim about another stream's work item is refused.
    #[tokio::test]
    async fn a_claim_on_another_streams_work_is_refused() {
        use oxplow_domain::stores::StreamStore as _;
        use oxplow_tasks::TaskStore as _;
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let mut other = fx.svc.stream_store.list().await.unwrap().pop().unwrap();
        other.id = StreamId::placeholder();
        other.title = "other".into();
        other.branch = "other".into();
        other.kind = oxplow_domain::StreamKind::Worktree;
        other.worktree_path = "/elsewhere".into();
        let other = fx.svc.stream_store.upsert(&other).await.unwrap();
        let there = new_thread(&fx.svc, other, "t").await;
        let mut task = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        task.id = oxplow_tasks::TaskId::placeholder();
        task.thread_id = Some(there.id);
        task.status = oxplow_tasks::TaskStatus::Ready;
        let foreign = fx.svc.task_store.insert(&task).await.unwrap();
        crate::test_fixtures::restate_task(&fx.svc, foreign).await;
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Agent {
                    thread_id: Some(fx.thread),
                    stream_id: None,
                },
                RECORD_CLAIM,
                json!({ "work_item": work_item_ref(foreign), "statement": "x", "kind": "other" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Denied { reason } if reason.contains("another stream")),
            "{err:?}"
        );
    }
}
