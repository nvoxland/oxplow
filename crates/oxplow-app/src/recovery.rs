//! Daemon recovery on startup.
//!
//! Closes any `agent_turn` rows the previous boot left open. The pane
//! that owned them is dead, so the turn can't ever `Stop` on its own
//! and the row would otherwise pin the work panel to a phantom
//! in-progress entry.
//!
//! Agent status needs no reset: it is the newest logged
//! `agent.status.changed`, and what the rail shows is derived from the
//! logged activity, so a turn this closes reads as ended.
//!
//! Called once from `Services::boot` after the DB is open. Idempotent.

use oxplow_domain::refs::build::work_item_ref;
use std::sync::Arc;

use tracing::info;

use oxplow_db::{EffortStore, SqliteEffortStore, SqliteTaskStore, SqliteThreadStore};
use oxplow_domain::stores::{AgentTurnStore, ThreadStore};
use oxplow_domain::DomainError;

use crate::snapshot_capture_registry::SnapshotCaptureRegistry;
use oxplow_domain::snapshot::SnapshotTrigger;

#[derive(Clone)]
pub struct RecoveryService {
    turns: Arc<dyn AgentTurnStore>,
    tasks: Arc<SqliteTaskStore>,
    efforts: Arc<SqliteEffortStore>,
    /// Optional wiring for bracketing an orphaned effort's close with an
    /// end snapshot. When absent (e.g. minimal test setups), orphan
    /// efforts are still closed — they finish with `finish(None, None)`:
    /// no end snapshot, so nothing for the close's reconcile to diff.
    threads: Option<Arc<SqliteThreadStore>>,
    snapshot_captures: Option<SnapshotCaptureRegistry>,
}

impl RecoveryService {
    pub fn new(
        turns: Arc<dyn AgentTurnStore>,
        tasks: Arc<SqliteTaskStore>,
        efforts: Arc<SqliteEffortStore>,
    ) -> Self {
        Self {
            turns,
            tasks,
            efforts,
            threads: None,
            snapshot_captures: None,
        }
    }

    /// Attach the thread store + per-stream snapshot capture registry so
    /// a restart-recovery orphan close captures an `EffortEnd` snapshot —
    /// the bracket the effort-lifecycle consumer then reconciles against,
    /// as for any close. Must be called after the registry is built and
    /// its streams registered (see `Services::new`).
    pub fn with_end_snapshots(
        mut self,
        threads: Arc<SqliteThreadStore>,
        snapshot_captures: SnapshotCaptureRegistry,
    ) -> Self {
        self.threads = Some(threads);
        self.snapshot_captures = Some(snapshot_captures);
        self
    }

    /// Close orphaned `agent_turn` rows and heal effort-lifecycle
    /// orphans (the reconciliation half of the transactional-
    /// boundaries design: durable intent first, stragglers healed at
    /// boot). Returns counts so callers can log them.
    pub async fn run(&self) -> Result<RecoveryReport, DomainError> {
        let mut closed_turns = 0usize;

        // We don't have a way to enumerate every thread that may have
        // an open turn without scanning agent_turn directly. Use the
        // index on (thread_id WHERE ended_at IS NULL) implicitly via
        // `list_all_open` (added on the trait below) to keep this
        // O(open turns) instead of O(threads).
        let open = self.turns.list_all_open().await?;
        for turn in open {
            self.turns
                .close(
                    &turn.id,
                    Some("interrupted_by_restart".into()),
                    oxplow_domain::hook::TurnOutcome::Restart,
                )
                .await?;
            closed_turns += 1;
        }

        // Lifecycle invariant: a thread-attached task is in_progress
        // ⟺ it has exactly one open effort. Heal both orphan
        // directions left by crashes (or by data that predates the
        // transactional transition).
        let in_progress = self.tasks.list_in_progress().await?;
        let in_progress_ids: std::collections::HashSet<_> =
            in_progress.iter().map(|t| t.id).collect();
        let mut closed_efforts = 0usize;
        for effort in self.efforts.list_all_open().await? {
            // Only oxplow tasks carry the status half of the invariant; an
            // effort on another provider's work item is opened and closed
            // by `effort.open` / `effort.close` and left alone here.
            let Some(task_id) = effort.task_id() else {
                continue;
            };
            if !in_progress_ids.contains(&task_id) {
                // Death/restart case: the worktree still reflects the dead
                // effort's final state, so bracket the effort with an
                // EffortEnd snapshot now (best-effort — it never blocks the
                // close). The close logs `effort.closed`, and the
                // effort-lifecycle consumer reconciles what the effort left
                // unclaimed, as it does for every close (tsk942).
                let end_snapshot = self.capture_orphan_end_snapshot(&effort).await;
                self.efforts.finish(&effort.id, end_snapshot, None).await?;
                closed_efforts += 1;
            }
        }
        let mut opened_efforts = 0usize;
        for task in &in_progress {
            let Some(thread_id) = task.thread_id else {
                continue;
            };
            if self
                .efforts
                .find_open_for_work_item(&work_item_ref(task.id))
                .await?
                .is_none()
            {
                self.efforts
                    .start(&work_item_ref(task.id), &thread_id, None)
                    .await?;
                opened_efforts += 1;
            }
        }

        info!(
            closed_turns,
            closed_efforts, opened_efforts, "daemon recovery complete"
        );
        Ok(RecoveryReport {
            closed_turns,
            closed_efforts,
            opened_efforts,
        })
    }

    /// Capture an `EffortEnd` snapshot for an orphaned effort so its
    /// close has a snapshot bracket to reconcile against. Returns the
    /// snapshot id, or `None` when the capture registry isn't wired, the
    /// effort has no start snapshot (nothing to bracket), the stream's
    /// capture service can't be resolved, or the capture fails. Drains
    /// the worktree's current state first (`enqueue_startup_diff`) since
    /// recovery runs before the boot startup sweep — without it the
    /// dirty set is empty and `request_snapshot` would just return the
    /// existing latest snapshot, missing edits that landed after the
    /// last capture but before the crash.
    async fn capture_orphan_end_snapshot(&self, effort: &oxplow_db::Effort) -> Option<i64> {
        // No start snapshot → no bracket → nothing to reconcile.
        effort.start_snapshot_id?;
        let threads = self.threads.as_ref()?;
        let registry = self.snapshot_captures.as_ref()?;
        let stream_id = match threads.get(&effort.thread_id).await {
            Ok(Some(thread)) => thread.stream_id,
            Ok(None) => return None,
            Err(e) => {
                tracing::warn!(error = %e, effort = %effort.id, "recovery: thread lookup failed");
                return None;
            }
        };
        let capture = registry.get(&stream_id)?;
        if let Err(e) = capture.enqueue_startup_diff().await {
            tracing::warn!(error = %e, effort = %effort.id, "recovery: startup diff failed");
            // Still attempt a snapshot — a partial drain is better than none.
        }
        match capture
            .request_snapshot(crate::snapshot_capture::TakeRequest {
                trigger: SnapshotTrigger::EffortEnd,
                thread_id: Some(effort.thread_id),
                turn_id: None,
                effort_id: Some(effort.id),
                budget: None,
            })
            .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %e, effort = %effort.id, "recovery: end snapshot failed");
                None
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RecoveryReport {
    pub closed_turns: usize,
    /// Open efforts finished because their task is no longer in_progress.
    pub closed_efforts: usize,
    /// Efforts opened for in_progress tasks that had none.
    pub opened_efforts: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteAgentTurnStore, SqliteStreamStore, SqliteThreadStore};
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{
        AgentTurn, AgentTurnId, Stream, StreamId, StreamKind, Thread, ThreadId, ThreadStatus,
        Timestamp,
    };

    #[tokio::test]
    async fn closes_open_turn_left_behind_by_prior_boot() {
        let db = Database::in_memory();
        let now = Timestamp::from_unix_ms(1);
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/p".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteStreamStore::new(db.clone()).upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::new(1),
            stream_id: s.id,
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            acp_agent: None,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteThreadStore::new(db.clone()).upsert(&t).await.unwrap();
        let turns = Arc::new(SqliteAgentTurnStore::new(db.clone()));

        let turn = AgentTurn {
            id: AgentTurnId::placeholder(),
            thread_id: t.id,
            prompt: "do".into(),
            answer: None,
            session_id: None,
            started_at: now,
            ended_at: None,
            start_snapshot_id: None,
            snapshot_id: None,
        };
        turns.open(&turn).await.unwrap();

        let svc = RecoveryService::new(
            turns.clone(),
            Arc::new(SqliteTaskStore::new(db.clone())),
            Arc::new(SqliteEffortStore::new(db.clone())),
        );
        let report = svc.run().await.unwrap();
        assert_eq!(report.closed_turns, 1);

        let still_open = turns.list_open(&t.id).await.unwrap();
        assert!(still_open.is_empty());
    }

    #[tokio::test]
    async fn heals_effort_lifecycle_orphans_in_both_directions() {
        use oxplow_domain::stores::TaskStore as _;
        use oxplow_domain::{Task, TaskActorKind, TaskAuthor, TaskId, TaskPriority, TaskStatus};

        let db = Database::in_memory();
        let now = Timestamp::from_unix_ms(1);
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/p".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteStreamStore::new(db.clone()).upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::new(1),
            stream_id: s.id,
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            acp_agent: None,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteThreadStore::new(db.clone()).upsert(&t).await.unwrap();

        let tasks = Arc::new(SqliteTaskStore::new(db.clone()));
        let efforts = Arc::new(SqliteEffortStore::new(db.clone()));
        let row = |title: &str, status: TaskStatus| Task {
            id: TaskId::placeholder(),
            thread_id: Some(t.id),
            parent_id: None,
            title: title.into(),
            description: String::new(),
            status,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
        };
        // Orphan A: in_progress task with NO open effort.
        let no_effort = tasks
            .insert(&row("no effort", TaskStatus::InProgress))
            .await
            .unwrap();
        // Orphan B: done task with an open effort left behind.
        let stale = tasks.insert(&row("stale", TaskStatus::Done)).await.unwrap();
        efforts
            .start(&work_item_ref(stale), &t.id, None)
            .await
            .unwrap();

        let svc = RecoveryService::new(
            Arc::new(SqliteAgentTurnStore::new(db.clone())),
            tasks.clone(),
            efforts.clone(),
        );
        let report = svc.run().await.unwrap();
        assert_eq!(report.closed_efforts, 1);
        assert_eq!(report.opened_efforts, 1);
        assert!(efforts
            .find_open_for_work_item(&work_item_ref(stale))
            .await
            .unwrap()
            .is_none());
        assert!(efforts
            .find_open_for_work_item(&work_item_ref(no_effort))
            .await
            .unwrap()
            .is_some());

        // Second run is a no-op — the invariant now holds.
        let again = svc.run().await.unwrap();
        assert_eq!(again.closed_efforts, 0);
        assert_eq!(again.opened_efforts, 0);
    }

    /// tsk942: recovery closes an orphaned effort and brackets it with an
    /// end snapshot. What the effort changed but never claimed is
    /// reconciled where every close's is — by the effort-lifecycle
    /// consumer, on the close's `effort.closed` — and recovery records
    /// none of it itself.
    #[tokio::test]
    async fn an_orphan_recovery_closes_is_reconciled_by_the_lifecycle_consumer() {
        use oxplow_db::EffortStore as _;
        use oxplow_domain::stores::TaskStore as _;
        use oxplow_domain::{Task, TaskActorKind, TaskAuthor, TaskId, TaskPriority, TaskStatus};

        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        let stream = svc.streams.list_streams().await.unwrap()[0].id;
        let capture = svc.snapshot_captures.get(&stream).unwrap();

        // The worktree as the effort found it, and a start snapshot.
        std::fs::write(root.join("foo.rs"), "v1").unwrap();
        capture.enqueue_startup_diff().await.unwrap();
        let start = capture
            .request_snapshot(SnapshotTrigger::EffortStart)
            .await
            .unwrap()
            .expect("start snapshot");

        // Orphaned: a task no longer in progress whose effort never closed
        // (the process died mid-effort), with a change it never claimed.
        let now = Timestamp::now();
        let task = svc
            .task_store
            .insert(&Task {
                id: TaskId::placeholder(),
                thread_id: Some(f.thread),
                parent_id: None,
                title: "dead".into(),
                description: String::new(),
                status: TaskStatus::Done,
                priority: TaskPriority::Medium,
                sort_index: 0,
                created_by: TaskActorKind::User,
                created_at: now,
                updated_at: now,
                completed_at: None,
                deleted_at: None,
                note_count: 0,
                author: Some(TaskAuthor::User),
            })
            .await
            .unwrap();
        let effort = svc
            .effort_store
            .start(&work_item_ref(task), &f.thread, Some(start))
            .await
            .unwrap();
        std::fs::write(root.join("foo.rs"), "version-two-longer").unwrap();
        // A run the effort saw and never claimed.
        let mut run = oxplow_db::NewMetricCapture::done(stream.value(), "tests", "junit");
        run.thread_id = Some(f.thread.value());
        run.trigger = Some("on-report".into());
        let run = svc.fact_store.record_facts(run, Vec::new()).await.unwrap();

        assert_eq!(svc.recovery.run().await.unwrap().closed_efforts, 1);
        let closed = svc
            .effort_store
            .get_effort(&effort.id)
            .await
            .unwrap()
            .unwrap();
        assert!(closed.ended_at.is_some(), "the orphan is closed");
        assert!(
            closed.end_snapshot_id.is_some(),
            "bracketed by an end snapshot"
        );
        let unattributed = || async {
            let files = svc
                .effort_store
                .list_unattributed_files(&effort.id)
                .await
                .unwrap();
            let runs = svc
                .attribution_store
                .list_refs(&effort.id, "run", oxplow_db::STATE_UNATTRIBUTED)
                .await
                .unwrap();
            (files, runs)
        };
        assert_eq!(
            unattributed().await,
            (Vec::new(), Vec::new()),
            "recovery reconciles nothing itself"
        );
        svc.tasks.settle_lifecycle().await;
        assert_eq!(
            unattributed().await,
            (vec!["foo.rs".to_string()], vec![format!("run:{run}")]),
            "the consumer reconciles both files and runs"
        );

        // Idempotent: nothing is left to close.
        assert_eq!(svc.recovery.run().await.unwrap().closed_efforts, 0);
    }

    #[tokio::test]
    async fn idempotent_when_nothing_to_recover() {
        let db = Database::in_memory();
        let turns = Arc::new(SqliteAgentTurnStore::new(db.clone()));
        let svc = RecoveryService::new(
            turns,
            Arc::new(SqliteTaskStore::new(db.clone())),
            Arc::new(SqliteEffortStore::new(db.clone())),
        );
        let report = svc.run().await.unwrap();
        assert_eq!(report.closed_turns, 0);
    }
}
