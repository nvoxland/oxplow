//! Persisted agent nudges: the informational steers oxplow surfaces to the
//! agent from the PostToolUse hook (report-less-test-run, coverage-target).
//! Previously fully ephemeral — see migration `V33__agent_nudge.sql` and
//! `.context/agent-model.md` (Nudge persistence).
//!
//! A thin typed read/write surface modeled on
//! [`crate::observation_store::SqliteEffortObservationStore`]. The one-shot
//! dedup (so a nudge fires at most once per effort) lives in the
//! service, so the store only ever records nudges that actually fired.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, EffortId, ThreadId, Timestamp};

use crate::database::Database;
use crate::database::{string_to_ts, ts_to_string};

/// One persisted nudge row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AgentNudge {
    pub id: i64,
    pub thread_id: String,
    /// Open effort the nudge fired against, if any (some nudge kinds fire
    /// thread-scoped with no open effort).
    pub effort_id: Option<String>,
    /// Well-known kind: `report-less-run` | `coverage-target` (open-ended).
    pub kind: String,
    /// The full message text that was surfaced to the agent.
    pub message: String,
    /// What caused it — the bash command (or commit sha).
    pub trigger: Option<String>,
    pub created_at: Timestamp,
    /// The turn it fired in (`agent_turn.id`), when known.
    pub turn_id: Option<i64>,
    /// When a hook response carried it to the agent; `None` until then.
    pub delivered_at: Option<Timestamp>,
}

/// Write-side input — `id` and `created_at` are assigned by the store.
#[derive(Debug, Clone, Default)]
pub struct NewAgentNudge {
    pub thread_id: String,
    pub effort_id: Option<String>,
    pub kind: String,
    pub message: String,
    pub trigger: Option<String>,
    pub turn_id: Option<i64>,
    /// The event that fired it; a second nudge of the same kind for the
    /// same cause (a redelivered event) is not written.
    pub cause: Option<String>,
}

fn row_to_nudge(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentNudge> {
    let created_at: String = row.get(6)?;
    let delivered_at: Option<String> = row.get(8)?;
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
        turn_id: row.get(7)?,
        delivered_at: delivered_at
            .as_deref()
            .map(string_to_ts)
            .transpose()
            .map_err(map_err)?,
    })
}

const SELECT_COLS: &str =
    "id, thread_id, effort_id, kind, message, trigger, created_at, turn_id, delivered_at";

#[derive(Clone)]
pub struct SqliteAgentNudgeStore {
    db: Database,
}

impl SqliteAgentNudgeStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Insert a nudge row. Returns the new row id, or `None` when a nudge
    /// of this kind was already recorded for the same cause.
    pub async fn record(&self, nudge: NewAgentNudge) -> Result<Option<i64>, DomainError> {
        self.db.transaction(move |tx| record_tx(tx, &nudge)).await
    }

    /// The thread's nudges no hook response has carried yet, oldest first,
    /// marked delivered in the same transaction — so each reaches the agent
    /// once, on whichever of the thread's hooks comes first.
    pub async fn take_undelivered(&self, thread_id: &str) -> Result<Vec<AgentNudge>, DomainError> {
        let thread_val = ThreadId::try_from_str(thread_id)
            .ok_or_else(|| DomainError::Invalid(format!("bad thread id: {thread_id}")))?
            .value();
        self.db
            .transaction(move |tx| {
                let now = ts_to_string(Timestamp::now());
                let mut stmt = tx
                    .prepare(&format!(
                        "UPDATE agent_nudge SET delivered_at = ?2
                          WHERE thread_id = ?1 AND delivered_at IS NULL
                          RETURNING {SELECT_COLS}"
                    ))
                    .map_err(crate::database::map_sql_err)?;
                let mut rows = stmt
                    .query_map(params![thread_val, now], row_to_nudge)
                    .map_err(crate::database::map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(crate::database::map_sql_err)?;
                // RETURNING's order is unspecified.
                rows.sort_by_key(|n| n.id);
                Ok(rows)
            })
            .await
    }

    /// Claim the one-shot `mark` for `effort_id` (`report-less-run`,
    /// `<extension>/<advisory>`, …). `true` the first time, `false` once
    /// it has fired — durably, across restarts.
    pub async fn claim_once(&self, effort_id: i64, mark: &str) -> Result<bool, DomainError> {
        let mark = mark.to_string();
        self.db
            .transaction(move |tx| claim_once_tx(tx, effort_id, &mark))
            .await
    }

    /// Whether `mark` has fired for `effort_id`.
    pub async fn has_fired(&self, effort_id: i64, mark: &str) -> Result<bool, DomainError> {
        let mark = mark.to_string();
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT EXISTS (SELECT 1 FROM effort_once_mark WHERE effort_id = ?1 AND mark = ?2)",
                    params![effort_id, mark],
                    |r| r.get(0),
                )
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

/// Write `nudge` in the caller's transaction; `None` when this kind was
/// already recorded for its cause.
pub fn record_tx(
    conn: &rusqlite::Connection,
    nudge: &NewAgentNudge,
) -> Result<Option<i64>, DomainError> {
    let sql_err = crate::database::map_sql_err;
    let thread_val = ThreadId::try_from_str(&nudge.thread_id)
        .ok_or_else(|| DomainError::Invalid(format!("bad thread id: {}", nudge.thread_id)))?
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
    conn.query_row(
        "INSERT INTO agent_nudge
           (thread_id, effort_id, kind, message, trigger, created_at, turn_id, cause)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (cause, kind, coalesce(effort_id, 0)) WHERE cause IS NOT NULL DO NOTHING
         RETURNING id",
        params![
            thread_val,
            effort_val,
            nudge.kind,
            nudge.message,
            nudge.trigger,
            now,
            nudge.turn_id,
            nudge.cause,
        ],
        |r| r.get(0),
    )
    .optional()
    .map_err(sql_err)
}

/// Claim `mark` for `effort_id` in the caller's transaction: `true` the
/// first time, `false` when it has already fired.
pub fn claim_once_tx(
    conn: &rusqlite::Connection,
    effort_id: i64,
    mark: &str,
) -> Result<bool, DomainError> {
    let n = conn
        .execute(
            "INSERT INTO effort_once_mark (effort_id, mark, fired_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (effort_id, mark) DO NOTHING",
            params![effort_id, mark, ts_to_string(Timestamp::now())],
        )
        .map_err(crate::database::map_sql_err)?;
    Ok(n == 1)
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
                    "INSERT INTO effort (work_item, thread_id, started_at)
                     VALUES ('work_item:oxplow:tsk' || ?1, 1, ?2)",
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
            ..Default::default()
        }
    }

    /// P3.2 (tsk472): a nudge reaches the agent exactly once, whichever hook
    /// response picks it up; a redelivered cause fires nothing new.
    #[tokio::test]
    async fn undelivered_nudges_are_taken_once_and_causes_dedupe() {
        let (store, thread, _effort) = fixture().await;
        let caused = |kind: &str| NewAgentNudge {
            cause: Some("evt-1".into()),
            ..sample(kind, Some("eff1"))
        };
        assert!(store
            .record(caused("report-less-run"))
            .await
            .unwrap()
            .is_some());
        assert!(store
            .record(caused("report-less-run"))
            .await
            .unwrap()
            .is_none());
        assert!(store
            .record(caused("coverage-target"))
            .await
            .unwrap()
            .is_some());
        // The same event and kind for another effort (here: none) is its
        // own nudge (tsk510).
        assert!(store
            .record(NewAgentNudge {
                cause: Some("evt-1".into()),
                ..sample("report-less-run", None)
            })
            .await
            .unwrap()
            .is_some());
        let taken = store.take_undelivered(&thread).await.unwrap();
        assert_eq!(
            taken.iter().map(|n| n.kind.as_str()).collect::<Vec<_>>(),
            vec!["report-less-run", "coverage-target", "report-less-run"],
            "oldest first"
        );
        assert!(taken.iter().all(|n| n.delivered_at.is_some()));
        assert!(store.take_undelivered(&thread).await.unwrap().is_empty());
    }

    /// A one-shot mark is durable: a new store over the same database (a
    /// restart) sees it, and claiming it again says so.
    #[tokio::test]
    async fn once_marks_survive_a_restart() {
        let (store, _thread, _effort) = fixture().await;
        assert!(store.claim_once(1, "report-less-run").await.unwrap());
        assert!(!store.claim_once(1, "report-less-run").await.unwrap());
        let restarted = SqliteAgentNudgeStore::new(store.db.clone());
        assert!(restarted.has_fired(1, "report-less-run").await.unwrap());
        assert!(!restarted.has_fired(1, "acme/other").await.unwrap());
    }

    #[tokio::test]
    async fn record_then_list_round_trips_fields() {
        let (store, _thread, effort) = fixture().await;
        let id = store
            .record(sample("report-less-run", Some("eff1")))
            .await
            .unwrap()
            .expect("a new nudge is written");
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
            .record(sample("coverage-target", Some("eff1")))
            .await
            .unwrap();
        // Deleting the parent effort removes its nudges (ON DELETE CASCADE).
        store
            .db
            .call(|conn| conn.execute("DELETE FROM effort WHERE id = 1", []))
            .await
            .unwrap();
        assert!(store.list_for_effort(&effort).await.unwrap().is_empty());
        // Thread-scoped lookup also empty (the row is gone, not orphaned).
        assert!(store.list_for_thread(&thread).await.unwrap().is_empty());
    }
}
