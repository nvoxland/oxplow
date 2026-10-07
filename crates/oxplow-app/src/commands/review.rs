//! A reviewer's verdicts on an effort's reasoning (P7.C4,
//! `.context/commands.md`): verifying a claim the agent made, and
//! confirming or dismissing a decision oxplow inferred. Each is a `Tx`
//! command a person (or a lens acting for one) runs — an agent checking
//! its own claims isn't a review, so an agent-driven run is refused — and
//! each is undone by its pair: `verify_claim` ↔ `unverify_claim`,
//! `confirm_decision` / `dismiss_decision` ↔ `reopen_decision`. Every run
//! logs what it decided, caused by the run.
//!
//! The claim and the decision are named by ref (`claim:7`,
//! `decision:3`), whatever effort they belong to.

use std::sync::Arc;

use oxplow_domain::events::schema::{
    EffortClaimVerified, EffortClaimVerifiedV1, EffortDecisionReviewed, EffortDecisionReviewedV1,
};
use oxplow_domain::refs::build::{claim_ref, decision_ref, effort_ref};
use oxplow_domain::{
    Atomicity, CommandCall, CommandEffect, CommandError, CommandSpec, Confirm, Invokers, Lifecycle,
};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Command, Handler, HandlerOutput, TxCtx};

pub const VERIFY_CLAIM: &str = "oxplow.effort.verify_claim";
pub const UNVERIFY_CLAIM: &str = "oxplow.effort.unverify_claim";
pub const CONFIRM_DECISION: &str = "oxplow.effort.confirm_decision";
pub const DISMISS_DECISION: &str = "oxplow.effort.dismiss_decision";
pub const REOPEN_DECISION: &str = "oxplow.effort.reopen_decision";

/// What a claim cites when a person verified it by looking.
pub const REVIEWER: &str = "reviewer";

/// A person, or a lens acting for one: never an agent — nor a lens acting
/// for an agent, which the agent policy denies like the agent itself
/// (`agent_policy::check_command`, on every agent-driven run, nested
/// ones included).
const REVIEWERS: Invokers = Invokers {
    human: true,
    agent: false,
    lens: true,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifyClaimInput {
    /// The claim (`claim:7`).
    pub claim: String,
    /// What backs it (`run:12`, a test, a file); `reviewer` when absent —
    /// the person checked it themselves.
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

fn invalid(field: &str, message: String) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message,
    }
}

fn parse<T: serde::de::DeserializeOwned>(input: serde_json::Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
}

/// The id a `<kind>:<n>` ref names.
fn id_of(item: &str, kind: &str, field: &str) -> Result<i64, CommandError> {
    item.strip_prefix(&format!("{kind}:"))
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| invalid(field, format!("`{item}` isn't a {kind} ref ({kind}:<id>)")))
}

fn spec(name: &str, summary: &str, schema: serde_json::Value) -> CommandSpec {
    CommandSpec {
        id: name.into(),
        summary: summary.into(),
        input_schema: schema,
        invokers: REVIEWERS,
        confirm: Confirm::Never,
        undoable: true,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        effect: CommandEffect::Record,
        needs: Vec::new(),
        ui: None,
    }
}

fn effort_ref_of(effort: Option<i64>) -> Option<String> {
    effort.map(|e| effort_ref(oxplow_domain::EffortId::new(e)))
}

/// A claim's effort and current evidence, or `Invalid` naming it.
fn claim_row(
    ctx: &TxCtx<'_>,
    id: i64,
    claim: &str,
) -> Result<(Option<i64>, Option<String>), CommandError> {
    ctx.conn
        .query_row(
            "SELECT effort_id, evidence_ref FROM claim WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| CommandError::from(oxplow_db::map_sql_err(e)))?
        .ok_or_else(|| invalid("/claim", format!("no claim `{claim}`")))
}

/// Set a claim's evidence, log it, and say how to put it back.
fn set_evidence(
    ctx: &TxCtx<'_>,
    claim: &str,
    effort: Option<i64>,
    evidence: Option<String>,
    inverse: CommandCall,
) -> Result<HandlerOutput, CommandError> {
    let id = id_of(claim, "claim", "/claim")?;
    ctx.conn
        .execute(
            "UPDATE claim SET evidence_ref = ?2 WHERE id = ?1",
            rusqlite::params![id, evidence],
        )
        .map_err(|e| CommandError::from(oxplow_db::map_sql_err(e)))?;
    let effort = effort_ref_of(effort);
    let mut subject = vec![claim_ref(id)];
    subject.extend(effort.clone());
    let event = ctx
        .events
        .typed::<EffortClaimVerified>(&EffortClaimVerifiedV1 {
            claim: claim_ref(id),
            effort,
            evidence: evidence.clone(),
        })
        .with_subject(subject);
    Ok(HandlerOutput {
        result: json!({ "claim": claim_ref(id), "evidence": evidence }),
        inverse: Some(inverse),
        events: vec![event],
        after_commit: None,
        unchanged: false,
    })
}

/// `effort.verify_claim { claim, evidence? }`.
pub fn verify_claim_command() -> Command {
    Command::new(
        spec(
            VERIFY_CLAIM,
            "Verify a claim an agent made about its work (`claim:<id>`), citing what backs it \
             (`reviewer` when you checked it yourself). A person's review.",
            serde_json::to_value(schemars::schema_for!(VerifyClaimInput)).expect("schema"),
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: VerifyClaimInput = parse(input)?;
            let id = id_of(&input.claim, "claim", "/claim")?;
            let (effort, evidence) = claim_row(ctx, id, &input.claim)?;
            if let Some(cited) = evidence {
                return Err(invalid(
                    "/claim",
                    format!("`{}` already cites `{cited}`", input.claim),
                ));
            }
            let backed_by = input.evidence.unwrap_or_else(|| REVIEWER.into());
            set_evidence(
                ctx,
                &input.claim,
                effort,
                Some(backed_by),
                CommandCall {
                    name: UNVERIFY_CLAIM.into(),
                    input: json!({ "claim": claim_ref(id) }),
                },
            )
        })),
    )
    .expect("effort.verify_claim registers")
}

/// `effort.unverify_claim { claim }`: take a verification back.
pub fn unverify_claim_command() -> Command {
    Command::new(
        spec(
            UNVERIFY_CLAIM,
            "Take back a claim's verification (`claim:<id>`): it cites nothing again.",
            serde_json::to_value(schemars::schema_for!(ClaimInput)).expect("schema"),
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ClaimInput = parse(input)?;
            let id = id_of(&input.claim, "claim", "/claim")?;
            let (effort, evidence) = claim_row(ctx, id, &input.claim)?;
            let Some(cited) = evidence else {
                return Err(invalid(
                    "/claim",
                    format!("`{}` cites nothing", input.claim),
                ));
            };
            set_evidence(
                ctx,
                &input.claim,
                effort,
                None,
                CommandCall {
                    name: VERIFY_CLAIM.into(),
                    input: json!({ "claim": claim_ref(id), "evidence": cited }),
                },
            )
        })),
    )
    .expect("effort.unverify_claim registers")
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
    let id = id_of(&input.decision, "decision", "/decision")?;
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
    let mut subject = vec![decision_ref(id)];
    subject.extend(effort.clone());
    let event = ctx
        .events
        .typed::<EffortDecisionReviewed>(&EffortDecisionReviewedV1 {
            decision: decision_ref(id),
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
        result: json!({ "decision": decision_ref(id), "provenance": to }),
        inverse: Some(CommandCall {
            name: inverse.into(),
            input: json!({ "decision": decision_ref(id) }),
        }),
        events: vec![event],
        after_commit: None,
        unchanged: false,
    })
}

fn decision_command(
    name: &'static str,
    summary: &str,
    from: &'static [&'static str],
    to: &'static str,
) -> Command {
    Command::new(
        spec(
            name,
            summary,
            serde_json::to_value(schemars::schema_for!(DecisionInput)).expect("schema"),
        ),
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            review_decision(ctx, parse(input)?, from, to)
        })),
    )
    .expect("a review command registers")
}

/// `effort.confirm_decision { decision }`: an inferred decision is right.
pub fn confirm_decision_command() -> Command {
    decision_command(
        CONFIRM_DECISION,
        "Confirm a decision oxplow inferred from an effort (`decision:<id>`): it was made, as \
         stated. A person's review.",
        &["inferred"],
        "confirmed",
    )
}

/// `effort.dismiss_decision { decision }`: an inferred decision is wrong.
pub fn dismiss_decision_command() -> Command {
    decision_command(
        DISMISS_DECISION,
        "Dismiss a decision oxplow inferred from an effort (`decision:<id>`): it wasn't made, \
         or not like that. A person's review.",
        &["inferred"],
        "dismissed",
    )
}

/// `effort.reopen_decision { decision }`: take a review back.
pub fn reopen_decision_command() -> Command {
    decision_command(
        REOPEN_DECISION,
        "Take back a review of an inferred decision (`decision:<id>`): it is unconfirmed again.",
        &["confirmed", "dismissed"],
        "inferred",
    )
}

pub fn commands() -> Vec<Command> {
    vec![
        verify_claim_command(),
        unverify_claim_command(),
        confirm_decision_command(),
        dismiss_decision_command(),
        reopen_decision_command(),
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
                oxplow_db::record_claim_tx(
                    tx,
                    &NewClaim {
                        thread_id: thread.value(),
                        work_item: Some(oxplow_domain::refs::build::work_item_ref(task)),
                        effort_id: Some(effort),
                        statement: "no behavior change".into(),
                        kind: "no_behavior_change".into(),
                        evidence_ref: None,
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
                    work_item: Some(oxplow_domain::refs::build::work_item_ref(fx.task)),
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

    async fn claim_view(
        fx: &crate::test_fixtures::EffortFixture,
        id: i64,
    ) -> (Option<String>, i64) {
        fx.svc
            .db
            .read(move |tx| {
                tx.query_row(
                    "SELECT evidence_ref, verified FROM v_claim WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// P7.C4: a person verifies a claim — it reads verified, citing
    /// `reviewer` — logged and undoable back to citing nothing; a claim
    /// already backed can't be verified over its evidence.
    #[tokio::test]
    async fn verifying_a_claim_makes_it_verified_and_undo_takes_it_back() {
        let (fx, claim, _) = fixture().await;
        let r = claim_ref(claim);
        assert_eq!(claim_view(&fx, claim).await, (None, 0));
        let out = fx
            .svc
            .commands
            .run(&Actor::Human, VERIFY_CLAIM, json!({ "claim": r }), false)
            .await
            .unwrap();
        assert_eq!(claim_view(&fx, claim).await, (Some("reviewer".into()), 1));
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
        assert!(
            err.to_string().contains("already cites `reviewer`"),
            "{err}"
        );

        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(claim_view(&fx, claim).await, (None, 0));
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

    /// P7.C4: an agent can't review its own work — not directly, and not
    /// through a lens acting for it.
    #[tokio::test]
    async fn an_agent_may_not_verify_a_claim_or_review_a_decision() {
        let (fx, claim, decision) = fixture().await;
        let agent = Actor::Agent {
            thread_id: Some(ThreadId::new(fx.thread.value())),
            stream_id: None,
        };
        let through_lens = Actor::Lens {
            lens_id: "acme/review".into(),
            on_behalf_of: Box::new(agent.clone()),
        };
        for actor in [agent, through_lens] {
            for (name, input) in [
                (VERIFY_CLAIM, json!({ "claim": claim_ref(claim) })),
                (
                    CONFIRM_DECISION,
                    json!({ "decision": decision_ref(decision) }),
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
        assert_eq!(claim_view(&fx, claim).await, (None, 0));
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
        let r = decision_ref(decision);
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
