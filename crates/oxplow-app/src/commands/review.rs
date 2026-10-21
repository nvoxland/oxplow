//! A reviewer's verdicts on an effort's reasoning (P7.C4,
//! `.context/commands.md`): verifying or dismissing a claim oxplow read in
//! the agent's answers, and confirming or dismissing a decision oxplow
//! inferred. Each is a `Tx` command a person (or a lens acting for one)
//! runs — an agent checking its own claims isn't a review, so an
//! agent-driven run is refused — and each is undone by its pair:
//! `verify_claim` ↔ `unverify_claim`, `dismiss_claim` ↔ `reopen_claim`,
//! `confirm_decision` / `dismiss_decision` ↔ `reopen_decision`. Every run
//! logs what it decided, caused by the run.
//!
//! The claim and the decision are named by ref (`claim:7`,
//! `decision:3`), whatever effort they belong to.

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_domain::events::schema::{
    EffortClaimDismissed, EffortClaimDismissedV1, EffortClaimVerified, EffortClaimVerifiedV1,
    EffortDecisionReviewed, EffortDecisionReviewedV1,
};
use oxplow_domain::uri::build::{claim_uri, decision_uri, effort_uri};
use oxplow_domain::{CommandCall, CommandError, Invokers};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::util::{invalid, parse, ref_id, schema};
use super::{Handler, HandlerOutput, TxCtx};

pub const VERIFY_CLAIM: &str = "oxplow.effort.verify_claim";
pub const UNVERIFY_CLAIM: &str = "oxplow.effort.unverify_claim";
pub const DISMISS_CLAIM: &str = "oxplow.effort.dismiss_claim";
pub const REOPEN_CLAIM: &str = "oxplow.effort.reopen_claim";
pub const CONFIRM_DECISION: &str = "oxplow.effort.confirm_decision";
pub const DISMISS_DECISION: &str = "oxplow.effort.dismiss_decision";
pub const REOPEN_DECISION: &str = "oxplow.effort.reopen_decision";

/// What a claim cites when a person verified it by looking.
pub const REVIEWER: &str = "reviewer";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifyClaimInput {
    /// The claim (`claim:7`).
    pub claim: String,
    /// What backs it (`run:12`, a test, a file). Absent: what it already
    /// cites, else `reviewer` — the person checked it themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnverifyClaimInput {
    /// The claim (`claim:7`).
    pub claim: String,
    /// What it cites after (what it cited before it was verified); absent,
    /// nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimInput {
    /// The claim (`claim:7`).
    pub claim: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionInput {
    /// The decision (`decision:3`).
    pub decision: String,
}

fn effort_ref_of(effort: Option<i64>) -> Option<String> {
    effort.map(|e| effort_uri(oxplow_domain::EffortId::new(e)))
}

/// A claim as a review reads it.
struct ClaimRow {
    id: i64,
    effort: Option<i64>,
    evidence: Option<String>,
    review: Option<String>,
}

/// The claim `claim` names, or `Invalid` naming it.
fn claim_row(ctx: &TxCtx<'_>, claim: &str) -> Result<ClaimRow, CommandError> {
    let id = ref_id::<i64>(claim, "claim", "/claim")?;
    ctx.conn
        .query_row(
            "SELECT effort_id, evidence_ref, review FROM claim WHERE id = ?1",
            [id],
            |r| {
                Ok(ClaimRow {
                    id,
                    effort: r.get(0)?,
                    evidence: r.get(1)?,
                    review: r.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|e| CommandError::from(oxplow_db::map_sql_err(e)))?
        .ok_or_else(|| invalid("/claim", format!("no claim `{claim}`")))
}

/// Refused unless the claim's review is `want` (`None`: unreviewed).
fn expect_review(row: &ClaimRow, want: Option<&str>) -> Result<(), CommandError> {
    if row.review.as_deref() == want {
        return Ok(());
    }
    Err(invalid(
        "/claim",
        match (&row.review, want) {
            (Some(was), _) => format!("`{}` is already {was}", claim_uri(row.id)),
            (None, Some(want)) => format!("`{}` isn't {want}", claim_uri(row.id)),
            (None, None) => unreachable!("matched above"),
        },
    ))
}

/// Set a claim's review and evidence, log it as `event`, and say how to
/// put it back.
fn set_review(
    ctx: &TxCtx<'_>,
    row: &ClaimRow,
    review: Option<&str>,
    evidence: Option<String>,
    event: oxplow_domain::events::Envelope,
    inverse: CommandCall,
) -> Result<HandlerOutput, CommandError> {
    ctx.conn
        .execute(
            "UPDATE claim SET review = ?2, evidence_ref = ?3 WHERE id = ?1",
            rusqlite::params![row.id, review, evidence],
        )
        .map_err(|e| CommandError::from(oxplow_db::map_sql_err(e)))?;
    let mut subject = vec![claim_uri(row.id)];
    subject.extend(effort_ref_of(row.effort));
    Ok(HandlerOutput {
        result: json!({ "claim": claim_uri(row.id), "review": review, "evidence": evidence }),
        inverse: Some(inverse),
        events: vec![event.with_subject(subject)],
        after_commit: None,
        unchanged: false,
    })
}

/// `effort.verify_claim { claim, evidence? }`: an unreviewed claim is
/// backed, by what it names, what it cites, or the reviewer's own check.
pub fn verify_claim_op() -> Op {
    Op::new(
        "efforts.write",
        "verify_claim",
        schema::<VerifyClaimInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: VerifyClaimInput = parse(input)?;
            let row = claim_row(ctx, &input.claim)?;
            expect_review(&row, None)?;
            let backed_by = input
                .evidence
                .or_else(|| row.evidence.clone())
                .unwrap_or_else(|| REVIEWER.into());
            let event = ctx
                .events
                .typed::<EffortClaimVerified>(&EffortClaimVerifiedV1 {
                    claim: claim_uri(row.id),
                    effort: effort_ref_of(row.effort),
                    evidence: Some(backed_by.clone()),
                });
            let inverse = CommandCall {
                name: UNVERIFY_CLAIM.into(),
                input: json!({ "claim": claim_uri(row.id), "evidence": row.evidence }),
            };
            set_review(ctx, &row, Some("verified"), Some(backed_by), event, inverse)
        })),
    )
    .open_to(Invokers::NO_AGENT)
}

/// `effort.unverify_claim { claim, evidence? }`: take a verification back;
/// it cites `evidence` (what it did before) after.
pub fn unverify_claim_op() -> Op {
    Op::new(
        "efforts.write",
        "unverify_claim",
        schema::<UnverifyClaimInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: UnverifyClaimInput = parse(input)?;
            let row = claim_row(ctx, &input.claim)?;
            expect_review(&row, Some("verified"))?;
            let event = ctx
                .events
                .typed::<EffortClaimVerified>(&EffortClaimVerifiedV1 {
                    claim: claim_uri(row.id),
                    effort: effort_ref_of(row.effort),
                    evidence: None,
                });
            let inverse = CommandCall {
                name: VERIFY_CLAIM.into(),
                input: json!({ "claim": claim_uri(row.id), "evidence": row.evidence }),
            };
            set_review(ctx, &row, None, input.evidence, event, inverse)
        })),
    )
    .open_to(Invokers::NO_AGENT)
}

/// `effort.dismiss_claim { claim }`: the agent didn't claim it, or didn't
/// mean it. It stays dismissed when the effort is read again.
pub fn dismiss_claim_op() -> Op {
    claim_dismissal_op("dismiss_claim", true)
}

/// `effort.reopen_claim { claim }`: take a dismissal back.
pub fn reopen_claim_op() -> Op {
    claim_dismissal_op("reopen_claim", false)
}

fn claim_dismissal_op(op: &'static str, dismiss: bool) -> Op {
    Op::new(
        "efforts.write",
        op,
        schema::<ClaimInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: ClaimInput = parse(input)?;
            let row = claim_row(ctx, &input.claim)?;
            expect_review(&row, (!dismiss).then_some("dismissed"))?;
            let event = ctx
                .events
                .typed::<EffortClaimDismissed>(&EffortClaimDismissedV1 {
                    claim: claim_uri(row.id),
                    effort: effort_ref_of(row.effort),
                    dismissed: dismiss,
                });
            let inverse = CommandCall {
                name: if dismiss { REOPEN_CLAIM } else { DISMISS_CLAIM }.into(),
                input: json!({ "claim": claim_uri(row.id) }),
            };
            let evidence = row.evidence.clone();
            set_review(
                ctx,
                &row,
                dismiss.then_some("dismissed"),
                evidence,
                event,
                inverse,
            )
        })),
    )
    .open_to(Invokers::NO_AGENT)
}

/// Move a decision's provenance from `from` to `to`, logging the
/// outcome. Its inverse reopens a verdict, or puts back the verdict a
/// reopen undid.
fn review_decision(
    ctx: &TxCtx<'_>,
    input: DecisionInput,
    from: &[&str],
    to: &str,
) -> Result<HandlerOutput, CommandError> {
    let id = ref_id::<i64>(&input.decision, "decision", "/decision")?;
    let (effort, provenance): (Option<i64>, String) = ctx
        .conn
        .query_row(
            "SELECT effort_id, provenance FROM decision WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| CommandError::from(oxplow_db::map_sql_err(e)))?
        .ok_or_else(|| invalid("/decision", format!("no decision `{}`", input.decision)))?;
    if !from.contains(&provenance.as_str()) {
        return Err(invalid(
            "/decision",
            format!(
                "`{}` is {provenance}; only a decision that is {} can be",
                input.decision,
                from.join(" or ")
            ),
        ));
    }
    ctx.conn
        .execute(
            "UPDATE decision SET provenance = ?2 WHERE id = ?1",
            rusqlite::params![id, to],
        )
        .map_err(|e| CommandError::from(oxplow_db::map_sql_err(e)))?;
    let effort = effort_ref_of(effort);
    let mut subject = vec![decision_uri(id)];
    subject.extend(effort.clone());
    let event = ctx
        .events
        .typed::<EffortDecisionReviewed>(&EffortDecisionReviewedV1 {
            decision: decision_uri(id),
            effort,
            outcome: to.into(),
        })
        .with_subject(subject);
    let inverse = match (to, provenance.as_str()) {
        ("inferred", "confirmed") => CONFIRM_DECISION,
        ("inferred", _) => DISMISS_DECISION,
        _ => REOPEN_DECISION,
    };
    Ok(HandlerOutput {
        result: json!({ "decision": decision_uri(id), "provenance": to }),
        inverse: Some(CommandCall {
            name: inverse.into(),
            input: json!({ "decision": decision_uri(id) }),
        }),
        events: vec![event],
        after_commit: None,
        unchanged: false,
    })
}

/// The `efforts.write` operation `op`: a decision in `from` moves to
/// `to`; undone by moving it back.
fn decision_op(op: &'static str, from: &'static [&'static str], to: &'static str) -> Op {
    Op::new(
        "efforts.write",
        op,
        schema::<DecisionInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            review_decision(ctx, parse(input)?, from, to)
        })),
    )
    // A person's review (through a lens too), never the agent's of its
    // own work.
    .open_to(oxplow_domain::Invokers::NO_AGENT)
}

/// `effort.confirm_decision { decision }`: an inferred decision is right.
pub fn confirm_decision_op() -> Op {
    decision_op("confirm_decision", &["inferred"], "confirmed")
}

/// `effort.dismiss_decision { decision }`: an inferred decision is wrong.
pub fn dismiss_decision_op() -> Op {
    decision_op("dismiss_decision", &["inferred"], "dismissed")
}

/// `effort.reopen_decision { decision }`: take a review back.
pub fn reopen_decision_op() -> Op {
    decision_op("reopen_decision", &["confirmed", "dismissed"], "inferred")
}

pub fn ops() -> Vec<Op> {
    vec![
        verify_claim_op(),
        unverify_claim_op(),
        dismiss_claim_op(),
        reopen_claim_op(),
        confirm_decision_op(),
        dismiss_decision_op(),
        reopen_decision_op(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{NewClaim, NewDecision};
    use oxplow_domain::{Actor, ThreadId};

    async fn fixture() -> (crate::test_fixtures::TaskEffortFixture, i64, i64) {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let effort = fx.effort.value();
        let (thread, task) = (fx.thread, fx.task);
        let claim = fx
            .svc
            .db
            .transaction(move |tx| {
                oxplow_db::insert_claim_tx(
                    tx,
                    oxplow_db::ClaimProvenance::Inferred,
                    &NewClaim {
                        thread_id: thread.value(),
                        work_item: Some(oxplow_tasks::work_item_ref(task)),
                        effort_id: Some(effort),
                        statement: "no behavior change".into(),
                        kind: "no_behavior_change".into(),
                        evidence_ref: None,
                        agent_session_id: None,
                    },
                )
            })
            .await
            .unwrap();
        fx.svc
            .reasoning_store
            .replace_inferred(
                effort,
                vec![NewDecision {
                    thread_id: fx.thread.value(),
                    work_item: Some(oxplow_tasks::work_item_ref(fx.task)),
                    effort_id: Some(effort),
                    question: "Which store?".into(),
                    choice: "SQLite".into(),
                    alternatives: vec!["files".into()],
                    confidence: "medium".into(),
                    why: "it's there".into(),
                }],
            )
            .await
            .unwrap();
        let decision: i64 = fx
            .svc
            .db
            .read(|tx| {
                tx.query_row("SELECT max(id) FROM decision", [], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        (fx, claim, decision)
    }

    async fn claim_view(fx: &crate::test_fixtures::EffortFixture, id: i64) -> Option<String> {
        claim_review(fx, id).await.1
    }

    /// A claim's review and what it cites, as `v_claim` shows them.
    async fn claim_review(
        fx: &crate::test_fixtures::EffortFixture,
        id: i64,
    ) -> (Option<String>, Option<String>) {
        fx.svc
            .db
            .read(move |tx| {
                tx.query_row(
                    "SELECT review, evidence_ref FROM v_claim WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// P7.C4: a person verifies a claim — it reads verified, citing
    /// `reviewer` — logged and undoable back to unreviewed, citing nothing;
    /// a reviewed claim can't be verified again.
    #[tokio::test]
    async fn verifying_a_claim_makes_it_verified_and_undo_takes_it_back() {
        let (fx, claim, _) = fixture().await;
        let r = claim_uri(claim);
        assert_eq!(claim_view(&fx, claim).await, None);
        let out = fx
            .svc
            .commands
            .run(&Actor::Human, VERIFY_CLAIM, json!({ "claim": r }), false)
            .await
            .unwrap();
        assert_eq!(claim_view(&fx, claim).await, Some("reviewer".into()));
        let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
        let logged = events
            .iter()
            .find(|e| {
                e.envelope.cause == out.event_id && e.envelope.event_type == "effort.claim_verified"
            })
            .expect("logged, caused by the run");
        assert!(logged.envelope.subject.contains(&r));
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                VERIFY_CLAIM,
                json!({ "claim": r, "evidence": "run:3" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("is already verified"), "{err}");

        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(claim_view(&fx, claim).await, None);
        // A claim named by a ref that doesn't exist is refused at /claim.
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                VERIFY_CLAIM,
                json!({ "claim": "claim:9999" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/claim"),
            "{err:?}"
        );
    }

    /// A claim that already cites something (what the agent said backed
    /// it) keeps it when verified, and gets it back when the verification
    /// is undone.
    #[tokio::test]
    async fn verifying_keeps_what_a_claim_cites_and_undo_restores_it() {
        let (fx, claim, _) = fixture().await;
        fx.svc
            .db
            .transaction(move |tx| {
                tx.execute(
                    "UPDATE claim SET evidence_ref = 'run:42' WHERE id = ?1",
                    [claim],
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let r = claim_uri(claim);
        let out = fx
            .svc
            .commands
            .run(&Actor::Human, VERIFY_CLAIM, json!({ "claim": r }), false)
            .await
            .unwrap();
        assert_eq!(
            claim_review(&fx, claim).await,
            (Some("verified".into()), Some("run:42".into()))
        );
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(
            claim_review(&fx, claim).await,
            (None, Some("run:42".into()))
        );
    }

    /// A person dismisses a claim the agent didn't make; it's logged,
    /// can't then be verified, and reopening (or undo) makes it unreviewed.
    #[tokio::test]
    async fn a_dismissed_claim_is_reopened_by_undo() {
        let (fx, claim, _) = fixture().await;
        let r = claim_uri(claim);
        let out = fx
            .svc
            .commands
            .run(&Actor::Human, DISMISS_CLAIM, json!({ "claim": r }), false)
            .await
            .unwrap();
        assert_eq!(claim_review(&fx, claim).await.0, Some("dismissed".into()));
        let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
        let logged = events
            .iter()
            .find(|e| {
                e.envelope.cause == out.event_id
                    && e.envelope.event_type == "effort.claim_dismissed"
            })
            .expect("logged, caused by the run");
        assert!(logged.envelope.subject.contains(&r));
        let err = fx
            .svc
            .commands
            .run(&Actor::Human, VERIFY_CLAIM, json!({ "claim": r }), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("is already dismissed"), "{err}");
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                REOPEN_CLAIM,
                json!({ "claim": claim_uri(9999) }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no claim"), "{err}");
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(claim_review(&fx, claim).await.0, None);
        let err = fx
            .svc
            .commands
            .run(&Actor::Human, REOPEN_CLAIM, json!({ "claim": r }), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("isn't dismissed"), "{err}");
    }

    /// P7.C4: an agent can't review its own work — not directly, and not
    /// through a lens acting for it.
    #[tokio::test]
    async fn an_agent_may_not_verify_a_claim_or_review_a_decision() {
        let (fx, claim, decision) = fixture().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(ThreadId::new(fx.thread.value())),
            stream_id: None,
        };
        let through_lens = Actor::Lens {
            lens_id: "acme/review".into(),
            on_behalf_of: Box::new(agent.clone()),
        };
        for actor in [agent, through_lens] {
            for (name, input) in [
                (VERIFY_CLAIM, json!({ "claim": claim_uri(claim) })),
                (
                    CONFIRM_DECISION,
                    json!({ "decision": decision_uri(decision) }),
                ),
            ] {
                let err = fx
                    .svc
                    .commands
                    .run(&actor, name, input, false)
                    .await
                    .unwrap_err();
                assert!(
                    matches!(err, CommandError::Denied { .. }),
                    "{name}: {err:?}"
                );
            }
        }
        assert_eq!(claim_view(&fx, claim).await, None);
    }

    async fn provenance(fx: &crate::test_fixtures::EffortFixture, id: i64) -> String {
        fx.svc
            .db
            .read(move |tx| {
                tx.query_row(
                    "SELECT provenance FROM v_decision WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// P7.C4: confirming moves an inferred decision to `confirmed` (so a
    /// list of the inferred ones no longer shows it), dismissing to
    /// `dismissed`; each is undoable back to `inferred`; a recorded
    /// decision isn't reviewed.
    #[tokio::test]
    async fn confirming_or_dismissing_an_inferred_decision_and_undoing_it() {
        let (fx, _, decision) = fixture().await;
        let r = decision_uri(decision);
        let inferred = || async {
            fx.svc
                .db
                .read(|tx| {
                    tx.query_row(
                        "SELECT count(*) FROM v_decision WHERE provenance = 'inferred'",
                        [],
                        |r| r.get::<_, i64>(0),
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap()
        };
        assert_eq!(inferred().await, 1);
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CONFIRM_DECISION,
                json!({ "decision": r }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(provenance(&fx, decision).await, "confirmed");
        assert_eq!(inferred().await, 0, "an inferred-decisions list drops it");
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(provenance(&fx, decision).await, "inferred");

        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                DISMISS_DECISION,
                json!({ "decision": r }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(provenance(&fx, decision).await, "dismissed");
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CONFIRM_DECISION,
                json!({ "decision": r }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("is dismissed"), "{err}");
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(provenance(&fx, decision).await, "inferred");
        let reviewed: Vec<String> = fx
            .svc
            .event_log_store
            .read_after(0, 500)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.envelope.event_type == "effort.decision_reviewed")
            .map(|e| e.envelope.payload["outcome"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            reviewed,
            vec!["confirmed", "inferred", "dismissed", "inferred"]
        );
    }
}
