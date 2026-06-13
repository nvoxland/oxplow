//! Daemon recovery on startup.
//!
//! Closes any `agent_turn` rows the previous boot left open. The pane
//! that owned them is dead, so the turn can't ever `Stop` on its own
//! and the row would otherwise pin the work panel to a phantom
//! in-progress entry.
//!
//! agent_status used to live here too; it's now an in-memory registry
//! that boots empty, so nothing to reset. hook_event likewise.
//!
//! Called once from `Services::boot` after the DB is open. Idempotent.

use std::sync::Arc;

use tracing::info;

use oxplow_db::{SqliteTaskEffortStore, SqliteTaskStore, SqliteThreadStore, TaskEffortStore};
use oxplow_domain::stores::{AgentTurnStore, ThreadStore};
use oxplow_domain::DomainError;

use crate::events::{EventBus, OxplowEvent, SnapshotSourceKind};
use crate::snapshot_capture_registry::SnapshotCaptureRegistry;
use crate::task_service::reconcile_unattributed_on_close;

#[derive(Clone)]
pub struct RecoveryService {
    turns: Arc<dyn AgentTurnStore>,
    tasks: Arc<SqliteTaskStore>,
    efforts: Arc<SqliteTaskEffortStore>,
    events: EventBus,
    /// Optional wiring for reconciling unattributed changes when a
    /// restart-recovery close brackets an orphaned effort. When absent
    /// (e.g. minimal test setups), orphan efforts are still closed —
    /// they just keep the legacy `finish(None, None)` behaviour with no
    /// end snapshot and no per-path reconciliation.
    threads: Option<Arc<SqliteThreadStore>>,
    snapshot_captures: Option<SnapshotCaptureRegistry>,
}

impl RecoveryService {
    pub fn new(
        turns: Arc<dyn AgentTurnStore>,
        tasks: Arc<SqliteTaskStore>,
        efforts: Arc<SqliteTaskEffortStore>,
        events: EventBus,
    ) -> Self {
        Self {
            turns,
            tasks,
            efforts,
            events,
            threads: None,
            snapshot_captures: None,
        }
    }

    /// Attach the thread store + per-stream snapshot capture registry so
    /// restart-recovery orphan closes capture an `EffortEnd` snapshot and
    /// record their `changed_but_not_claimed` residue as unattributed
    /// (the death/restart counterpart to the in-process close-time
    /// reconciliation). Must be called after the registry is built and
    /// its streams registered (see `Services::new`).
    pub fn with_snapshot_reconcile(
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
                .close(&turn.id, Some("interrupted_by_restart".into()))
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
            if !in_progress_ids.contains(&effort.task_id) {
                // Death/restart case: the worktree still reflects the dead
                // effort's final state, so bracket the effort with an
                // EffortEnd snapshot and reconcile its unclaimed residue
                // as unattributed. Best-effort — never blocks the close.
                let end_snapshot = self.capture_orphan_end_snapshot(&effort).await;
                self.efforts.finish(&effort.id, end_snapshot, None).await?;
                if end_snapshot.is_some() {
                    self.reconcile_orphan_unattributed(&effort.id).await;
                }
                closed_efforts += 1;
            }
        }
        let mut opened_efforts = 0usize;
        for task in &in_progress {
            let Some(thread_id) = task.thread_id else {
                continue;
            };
            if self.efforts.find_open_for_task(task.id).await?.is_none() {
                self.efforts.start(task.id, &thread_id, None).await?;
                opened_efforts += 1;
            }
        }

        if closed_turns > 0 {
            self.events.emit(OxplowEvent::HookEventsChanged);
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
    /// snapshot id, or `None` when reconciliation isn't wired, the
    /// effort has no start snapshot (nothing to bracket), the stream's
    /// capture service can't be resolved, or the capture fails. Drains
    /// the worktree's current state first (`enqueue_startup_diff`) since
    /// recovery runs before the boot startup sweep — without it the
    /// dirty set is empty and `request_snapshot` would just return the
    /// existing latest snapshot, missing edits that landed after the
    /// last capture but before the crash.
    async fn capture_orphan_end_snapshot(&self, effort: &oxplow_db::TaskEffort) -> Option<i64> {
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
            .request_snapshot(SnapshotSourceKind::EffortEnd)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %e, effort = %effort.id, "recovery: end snapshot failed");
                None
            }
        }
    }

    /// Record an orphaned effort's `changed_but_not_claimed` residue as
    /// unattributed, now that recovery has stamped its end snapshot.
    async fn reconcile_orphan_unattributed(&self, effort_id: &oxplow_domain::EffortId) {
        let Some(registry) = self.snapshot_captures.as_ref() else {
            return;
        };
        // Any registered service shares the same SqliteSnapshotStore, so
        // the primary's handle is fine for the diff read.
        let Some(capture) = registry
            .primary()
            .or_else(|| registry.list().into_iter().next())
        else {
            return;
        };
        let marked =
            reconcile_unattributed_on_close(&self.efforts, capture.store(), effort_id).await;
        if !marked.is_empty() {
            tracing::debug!(
                effort = %effort_id,
                count = marked.len(),
                "recovery: recorded unattributed changes on orphan close",
            );
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
            task_id: None,
            prompt: "do".into(),
            answer: None,
            session_id: None,
            started_at: now,
            ended_at: None,
        };
        turns.open(&turn).await.unwrap();

        let svc = RecoveryService::new(
            turns.clone(),
            Arc::new(SqliteTaskStore::new(db.clone())),
            Arc::new(SqliteTaskEffortStore::new(db.clone())),
            EventBus::new(),
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
        let efforts = Arc::new(SqliteTaskEffortStore::new(db.clone()));
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
        efforts.start(stale, &t.id, None).await.unwrap();

        let svc = RecoveryService::new(
            Arc::new(SqliteAgentTurnStore::new(db.clone())),
            tasks.clone(),
            efforts.clone(),
            EventBus::new(),
        );
        let report = svc.run().await.unwrap();
        assert_eq!(report.closed_efforts, 1);
        assert_eq!(report.opened_efforts, 1);
        assert!(efforts.find_open_for_task(stale).await.unwrap().is_none());
        assert!(efforts
            .find_open_for_task(no_effort)
            .await
            .unwrap()
            .is_some());

        // Second run is a no-op — the invariant now holds.
        let again = svc.run().await.unwrap();
        assert_eq!(again.closed_efforts, 0);
        assert_eq!(again.opened_efforts, 0);
    }

    #[tokio::test]
    async fn recovery_close_of_orphan_effort_records_unattributed_changes() {
        use oxplow_domain::stores::TaskStore as _;
        use oxplow_domain::{Task, TaskActorKind, TaskAuthor, TaskId, TaskPriority, TaskStatus};
        use std::time::Duration;

        use crate::blob_store::BlobStore;
        use crate::events::SnapshotSourceKind;
        use crate::snapshot_capture::SnapshotCaptureService;
        use crate::snapshot_capture_registry::{
            SnapshotCaptureRegistry, SnapshotCaptureRegistryConfig,
        };
        use oxplow_db::SqliteSnapshotStore;
        use oxplow_fs_watch::WorkspaceFilter;

        let project = tempfile::tempdir().unwrap();
        let db = Database::in_memory();
        let now = Timestamp::from_unix_ms(1);
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: project.path().to_string_lossy().into(),
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
        let efforts = Arc::new(SqliteTaskEffortStore::new(db.clone()));
        let thread_store = Arc::new(SqliteThreadStore::new(db.clone()));
        let snapshot_store = Arc::new(SqliteSnapshotStore::new(db.clone()));

        // Per-stream capture service over the real worktree, with a zero
        // predrain so captures resolve synchronously in the test.
        let reg = SnapshotCaptureRegistry::new(SnapshotCaptureRegistryConfig {
            snapshot_store: snapshot_store.clone(),
            blobs: BlobStore::new(project.path().join(".oxplow/snapshots")),
            max_file_bytes: 1_000_000,
            workspace_filter: WorkspaceFilter::default(),
            events: EventBus::new(),
        });
        let capture = Arc::new(
            SnapshotCaptureService::new(
                snapshot_store.clone(),
                BlobStore::new(project.path().join(".oxplow/snapshots")),
                project.path().to_path_buf(),
                s.id,
                1_000_000,
                WorkspaceFilter::default(),
            )
            .with_predrain_delay(Duration::ZERO),
        );
        reg.insert_for_test(s.id, capture.clone());

        // Initial worktree state + a START snapshot to anchor the bracket.
        std::fs::write(project.path().join("foo.rs"), "v1").unwrap();
        capture.enqueue_startup_diff().await.unwrap();
        let start_id = capture
            .request_snapshot(SnapshotSourceKind::EffortStart)
            .await
            .unwrap()
            .expect("start snapshot");

        // Orphaned effort: a not-in_progress task with an open effort that
        // has a start snapshot but never closed (process died mid-effort).
        let task_row = Task {
            id: TaskId::placeholder(),
            thread_id: Some(t.id),
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
        };
        let task_id = tasks.insert(&task_row).await.unwrap();
        let effort = efforts.start(task_id, &t.id, Some(start_id)).await.unwrap();

        // An unclaimed worktree change the dead effort left behind.
        std::fs::write(project.path().join("foo.rs"), "version-two-longer").unwrap();

        let svc = RecoveryService::new(
            Arc::new(SqliteAgentTurnStore::new(db.clone())),
            tasks.clone(),
            efforts.clone(),
            EventBus::new(),
        )
        .with_snapshot_reconcile(thread_store.clone(), reg.clone());
        svc.run().await.unwrap();

        // The effort closed with an end snapshot, and the unclaimed change
        // is recorded as unattributed (not silently attributed).
        let closed = efforts.get_effort(&effort.id).await.unwrap().unwrap();
        assert!(closed.ended_at.is_some(), "orphan effort should be closed");
        assert!(
            closed.end_snapshot_id.is_some(),
            "recovery should capture an end snapshot to bracket the effort",
        );
        let unattributed = efforts.list_unattributed_files(&effort.id).await.unwrap();
        assert_eq!(
            unattributed,
            vec!["foo.rs".to_string()],
            "the unclaimed worktree change must be recorded as unattributed",
        );

        // Idempotent: re-running recovery finds no open efforts to close.
        let again = svc.run().await.unwrap();
        assert_eq!(again.closed_efforts, 0);
    }

    #[tokio::test]
    async fn idempotent_when_nothing_to_recover() {
        let db = Database::in_memory();
        let turns = Arc::new(SqliteAgentTurnStore::new(db.clone()));
        let svc = RecoveryService::new(
            turns,
            Arc::new(SqliteTaskStore::new(db.clone())),
            Arc::new(SqliteTaskEffortStore::new(db.clone())),
            EventBus::new(),
        );
        let report = svc.run().await.unwrap();
        assert_eq!(report.closed_turns, 0);
    }
}
