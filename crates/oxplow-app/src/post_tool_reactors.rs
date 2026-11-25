//! Post-tool reactors (P3.6): collection and post-tool advisories,
//! as async consumers of `agent.tool.finished` on the event pump. They
//! used to run on a detached task the hook route spawned and waited up to
//! 2.5 s for; now they are durable (a crash re-delivers the event), keyed
//! by the event (a redelivery records nothing twice) and anchored to the
//! turn the tool ran in. What they want the agent to hear is
//! persisted as a nudge; the hook response takes the thread's undelivered
//! nudges (`AgentContext::post_tool_context`), so a nudge that finishes after
//! its hook's window goes out on the next one instead of being lost.
//!
//! - `collection` — a shell call's test / analysis / coverage runs
//!   (`CollectionService::on_shell_run`), from the command and exit code
//!   its `agent.tool.finished` carries.
//! - `advisories.post_tool` — the enabled extensions' post-tool-use
//!   advisories for the turn the tool ran in.
//! - `collection.run_reports` — a run reported any other way
//!   (`oxplow.test.record_run`, a by-hand sync) has its coverage read from its
//!   `test.run.recorded` (`CollectionService::on_test_run_recorded`).

use async_trait::async_trait;
use oxplow_db::Database;
use oxplow_domain::events::schema::{
    AgentToolFinished, AgentToolRequested, EventType, TestRunRecorded,
};
use oxplow_domain::{DomainError, StoredEvent};

use crate::advisories::AdvisoryDeps;
use crate::collection::{CollectionService, RunCause};
use crate::event_pump::AsyncEventConsumer;

pub const COLLECTION: &str = "collection";
pub const POST_TOOL_ADVISORIES: &str = "advisories.post_tool";
pub const IMMEDIATE_ADVISORIES: &str = "advisories.post_tool.immediate";
/// The two consumers that run post-tool advisories: not an advisory's
/// `after:` target.
pub const ADVISORY_CONSUMERS: &[&str] = &[POST_TOOL_ADVISORIES, IMMEDIATE_ADVISORIES];
pub const RUN_REPORTS: &str = "collection.run_reports";

/// The run `event` finished, with when it started: its cause, the tool
/// call's `agent.tool.requested`, when there is one.
pub(crate) async fn cause_of(db: &Database, event: &StoredEvent) -> Result<RunCause, DomainError> {
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
        let p = &event.envelope.payload;
        if p["kind"].as_str() != Some("shell") {
            return Ok(());
        }
        // The whole command; a call logged before it was kept (v1) has
        // its detail.
        let Some(command) = p["command"].as_str().or_else(|| p["detail"].as_str()) else {
            return Ok(());
        };
        self.collection
            .on_shell_run(
                &thread,
                crate::collection::ShellRun {
                    command,
                    exit_code: p["exit_code"].as_i64(),
                    duration_ms: p["duration_ms"].as_i64(),
                },
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

/// Post-tool advisories: `ordered` runs those that name an `after:`, the
/// other those that name none, as soon as the call is projected.
pub struct PostToolAdvisories {
    pub deps: AdvisoryDeps,
    pub ordered: bool,
}

#[async_trait]
impl AsyncEventConsumer for PostToolAdvisories {
    fn name(&self) -> &'static str {
        if self.ordered {
            POST_TOOL_ADVISORIES
        } else {
            IMMEDIATE_ADVISORIES
        }
    }

    /// The ordered advisories wait on what their targets run in (and on
    /// the immediate ones, so a call's hints keep their order).
    fn after(&self) -> Vec<String> {
        if !self.ordered {
            return Vec::new();
        }
        let catalog = self.deps.extension_catalog.unpinned(
            self.deps.worktrees.project_dir(),
            "a delivery gate: the declared targets of the running advisories, nothing else",
        );
        let mut gates = vec![IMMEDIATE_ADVISORIES.to_string()];
        gates.extend(crate::advisories::ordered_gates(&catalog));
        gates
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == AgentToolFinished::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        if let Some(thread) = event.envelope.anchors.thread_id {
            crate::advisories::for_tool_event(&self.deps, &thread, event, self.ordered).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{HookEnvelope, ToolDecision};
    use oxplow_domain::HookKind;
    use serde_json::json;

    fn hook(
        thread: oxplow_domain::ThreadId,
        kind: HookKind,
        body: serde_json::Value,
    ) -> HookEnvelope {
        HookEnvelope {
            session_title: None,
            turn_run: None,
            reading: Default::default(),
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
            tool: crate::test_fixtures::claude_tool(kind, &body),
            subagent: None,
            harness_turn: None,
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

    /// P3.6: a Bash test run is recorded by the collection
    /// reactor once — a redelivered event writes no second capture — and
    /// its `test.run.recorded` is anchored to the turn and caused by the
    /// tool event.
    #[tokio::test]
    async fn a_test_run_is_recorded_once_and_anchored_to_its_turn() {
        let f = crate::test_fixtures::services_with_thread().await;
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
        assert_eq!(run.cause.as_ref(), Some(&finished.envelope.id));
        assert_eq!(run.payload["command"], "cargo test");
    }

    /// A test run takes its time from the shell call's: the harness's
    /// `duration_ms` on the finished tool call is `v_test_run.duration_ms`.
    #[tokio::test]
    async fn a_test_run_records_how_long_its_shell_call_took() {
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        svc.hook_ingest
            .ingest(hook(f.thread, HookKind::UserPromptSubmit, json!({})))
            .await
            .unwrap();
        let mut body = bash("cargo test");
        body["duration_ms"] = json!(4200);
        svc.hook_ingest
            .ingest(hook(f.thread, HookKind::PostToolUse, body))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(count(svc, "SELECT duration_ms FROM v_test_run").await, 4200);
    }

    /// a run's capture carries the turn its tool call was made in
    /// — the event's own anchor, however late the reactor records it (here
    /// after that turn ended and another began) — and `v_test_run` and
    /// `v_capture` expose it.
    #[tokio::test]
    async fn a_run_capture_carries_the_tools_turn() {
        let f = crate::test_fixtures::services_with_thread().await;
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

    /// The report notice is for the person. A late agent nudge reaches
    /// the agent through its next hook response, once.
    #[tokio::test]
    async fn hook_only_delivers_agent_nudges_once_by_thread() {
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        svc.config.write().unwrap().testing.command = Some("bun run test:collect".into());
        // The run is in a turn, so its nudge is paced by the turn's request.
        svc.hook_ingest
            .ingest(hook(f.thread, HookKind::UserPromptSubmit, json!({})))
            .await
            .unwrap();
        let event = svc
            .hook_ingest
            .ingest(hook(f.thread, HookKind::PostToolUse, bash("bun test")))
            .await
            .unwrap();
        assert!(event.finished_tool_seq.is_some());
        assert_eq!(
            svc.agent_context
                .post_tool_context(svc, &f.thread, None, event.finished_tool_seq)
                .await,
            None,
            "the hook carries no person notice"
        );
        svc.event_pump
            .settle(&[super::COLLECTION], std::time::Duration::from_secs(5))
            .await;
        let notices = svc
            .nudge_store
            .list_for_thread(&f.thread.to_string())
            .await
            .unwrap();
        assert!(notices.iter().any(|notice| {
            notice.kind == "report-less-run"
                && notice.audience == oxplow_db::Audience::Person
                && notice.message.contains("bun run test:collect")
        }));
        // A nudge written after its hook answered goes out on the next hook.
        svc.collection
            .persist_nudge(
                &f.thread,
                crate::collection::Raised::agent("late", "a late nudge", "cmd"),
                crate::collection::RunOrigin::Command { turn: None },
            )
            .await;
        assert_eq!(
            svc.agent_context
                .post_tool_context(svc, &f.thread, None, None)
                .await
                .as_deref(),
            Some("a late nudge")
        );
        assert_eq!(
            svc.agent_context
                .post_tool_context(svc, &f.thread, None, None)
                .await,
            None
        );
    }

    #[tokio::test]
    async fn immediate_and_collection_ordered_advisories_process_the_same_tool_event() {
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        let path = f._dir.path().join(".oxplow/extensions/guide");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("extension.yaml"),
            r#"
manifest: 2
name: guide
intent: { purpose: test }
permissions: [agent.write, sql.read]
advisories:
  - id: immediate
    on: post-tool-use
    once_per: turn
    query: SELECT 'Immediate' AS message
  - id: collected
    on: post-tool-use
    after: [collection]
    once_per: turn
    query: SELECT 'Saw test results' AS message FROM v_test_run WHERE turn_id = :turn_id
"#,
        )
        .unwrap();
        crate::test_fixtures::activate(svc).await;
        svc.hook_ingest
            .ingest(hook(f.thread, HookKind::UserPromptSubmit, json!({})))
            .await
            .unwrap();
        let event = svc
            .hook_ingest
            .ingest(hook(f.thread, HookKind::PostToolUse, bash("cargo test")))
            .await
            .unwrap();
        let seq = event.finished_tool_seq.unwrap();
        let timeout = std::time::Duration::from_secs(5);
        // Let the background consumers run as they do in a managed session.
        let background = svc.event_pump.clone().spawn();
        assert!(
            svc.event_pump
                .wait_for_hook_event(super::IMMEDIATE_ADVISORIES, seq, timeout)
                .await
        );
        let text = svc
            .agent_context
            .post_tool_context(svc, &f.thread, None, Some(seq))
            .await
            .unwrap();
        assert!(
            text.contains("Immediate") && text.contains("Saw test results"),
            "{text}"
        );
        assert_eq!(
            svc.agent_context
                .post_tool_context(svc, &f.thread, None, None)
                .await,
            None
        );
        background.abort();
    }

    #[tokio::test]
    async fn claude_post_edit_hook_respects_the_oxtask_opt_in() {
        use oxplow_domain::Actor;

        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .finish();
        let _trace = tracing::subscriber::set_default(subscriber);
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        let background = svc.event_pump.clone().spawn();
        svc.hook_ingest
            .ingest(hook(f.thread, HookKind::UserPromptSubmit, json!({})))
            .await
            .unwrap();
        for (workflow, path) in [
            ("off", "one.txt"),
            ("agent_tracks", "two.txt"),
            ("agent_tracks", "three.txt"),
        ] {
            svc.commands
                .run(
                    &Actor::Human,
                    "oxtask.tasks.set_workflow",
                    json!({"workflow": workflow}),
                    true,
                )
                .await
                .unwrap();
            let event = svc
                .hook_ingest
                .ingest(hook(
                    f.thread,
                    HookKind::PostToolUse,
                    json!({
                        "tool_name": "Edit", "tool_input": { "file_path": path },
                        "tool_response": { "success": true }
                    }),
                ))
                .await
                .unwrap();
            let text = svc
                .agent_context
                .post_tool_context(svc, &f.thread, Some("claude"), event.finished_tool_seq)
                .await;
            if path == "two.txt" {
                assert!(
                    text.is_some_and(|text| text.contains("no task is in progress on this thread"))
                );
            } else {
                assert_eq!(text, None);
            }
        }
        background.abort();
    }
}
