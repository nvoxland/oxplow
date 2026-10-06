//! Reporting on an effort: `effort.report` — what an agent says it did
//! when it finishes a work item: a summary and the impacts beyond its edits.
//! Which files and runs were its is observed, never reported
//! (`.context/work-tracking.md`). `External`: it checks the summary's links
//! against the worktree, so it can't run in the bus's transaction.
//! Finishing a task is `command.sequence [work_item.transition,
//! effort.report]`: one audit row.
//!
//! An agent reports only on its own thread. The result carries
//! `link_warnings` and a `decision_hint` for a big effort with no recorded
//! decisions.

use std::path::PathBuf;
use std::sync::Arc;

use oxplow_db::{Database, EffortStore as _, SqliteEffortStore};
use oxplow_domain::refs::build::{thread_ref, validate_work_item_ref};
use oxplow_domain::vcs::Vcs;
use oxplow_domain::{
    Actor, Atomicity, CommandEffect, CommandError, CommandSpec, Confirm, Invokers, Lifecycle,
    TaskImpact, ThreadId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::thread::parse_thread_ref;
use super::{Command, Handler, HandlerOutput, Invocation};
use crate::sql_gateway::SqlGateway;
use crate::task_service::TaskService;

pub const REPORT: &str = "effort.report";

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
    /// Outcomes beyond the edits: `{ kind, id, action? }` — a wiki page,
    /// a task, a commit, a finding.
    #[serde(default)]
    pub impacts: Vec<TaskImpact>,
}

/// What reporting reads and writes.
#[derive(Clone)]
pub struct EffortDeps {
    pub tasks: TaskService,
    pub efforts: Arc<SqliteEffortStore>,
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
    use oxplow_db::page_ref_projections::{impact_kind, IMPACT_KINDS};
    if let Some((i, imp)) = input
        .impacts
        .iter()
        .enumerate()
        .find(|(_, imp)| impact_kind(&imp.kind).is_none())
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
    if summary.is_some() || !input.impacts.is_empty() {
        deps.tasks
            .record_effort(
                &deps.efforts,
                &input.work_item,
                &thread,
                summary.clone(),
                &input.impacts,
            )
            .await
            .map_err(failed)?;
    }
    let effort = deps
        .efforts
        .most_recent_for_work_item(&input.work_item)
        .await
        .map_err(failed)?
        .map(|e| e.id);
    let decision_hint = match effort {
        Some(effort) => crate::reasoning::missing_decisions_hint(&deps.sql, effort.value()).await,
        None => None,
    };
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
        "link_warnings": link_warnings,
        "decision_hint": decision_hint,
    }))
}

/// `effort.report { work_item, thread?, summary?, impacts? }`.
pub fn report_command(deps: EffortDeps) -> Command {
    Command::new(
        spec(
            REPORT,
            "Report what you did on a work item: a `summary` and any `impacts` beyond the \
             edits (a wiki page, a task, a commit, a finding). Optional: the files and test \
             runs that were yours are observed. Returns `{ effort, link_warnings, \
             decision_hint }`.",
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

/// The reporting commands, for the bus.
pub fn commands(deps: EffortDeps) -> Vec<Command> {
    vec![report_command(deps)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{new_thread, services_with_effort, EffortFixture};
    use oxplow_db::SqliteCommandAuditStore;
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
    /// under one audit row (`command.sequence`), and the effort carries the
    /// summary.
    #[tokio::test]
    async fn completing_a_task_is_one_audited_run() {
        let fx = services_with_effort().await;
        let audit = SqliteCommandAuditStore::new(fx.svc.db.clone());
        let before = audit.list_recent(50).await.unwrap().len();
        let (task, report) = complete(&fx, json!({ "summary": "shipped it" })).await;
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

    /// An impact names its target in the one documented vocabulary
    /// (`wiki | task | file | directory | git_commit | finding`); any other
    /// kind is refused at its index, not dropped.
    #[tokio::test]
    async fn an_impact_of_another_kind_is_refused() {
        let fx = services_with_effort().await;
        for kind in ["commit", "work_item", "dir", "git-commit", "page"] {
            let err = fx
                .svc
                .commands
                .run(
                    &agent(&fx),
                    REPORT,
                    json!({
                        "work_item": work_item_ref(fx.task),
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

    /// A big effort with no recorded decisions is nudged to record them.
    #[tokio::test]
    async fn a_big_report_without_decisions_is_nudged() {
        let fx = services_with_effort().await;
        use oxplow_db::EffortStore as _;
        for i in 0..9 {
            fx.svc
                .effort_store
                .record_file(
                    &fx.effort,
                    &format!("src/f{i}.rs"),
                    oxplow_db::EffortFileChange::Updated,
                    oxplow_db::effort_store::FileRefVersion {
                        local_snapshot_id: 0,
                        closest_vcs_rev: None,
                        vcs_rev_exact: false,
                    },
                )
                .await
                .unwrap();
        }
        let (_, report) = complete(&fx, json!({ "summary": "done" })).await;
        let hint = report["decision_hint"].as_str().expect("a hint");
        assert!(hint.contains("effort.record_decision"), "{hint}");
    }
}
