//! Reporting on an effort (P8.A7): `effort.report` — what an agent says it
//! did when it finishes a work item (summary, files, impacts, the test
//! runs that were its) — and `effort.amend`, correcting that afterwards.
//! Both are `External`: they read the worktree (whether a touched file
//! still exists, which paths the project snapshots) and the snapshot
//! diff the report is checked against, so they can't run in the bus's
//! transaction. Finishing a task is `command.sequence [work_item.transition,
//! effort.report]`: one audit row, the transition first so the effort the
//! report attaches to has closed.
//!
//! An agent reports and amends only on its own thread. The report's result
//! is the agent's feedback: the `file_review` (what it claimed against
//! what the snapshot diff saw), `link_warnings`, and a `decision_hint` for
//! a big effort with no recorded decisions. A discrepancy, or test runs
//! left unattributed, stashes the effort for the Stop hook's EFFORT
//! REVIEW, which points at `effort.amend`.

use std::path::PathBuf;
use std::sync::Arc;

use oxplow_db::{
    Database, EffortFileChange, EffortStore as _, SqliteAttributionStore, SqliteEffortStore,
    SqliteSnapshotStore, STATE_ACKNOWLEDGED, STATE_CLAIMED, STATE_UNATTRIBUTED,
};
use oxplow_domain::refs::build::{thread_ref, validate_work_item_ref};
use oxplow_domain::vcs::Vcs;
use oxplow_domain::{
    Actor, Atomicity, CommandEffect, CommandError, CommandSpec, Confirm, EffortId, Invokers,
    Lifecycle, TaskImpact, ThreadId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::thread::parse_thread_ref;
use super::{Command, Handler, HandlerOutput, Invocation};
use crate::file_ref_version::ResolvedFileVersion;
use crate::sql_gateway::SqlGateway;
use crate::task_service::{compute_effort_file_review, TaskService};
use crate::thread_runtime::ThreadRuntimeRegistry;

pub const REPORT: &str = "effort.report";
pub const AMEND: &str = "effort.amend";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportInput {
    /// The work item (`work_item:oxplow:tsk42`).
    pub work_item: String,
    /// The thread that did it (`thread:thr3`); an agent's is always its
    /// own; a person's defaults to the item's last effort's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// What shipped (markdown; `[[…]]` links are checked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Repo-relative paths edited for it.
    #[serde(default)]
    pub touched_files: Vec<String>,
    /// Outcomes beyond the edits: `{ kind, id, action? }` — a wiki page,
    /// a task, a commit, a finding.
    #[serde(default)]
    pub impacts: Vec<TaskImpact>,
    /// Test runs (`run:<id>`) that were this effort's.
    #[serde(default)]
    pub claim_runs: Vec<String>,
    /// Test runs that weren't.
    #[serde(default)]
    pub disclaim_runs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AmendInput {
    /// The effort (`eff12`).
    pub effort: String,
    /// Paths to claim.
    #[serde(default)]
    pub add_files: Vec<String>,
    /// Paths to disclaim (acknowledged, so they aren't flagged again).
    #[serde(default)]
    pub remove_files: Vec<String>,
    /// Test runs (`run:<id>`) to claim.
    #[serde(default)]
    pub claim_runs: Vec<String>,
    /// Test runs to disclaim.
    #[serde(default)]
    pub disclaim_runs: Vec<String>,
}

/// What reporting reads and writes.
#[derive(Clone)]
pub struct EffortDeps {
    pub tasks: TaskService,
    pub efforts: Arc<SqliteEffortStore>,
    pub snapshots: Arc<SqliteSnapshotStore>,
    pub attribution: Arc<SqliteAttributionStore>,
    pub runtime: Arc<ThreadRuntimeRegistry>,
    pub sql: SqlGateway,
    pub db: Database,
    /// The ref kinds a summary's links may name.
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    pub project_dir: PathBuf,
    pub vcs: Arc<dyn Vcs>,
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

fn failed(e: impl std::fmt::Display) -> CommandError {
    CommandError::Failed {
        message: e.to_string(),
    }
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
                    thread_ref(own)
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

/// Claim and disclaim test runs on an effort's attribution ledger.
async fn settle_runs(
    deps: &EffortDeps,
    effort: &EffortId,
    claim: &[String],
    disclaim: &[String],
) -> Result<(), CommandError> {
    for (refs, state) in [(claim, STATE_CLAIMED), (disclaim, STATE_ACKNOWLEDGED)] {
        for run in refs.iter().filter(|r| !r.is_empty()) {
            deps.attribution
                .set_state(effort, "run", run, state, None)
                .await
                .map_err(failed)?;
        }
    }
    Ok(())
}

fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

fn spec(name: &str, summary: &str, schema: Value) -> CommandSpec {
    CommandSpec {
        name: name.into(),
        summary: summary.into(),
        input_schema: schema,
        invokers: Invokers::ALL,
        confirm: Confirm::Never,
        undoable: false,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::External,
        effect: CommandEffect::Record,
    }
}

async fn report(
    deps: &EffortDeps,
    actor: Actor,
    input: ReportInput,
) -> Result<Value, CommandError> {
    validate_work_item_ref(&input.work_item).map_err(|e| invalid("/work_item", e.to_string()))?;
    // The report reads the settled effort: a transition just before it
    // (the close's) has its snapshot bracket pinned.
    deps.tasks.settle_lifecycle().await;
    let named = input.thread.as_deref().map(parse_thread_ref).transpose()?;
    let last = deps
        .efforts
        .most_recent_for_work_item(&input.work_item)
        .await
        .map_err(failed)?;
    let thread = match agents_thread(&actor, named)? {
        Some(own) => {
            if let Some(e) = last.as_ref().filter(|e| e.thread_id != own) {
                return Err(CommandError::Denied {
                    reason: format!(
                        "`{}` was worked on `{}`, not this agent's thread",
                        input.work_item,
                        thread_ref(e.thread_id)
                    ),
                });
            }
            own
        }
        None => named
            .or(last.as_ref().map(|e| e.thread_id))
            .ok_or_else(|| {
                invalid(
                    "/thread",
                    format!("`{}` has no effort yet — name the thread", input.work_item),
                )
            })?,
    };
    let summary = input.summary.filter(|s| !s.trim().is_empty());
    // A path the project never snapshots is dropped before both the record
    // and the review: the diff could never confirm it either way.
    let touched = deps
        .tasks
        .claimable_paths(&thread, &input.touched_files)
        .await;
    let review = if summary.is_some() || !touched.is_empty() || !input.impacts.is_empty() {
        let worktree = worktree_of(&deps.db, thread).await;
        deps.tasks
            .record_effort(
                &deps.efforts,
                &input.work_item,
                &thread,
                &touched,
                summary.clone(),
                &input.impacts,
                worktree.as_deref(),
            )
            .await
            .map_err(failed)?;
        compute_effort_file_review(&deps.efforts, &deps.snapshots, &input.work_item, &touched).await
    } else {
        None
    };
    let effort = deps
        .efforts
        .most_recent_for_work_item(&input.work_item)
        .await
        .map_err(failed)?
        .map(|e| e.id);
    let mut decision_hint = None;
    if let Some(effort) = effort {
        // The run claims first, so a fully reconciled effort doesn't nag.
        settle_runs(deps, &effort, &input.claim_runs, &input.disclaim_runs).await?;
        let residue = !deps
            .attribution
            .list_refs(&effort, "run", STATE_UNATTRIBUTED)
            .await
            .unwrap_or_default()
            .is_empty();
        if review.is_some() || residue {
            deps.runtime.record_pending_effort_review(&thread, effort);
        }
        decision_hint = crate::reasoning::missing_decisions_hint(&deps.sql, effort.value()).await;
    }
    let link_warnings = match &summary {
        Some(body) => {
            // Files in the thread's worktree (tsk895).
            let root = worktree_of(&deps.db, thread)
                .await
                .unwrap_or_else(|| deps.project_dir.clone());
            crate::link_check::check_links_at(&deps.db, &deps.vocabulary, &root, &*deps.vcs, body)
                .await
        }
        None => Vec::new(),
    };
    Ok(json!({
        "effort": effort.map(|e| e.to_string()),
        "file_review": review,
        "link_warnings": link_warnings,
        "decision_hint": decision_hint,
    }))
}

async fn amend(deps: &EffortDeps, actor: Actor, input: AmendInput) -> Result<Value, CommandError> {
    let id = EffortId::try_from_str(&input.effort).ok_or_else(|| {
        invalid(
            "/effort",
            format!("`{}` isn't an effort id (eff…)", input.effort),
        )
    })?;
    let effort = deps
        .efforts
        .get_effort(&id)
        .await
        .map_err(failed)?
        .ok_or_else(|| invalid("/effort", format!("no effort `{id}`")))?;
    if let Some(own) = agents_thread(&actor, None)? {
        if effort.thread_id != own {
            return Err(CommandError::Denied {
                reason: format!(
                    "an agent amends only its own thread's efforts (`{}`)",
                    thread_ref(own)
                ),
            });
        }
    }
    for path in input.remove_files.iter().filter(|p| !p.is_empty()) {
        deps.efforts.remove_file(&id, path).await.map_err(failed)?;
        // Acknowledged, so the Stop hook's recompute doesn't flag the
        // same path again; claiming it later clears that.
        deps.efforts
            .acknowledge_unclaimed_path(&id, path)
            .await
            .map_err(failed)?;
    }
    // Every added path takes the effort's snapshot-version pin.
    let version: ResolvedFileVersion = deps.tasks.resolve_effort_file_version(&effort).await;
    let added = deps
        .tasks
        .claimable_paths(&effort.thread_id, &input.add_files)
        .await;
    for path in &added {
        // The amend carries no stat, and the change kind is informational.
        deps.efforts
            .record_file(&id, path, EffortFileChange::Updated, version.as_ref())
            .await
            .map_err(failed)?;
        deps.efforts
            .forget_acknowledged_path(&id, path)
            .await
            .map_err(failed)?;
    }
    settle_runs(deps, &id, &input.claim_runs, &input.disclaim_runs).await?;
    Ok(json!({
        "effort": id.to_string(),
        "added": added,
        "removed": input.remove_files,
        "claimed_runs": input.claim_runs,
        "disclaimed_runs": input.disclaim_runs,
    }))
}

/// `effort.report { work_item, thread?, summary?, touched_files?, impacts?,
/// claim_runs?, disclaim_runs? }`.
pub fn report_command(deps: EffortDeps) -> Command {
    Command::new(
        spec(
            REPORT,
            "Report what you did on a work item: `summary`, the `touched_files`, `impacts` \
             beyond the edits, and the test runs that were yours (`claim_runs`) or weren't \
             (`disclaim_runs`). Returns `{ effort, file_review, link_warnings, decision_hint }`: \
             a non-null `file_review` means the snapshot diff disagreed with your files — fix \
             it with `effort.amend`, or leave it if your list was right.",
            schema::<ReportInput>(),
        ),
        Handler::External(Arc::new(move |Invocation { actor, .. }, input| {
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
    .expect("effort.report is a valid command")
}

/// `effort.amend { effort, add_files?, remove_files?, claim_runs?, disclaim_runs? }`.
pub fn amend_command(deps: EffortDeps) -> Command {
    Command::new(
        spec(
            AMEND,
            "Correct an effort's attribution after the fact: claim or disclaim files \
             (`add_files` / `remove_files`) when the snapshot diff disagreed with what you \
             reported, and test runs (`claim_runs` / `disclaim_runs`, the `run:<id>` refs the \
             EFFORT REVIEW names). An agent amends only its own thread's efforts.",
            schema::<AmendInput>(),
        ),
        Handler::External(Arc::new(move |Invocation { actor, .. }, input| {
            let deps = deps.clone();
            Box::pin(async move {
                let input: AmendInput = parse(input)?;
                Ok(HandlerOutput {
                    result: amend(&deps, actor, input).await?,
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .expect("effort.amend is a valid command")
}

/// The reporting commands, for the bus.
pub fn commands(deps: EffortDeps) -> Vec<Command> {
    vec![report_command(deps.clone()), amend_command(deps)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{new_thread, services_with_effort, EffortFixture};
    use oxplow_db::{SqliteCommandAuditStore, STATE_ACKNOWLEDGED, STATE_CLAIMED};
    use oxplow_domain::refs::build::work_item_ref;
    use oxplow_domain::StreamId;

    fn agent(fx: &EffortFixture) -> Actor {
        Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    /// What an agent runs to finish its task: the transition to done and
    /// the report, as one `command.sequence`. Returns the task's row and
    /// the report's result.
    async fn complete(fx: &EffortFixture, mut report: Value) -> (Value, Value) {
        let item = work_item_ref(fx.task);
        report["work_item"] = item.clone().into();
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
        fx.svc.tasks.settle_lifecycle().await;
        let children = &out.result["children"];
        (children[0]["result"].clone(), children[1]["result"].clone())
    }

    /// Finishing a task is one run: the transition and the report land
    /// under one audit row (`command.sequence`), the effort carries the
    /// summary and files, and the run claim settles the ledger.
    #[tokio::test]
    async fn completing_a_task_is_one_audited_run() {
        let fx = services_with_effort().await;
        let audit = SqliteCommandAuditStore::new(fx.svc.db.clone());
        let before = audit.list_recent(50).await.unwrap().len();
        let (task, report) = complete(
            &fx,
            json!({
                "summary": "shipped it",
                "touched_files": ["src/a.rs"],
                "claim_runs": ["run:9"],
            }),
        )
        .await;
        assert_eq!(task["status"], "done", "{task}");
        assert_eq!(report["effort"], json!(fx.effort.to_string()));

        let rows = audit.list_recent(50).await.unwrap();
        assert_eq!(rows.len(), before + 1, "one audit row for the whole close");
        assert_eq!(rows[0].command, crate::commands::compose::SEQUENCE);

        let effort = fx
            .svc
            .effort_store
            .most_recent_for_work_item(&work_item_ref(fx.task))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(effort.summary.as_deref(), Some("shipped it"));
        let files = fx.svc.effort_store.list_files(&effort.id).await.unwrap();
        assert_eq!(
            files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["src/a.rs"]
        );
        assert_eq!(
            fx.svc
                .attribution_store
                .list_refs(&effort.id, "run", STATE_CLAIMED)
                .await
                .unwrap(),
            ["run:9"]
        );
    }

    /// An agent reports only on its own thread's work.
    #[tokio::test]
    async fn an_agent_reports_only_on_its_own_threads_work() {
        let fx = services_with_effort().await;
        let other = new_thread(&fx.svc, StreamId::new(1), "other").await;
        let stranger = Actor::Agent {
            thread_id: Some(other.id),
            stream_id: None,
        };
        let err = fx
            .svc
            .commands
            .run(
                &stranger,
                REPORT,
                json!({ "work_item": work_item_ref(fx.task), "summary": "mine" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    }

    /// Amending claims and disclaims files (a disclaim is acknowledged;
    /// re-claiming clears it) and runs; another thread's agent is refused.
    #[tokio::test]
    async fn an_amend_settles_files_and_runs() {
        let fx = services_with_effort().await;
        let effort = fx.effort.to_string();
        let run = |input: Value| {
            let svc = fx.svc.clone();
            let actor = agent(&fx);
            async move { svc.commands.run(&actor, AMEND, input, false).await }
        };
        run(json!({ "effort": effort, "add_files": ["src/keep.rs", "src/drop.rs"] }))
            .await
            .unwrap();
        run(json!({
            "effort": effort,
            "remove_files": ["src/drop.rs"],
            "claim_runs": ["run:1"],
            "disclaim_runs": ["run:2"],
        }))
        .await
        .unwrap();
        let store = &fx.svc.effort_store;
        let files = store.list_files(&fx.effort).await.unwrap();
        assert_eq!(
            files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["src/keep.rs"]
        );
        assert_eq!(
            store.list_acknowledged_paths(&fx.effort).await.unwrap(),
            ["src/drop.rs"]
        );
        let ledger = |state| fx.svc.attribution_store.list_refs(&fx.effort, "run", state);
        assert_eq!(ledger(STATE_CLAIMED).await.unwrap(), ["run:1"]);
        assert_eq!(ledger(STATE_ACKNOWLEDGED).await.unwrap(), ["run:2"]);

        run(json!({ "effort": effort, "add_files": ["src/drop.rs"] }))
            .await
            .unwrap();
        assert!(store
            .list_acknowledged_paths(&fx.effort)
            .await
            .unwrap()
            .is_empty());

        let other = new_thread(&fx.svc, StreamId::new(1), "other").await;
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Agent {
                    thread_id: Some(other.id),
                    stream_id: None,
                },
                AMEND,
                json!({ "effort": effort, "add_files": ["x.rs"] }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    }

    /// A big effort with no recorded decisions is nudged to record them.
    #[tokio::test]
    async fn a_big_report_without_decisions_is_nudged() {
        let fx = services_with_effort().await;
        let files: Vec<String> = (0..9).map(|i| format!("src/f{i}.rs")).collect();
        let (_, report) = complete(&fx, json!({ "summary": "done", "touched_files": files })).await;
        let hint = report["decision_hint"].as_str().expect("a hint");
        assert!(hint.contains("effort.record_decision"), "{hint}");
    }
}
