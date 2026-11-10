//! Daemon recovery on startup.
//!
//! Closes any `agent_turn` rows the previous boot left open (and loses the
//! jobs with no report: their processes are gone too). The pane
//! that owned them is dead, so the turn can't ever `Stop` on its own
//! and the row would otherwise pin the work panel to a phantom
//! in-progress entry. It closes turns, not `agent_session` rows: a
//! session is the slot the person opened, which outlives its process —
//! its tab comes back after a restart and resumes.
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
    db: oxplow_db::Database,
}

impl RecoveryService {
    pub fn new(turns: Arc<dyn AgentTurnStore>, db: oxplow_db::Database) -> Self {
        Self { turns, db }
    }

    /// Close orphaned `agent_turn` rows. Returns the count so callers can
    /// log it.
    pub async fn run(&self) -> Result<RecoveryReport, DomainError> {
        let mut closed_turns = 0usize;
        for turn in self.turns.list_all_open().await? {
            self.turns
                .close(&turn.id, None, oxplow_domain::hook::TurnOutcome::Restart)
                .await?;
            closed_turns += 1;
        }
        // Their jobs go with them: a job with no report now never will
        // (a request would read `waiting_on_work` until its session
        // resumed).
        let lost_jobs = self
            .db
            .transaction(|tx| {
                oxplow_db::agent_job_store::lose_all_unreported_tx(
                    tx,
                    oxplow_domain::Timestamp::now(),
                )
            })
            .await?;
        info!(closed_turns, lost_jobs, "daemon recovery complete");
        Ok(RecoveryReport {
            closed_turns,
            lost_jobs,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RecoveryReport {
    pub closed_turns: usize,
    pub lost_jobs: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteAgentTurnStore, SqliteStreamStore, SqliteThreadStore};
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{
        AgentTurn, AgentTurnId, Stream, StreamId, StreamKind, Thread, ThreadId, Timestamp,
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
            host: oxplow_domain::HostId::LOCAL,
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
            sort_index: 0,
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
            mark: None,
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
            source: Some(oxplow_domain::hook::TurnSource::Person),
        };
        turns.open(&turn).await.unwrap();

        let svc = RecoveryService::new(turns.clone(), db.clone());
        let report = svc.run().await.unwrap();
        assert_eq!(report.closed_turns, 1);

        let still_open = turns.list_open(&t.id).await.unwrap();
        assert!(still_open.is_empty());
    }

    /// A job with no report when oxplow starts is lost with its process; a
    /// reported one is left alone.
    #[tokio::test]
    async fn jobs_with_no_report_are_lost_at_start() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            tx.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/p', 'a', 'a');
                 INSERT INTO threads (id, stream_id, title, created_at, updated_at) VALUES (1, 1, 'T', 'a', 'a');
                 INSERT INTO agent_job (thread_id, harness_job_id, seen_at) VALUES (1, 'running', 'a');
                 INSERT INTO agent_job (thread_id, harness_job_id, seen_at, reported_at) VALUES (1, 'done', 'a', 'b');",
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
        let turns = Arc::new(SqliteAgentTurnStore::new(db.clone()));
        let report = RecoveryService::new(turns.clone(), db.clone())
            .run()
            .await
            .unwrap();
        assert_eq!(report.lost_jobs, 1);
        let lost: Vec<(String, bool)> = db
            .read(|c| {
                let mut stmt = c
                    .prepare(
                        "SELECT harness_job_id, lost_at IS NOT NULL FROM agent_job ORDER BY id",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(oxplow_db::map_sql_err)?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(
            lost,
            vec![("running".to_string(), true), ("done".to_string(), false)]
        );
    }

    #[tokio::test]
    async fn idempotent_when_nothing_to_recover() {
        let db = Database::in_memory();
        let turns = Arc::new(SqliteAgentTurnStore::new(db.clone()));
        let svc = RecoveryService::new(turns, db.clone());
        let report = svc.run().await.unwrap();
        assert_eq!(report.closed_turns, 0);
    }
}
