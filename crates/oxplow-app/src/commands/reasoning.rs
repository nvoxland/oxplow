//! The agent's reasoning as records (P8.A7). `oxplow.effort.record_decision`:
//! a fork the agent took without asking, which it records. Claims ("tests
//! pass", "no behavior change") it states in `oxplow.effort.report`; the
//! rest a model reads in an effort's answers, and
//! `oxplow.effort.replace_inferred_claims` is their writer, as
//! `oxplow.effort.replace_inferred_decisions` is for the decisions it
//! proposes. They land in `v_decision` / `v_claim` for the
//! review. All are `Record` and never ask. An agent's decision goes on its
//! own thread whatever thread it names; the effort is the one open for
//! `work_item` (which must be its stream's work), else its thread's open
//! effort.

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_db::effort_store::find_open_for_work_item_tx;
use oxplow_db::reasoning_store::{
    record_decision_tx, replace_claims_tx, replace_inferred_tx, ClaimProvenance, NewClaim,
    NewDecision,
};
use oxplow_db::thread_store::get_tx as thread_tx;
use oxplow_domain::events::schema::{EffortDecisionsInferred, EffortDecisionsInferredV1};
use oxplow_domain::uri::build::validate_work_item_uri;
use oxplow_domain::{CommandError, ThreadId};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::thread::acting_thread;
use super::util::{invalid, parse, ref_id, schema, sql};
use super::{Handler, HandlerOutput, TxCtx};

pub const RECORD_DECISION: &str = "oxplow.effort.record_decision";
pub const REPLACE_INFERRED_CLAIMS: &str = "oxplow.effort.replace_inferred_claims";
pub const REPLACE_INFERRED_DECISIONS: &str = "oxplow.effort.replace_inferred_decisions";

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

/// `effort.replace_inferred_claims`'s input: the claims a model read in one
/// effort's answers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InferredClaimsInput {
    /// The effort (`effort:eff12`).
    pub effort: String,
    /// What it holds after: every claim no reviewer acted on is replaced.
    #[schemars(length(max = 8))]
    pub claims: Vec<ClaimInput>,
}

/// One claim the agent made about its work, in its own words: stated in
/// `oxplow.effort.report`'s `claims`, or read in its answers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimInput {
    /// The claim, as the agent put it ("the parser tests pass").
    #[schemars(length(min = 1, max = 1000))]
    pub statement: String,
    pub kind: ClaimKind,
    /// What the agent cited for it (`run:<id>`, a test name, a file), if
    /// anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 400))]
    pub evidence_ref: Option<String>,
}

/// What a claim is about.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClaimKind {
    /// The tests pass.
    TestsPass,
    /// Behavior is unchanged (a refactor, a rename).
    NoBehaviorChange,
    /// A case is handled (empty input, a timeout).
    HandlesCase,
    Other,
}

impl ClaimKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ClaimKind::TestsPass => "tests_pass",
            ClaimKind::NoBehaviorChange => "no_behavior_change",
            ClaimKind::HandlesCase => "handles_case",
            ClaimKind::Other => "other",
        }
    }
}

/// `effort.replace_inferred_decisions`'s input: a model's proposals for one
/// effort.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InferredDecisionsInput {
    /// The effort (`effort:eff12`).
    pub effort: String,
    /// What it holds after: every inferred decision it had is replaced.
    #[schemars(length(max = 8))]
    pub decisions: Vec<InferredDecision>,
}

/// One proposed decision.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InferredDecision {
    /// The fork: what had to be decided.
    #[schemars(length(min = 1, max = 1000))]
    pub question: String,
    /// What was chosen.
    #[schemars(length(min = 1, max = 1000))]
    pub choice: String,
    /// The options not taken.
    #[serde(default)]
    #[schemars(length(max = 16))]
    pub alternatives: Vec<String>,
    #[serde(default)]
    #[schemars(length(max = 4000))]
    pub why: String,
    /// How sure the model is that it was a real decision; absent, `low`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    fn as_str(self) -> &'static str {
        match self {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
            Confidence::High => "high",
        }
    }
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
    validate_work_item_uri(work_item).map_err(|e| invalid("/work_item", e.to_string()))?;
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

/// `effort.replace_inferred_decisions { effort, decisions }`: the only
/// writer of provenance `inferred` — what a model proposed the agent
/// decided without recording it. Replaces the effort's inferred
/// decisions; its recorded, confirmed and dismissed ones stay. The thread
/// and work item are the effort's. Inferred decisions are a person's to
/// review and never reach the agent, so any invoker may propose them.
pub fn replace_inferred_decisions_op() -> Op {
    Op::new(
        "efforts.write",
        "replace_inferred_decisions",
        schema::<InferredDecisionsInput>(),
        false,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: InferredDecisionsInput = parse(input)?;
            let effort: oxplow_domain::EffortId = ref_id(&input.effort, "effort", "/effort")?;
            let (thread_id, work_item) = effort_place(ctx, effort, &input.effort)?;
            let decisions: Vec<NewDecision> = input
                .decisions
                .into_iter()
                .map(|d| NewDecision {
                    thread_id,
                    work_item: work_item.clone(),
                    effort_id: Some(effort.value()),
                    question: d.question,
                    choice: d.choice,
                    alternatives: d.alternatives,
                    confidence: d.confidence.unwrap_or(Confidence::Low).as_str().into(),
                    why: d.why,
                })
                .collect();
            let count = replace_inferred_tx(ctx.conn, effort.value(), &decisions)
                .map_err(CommandError::from)?;
            let effort_ref = oxplow_domain::uri::build::effort_uri(effort);
            let event = ctx
                .events
                .typed::<EffortDecisionsInferred>(&EffortDecisionsInferredV1 {
                    effort: effort_ref.clone(),
                    work_item: work_item.clone(),
                    count: count as u32,
                })
                .with_subject(vec![effort_ref.clone()]);
            Ok(HandlerOutput {
                result: json!({ "effort": effort_ref, "count": count }),
                events: vec![event],
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `effort.replace_inferred_claims { effort, claims }`: the writer of
/// inferred claims — what a model read the agent claim in an effort's
/// answers ("tests pass", "no behavior change"). Replaces the effort's
/// inferred claims a reviewer hasn't acted on; the agent's recorded ones,
/// dismissed and reviewer-verified ones stay. The
/// thread and work item are the effort's. Whether each is backed is a
/// review policy's call (oxplow-bundled's `claim_status`).
pub fn replace_inferred_claims_op() -> Op {
    Op::new(
        "efforts.write",
        "replace_inferred_claims",
        schema::<InferredClaimsInput>(),
        false,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: InferredClaimsInput = parse(input)?;
            let effort: oxplow_domain::EffortId = ref_id(&input.effort, "effort", "/effort")?;
            let (thread_id, work_item) = effort_place(ctx, effort, &input.effort)?;
            let claims: Vec<NewClaim> = input
                .claims
                .into_iter()
                .map(|c| NewClaim {
                    thread_id,
                    work_item: work_item.clone(),
                    effort_id: Some(effort.value()),
                    statement: c.statement,
                    kind: c.kind.as_str().into(),
                    evidence_ref: c.evidence_ref,
                    agent_session_id: None,
                })
                .collect();
            let count =
                replace_claims_tx(ctx.conn, effort.value(), ClaimProvenance::Inferred, &claims)
                    .map_err(CommandError::from)?;
            Ok(HandlerOutput {
                result: json!({
                    "effort": oxplow_domain::uri::build::effort_uri(effort),
                    "count": count,
                }),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// An effort's thread and work item, or `Invalid` at `/effort`.
fn effort_place(
    ctx: &TxCtx<'_>,
    effort: oxplow_domain::EffortId,
    raw: &str,
) -> Result<(i64, Option<String>), CommandError> {
    ctx.conn
        .query_row(
            "SELECT thread_id, work_item FROM effort WHERE id = ?1",
            [effort.value()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sql)?
        .ok_or_else(|| invalid("/effort", format!("no effort `{raw}`")))
}

/// The reasoning commands, for the bus.
pub fn ops() -> Vec<Op> {
    vec![
        record_decision_op(),
        replace_inferred_decisions_op(),
        replace_inferred_claims_op(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Actor;
    use oxplow_tasks::work_item_ref;

    /// A decision names its work item; a bad confidence is the caller's
    /// error, and a person must name the thread.
    #[tokio::test]
    async fn a_decision_attaches_to_its_work_items_effort() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let agent = Actor::Agent {
            session_id: None,
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

    /// A model's proposals replace the effort's inferred decisions and
    /// nothing else: the agent's recorded one and a reviewed one stay.
    /// The thread and work item come from the effort; an effect may call
    /// it; the replacement is logged with its count.
    #[tokio::test]
    async fn inferred_decisions_replace_only_the_efforts_inferred_ones() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let effort = fx.effort.value();
        let d = |q: &str| NewDecision {
            thread_id: fx.thread.value(),
            work_item: Some(work_item_ref(fx.task)),
            effort_id: Some(effort),
            question: q.into(),
            choice: "c".into(),
            alternatives: vec![],
            confidence: "medium".into(),
            why: String::new(),
        };
        fx.svc
            .db
            .transaction({
                let recorded = d("recorded");
                move |tx| record_decision_tx(tx, &recorded)
            })
            .await
            .unwrap();
        fx.svc
            .reasoning_store
            .replace_inferred(effort, vec![d("reviewed"), d("stale")])
            .await
            .unwrap();
        fx.svc
            .db
            .transaction(|tx| {
                tx.execute(
                    "UPDATE decision SET provenance = 'confirmed' WHERE question = 'reviewed'",
                    [],
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();

        let effect = Actor::Effect {
            effect: "oxplow-bundled/store-reasoning".into(),
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let out = fx
            .svc
            .commands
            .run(
                &effect,
                REPLACE_INFERRED_DECISIONS,
                json!({
                    "effort": oxplow_domain::uri::build::effort_uri(fx.effort),
                    "decisions": [
                        { "question": "Which CSV library?", "choice": "csv crate",
                          "alternatives": ["hand-rolled"], "why": "quoting" },
                    ],
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["count"], json!(1));

        let rows = fx
            .svc
            .db
            .read(move |c| {
                let mut s = c
                    .prepare(
                        "SELECT question, provenance, thread_id, work_item, confidence
                         FROM decision WHERE effort_id = ?1 ORDER BY id",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = s
                    .query_map([effort], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, String>(4)?,
                        ))
                    })
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(rows)
            })
            .await
            .unwrap();
        let item = Some(work_item_ref(fx.task));
        let t = fx.thread.value();
        assert_eq!(
            rows,
            vec![
                (
                    "recorded".into(),
                    "recorded".into(),
                    t,
                    item.clone(),
                    "medium".into()
                ),
                (
                    "reviewed".into(),
                    "confirmed".into(),
                    t,
                    item.clone(),
                    "medium".into()
                ),
                (
                    "Which CSV library?".into(),
                    "inferred".into(),
                    t,
                    item.clone(),
                    "low".into()
                ),
            ]
        );

        let logged = fx
            .svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT json_extract(payload, '$.count'), json_extract(payload, '$.work_item')
                     FROM v_event WHERE type = 'effort.decisions_inferred'",
                    [],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(logged, (1, work_item_ref(fx.task)));
    }

    /// An unknown effort, a ninth proposal and an unknown confidence are
    /// the caller's errors, and nothing changes.
    /// A model's reading of the claims an effort's answers made replaces
    /// the effort's unreviewed claims; verified and dismissed ones stay.
    /// Thread and work item are the effort's.
    #[tokio::test]
    async fn inferred_claims_replace_the_efforts_unreviewed_ones() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let effort = fx.effort.value();
        let item = Some(work_item_ref(fx.task));
        let seed = |statement: &'static str, review: Option<&'static str>| {
            let svc = fx.svc.clone();
            let claim = NewClaim {
                thread_id: fx.thread.value(),
                work_item: item.clone(),
                effort_id: Some(effort),
                statement: statement.into(),
                kind: "other".into(),
                evidence_ref: None,
                agent_session_id: None,
            };
            async move {
                svc.db
                    .transaction(move |tx| {
                        let id = oxplow_db::insert_claim_tx(
                            tx,
                            oxplow_db::ClaimProvenance::Inferred,
                            &claim,
                        )?;
                        tx.execute(
                            "UPDATE claim SET review = ?2 WHERE id = ?1",
                            rusqlite::params![id, review],
                        )
                        .map_err(oxplow_db::map_sql_err)?;
                        Ok(id)
                    })
                    .await
                    .unwrap()
            }
        };
        seed("dismissed", Some("dismissed")).await;
        seed("verified", Some("verified")).await;
        seed("stale", None).await;

        let effect = Actor::Effect {
            effect: "oxplow-bundled/store-reasoning".into(),
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let out = fx
            .svc
            .commands
            .run(
                &effect,
                REPLACE_INFERRED_CLAIMS,
                json!({
                    "effort": oxplow_domain::uri::build::effort_uri(fx.effort),
                    "claims": [
                        { "statement": "The parser tests pass.", "kind": "tests_pass" },
                    ],
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["count"], json!(1));
        let rows = fx
            .svc
            .db
            .read(move |c| {
                let mut s = c
                    .prepare(
                        "SELECT statement, kind, thread_id, work_item, review
                         FROM claim WHERE effort_id = ?1 ORDER BY id",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = s
                    .query_map([effort], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, Option<String>>(4)?,
                        ))
                    })
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(rows)
            })
            .await
            .unwrap();
        let t = fx.thread.value();
        assert_eq!(
            rows,
            vec![
                (
                    "dismissed".into(),
                    "other".into(),
                    t,
                    item.clone(),
                    Some("dismissed".into())
                ),
                (
                    "verified".into(),
                    "other".into(),
                    t,
                    item.clone(),
                    Some("verified".into())
                ),
                (
                    "The parser tests pass.".into(),
                    "tests_pass".into(),
                    t,
                    item.clone(),
                    None
                ),
            ]
        );
        let bad = fx
            .svc
            .commands
            .run(
                &effect,
                REPLACE_INFERRED_CLAIMS,
                json!({
                    "effort": oxplow_domain::uri::build::effort_uri(fx.effort),
                    "claims": [{ "statement": "x", "kind": "vibes" }],
                }),
                false,
            )
            .await
            .unwrap_err();
        assert!(bad.to_string().contains("/claims/0/kind"), "{bad}");
    }

    #[tokio::test]
    async fn inferred_decisions_are_checked() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let run = |input: serde_json::Value| {
            let svc = fx.svc.clone();
            async move {
                svc.commands
                    .run(&Actor::Human, REPLACE_INFERRED_DECISIONS, input, false)
                    .await
                    .unwrap_err()
            }
        };
        let one = json!({ "question": "q", "choice": "c" });
        let effort = oxplow_domain::uri::build::effort_uri(fx.effort);
        for input in [
            json!({ "effort": "effort:eff999", "decisions": [one] }),
            json!({ "effort": effort, "decisions": vec![one.clone(); 9] }),
            json!({ "effort": effort, "decisions": [{ "question": "q", "choice": "c", "confidence": "sure" }] }),
        ] {
            let err = run(input.clone()).await;
            assert!(
                matches!(err, CommandError::Invalid { .. }),
                "{input}: {err:?}"
            );
        }
    }
}
