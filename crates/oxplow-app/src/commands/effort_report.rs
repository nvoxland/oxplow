//! Reporting on an effort: `oxplow.effort.report` — optional words on the
//! thread's current (else latest) effort: a summary, the impacts beyond
//! its edits, and the claims the agent can back (`v_claim`, `recorded`). It opens and closes nothing; which files and runs were the
//! effort's is observed, and an effort closed without a summary takes its
//! last turn's final message (`.context/work-tracking.md`). `External`: it
//! checks the summary's links against the worktree, so it can't run in the
//! bus's transaction.
//!
//! An agent reports only on its own thread. The result carries
//! `link_warnings`.

use crate::commands::ops::Op;
use std::path::PathBuf;
use std::sync::Arc;

use oxplow_db::{Database, EffortStore as _, NewClaim, SqliteEffortStore};
use oxplow_domain::uri::build::thread_uri;
use oxplow_domain::vcs::Vcs;
use oxplow_domain::{Actor, CommandError, EffortImpact, ThreadId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::reasoning::ClaimInput;
use super::util::{failed, invalid, parse, ref_id, schema};
use super::{Handler, HandlerOutput, Invocation};
use crate::effort_service::EffortService;
use crate::sql_gateway::SqlGateway;

pub const REPORT: &str = "oxplow.effort.report";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportInput {
    /// The thread whose effort it is (`oxplow:oxplow/thread?id=thr3`): an agent's is always
    /// its own; a person names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// What shipped (markdown; `[[…]]` links are checked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Outcomes beyond the edits: `{ kind, id, action? }` — a wiki page,
    /// a work item, a commit, a finding.
    #[serde(default)]
    pub impacts: Vec<EffortImpact>,
    /// What you claim about the work and can back ("the parser tests
    /// pass", citing `oxplow:oxplow/run?id=<id>`): recorded on the effort for its review,
    /// replacing the claims an earlier report stated that nobody reviewed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 8))]
    pub claims: Option<Vec<ClaimInput>>,
}

/// What reporting reads and writes.
#[derive(Clone)]
pub struct EffortDeps {
    /// Settles a close's lifecycle (its end snapshot) before the report reads it.
    pub lifecycle: EffortService,
    pub efforts: Arc<SqliteEffortStore>,
    pub sql: SqlGateway,
    pub db: Database,
    /// The ref kinds a summary's links may name.
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    pub project_dir: PathBuf,
    pub vcs: Arc<dyn Vcs>,
}

/// The agent's own thread; an agent without one, or naming another, is
/// refused. `None` for a person.
fn agents_thread(actor: &Actor, named: Option<ThreadId>) -> Result<Option<ThreadId>, CommandError> {
    match actor.agent_thread() {
        None => Ok(None),
        Some(None) => Err(CommandError::Denied {
            reason: "an agent without a thread can't report on an effort".into(),
        }),
        Some(Some(own)) => match named {
            Some(t) if t != own => Err(CommandError::Denied {
                reason: format!(
                    "an agent reports only on its own thread (`{}`)",
                    thread_uri(own)
                ),
            }),
            _ => Ok(Some(own)),
        },
    }
}

/// The thread's stream's worktree, where touched files are looked for.
async fn worktree_of(db: &Database, thread: ThreadId) -> Option<PathBuf> {
    db.read(move |c| Ok(crate::link_check::worktree_of_tx(c, thread)))
        .await
        .ok()
        .flatten()
}

async fn report(
    deps: &EffortDeps,
    actor: Actor,
    input: ReportInput,
) -> Result<Value, CommandError> {
    use oxplow_db::page_ref_projections::IMPACT_KINDS;
    if let Some((i, imp)) = input
        .impacts
        .iter()
        .enumerate()
        .find(|(_, imp)| !IMPACT_KINDS.contains(&imp.kind.as_str()))
    {
        return Err(invalid(
            &format!("/impacts/{i}/kind"),
            format!(
                "`{}` isn't an impact kind: one of {}",
                imp.kind,
                IMPACT_KINDS.join(", ")
            ),
        ));
    }
    let named = input
        .thread
        .as_deref()
        .map(|t| ref_id(t, oxplow_domain::uri::resources::THREAD, "/thread"))
        .transpose()?;
    let thread = match agents_thread(&actor, named)? {
        Some(own) => own,
        None => named.ok_or_else(|| invalid("/thread", "name the thread"))?,
    };
    // A close just before (the policy's, on the item finishing) settles
    // first, so the report lands on the effort it closed.
    deps.lifecycle.settle_lifecycle().await;
    let effort = match deps
        .efforts
        .find_open_for_thread(&thread)
        .await
        .map_err(failed)?
    {
        Some(e) => e,
        None => deps
            .efforts
            .latest_for_thread(thread)
            .await
            .map_err(failed)?
            .ok_or_else(|| {
                invalid(
                    "/thread",
                    format!("`{}` has no effort to report on", thread_uri(thread)),
                )
            })?,
    };
    let summary = input.summary.filter(|s| !s.trim().is_empty());
    let impacts = (!input.impacts.is_empty()).then_some(input.impacts);
    let session = match &actor {
        Actor::Agent { session_id, .. } => session_id.map(|s| s.value()),
        _ => None,
    };
    let claims = input.claims.map(|claims| {
        claims
            .into_iter()
            .map(|c| NewClaim {
                thread_id: thread.value(),
                work_item: effort.work_item.clone(),
                effort_id: Some(effort.id.value()),
                statement: c.statement,
                kind: c.kind.as_str().into(),
                evidence_ref: c.evidence_ref,
                agent_session_id: session,
            })
            .collect()
    });
    if summary.is_some() || impacts.is_some() || claims.is_some() {
        // All land or none: a run that fails leaves the effort as it was.
        deps.efforts
            .record_report(&effort.id, summary.clone(), impacts, claims)
            .await
            .map_err(failed)?;
    }
    let link_warnings = match &summary {
        Some(body) => {
            let root = worktree_of(&deps.db, thread)
                .await
                .unwrap_or_else(|| deps.project_dir.clone());
            crate::link_check::check_links_at(&deps.db, &deps.vocabulary, &root, &*deps.vcs, body)
                .await
        }
        None => Vec::new(),
    };
    Ok(json!({
        "effort": oxplow_domain::uri::build::effort_uri(effort.id),
        "link_warnings": link_warnings,
    }))
}

/// `effort.report { thread?, summary?, impacts?, claims? }`.
pub fn report_op(deps: EffortDeps) -> Op {
    Op::new(
        "efforts.write",
        "report",
        schema::<ReportInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let actor = invocation.actor().clone();
            let deps = deps.clone();
            Box::pin(async move {
                let input: ReportInput = parse(input)?;
                Ok(HandlerOutput {
                    result: report(&deps, actor, input).await?,
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

/// The reporting commands, for the bus.
pub fn ops(deps: EffortDeps) -> Vec<Op> {
    vec![report_op(deps)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{new_thread, services_with_effort, EffortFixture};
    use oxplow_db::SqliteCommandAuditStore;
    use oxplow_domain::StreamId;
    use oxplow_tasks::work_item_ref;

    fn agent(fx: &EffortFixture) -> Actor {
        Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    /// What an agent runs to finish its task: the transition to done and
    /// the report, as one `oxplow.command.sequence`. Returns the task's row and
    /// the report's result.
    async fn complete(
        fx: &crate::test_fixtures::TaskEffortFixture,
        report: Value,
    ) -> (Value, Value) {
        let item = work_item_ref(fx.task);
        let out = fx
            .svc
            .commands
            .run(
                &agent(fx),
                crate::commands::compose::SEQUENCE,
                json!({ "calls": [
                    { "name": crate::commands::work_item::NAME,
                      "input": { "ref": item, "to": "done" } },
                    { "name": REPORT, "input": report },
                ] }),
                false,
            )
            .await
            .unwrap();
        fx.svc.efforts.settle_lifecycle().await;
        let children = &out.result["children"];
        (children[0]["result"].clone(), children[1]["result"].clone())
    }

    /// Finishing a task is one run: the transition and the report land
    /// under one audit row (`oxplow.command.sequence`), and the effort carries the
    /// summary.
    #[tokio::test]
    async fn completing_a_task_is_one_audited_run() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let audit = SqliteCommandAuditStore::new(fx.svc.db.clone());
        let before = audit.list_recent(50).await.unwrap().len();
        let (task, report) = complete(&fx, json!({ "summary": "shipped it" })).await;
        assert_eq!(task["state"], "done", "{task}");
        assert_eq!(
            report["effort"],
            json!(oxplow_domain::uri::build::effort_uri(fx.effort))
        );

        let rows = audit.list_recent(50).await.unwrap();
        assert_eq!(rows.len(), before + 1, "one audit row for the whole close");
        assert_eq!(rows[0].command, crate::commands::compose::SEQUENCE);

        use oxplow_db::EffortStore as _;
        let effort = fx
            .svc
            .effort_store
            .get_effort(&fx.effort)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(effort.summary.as_deref(), Some("shipped it"));
    }

    /// A report lands on the thread's latest effort once it closed; a
    /// thread with none has nothing to report on, and a person names the
    /// thread.
    #[tokio::test]
    async fn a_report_lands_on_the_threads_latest_effort() {
        use oxplow_db::EffortStore as _;
        let fx = services_with_effort().await;
        fx.svc
            .effort_store
            .finish(&fx.effort, None, None)
            .await
            .unwrap();
        let thread = oxplow_domain::uri::build::thread_uri(fx.thread);
        fx.svc
            .commands
            .run(
                &Actor::Human,
                REPORT,
                json!({ "thread": thread, "summary": "late words" }),
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
        assert_eq!(effort.summary.as_deref(), Some("late words"));

        let err = fx
            .svc
            .commands
            .run(&Actor::Human, REPORT, json!({ "summary": "s" }), false)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/thread"),
            "{err:?}"
        );
        let other = new_thread(&fx.svc, StreamId::new(1), "other").await;
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                REPORT,
                json!({ "thread": oxplow_domain::uri::build::thread_uri(other.id), "summary": "s" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no effort"), "{err}");
    }

    /// An agent reports only on its own thread's work.
    #[tokio::test]
    async fn an_agent_reports_only_on_its_own_threads_work() {
        let fx = services_with_effort().await;
        let other = new_thread(&fx.svc, StreamId::new(1), "other").await;
        let stranger = Actor::Agent {
            session_id: None,
            thread_id: Some(other.id),
            stream_id: None,
        };
        let err = fx
            .svc
            .commands
            .run(
                &stranger,
                REPORT,
                json!({ "thread": oxplow_domain::uri::build::thread_uri(fx.thread), "summary": "mine" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    }

    /// An impact names its target in the one documented vocabulary
    /// (`wiki | work_item | file | directory | git_commit | finding`); any
    /// other kind is refused at its index, not dropped.
    #[tokio::test]
    async fn an_impact_of_another_kind_is_refused() {
        let fx = services_with_effort().await;
        for kind in ["commit", "task", "dir", "git-commit", "page"] {
            let err = fx
                .svc
                .commands
                .run(
                    &agent(&fx),
                    REPORT,
                    json!({
                        "summary": "s",
                        "impacts": [
                            { "kind": "wiki", "id": "a-page" },
                            { "kind": kind, "id": "abc1234" },
                        ],
                    }),
                    false,
                )
                .await
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), message }
                    if f == "/impacts/1/kind" && message.contains("git_commit")),
                "{kind}: {err:?}"
            );
        }
    }

    /// With no report, an effort's summary is its last turn's final
    /// message; a report's summary wins.
    #[tokio::test]
    async fn the_summary_defaults_to_the_last_turns_final_message() {
        let fx = services_with_effort().await;
        let turn = |payload: &'static str| {
            let svc = fx.svc.clone();
            let thread = fx.thread;
            async move {
                for (kind, body) in [
                    (oxplow_domain::hook::HookKind::UserPromptSubmit, "{}"),
                    (oxplow_domain::hook::HookKind::Stop, payload),
                ] {
                    svc.hook_ingest
                        .ingest(crate::hook_ingest::HookEnvelope {
                            session_title: None,
                            turn_run: None,
                            kind,
                            thread_id: Some(thread),
                            stream_id: None,
                            agent_session_id: None,
                            session_id: Some("s".into()),
                            payload_json: body.into(),
                            prompt: Some("go".into()),
                            decision: None,
                            tool: None,
                            subagent: None,
                            peer: None,
                        })
                        .await
                        .unwrap();
                }
            }
        };
        turn(r#"{"last_assistant_message":"first answer"}"#).await;
        turn(r#"{"last_assistant_message":"Fixed the parser."}"#).await;
        let summary = || async {
            let out = fx
                .svc
                .sql
                .query_sql(
                    "SELECT summary FROM v_effort WHERE id = ?1",
                    vec![oxplow_db::SqlCell::Int(fx.effort.value())],
                    None,
                )
                .await
                .unwrap();
            serde_json::to_value(&out.rows[0][0]).unwrap()
        };
        assert_eq!(summary().await, json!("Fixed the parser."));
        fx.svc
            .commands
            .run(
                &agent(&fx),
                REPORT,
                json!({ "summary": "Rewrote the parser." }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(summary().await, json!("Rewrote the parser."));
    }
    /// Claims the agent states in its report are recorded on the effort
    /// beside the ones a model reads in its answers: reading the answers
    /// again leaves them, and reporting again restates the unreviewed ones
    /// while a verified one stays.
    #[tokio::test]
    async fn reported_claims_are_recorded_beside_the_inferred_ones() {
        let fx = services_with_effort().await;
        let effort = oxplow_domain::uri::build::effort_uri(fx.effort);
        let report = |claims: Value| {
            let svc = fx.svc.clone();
            let actor = agent(&fx);
            async move {
                svc.commands
                    .run(&actor, REPORT, json!({ "claims": claims }), false)
                    .await
                    .unwrap()
            }
        };
        report(json!([
            { "statement": "the parser tests pass", "kind": "tests_pass", "evidence_ref": "oxplow:oxplow/run?id=1" },
            { "statement": "no behavior change", "kind": "no_behavior_change" },
        ]))
        .await;
        fx.svc
            .commands
            .run(
                &Actor::Human,
                crate::commands::reasoning::REPLACE_INFERRED_CLAIMS,
                json!({ "effort": effort, "claims": [
                    { "statement": "handles empty input", "kind": "handles_case" },
                ] }),
                false,
            )
            .await
            .unwrap();
        let claims = || async {
            let out = fx
                .svc
                .sql
                .query_sql(
                    "SELECT statement, provenance, evidence_ref, review FROM v_claim
                     WHERE effort_id = ?1 ORDER BY statement",
                    vec![oxplow_db::SqlCell::Int(fx.effort.value())],
                    None,
                )
                .await
                .unwrap();
            serde_json::to_value(&out.rows).unwrap()
        };
        assert_eq!(
            claims().await,
            json!([
                ["handles empty input", "inferred", null, null],
                ["no behavior change", "recorded", null, null],
                [
                    "the parser tests pass",
                    "recorded",
                    "oxplow:oxplow/run?id=1",
                    null
                ],
            ])
        );

        fx.svc
            .db
            .transaction(|tx| {
                tx.execute(
                    "UPDATE claim SET review = 'verified' WHERE statement = 'no behavior change'",
                    [],
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        report(json!([{ "statement": "the lexer tests pass", "kind": "tests_pass" }])).await;
        assert_eq!(
            claims().await,
            json!([
                ["handles empty input", "inferred", null, null],
                ["no behavior change", "recorded", null, "verified"],
                ["the lexer tests pass", "recorded", null, null],
            ])
        );
    }
}
