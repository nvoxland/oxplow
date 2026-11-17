//! The turn-end snapshot (P2.3; `.context/agent-model.md`
//! "Snapshot tracking").
//!
//! When a turn ends (Stop, or an interrupt — every harness reaches this
//! through `HookIngestService::ingest`), the worktree is snapshotted with a
//! `turn_end` take anchored to the turn and its thread. "What changed this
//! turn" is found by time on the stream's snapshot line (the `turn` change
//! kind, `change_analysis::end_take`; its files are `v_turn_file`) — NOT the
//! take's op parent, which other takes during the turn (a commit, another
//! thread's turn) move.
//!
//! **The budget.** The Stop hook is on the agent's critical path, so it
//! waits at most `snapshotTurnBudgetMs` (default 2000). A take that runs
//! longer is never aborted — a partial snapshot is worse than a late one —
//! it keeps going in the background, and its op row and `snapshot.taken`
//! event record `over_budget` (plus a warn log). Over-budget is visible,
//! never silent.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use oxplow_config::OxplowConfig;
use oxplow_db::SqliteThreadStore;
use oxplow_domain::snapshot::SnapshotTrigger;
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{AgentTurnId, ThreadId};

/// Takes the snapshot a turn ends at. `HookIngestService` calls it after
/// the turn's row is closed; absent in bare ingest tests.
#[async_trait]
pub trait TurnSnapshots: Send + Sync {
    /// Snapshot the worktree for `turn` of `thread`, waiting at most the
    /// configured budget. Never fails the hook: problems are logged.
    async fn take_turn_end(&self, thread: ThreadId, turn: AgentTurnId);
}

/// The production [`TurnSnapshots`]: the thread's stream's capture service.
pub struct CaptureTurnSnapshots {
    /// The snapshot implementations: the take is the active one's mark.
    pub snapshots: Arc<oxplow_domain::snapshot::SnapshotRegistry>,
    pub threads: Arc<SqliteThreadStore>,
    pub config: Arc<RwLock<OxplowConfig>>,
}

#[async_trait]
impl TurnSnapshots for CaptureTurnSnapshots {
    async fn take_turn_end(&self, thread: ThreadId, turn: AgentTurnId) {
        let stream = match self.threads.get(&thread).await {
            Ok(Some(t)) => t.stream_id,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(error = %e, %thread, "turn-end snapshot: thread lookup failed");
                return;
            }
        };
        let Some(snapshots) = self.snapshots.active() else {
            return;
        };
        let budget = Duration::from_millis(
            self.config
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .snapshot_turn_budget_ms,
        );
        let req = oxplow_domain::snapshot::MarkRequest {
            thread: Some(thread),
            turn: Some(turn.value()),
            budget: Some(budget),
            ..oxplow_domain::snapshot::MarkRequest::new(stream, SnapshotTrigger::TurnEnd)
        };
        // Spawned so it keeps running if we stop waiting at the budget.
        let take =
            tokio::spawn(async move { snapshots.mark(&req).await.map_err(|e| e.to_string()) });
        match tokio::time::timeout(budget, take).await {
            Ok(Ok(Ok(_))) => {}
            Ok(Ok(Err(e))) => tracing::warn!(error = %e, %turn, "turn-end snapshot failed"),
            Ok(Err(e)) => tracing::warn!(error = %e, %turn, "turn-end snapshot task panicked"),
            Err(_) => tracing::info!(
                %turn,
                budget_ms = budget.as_millis() as u64,
                "turn-end snapshot still running past its budget; it will be recorded as over budget",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::hook::HookKind;
    use oxplow_domain::stores::AgentTurnStore;

    use crate::hook_ingest::HookEnvelope;
    use crate::test_fixtures::services_with_thread;

    fn envelope(kind: HookKind, thread: ThreadId) -> HookEnvelope {
        HookEnvelope {
            session_title: None,
            turn_run: None,
            reading: Default::default(),
            kind,
            thread_id: Some(thread),
            stream_id: None,
            agent_session_id: None,
            session_id: Some("s".into()),
            payload_json: "{}".into(),
            prompt: Some("go".into()),
            decision: None,
            tool: None,
            subagent: None,
            harness_turn: None,
        }
    }

    /// a turn a session end closes (an exit mid-turn) ends
    /// at a snapshot too, as one a Stop closes does.
    #[tokio::test]
    async fn a_turn_a_session_end_closes_ends_at_a_snapshot() {
        let f = services_with_thread().await;
        let svc = &f.svc;
        let stream = svc.streams.list_streams().await.unwrap()[0].id;
        let capture = svc.snapshot_captures.get(&stream).unwrap();
        capture.enqueue_startup_diff().await.unwrap();
        capture
            .request_snapshot(SnapshotTrigger::Startup)
            .await
            .unwrap();
        svc.hook_ingest
            .ingest(envelope(HookKind::UserPromptSubmit, f.thread))
            .await
            .unwrap();
        let turn = svc.agent_turn_store.list_open(&f.thread).await.unwrap()[0].id;
        svc.hook_ingest
            .ingest(HookEnvelope {
                payload_json: r#"{"reason":"prompt_input_exit"}"#.into(),
                ..envelope(HookKind::SessionEnd, f.thread)
            })
            .await
            .unwrap();
        // Nothing changed in the turn: its end take answered the stream's
        // current snapshot and logged no operation, only its event.
        let ops = svc.snapshot_store.list_ops(stream, 5).await.unwrap();
        assert!(ops.iter().all(|o| o.trigger != SnapshotTrigger::TurnEnd));
        let takes = turn_end_events(svc, turn.value()).await;
        assert_eq!(takes.len(), 1);
        assert_eq!(takes[0].envelope.payload["unchanged"], true);
        assert_eq!(takes[0].envelope.payload["snapshot"], ops[0].snapshot_id);
    }

    /// The `snapshot.taken` events a turn's end take logged.
    async fn turn_end_events(svc: &crate::Services, turn: i64) -> Vec<oxplow_domain::StoredEvent> {
        svc.event_log_store
            .read_after(0, 10_000)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| {
                e.envelope.event_type == "snapshot.taken"
                    && e.envelope.anchors.turn_id == Some(turn)
            })
            .collect()
    }

    #[tokio::test]
    async fn a_turn_ends_at_a_snapshot_and_its_diff_runs_from_where_it_started() {
        let f = services_with_thread().await;
        let svc = &f.svc;
        let stream = svc.streams.list_streams().await.unwrap()[0].id;
        let capture = svc.snapshot_captures.get(&stream).unwrap();
        // A baseline before the turn.
        std::fs::write(svc.layout.project_dir.join("seed.txt"), "seed").unwrap();
        capture.enqueue_startup_diff().await.unwrap();
        let baseline = capture
            .request_snapshot(SnapshotTrigger::Startup)
            .await
            .unwrap()
            .unwrap();
        svc.hook_ingest
            .ingest(envelope(HookKind::UserPromptSubmit, f.thread))
            .await
            .unwrap();
        let turn = svc.agent_turn_store.list_open(&f.thread).await.unwrap()[0].id;

        // The agent edits a file, and another take captures the edit before
        // the turn ends — which used to make the turn look empty.
        std::fs::write(svc.layout.project_dir.join("made.txt"), "by the agent").unwrap();
        capture.enqueue_startup_diff().await.unwrap();
        capture
            .request_snapshot(SnapshotTrigger::Manual)
            .await
            .unwrap();
        svc.hook_ingest
            .ingest(envelope(HookKind::Stop, f.thread))
            .await
            .unwrap();

        // The other take captured the edit; Stop found nothing new, so
        // the turn's end take logged an event but no operation.
        let ops = svc.snapshot_store.list_ops(stream, 5).await.unwrap();
        let op = &ops[0];
        assert_eq!(op.trigger, SnapshotTrigger::Manual);
        let takes = turn_end_events(svc, turn.value()).await;
        assert_eq!(takes.len(), 1);
        let take = &takes[0].envelope;
        assert_eq!(take.payload["unchanged"], true);
        assert_eq!(take.payload["snapshot"], op.snapshot_id);
        assert_eq!(take.payload["budget_ms"], 2000);
        assert_eq!(take.anchors.thread_id, Some(f.thread));
        // From the baseline to where the turn ended, the edit is there.
        let changed = svc
            .snapshot_store
            .diff_snapshots(Some(baseline), op.snapshot_id)
            .await
            .unwrap();
        assert!(changed.iter().any(|c| c.path == "made.txt"), "{changed:?}");

        // A second Stop (a harness repeating itself) closes nothing and
        // takes nothing.
        svc.hook_ingest
            .ingest(envelope(HookKind::Stop, f.thread))
            .await
            .unwrap();
        assert_eq!(turn_end_events(svc, turn.value()).await.len(), 1);

        // The turn's own events carry the anchor too — the status its
        // prompt set (running) and the one its Stop set (idle).
        let mut anchored: Vec<String> = svc
            .event_log_store
            .read_after(0, 1000)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.envelope.anchors.turn_id == Some(turn.value()))
            .map(|e| e.envelope.event_type)
            .collect();
        anchored.sort();
        assert_eq!(
            anchored,
            vec![
                "agent.prompt.submitted",
                "agent.status.changed",
                "agent.status.changed",
                "agent.turn.ended",
                "agent.turn.started",
                "snapshot.taken"
            ]
        );
    }

    #[tokio::test]
    async fn a_slow_take_does_not_hold_the_hook_past_its_budget() {
        let f = services_with_thread().await;
        let svc = &f.svc;
        let stream = svc.streams.list_streams().await.unwrap()[0].id;
        svc.config.write().unwrap().snapshot_turn_budget_ms = 100;
        // A capture that can't finish in 100 ms: its predrain alone is 600.
        let slow = crate::snapshot_capture::SnapshotCaptureService::new(
            svc.snapshot_store.clone(),
            svc.blobs.clone(),
            svc.layout.project_dir.clone(),
            std::sync::Arc::new(crate::vcs::GitProvider),
            stream,
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_predrain_delay(Duration::from_millis(600));
        // Something for the take to record.
        std::fs::write(svc.layout.project_dir.join("big.txt"), "work").unwrap();
        slow.enqueue_startup_diff().await.unwrap();
        svc.snapshot_captures
            .insert_for_test(stream, Arc::new(slow));
        svc.hook_ingest
            .ingest(envelope(HookKind::UserPromptSubmit, f.thread))
            .await
            .unwrap();
        let turn = svc.agent_turn_store.list_open(&f.thread).await.unwrap()[0].id;
        let started = std::time::Instant::now();
        svc.hook_ingest
            .ingest(envelope(HookKind::Stop, f.thread))
            .await
            .unwrap();
        let waited = started.elapsed();
        assert!(
            waited < Duration::from_millis(500),
            "hook waited {waited:?}"
        );

        // The take finishes in the background and says it ran over.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let op = loop {
            let ops = svc.snapshot_store.list_ops(stream, 5).await.unwrap();
            if let Some(op) = ops.into_iter().find(|o| o.turn_id == Some(turn.value())) {
                break op;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "turn-end take never landed"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        assert!(op.over_budget);
        assert_eq!(op.budget_ms, Some(100));
        assert!(op.elapsed_ms > 100);
    }
}
