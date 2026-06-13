//! Persisted agent nudges: the informational steers oxplow surfaces to the
//! agent from the PostToolUse hook (report-less-test-run + commit-hygiene).
//! Previously fully ephemeral — see migration `V33__agent_nudge.sql` and
//! `.context/agent-model.md` (Nudge persistence).
//!
//! A thin typed read/write surface modeled on
//! [`crate::observation_store::SqliteEffortObservationStore`]. The one-shot
//! dedup (so a nudge fires at most once per effort/commit) lives in the
//! service, so the store only ever records nudges that actually fired.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, EffortId, ThreadId, Timestamp};

use crate::database::Database;

fn ts_to_string(ts: Timestamp) -> String {
    serde_json::to_string(&ts)
        .expect("Timestamp serializes to JSON")
        .trim_matches('"')
        .to_string()
}

fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DomainError::Invalid(format!("bad timestamp: {e}")))
}

/// One persisted nudge row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AgentNudge {
    pub id: i64,
    pub thread_id: String,
    /// Open effort the nudge fired against, if any (some nudge kinds fire
    /// thread-scoped with no open effort).
    pub effort_id: Option<String>,
    /// Well-known kind: `report-less-run` | `commit-hygiene` (open-ended).
    pub kind: String,
    /// The full message text that was surfaced to the agent.
    pub message: String,
    /// What caused it — the bash command (or commit sha).
    pub trigger: Option<String>,
    pub created_at: Timestamp,
}

/// Write-side input — `id` and `created_at` are assigned by the store.
#[derive(Debug, Clone)]
pub struct NewAgentNudge {
    pub thread_id: String,
    pub effort_id: Option<String>,
    pub kind: String,
    pub message: String,
    pub trigger: Option<String>,
}

fn row_to_nudge(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentNudge> {
    let created_at: String = row.get(6)?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(AgentNudge {
        id: row.get(0)?,
        thread_id: ThreadId::new(row.get::<_, i64>(1)?).to_string(),
        effort_id: row
            .get::<_, Option<i64>>(2)?
            .map(|v| EffortId::new(v).to_string()),
        kind: row.get(3)?,
        message: row.get(4)?,
        trigger: row.get(5)?,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
    })
}

const SELECT_COLS: &str = "id, thread_id, effort_id, kind, message, trigger, created_at";

#[derive(Clone)]
pub struct SqliteAgentNudgeStore {
    db: Database,
}

impl SqliteAgentNudgeStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Insert a nudge row. Returns the new row id.
    pub async fn record(&self, nudge: NewAgentNudge) -> Result<i64, DomainError> {
        self.db
            .call_mut(move |conn| {
                let sql_err = crate::database::map_sql_err;
                let thread_val = ThreadId::try_from_str(&nudge.thread_id)
                    .ok_or_else(|| {
                        DomainError::Invalid(format!("bad thread id: {}", nudge.thread_id))
                    })?
                    .value();
                let effort_val = match &nudge.effort_id {
                    Some(e) => Some(
                        EffortId::try_from_str(e)
                            .ok_or_else(|| DomainError::Invalid(format!("bad effort id: {e}")))?
                            .value(),
                    ),
                    None => None,
                };
                let now = ts_to_string(Timestamp::now());
                conn.execute(
                    "INSERT INTO agent_nudge
                       (thread_id, effort_id, kind, message, trigger, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        thread_val,
                        effort_val,
                        nudge.kind,
                        nudge.message,
                        nudge.trigger,
                        now,
                    ],
                )
                .map_err(sql_err)?;
                Ok(conn.last_insert_rowid())
            })
            .await
    }

    /// Nudges fired against an effort, newest-first.
    pub async fn list_for_effort(&self, effort_id: &str) -> Result<Vec<AgentNudge>, DomainError> {
        let effort_val = EffortId::try_from_str(effort_id).map(|e| e.value());
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {SELECT_COLS} FROM agent_nudge
                      WHERE effort_id = ?1
                      ORDER BY created_at DESC, id DESC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![effort_val], row_to_nudge)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// All nudges for a thread (effort-scoped and thread-only), newest-first.
    pub async fn list_for_thread(&self, thread_id: &str) -> Result<Vec<AgentNudge>, DomainError> {
        let thread_val = ThreadId::try_from_str(thread_id).map(|t| t.value());
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {SELECT_COLS} FROM agent_nudge
                      WHERE thread_id = ?1
                      ORDER BY created_at DESC, id DESC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![thread_val], row_to_nudge)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal stream → thread → task → effort chain so FK-on
    /// inserts of nudges succeed. Returns `(store, thread_id, effort_id)`.
    async fn fixture() -> (SqliteAgentNudgeStore, String, String) {
        let db = Database::in_memory();
        let db2 = db.clone();
        tokio::task::spawn_blocking(move || {
            db2.with_conn(|conn| {
                let now = "2026-06-13T00:00:00Z";
                conn.execute(
                    "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                     VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'main', '/r', ?1, ?1)",
                    [now],
                )?;
                conn.execute(
                    "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                     VALUES (1, 1, 't', 'active', ?1, ?1)",
                    [now],
                )?;
                conn.execute(
                    "INSERT INTO task (thread_id, title, status, priority, created_by, created_at, updated_at)
                     VALUES (1, 't', 'in_progress', 'medium', 'user', ?1, ?1)",
                    [now],
                )?;
                let task_id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO task_effort (task_id, thread_id, started_at)
                     VALUES (?1, 1, ?2)",
                    params![task_id, now],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
        .unwrap();
        (
            SqliteAgentNudgeStore::new(db),
            "thr1".to_string(),
            "eff1".to_string(),
        )
    }

    fn sample(kind: &str, effort: Option<&str>) -> NewAgentNudge {
        NewAgentNudge {
            thread_id: "thr1".into(),
            effort_id: effort.map(str::to_string),
            kind: kind.into(),
            message: format!("a {kind} nudge"),
            trigger: Some("cargo test".into()),
        }
    }

    #[tokio::test]
    async fn record_then_list_round_trips_fields() {
        let (store, _thread, effort) = fixture().await;
        let id = store
            .record(sample("report-less-run", Some("eff1")))
            .await
            .unwrap();
        let got = store.list_for_effort(&effort).await.unwrap();
        assert_eq!(got.len(), 1);
        let n = &got[0];
        assert_eq!(n.id, id);
        assert_eq!(n.kind, "report-less-run");
        assert_eq!(n.effort_id.as_deref(), Some("eff1"));
        assert_eq!(n.thread_id, "thr1");
        assert_eq!(n.message, "a report-less-run nudge");
        assert_eq!(n.trigger.as_deref(), Some("cargo test"));
    }

    #[tokio::test]
    async fn list_for_thread_includes_effort_scoped_and_thread_only() {
        let (store, thread, _effort) = fixture().await;
        store
            .record(sample("report-less-run", Some("eff1")))
            .await
            .unwrap();
        // A thread-only nudge (no open effort).
        store.record(sample("configure", None)).await.unwrap();
        let all = store.list_for_thread(&thread).await.unwrap();
        assert_eq!(all.len(), 2);
        // The effort filter only sees the effort-scoped one.
        let eff = store.list_for_effort("eff1").await.unwrap();
        assert_eq!(eff.len(), 1);
        assert_eq!(eff[0].kind, "report-less-run");
    }

    #[tokio::test]
    async fn deleting_effort_cascades_to_nudges() {
        let (store, thread, effort) = fixture().await;
        store
            .record(sample("commit-hygiene", Some("eff1")))
            .await
            .unwrap();
        // Deleting the parent effort removes its nudges (ON DELETE CASCADE).
        store
            .db
            .call(|conn| conn.execute("DELETE FROM task_effort WHERE id = 1", []))
            .await
            .unwrap();
        assert!(store.list_for_effort(&effort).await.unwrap().is_empty());
        // Thread-scoped lookup also empty (the row is gone, not orphaned).
        assert!(store.list_for_thread(&thread).await.unwrap().is_empty());
    }
}
