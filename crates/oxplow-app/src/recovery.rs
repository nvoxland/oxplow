//! Daemon recovery on startup.
//!
//! Closes any `agent_turn` rows the previous boot left open. The pane
//! that owned them is dead, so the turn can't ever `Stop` on its own
//! and the row would otherwise pin the work panel to a phantom
//! in-progress entry.
//!
//! Agent status needs no reset: it is the newest logged
//! `agent.status.changed`, and what the rail shows is derived from the
//! logged activity, so a turn this closes reads as ended. An effort left
//! open is legal across a restart (a task's status never implied one), so
//! nothing here touches efforts.
//!
//! Called once from `Services::boot` after the DB is open. Idempotent.

use std::sync::Arc;

use tracing::info;

use oxplow_domain::stores::AgentTurnStore;
use oxplow_domain::DomainError;

#[derive(Clone)]
pub struct RecoveryService {
    turns: Arc<dyn AgentTurnStore>,
}

impl RecoveryService {
    pub fn new(turns: Arc<dyn AgentTurnStore>) -> Self {
        Self { turns }
    }

    /// Close orphaned `agent_turn` rows. Returns the count so callers can
    /// log it.
    pub async fn run(&self) -> Result<RecoveryReport, DomainError> {
        let mut closed_turns = 0usize;
        for turn in self.turns.list_all_open().await? {
            self.turns
                .close(
                    &turn.id,
                    Some("interrupted_by_restart".into()),
                    oxplow_domain::hook::TurnOutcome::Restart,
                )
                .await?;
            closed_turns += 1;
        }
        info!(closed_turns, "daemon recovery complete");
        Ok(RecoveryReport { closed_turns })
    }
}

#[derive(Debug, Clone)]
pub struct RecoveryReport {
    pub closed_turns: usize,
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
            agent_session_id: None,
            prompt: "do".into(),
            answer: None,
            session_id: None,
            started_at: now,
            ended_at: None,
            start_snapshot_id: None,
            snapshot_id: None,
        };
        turns.open(&turn).await.unwrap();

        let svc = RecoveryService::new(turns.clone());
        let report = svc.run().await.unwrap();
        assert_eq!(report.closed_turns, 1);

        let still_open = turns.list_open(&t.id).await.unwrap();
        assert!(still_open.is_empty());
    }

    #[tokio::test]
    async fn idempotent_when_nothing_to_recover() {
        let db = Database::in_memory();
        let turns = Arc::new(SqliteAgentTurnStore::new(db.clone()));
        let svc = RecoveryService::new(turns);
        let report = svc.run().await.unwrap();
        assert_eq!(report.closed_turns, 0);
    }
}
