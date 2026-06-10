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

use oxplow_db::{SqliteTaskEffortStore, SqliteTaskStore, TaskEffortStore};
use oxplow_domain::stores::AgentTurnStore;
use oxplow_domain::DomainError;

use crate::events::{EventBus, OxplowEvent};

#[derive(Clone)]
pub struct RecoveryService {
    turns: Arc<dyn AgentTurnStore>,
    tasks: Arc<SqliteTaskStore>,
    efforts: Arc<SqliteTaskEffortStore>,
    events: EventBus,
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
        }
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
                self.efforts.finish(&effort.id, None, None).await?;
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
