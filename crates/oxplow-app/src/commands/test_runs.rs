//! Test evidence an agent hands oxplow (P8.A8): `test.record_run` (a run
//! the Bash hook couldn't see — a sub-agent's), `test.ingest_coverage` and
//! `test.ingest_analysis` (a report file oxplow parses itself). `External`
//! over the `CollectionService`: they read the stream's worktree and report
//! files, and the capture with its `test.run.recorded` /
//! `test.coverage.recorded` / analysis facts commits in the collector's own
//! transaction. Each is audited to the actor; an agent's go on its own
//! thread whatever it names, a person names one.

use std::sync::Arc;

use oxplow_domain::refs::build::{task_of_work_item_ref, validate_work_item_ref};
use oxplow_domain::{
    Actor, Atomicity, CommandEffect, CommandError, CommandSpec, Confirm, Invokers, Lifecycle,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::comment::author_of;
use super::thread::acting_thread_of;
use super::{Command, Handler, HandlerOutput};
use crate::collection::{AnalysisIngest, CollectionService, CoverageIngest};

pub const RECORD_RUN: &str = "test.record_run";
pub const INGEST_COVERAGE: &str = "test.ingest_coverage";
pub const INGEST_ANALYSIS: &str = "test.ingest_analysis";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordRunInput {
    /// The thread (`thread:thr3`); an agent's is always its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// The work item the run was for (`work_item:oxplow:tsk42`), so it's
    /// attributed to that effort even with sibling efforts open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item: Option<String>,
    /// The command that ran the tests.
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passed: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestInput {
    /// The thread (`thread:thr3`); an agent's is always its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// The report; absent, the project's `collection` profile's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_path: Option<String>,
    /// Its format (`lcov`, `cobertura`, `eslint-json`, …); absent, the
    /// profile's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

fn parse<T: serde::de::DeserializeOwned>(input: Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
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

/// An ingest's outcome, as the caller reads it.
pub fn coverage_json(outcome: &CoverageIngest) -> Value {
    match outcome {
        CoverageIngest::NoOpenEffort => json!({ "status": "no_open_effort" }),
        CoverageIngest::NotConfigured => json!({
            "status": "not_configured",
            "hint": "set collection.coverageReportPath + coverageFormat in .oxplow/project.yaml (run /oxplow:configure)",
        }),
        CoverageIngest::ReportMissing(path) => json!({ "status": "report_missing", "path": path }),
        CoverageIngest::StaleReport(path) => json!({ "status": "stale_report", "path": path }),
        CoverageIngest::ParseError(err) => json!({ "status": "parse_error", "error": err }),
        CoverageIngest::NoBaseline => json!({ "status": "no_baseline" }),
        CoverageIngest::NoChangedCoverage => json!({ "status": "no_changed_coverage" }),
        CoverageIngest::Stored {
            observation_id,
            summary_pct,
            changed_lines,
            covered_lines,
        } => json!({
            "status": "stored",
            "observationId": observation_id,
            "summaryPct": summary_pct,
            "changedLines": changed_lines,
            "coveredLines": covered_lines,
        }),
    }
}

/// An analysis ingest's outcome, as the caller reads it.
pub fn analysis_json(outcome: &AnalysisIngest) -> Value {
    match outcome {
        AnalysisIngest::NoOpenEffort => json!({ "status": "no_open_effort" }),
        AnalysisIngest::NotConfigured => json!({
            "status": "not_configured",
            "hint": "add an analysis report (e.g. format eslint-json / clippy-json) to collection.reports in .oxplow/project.yaml, or pass report_path + format explicitly",
        }),
        AnalysisIngest::ReportMissing(path) => json!({ "status": "report_missing", "path": path }),
        AnalysisIngest::StaleReport(path) => json!({ "status": "stale_report", "path": path }),
        AnalysisIngest::ParseError(err) => json!({ "status": "parse_error", "error": err }),
        AnalysisIngest::Stored {
            observation_id,
            error_count,
            warning_count,
            info_count,
            note_count,
            findings,
        } => json!({
            "status": "stored",
            "observationId": observation_id,
            "errorCount": error_count,
            "warningCount": warning_count,
            "infoCount": info_count,
            "noteCount": note_count,
            "findings": findings,
        }),
    }
}

/// `test.record_run { thread?, work_item?, command, duration_ms?, passed?, failed?, total? }`.
pub fn record_run_command(collection: CollectionService) -> Command {
    Command::new(
        spec(
            RECORD_RUN,
            "Record a test run with pass/fail counts the Bash hook can't see, marked `asserted`. \
             oxplow records the main agent's runs from the Bash hook already, but a dispatched \
             sub-agent's are invisible to it, so a sub-agent should run this for its test runs. \
             Pass `work_item` (from your brief) so the run lands on your item's effort even with \
             sibling efforts open. Returns `{ recorded, observationId }`.",
            schema::<RecordRunInput>(),
        ),
        Handler::External(Arc::new(move |actor: Actor, input| {
            let collection = collection.clone();
            Box::pin(async move {
                let input: RecordRunInput = parse(input)?;
                let thread = acting_thread_of(&actor, input.thread.as_deref())?;
                let task = match input.work_item.as_deref() {
                    Some(item) => {
                        validate_work_item_ref(item).map_err(|e| CommandError::Invalid {
                            field: Some("/work_item".into()),
                            message: e.to_string(),
                        })?;
                        task_of_work_item_ref(item)
                    }
                    None => None,
                };
                let id = collection
                    .record_test_run(
                        &thread,
                        &input.command,
                        None,
                        input.duration_ms,
                        input.passed,
                        input.failed,
                        input.total,
                        "asserted",
                        author_of(&actor),
                        None,
                        task,
                    )
                    .await?;
                Ok(HandlerOutput {
                    result: json!({ "recorded": id.is_some(), "observationId": id }),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .expect("test.record_run is a valid command")
}

/// `test.ingest_coverage { thread?, report_path?, format? }`.
pub fn ingest_coverage_command(collection: CollectionService) -> Command {
    Command::new(
        spec(
            INGEST_COVERAGE,
            "Ingest a coverage report into the thread's open effort as diff coverage over the \
             lines that effort changed. oxplow parses it (cobertura / lcov / jacoco-xml) — point \
             at the report, never report numbers yourself. `report_path` / `format` default to \
             the project's `collection` profile. Returns a status: `stored` (with `summaryPct`) \
             or why nothing landed (no_open_effort / not_configured / report_missing / \
             no_baseline / no_changed_coverage).",
            schema::<IngestInput>(),
        ),
        Handler::External(Arc::new(move |actor: Actor, input| {
            let collection = collection.clone();
            Box::pin(async move {
                let input: IngestInput = parse(input)?;
                let thread = acting_thread_of(&actor, input.thread.as_deref())?;
                let outcome = collection
                    .ingest_coverage(&thread, input.report_path, input.format, false)
                    .await?;
                Ok(HandlerOutput {
                    result: coverage_json(&outcome),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .expect("test.ingest_coverage is a valid command")
}

/// `test.ingest_analysis { thread?, report_path?, format? }`.
pub fn ingest_analysis_command(collection: CollectionService) -> Command {
    Command::new(
        spec(
            INGEST_ANALYSIS,
            "Ingest a static-analysis report (linter / analyzer findings) into the thread's open \
             effort. oxplow parses it through the collector registry (`eslint-json`, \
             `clippy-json`, …) — point at the report, never report counts yourself. \
             `report_path` / `format` default to the first analysis report in the `collection` \
             profile. Returns a status: `stored` (per-severity counts) or why nothing landed \
             (no_open_effort / not_configured / report_missing / parse_error).",
            schema::<IngestInput>(),
        ),
        Handler::External(Arc::new(move |actor: Actor, input| {
            let collection = collection.clone();
            Box::pin(async move {
                let input: IngestInput = parse(input)?;
                let thread = acting_thread_of(&actor, input.thread.as_deref())?;
                let outcome = collection
                    .ingest_analysis(&thread, input.report_path, input.format, false)
                    .await?;
                Ok(HandlerOutput {
                    result: analysis_json(&outcome),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .expect("test.ingest_analysis is a valid command")
}

/// The test-evidence commands, for the bus.
pub fn commands(collection: CollectionService) -> Vec<Command> {
    vec![
        record_run_command(collection.clone()),
        ingest_coverage_command(collection.clone()),
        ingest_analysis_command(collection),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{new_thread, services_with_effort};
    use oxplow_db::SqliteCommandAuditStore;
    use oxplow_domain::refs::build::thread_ref;
    use oxplow_domain::StreamId;

    const COBERTURA: &str = r#"<?xml version="1.0"?>
<coverage><packages><package name="p"><classes>
  <class name="Foo" filename="src/foo.rs"><lines>
    <line number="1" hits="3"/><line number="2" hits="0"/>
  </lines></class>
</classes></package></packages></coverage>"#;

    /// An agent's coverage ingest runs the collector's and is audited to
    /// it (what the collector records, and its `test.coverage.recorded`,
    /// are `collection.rs`'s tests); naming another thread is refused.
    #[tokio::test]
    async fn a_coverage_ingest_is_audited() {
        let fx = services_with_effort().await;
        std::fs::write(fx._dir.path().join("coverage.xml"), COBERTURA).unwrap();
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let out = fx
            .svc
            .commands
            .run(
                &agent,
                INGEST_COVERAGE,
                json!({ "report_path": "coverage.xml", "format": "cobertura" }),
                false,
            )
            .await
            .unwrap();
        assert!(out.result["status"].is_string(), "{}", out.result);
        let audit = SqliteCommandAuditStore::new(fx.svc.db.clone())
            .list_recent(5)
            .await
            .unwrap();
        assert_eq!(audit[0].command, INGEST_COVERAGE);
        assert_eq!(
            audit[0].actor_kind,
            oxplow_domain::events::schema::ActorKind::Agent
        );

        let other = new_thread(&fx.svc, StreamId::new(1), "other").await;
        let err = fx
            .svc
            .commands
            .run(
                &agent,
                INGEST_COVERAGE,
                json!({ "thread": thread_ref(other.id) }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    }

    /// A recorded run is the agent's `asserted` run, on its own thread.
    #[tokio::test]
    async fn a_recorded_run_lands_on_the_agents_thread() {
        let fx = services_with_effort().await;
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Agent {
                    thread_id: Some(fx.thread),
                    stream_id: None,
                },
                RECORD_RUN,
                json!({ "command": "cargo test", "passed": 3, "failed": 0, "total": 3 }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["recorded"], true, "{}", out.result);
        let events = fx.svc.event_log_store.read_after(0, 1000).await.unwrap();
        let run = events
            .iter()
            .find(|e| e.envelope.event_type == "test.run.recorded")
            .expect("the run is logged");
        assert_eq!(run.envelope.anchors.thread_id, Some(fx.thread));
    }
}
