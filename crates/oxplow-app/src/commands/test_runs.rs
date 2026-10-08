//! Test evidence an agent hands oxplow: `oxplow.test.record_run`, a run whose
//! counts oxplow couldn't read from its output. `External` over the
//! `CollectionService`: the capture with its `test.run.recorded` commits in
//! the collector's own transaction. Audited to the actor; an agent's goes
//! on its own thread whatever it names, a person names one. A report file
//! oxplow parses itself is a report collector's, run by `oxplow.collector.sync`.

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::comment::author_of;
use super::thread::acting_thread_of;
use super::{Handler, HandlerOutput, Invocation};
use crate::collection::CollectionService;

pub const RECORD_RUN: &str = "oxplow.test.record_run";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordRunInput {
    /// The thread (`thread:thr3`); an agent's is always its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
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

fn parse<T: serde::de::DeserializeOwned>(input: Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
}

fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

/// `test.record_run { thread?, command, duration_ms?, passed?, failed?, total? }`.
pub fn record_run_op(collection: CollectionService) -> Op {
    Op::new(
        "test_runs.write",
        "record_run",
        schema::<RecordRunInput>(),
        false,
        Handler::External(Arc::new(move |Invocation { actor, .. }, input| {
            let collection = collection.clone();
            Box::pin(async move {
                let input: RecordRunInput = parse(input)?;
                let thread = acting_thread_of(&actor, input.thread.as_deref())?;
                let turn = collection.command_turn(&actor, thread).await;
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
                        turn,
                    )
                    .await?;
                Ok(HandlerOutput {
                    result: json!({ "recorded": id.is_some(), "observationId": id }),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

/// The test-evidence commands, for the bus.
pub fn ops(collection: CollectionService) -> Vec<Op> {
    vec![record_run_op(collection)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{services_with_effort, EffortFixture};
    use oxplow_domain::Actor;

    const COBERTURA: &str = r#"<?xml version="1.0"?>
<coverage><packages><package name="p"><classes>
  <class name="Foo" filename="src/foo.rs"><lines>
    <line number="1" hits="3"/><line number="2" hits="0"/>
  </lines></class>
</classes></package></packages></coverage>"#;

    /// tsk863: `oxplow.collector.sync` runs a project report collector by hand —
    /// the agent's own thread records it, audited to the agent; a person
    /// names the thread, and `thread` is refused for any other collector.
    #[tokio::test]
    async fn collector_sync_reads_a_report_into_the_agents_thread() {
        let fx = services_with_effort().await;
        // Coverage is recorded while a metric consumes it.
        fx.svc.metrics.seed_catalog().await;
        std::fs::write(fx._dir.path().join("coverage.xml"), COBERTURA).unwrap();
        fx.svc.config.write().unwrap().collectors =
            vec![crate::collection::test_support::report_collector(
                "tests.coverage",
                "coverage",
                "oxplow:cobertura",
                "coverage.xml",
                "test",
            )];
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let sync = |actor: Actor, input: Value| {
            let bus = fx.svc.commands.clone();
            async move { bus.run(&actor, "oxplow.collector.sync", input, false).await }
        };
        let out = sync(
            agent.clone(),
            json!({ "owner": "project", "id": "tests.coverage" }),
        )
        .await
        .unwrap();
        assert_eq!(out.result["recorded"]["status"], "stored", "{}", out.result);
        assert_eq!(out.result["recorded"]["records"], "coverage");
        let events = fx.svc.event_log_store.read_after(0, 1000).await.unwrap();
        let coverage = events
            .iter()
            .find(|e| e.envelope.event_type == "test.coverage.recorded")
            .expect("the coverage is logged");
        assert_eq!(coverage.envelope.anchors.thread_id, Some(fx.thread));

        let err = sync(
            Actor::Human,
            json!({ "owner": "project", "id": "tests.coverage" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
        let err = sync(
            agent,
            json!({ "owner": "project", "id": "other", "thread": "thread:thr1" }),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/thread"),
            "{err:?}"
        );
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

    /// tsk923: a run a person reports on a thread whose agent is mid-turn
    /// isn't the agent's: no turn, on the capture or on its event.
    #[tokio::test]
    async fn a_persons_reported_run_carries_no_turn() {
        let fx = services_with_effort().await;
        open_turn(&fx).await;
        fx.svc
            .commands
            .run(
                &Actor::Human,
                RECORD_RUN,
                json!({
                    "command": "cargo test",
                    "thread": format!("thread:{}", fx.thread),
                    "passed": 1, "failed": 0, "total": 1,
                }),
                false,
            )
            .await
            .unwrap();
        let (open, on_capture) = turns(&fx).await;
        assert!(open.is_some(), "the agent is mid-turn");
        assert_eq!(on_capture, None);
        let events = fx.svc.event_log_store.read_after(0, 1000).await.unwrap();
        let run = events
            .iter()
            .find(|e| e.envelope.event_type == "test.run.recorded")
            .unwrap();
        assert_eq!(run.envelope.anchors.turn_id, None);
    }

    /// The agent's turn opened by a prompt on `fx`'s thread.
    async fn open_turn(fx: &EffortFixture) {
        fx.svc
            .hook_ingest
            .ingest(crate::hook_ingest::HookEnvelope {
                kind: oxplow_domain::HookKind::UserPromptSubmit,
                thread_id: Some(fx.thread),
                stream_id: None,
                agent_session_id: None,
                session_id: Some("s".into()),
                payload_json: "{}".into(),
                prompt: Some("go".into()),
                decision: None,
            })
            .await
            .unwrap();
    }

    /// The open turn, and the turn the run's capture carries.
    async fn turns(fx: &EffortFixture) -> (Option<i64>, Option<i64>) {
        fx.svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT (SELECT id FROM agent_turn WHERE ended_at IS NULL), \
                            (SELECT turn_id FROM metric_capture WHERE producer IN ('tests', 'test-run'))",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// tsk483: a run an agent reports by command is in the turn open on
    /// its thread — on the capture and on its event alike.
    #[tokio::test]
    async fn a_reported_run_carries_the_open_turn() {
        let fx = services_with_effort().await;
        fx.svc
            .hook_ingest
            .ingest(crate::hook_ingest::HookEnvelope {
                kind: oxplow_domain::HookKind::UserPromptSubmit,
                thread_id: Some(fx.thread),
                stream_id: None,
                agent_session_id: None,
                session_id: Some("s".into()),
                payload_json: "{}".into(),
                prompt: Some("go".into()),
                decision: None,
            })
            .await
            .unwrap();
        fx.svc
            .commands
            .run(
                &Actor::Agent {
                    thread_id: Some(fx.thread),
                    stream_id: None,
                },
                RECORD_RUN,
                json!({ "command": "cargo test", "passed": 1, "failed": 0, "total": 1 }),
                false,
            )
            .await
            .unwrap();
        let (open, on_capture): (i64, Option<i64>) = fx
            .svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT (SELECT id FROM agent_turn WHERE ended_at IS NULL), \
                            (SELECT turn_id FROM metric_capture WHERE producer IN ('tests', 'test-run'))",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(on_capture, Some(open));
        let events = fx.svc.event_log_store.read_after(0, 1000).await.unwrap();
        let run = events
            .iter()
            .find(|e| e.envelope.event_type == "test.run.recorded")
            .unwrap();
        assert_eq!(run.envelope.anchors.turn_id, Some(open));
    }
}
