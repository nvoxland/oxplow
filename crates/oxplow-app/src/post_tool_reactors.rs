//! Post-tool reactors (P3.6, tsk476): collection and post-tool advisories,
//! as async consumers of `agent.tool.finished` on the event pump. They
//! used to run on a detached task the hook route spawned and waited up to
//! 2.5 s for; now they are durable (a crash re-delivers the event), keyed
//! by the event (a redelivery records nothing twice) and anchored to the
//! turn and effort the tool ran in. What they want the agent to hear is
//! persisted as a nudge; the hook response takes the thread's undelivered
//! nudges (`AgentContext::post_tool_context`), so a nudge that finishes after
//! its hook's window goes out on the next one instead of being lost.
//!
//! - `collection` — a Bash command's test / analysis / coverage runs
//!   (`CollectionService::on_post_tool_use`), reading the command
//!   and its output back from `event_content`.
//! - `advisories.post_tool` — the enabled extensions' post-tool-use
//!   advisories for the effort the tool ran in.
//! - `collection.run_reports` — a run reported any other way
//!   (`oxplow.test.record_run`, a by-hand sync) has its coverage read from its
//!   `test.run.recorded` (`CollectionService::on_test_run_recorded`,
//!   tsk1015).

use async_trait::async_trait;
use oxplow_db::{event_content_store, Database};
use oxplow_domain::events::schema::{
    AgentToolFinished, AgentToolRequested, EventType, TestRunRecorded,
};
use oxplow_domain::{DomainError, StoredEvent};

use crate::advisories::AdvisoryDeps;
use crate::collection::{CollectionService, RunCause};
use crate::event_pump::AsyncEventConsumer;
use crate::extensions::AdvisoryOn;

pub const COLLECTION: &str = "collection";
pub const POST_TOOL_ADVISORIES: &str = "advisories.post_tool";
pub const RUN_REPORTS: &str = "collection.run_reports";

/// The run `event` finished, with when it started: its cause, the tool
/// call's `agent.tool.requested` (tsk888), when there is one.
async fn cause_of(db: &Database, event: &StoredEvent) -> Result<RunCause, DomainError> {
    let started = match event.envelope.cause.clone() {
        Some(id) => db
            .read(move |tx| oxplow_db::event_log_store::get_tx(tx, &id))
            .await?
            .filter(|c| c.envelope.event_type == AgentToolRequested::TYPE)
            .map(|c| c.envelope.at),
        None => None,
    };
    Ok(RunCause {
        event_id: event.envelope.id.as_str().to_string(),
        seq: event.seq,
        anchors: event.envelope.anchors.clone(),
        at: event.envelope.at,
        started,
    })
}

/// A stored body as JSON (`Null` when retention removed it).
async fn content(
    db: &Database,
    event: &StoredEvent,
    key: &str,
) -> Result<serde_json::Value, DomainError> {
    let Some(hash) = event.envelope.payload[key]["hash"].as_str() else {
        return Ok(serde_json::Value::Null);
    };
    Ok(event_content_store::read(db, hash)
        .await?
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(serde_json::Value::Null))
}

pub struct CollectionConsumer {
    pub collection: CollectionService,
    pub db: Database,
}

#[async_trait]
impl AsyncEventConsumer for CollectionConsumer {
    fn name(&self) -> &'static str {
        COLLECTION
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == AgentToolFinished::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let Some(thread) = event.envelope.anchors.thread_id else {
            return Ok(());
        };
        if event.envelope.payload["tool"].as_str() != Some("Bash") {
            return Ok(());
        }
        // The hook payload the collector parses, rebuilt from the event.
        let payload = serde_json::json!({
            "tool_name": "Bash",
            "tool_input": content(&self.db, event, "input").await?,
            "tool_response": content(&self.db, event, "output").await?,
        });
        self.collection
            .on_post_tool_use(
                &thread,
                &payload.to_string(),
                crate::collection::RunOrigin::Event(&cause_of(&self.db, event).await?),
            )
            .await
            .map(|_| ())
    }
}

/// A recorded run's coverage, read from its `test.run.recorded`.
pub struct RunReportsConsumer {
    pub collection: CollectionService,
}

#[async_trait]
impl AsyncEventConsumer for RunReportsConsumer {
    fn name(&self) -> &'static str {
        RUN_REPORTS
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == TestRunRecorded::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        self.collection.on_test_run_recorded(event).await
    }
}

pub struct PostToolAdvisories {
    pub deps: AdvisoryDeps,
}

#[async_trait]
impl AsyncEventConsumer for PostToolAdvisories {
    fn name(&self) -> &'static str {
        POST_TOOL_ADVISORIES
    }

    /// An advisory reads what collection recorded for the same run
    /// (`v_effort_observation`), so it sees the run only after collection.
    fn after(&self) -> Vec<String> {
        vec![COLLECTION.to_string()]
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == AgentToolFinished::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        if let Some(thread) = event.envelope.anchors.thread_id {
            crate::advisories::for_thread(
                &self.deps,
                &thread,
                AdvisoryOn::PostToolUse,
                Some(&cause_of(&self.deps.db, event).await?),
            )
            .await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{HookEnvelope, ToolDecision};
    use oxplow_db::EffortStore as _;
    use oxplow_domain::HookKind;
    use serde_json::json;

    fn hook(
        thread: oxplow_domain::ThreadId,
        kind: HookKind,
        body: serde_json::Value,
    ) -> HookEnvelope {
        HookEnvelope {
            kind,
            thread_id: Some(thread),
            stream_id: None,
            agent_session_id: None,
            session_id: Some("s".into()),
            payload_json: body.to_string(),
            prompt: Some("go".into()),
            decision: (kind == HookKind::PreToolUse).then_some(ToolDecision {
                allowed: true,
                reason: None,
            }),
        }
    }

    fn bash(command: &str) -> serde_json::Value {
        json!({"tool_name": "Bash", "tool_input": {"command": command}, "tool_response": {"exit_code": 0, "stdout": "ok"}})
    }

    async fn count(svc: &crate::Services, sql: &'static str) -> i64 {
        let sl = crate::sql_gateway::SqlGateway::new(svc.db.clone());
        let rows = sl.query_sql(sql, vec![], None).await.unwrap().rows;
        serde_json::to_value(&rows).unwrap()[0][0].as_i64().unwrap()
    }

    /// P3.6 (tsk476): a Bash test run is recorded by the collection
    /// reactor once — a redelivered event writes no second capture — and
    /// its `test.run.recorded` is anchored to the turn and caused by the
    /// tool event.
    #[tokio::test]
    async fn a_test_run_is_recorded_once_and_anchored_to_its_turn() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        svc.hook_ingest
            .ingest(hook(f.thread, HookKind::UserPromptSubmit, json!({})))
            .await
            .unwrap();
        svc.hook_ingest
            .ingest(hook(f.thread, HookKind::PostToolUse, bash("cargo test")))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        svc.event_log_store
            .set_checkpoint(super::COLLECTION.into(), 0)
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(count(svc, "SELECT count(*) FROM v_test_run").await, 1);
        let events = svc.event_log_store.read_after(0, 1000).await.unwrap();
        let finished = events
            .iter()
            .find(|e| e.envelope.event_type == "agent.tool.finished")
            .unwrap();
        let runs: Vec<_> = events
            .iter()
            .filter(|e| e.envelope.event_type == "test.run.recorded")
            .collect();
        assert_eq!(runs.len(), 1);
        let run = &runs[0].envelope;
        // The run carries the tool event's own anchors.
        assert_eq!(run.anchors.turn_id, finished.envelope.anchors.turn_id);
        assert_eq!(run.anchors.effort_id, Some(f.effort));
        assert_eq!(run.cause.as_ref(), Some(&finished.envelope.id));
        assert_eq!(run.payload["command"], "cargo test");
    }

    /// tsk483: a run's capture carries the turn its tool call was made in
    /// — the event's own anchor, however late the reactor records it (here
    /// after that turn ended and another began) — and `v_test_run` and
    /// `v_capture` expose it.
    #[tokio::test]
    async fn a_run_capture_carries_the_tools_turn() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        svc.hook_ingest
            .ingest(hook(f.thread, HookKind::UserPromptSubmit, json!({})))
            .await
            .unwrap();
        svc.hook_ingest
            .ingest(hook(f.thread, HookKind::PostToolUse, bash("cargo test")))
            .await
            .unwrap();
        svc.hook_ingest
            .ingest(hook(f.thread, HookKind::Stop, json!({})))
            .await
            .unwrap();
        svc.hook_ingest
            .ingest(hook(f.thread, HookKind::UserPromptSubmit, json!({})))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        let events = svc.event_log_store.read_after(0, 1000).await.unwrap();
        let ran_in = events
            .iter()
            .find(|e| e.envelope.event_type == "agent.tool.finished")
            .and_then(|e| e.envelope.anchors.turn_id)
            .expect("the tool call was in a turn");
        assert_eq!(
            count(svc, "SELECT turn_id FROM v_test_run").await,
            ran_in,
            "v_test_run"
        );
        assert_eq!(
            count(
                svc,
                "SELECT turn_id FROM v_capture WHERE producer IN ('tests', 'test-run')"
            )
            .await,
            ran_in,
            "v_capture"
        );
        assert_ne!(
            count(svc, "SELECT max(id) FROM v_agent_turn").await,
            ran_in,
            "a later turn is open by then"
        );
    }

    /// The effort the command ran in owns the run, even when the reactor
    /// records it after that effort closed.
    #[tokio::test]
    async fn a_run_recorded_after_the_close_belongs_to_its_effort() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        svc.hook_ingest
            .ingest(hook(
                f.thread,
                HookKind::PostToolUse,
                bash("cargo test -p x"),
            ))
            .await
            .unwrap();
        svc.effort_store
            .finish(&f.effort, None, None)
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            count(svc, "SELECT count(*) FROM v_test_run WHERE effort_id = 1").await,
            1
        );
    }

    /// A report-less run's nudge reaches the agent through the hook
    /// response once; one that misses its hook goes out on the next.
    #[tokio::test]
    async fn nudges_are_delivered_once_by_thread() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        svc.config.write().unwrap().testing.command = Some("bun run test:collect".into());
        svc.hook_ingest
            .ingest(hook(f.thread, HookKind::PostToolUse, bash("bun test")))
            .await
            .unwrap();
        let body = bash("bun test");
        let first = svc
            .agent_context
            .post_tool_context(svc, &f.thread, Some("s"), &body)
            .await;
        assert!(
            first
                .as_deref()
                .is_some_and(|m| m.contains("bun run test:collect")),
            "{first:?}"
        );
        assert_eq!(
            svc.agent_context
                .post_tool_context(svc, &f.thread, Some("s"), &body)
                .await,
            None,
            "delivered once"
        );
        // A nudge written after its hook answered goes out on the next hook.
        svc.collection
            .persist_nudge(
                &f.thread,
                None,
                crate::collection::Raised::agent("late", "a late nudge", "cmd"),
                crate::collection::RunOrigin::Command { turn: None },
            )
            .await;
        assert_eq!(
            svc.agent_context
                .post_tool_context(svc, &f.thread, Some("s"), &json!({"tool_name": "Read"}))
                .await
                .as_deref(),
            Some("a late nudge")
        );
    }
}
