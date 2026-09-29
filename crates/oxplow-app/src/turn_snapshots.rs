//! The turn-end snapshot (P2.3, tsk425; `.context/target-architecture.md`
//! §4.3, §6.1; `.context/agent-model.md` "Snapshot tracking").
//!
//! When a turn ends (Stop, or an interrupt — every harness reaches this
//! through `HookIngestService::ingest`), the worktree is snapshotted with a
//! `turn_end` take anchored to the turn, its thread and the thread's open
//! effort, and `agent_turn.snapshot_id` points at it. "What changed this
//! turn" is `agent_turn.start_snapshot_id` (recorded when the turn opened)
//! → `snapshot_id` — NOT the take's op parent, which other takes during
//! the turn (an effort closing, a commit, another thread's turn) move.
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
use oxplow_db::effort_store::EffortStore;
use oxplow_db::{SqliteEffortStore, SqliteThreadStore};
use oxplow_domain::snapshot::SnapshotTrigger;
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{AgentTurnId, ThreadId};

use crate::snapshot_capture::TakeRequest;
use crate::snapshot_capture_registry::SnapshotCaptureRegistry;

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
    pub captures: SnapshotCaptureRegistry,
    pub threads: Arc<SqliteThreadStore>,
    pub efforts: Arc<SqliteEffortStore>,
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
        let Some(capture) = self.captures.get(&stream) else {
            return;
        };
        let effort = self
            .efforts
            .find_single_open_for_thread(&thread)
            .await
            .ok()
            .flatten()
            .map(|e| e.id);
        let budget = Duration::from_millis(
            self.config
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .snapshot_turn_budget_ms,
        );
        let req = TakeRequest {
            trigger: SnapshotTrigger::TurnEnd,
            thread_id: Some(thread),
            turn_id: Some(turn.value()),
            effort_id: effort,
            budget: Some(budget),
        };
        // Spawned so it keeps running if we stop waiting at the budget.
        let take = tokio::spawn(async move {
            capture
                .request_snapshot(req)
                .await
                .map_err(|e| e.to_string())
        });
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
    use crate::test_fixtures::services_with_effort;

    fn envelope(kind: HookKind, thread: ThreadId) -> HookEnvelope {
        HookEnvelope {
            kind,
            thread_id: Some(thread),
            stream_id: None,
            session_id: Some("s".into()),
            payload_json: "{}".into(),
            prompt: Some("go".into()),
        }
    }

    #[tokio::test]
    async fn a_turn_ends_at_a_snapshot_and_its_diff_runs_from_where_it_started() {
        let f = services_with_effort().await;
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
        let opened = svc.agent_turn_store.get(&turn).await.unwrap().unwrap();
        assert_eq!(opened.start_snapshot_id, Some(baseline));

        // The agent edits a file, then closes its effort (complete_task's
        // effort_end take captures the edit) before the turn ends — the
        // usual flow, which used to make the turn look empty.
        std::fs::write(svc.layout.project_dir.join("made.txt"), "by the agent").unwrap();
        capture.enqueue_startup_diff().await.unwrap();
        capture
            .request_snapshot(SnapshotTrigger::EffortEnd)
            .await
            .unwrap();
        svc.hook_ingest
            .ingest(envelope(HookKind::Stop, f.thread))
            .await
            .unwrap();

        let ops = svc.snapshot_store.list_ops(stream, 5).await.unwrap();
        let op = &ops[0];
        assert_eq!(op.trigger, SnapshotTrigger::TurnEnd);
        assert_eq!(op.turn_id, Some(turn.value()));
        assert_eq!(op.thread_id, Some(f.thread));
        assert_eq!(op.effort_id, Some(f.effort));
        assert_eq!(op.budget_ms, Some(2000));
        assert_eq!(
            op.parent_snapshot_id,
            Some(op.snapshot_id),
            "nothing new at Stop"
        );
        let ended = svc.agent_turn_store.get(&turn).await.unwrap().unwrap();
        assert_eq!(ended.snapshot_id, Some(op.snapshot_id));
        // The turn's diff is start → end, and it has the edit.
        let changed = svc
            .snapshot_store
            .diff_snapshots(ended.start_snapshot_id, op.snapshot_id)
            .await
            .unwrap();
        assert!(changed.iter().any(|c| c.path == "made.txt"), "{changed:?}");

        // A second Stop (a harness repeating itself) closes nothing and
        // takes nothing.
        svc.hook_ingest
            .ingest(envelope(HookKind::Stop, f.thread))
            .await
            .unwrap();
        let after = svc.snapshot_store.list_ops(stream, 10).await.unwrap();
        assert_eq!(
            after
                .iter()
                .filter(|o| o.turn_id == Some(turn.value()))
                .count(),
            1
        );

        // The turn's own events carry the anchor too.
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
            vec!["agent.turn.ended", "agent.turn.started", "snapshot.taken"]
        );
    }

    #[tokio::test]
    async fn a_slow_take_does_not_hold_the_hook_past_its_budget() {
        let f = services_with_effort().await;
        let svc = &f.svc;
        let stream = svc.streams.list_streams().await.unwrap()[0].id;
        svc.config.write().unwrap().snapshot_turn_budget_ms = 100;
        // A capture that can't finish in 100 ms: its predrain alone is 600.
        let slow = crate::snapshot_capture::SnapshotCaptureService::new(
            svc.snapshot_store.clone(),
            svc.blobs.clone(),
            svc.layout.project_dir.clone(),
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
