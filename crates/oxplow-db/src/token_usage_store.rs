//! Agent token-usage store (tsk104): per-turn token accounting parsed from
//! the agent session transcript, attributed to a `task_effort` + thread,
//! plus a per-session read cursor so successive Stops only sum the new tail
//! of the transcript.
//!
//! Modeled on `observation_store` (typed-id columns stored as raw INTEGER,
//! `db.call`/`db.call_mut`, `map_sql_err`). See migration
//! `V35__agent_token_usage.sql` and `.context/data-model.md`.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, EffortId, StreamId, ThreadId, Timestamp};

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

/// One persisted token-usage row (one agent turn's delta).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AgentTokenUsage {
    pub id: i64,
    pub stream_id: String,
    pub thread_id: String,
    /// Nullable — a Stop can land with no open effort.
    pub effort_id: Option<String>,
    pub session_id: String,
    /// `claude` | `codex` | `opencode`.
    pub agent_kind: String,
    /// Actual model the turn ran on (e.g. `claude-opus-4-8`), for later cost.
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
    /// How many assistant messages contributed to this row.
    pub message_count: i64,
    /// Always `observed` — oxplow read the transcript directly.
    pub provenance: String,
    pub recorded_at: Timestamp,
}

/// Write-side input — `id` and `recorded_at` are assigned by the store.
#[derive(Debug, Clone)]
pub struct NewAgentTokenUsage {
    pub stream_id: String,
    pub thread_id: String,
    pub effort_id: Option<String>,
    pub session_id: String,
    pub agent_kind: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub message_count: i64,
}

/// Aggregated totals across a set of usage rows (per effort or per thread).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct TokenUsageTotals {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
    /// input + output + cache-creation + cache-read.
    pub total_tokens: i64,
    pub message_count: i64,
    /// Number of usage rows (turns) summed.
    pub turns: i64,
}

const SELECT_COLS: &str = "id, stream_id, thread_id, effort_id, session_id, agent_kind, \
     model, input_tokens, output_tokens, cache_creation_input_tokens, \
     cache_read_input_tokens, message_count, provenance, recorded_at";

fn row_to_usage(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentTokenUsage> {
    let recorded_at: String = row.get(13)?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(AgentTokenUsage {
        id: row.get(0)?,
        stream_id: StreamId::new(row.get::<_, i64>(1)?).to_string(),
        thread_id: ThreadId::new(row.get::<_, i64>(2)?).to_string(),
        effort_id: row
            .get::<_, Option<i64>>(3)?
            .map(|v| EffortId::new(v).to_string()),
        session_id: row.get(4)?,
        agent_kind: row.get(5)?,
        model: row.get(6)?,
        input_tokens: row.get(7)?,
        output_tokens: row.get(8)?,
        cache_creation_input_tokens: row.get(9)?,
        cache_read_input_tokens: row.get(10)?,
        message_count: row.get(11)?,
        provenance: row.get(12)?,
        recorded_at: string_to_ts(&recorded_at).map_err(map_err)?,
    })
}

#[derive(Clone)]
pub struct SqliteTokenUsageStore {
    db: Database,
}

impl SqliteTokenUsageStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Insert one usage row. Returns the new row id.
    pub async fn record(&self, usage: NewAgentTokenUsage) -> Result<i64, DomainError> {
        self.db
            .call_mut(move |conn| {
                let sql_err = crate::database::map_sql_err;
                let stream_val = StreamId::try_from_str(&usage.stream_id)
                    .ok_or_else(|| {
                        DomainError::Invalid(format!("bad stream id: {}", usage.stream_id))
                    })?
                    .value();
                let thread_val = ThreadId::try_from_str(&usage.thread_id)
                    .ok_or_else(|| {
                        DomainError::Invalid(format!("bad thread id: {}", usage.thread_id))
                    })?
                    .value();
                let effort_val = match usage.effort_id.as_deref() {
                    Some(e) => Some(
                        EffortId::try_from_str(e)
                            .ok_or_else(|| DomainError::Invalid(format!("bad effort id: {e}")))?
                            .value(),
                    ),
                    None => None,
                };
                let now = ts_to_string(Timestamp::now());
                conn.execute(
                    "INSERT INTO agent_token_usage
                       (stream_id, thread_id, effort_id, session_id, agent_kind, model,
                        input_tokens, output_tokens, cache_creation_input_tokens,
                        cache_read_input_tokens, message_count, provenance, recorded_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'observed', ?12)",
                    params![
                        stream_val,
                        thread_val,
                        effort_val,
                        usage.session_id,
                        usage.agent_kind,
                        usage.model,
                        usage.input_tokens,
                        usage.output_tokens,
                        usage.cache_creation_input_tokens,
                        usage.cache_read_input_tokens,
                        usage.message_count,
                        now,
                    ],
                )
                .map_err(sql_err)?;
                Ok(conn.last_insert_rowid())
            })
            .await
    }

    /// Usage rows for an effort, newest-first.
    pub async fn list_for_effort(
        &self,
        effort_id: &str,
    ) -> Result<Vec<AgentTokenUsage>, DomainError> {
        let effort_val = EffortId::try_from_str(effort_id)
            .ok_or_else(|| DomainError::Invalid(format!("bad effort id: {effort_id}")))?
            .value();
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {SELECT_COLS} FROM agent_token_usage
                      WHERE effort_id = ?1
                      ORDER BY recorded_at DESC, id DESC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![effort_val], row_to_usage)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Summed totals for one effort.
    pub async fn totals_for_effort(
        &self,
        effort_id: &str,
    ) -> Result<TokenUsageTotals, DomainError> {
        let effort_val = EffortId::try_from_str(effort_id)
            .ok_or_else(|| DomainError::Invalid(format!("bad effort id: {effort_id}")))?
            .value();
        self.db
            .call(move |conn| {
                conn.query_row(
                    &totals_sql("effort_id = ?1"),
                    params![effort_val],
                    totals_from_row,
                )
            })
            .await
    }

    /// Summed totals for one thread (all efforts + effort-less turns).
    pub async fn totals_for_thread(
        &self,
        thread_id: &str,
    ) -> Result<TokenUsageTotals, DomainError> {
        let thread_val = ThreadId::try_from_str(thread_id)
            .ok_or_else(|| DomainError::Invalid(format!("bad thread id: {thread_id}")))?
            .value();
        self.db
            .call(move |conn| {
                conn.query_row(
                    &totals_sql("thread_id = ?1"),
                    params![thread_val],
                    totals_from_row,
                )
            })
            .await
    }

    /// Current read offset for a session, or `None` if never recorded.
    pub async fn cursor(&self, session_id: &str) -> Result<Option<u64>, DomainError> {
        let session_id = session_id.to_string();
        self.db
            .call(move |conn| {
                let mut stmt = conn
                    .prepare("SELECT byte_offset FROM agent_token_cursor WHERE session_id = ?1")?;
                let mut rows = stmt.query_map(params![session_id], |r| r.get::<_, i64>(0))?;
                match rows.next() {
                    Some(v) => Ok(Some(v? as u64)),
                    None => Ok(None),
                }
            })
            .await
    }

    /// Set (upsert) the read offset for a session.
    pub async fn set_cursor(&self, session_id: &str, offset: u64) -> Result<(), DomainError> {
        let session_id = session_id.to_string();
        self.db
            .call_mut(move |conn| {
                let sql_err = crate::database::map_sql_err;
                let now = ts_to_string(Timestamp::now());
                conn.execute(
                    "INSERT INTO agent_token_cursor (session_id, byte_offset, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(session_id) DO UPDATE SET
                       byte_offset = excluded.byte_offset,
                       updated_at = excluded.updated_at",
                    params![session_id, offset as i64, now],
                )
                .map_err(sql_err)?;
                Ok(())
            })
            .await
    }
}

fn totals_sql(predicate: &str) -> String {
    format!(
        "SELECT
           COALESCE(SUM(input_tokens), 0),
           COALESCE(SUM(output_tokens), 0),
           COALESCE(SUM(cache_creation_input_tokens), 0),
           COALESCE(SUM(cache_read_input_tokens), 0),
           COALESCE(SUM(message_count), 0),
           COUNT(*)
         FROM agent_token_usage
         WHERE {predicate}"
    )
}

fn totals_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TokenUsageTotals> {
    let input: i64 = row.get(0)?;
    let output: i64 = row.get(1)?;
    let cache_creation: i64 = row.get(2)?;
    let cache_read: i64 = row.get(3)?;
    Ok(TokenUsageTotals {
        input_tokens: input,
        output_tokens: output,
        cache_creation_input_tokens: cache_creation,
        cache_read_input_tokens: cache_read,
        total_tokens: input + output + cache_creation + cache_read,
        message_count: row.get(4)?,
        turns: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal stream → thread → task → effort chain so FK-on
    /// inserts succeed. Returns `(store, effort_id)`.
    async fn fixture() -> (SqliteTokenUsageStore, String) {
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
        (SqliteTokenUsageStore::new(db), "eff1".to_string())
    }

    fn sample(effort_id: Option<&str>) -> NewAgentTokenUsage {
        NewAgentTokenUsage {
            stream_id: "str1".into(),
            thread_id: "thr1".into(),
            effort_id: effort_id.map(str::to_string),
            session_id: "sess-1".into(),
            agent_kind: "claude".into(),
            model: Some("claude-opus-4-8".into()),
            input_tokens: 100,
            output_tokens: 20,
            cache_creation_input_tokens: 50,
            cache_read_input_tokens: 200,
            message_count: 2,
        }
    }

    #[tokio::test]
    async fn record_then_list_round_trips_fields() {
        let (store, effort) = fixture().await;
        let id = store.record(sample(Some("eff1"))).await.unwrap();
        let got = store.list_for_effort(&effort).await.unwrap();
        assert_eq!(got.len(), 1);
        let u = &got[0];
        assert_eq!(u.id, id);
        assert_eq!(u.effort_id.as_deref(), Some("eff1"));
        assert_eq!(u.thread_id, "thr1");
        assert_eq!(u.agent_kind, "claude");
        assert_eq!(u.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.output_tokens, 20);
        assert_eq!(u.cache_creation_input_tokens, 50);
        assert_eq!(u.cache_read_input_tokens, 200);
        assert_eq!(u.message_count, 2);
        assert_eq!(u.provenance, "observed");
    }

    #[tokio::test]
    async fn totals_sum_across_rows() {
        let (store, effort) = fixture().await;
        store.record(sample(Some("eff1"))).await.unwrap();
        store.record(sample(Some("eff1"))).await.unwrap();

        let eff = store.totals_for_effort(&effort).await.unwrap();
        assert_eq!(eff.input_tokens, 200);
        assert_eq!(eff.output_tokens, 40);
        assert_eq!(eff.cache_creation_input_tokens, 100);
        assert_eq!(eff.cache_read_input_tokens, 400);
        assert_eq!(eff.total_tokens, 200 + 40 + 100 + 400);
        assert_eq!(eff.message_count, 4);
        assert_eq!(eff.turns, 2);

        let thread = store.totals_for_thread("thr1").await.unwrap();
        assert_eq!(thread.total_tokens, eff.total_tokens);
        assert_eq!(thread.turns, 2);
    }

    #[tokio::test]
    async fn thread_totals_include_effortless_turns() {
        let (store, _effort) = fixture().await;
        store.record(sample(Some("eff1"))).await.unwrap();
        store.record(sample(None)).await.unwrap(); // Stop with no open effort
        let thread = store.totals_for_thread("thr1").await.unwrap();
        assert_eq!(thread.turns, 2);
        assert_eq!(thread.input_tokens, 200);
        // Effort totals only count the attributed row.
        let eff = store.totals_for_effort("eff1").await.unwrap();
        assert_eq!(eff.turns, 1);
    }

    #[tokio::test]
    async fn empty_totals_are_zero() {
        let (store, effort) = fixture().await;
        let eff = store.totals_for_effort(&effort).await.unwrap();
        assert_eq!(eff, TokenUsageTotals::default());
    }

    #[tokio::test]
    async fn cursor_round_trips_and_upserts() {
        let (store, _effort) = fixture().await;
        assert_eq!(store.cursor("sess-1").await.unwrap(), None);
        store.set_cursor("sess-1", 1234).await.unwrap();
        assert_eq!(store.cursor("sess-1").await.unwrap(), Some(1234));
        store.set_cursor("sess-1", 5678).await.unwrap();
        assert_eq!(store.cursor("sess-1").await.unwrap(), Some(5678));
    }

    #[tokio::test]
    async fn deleting_effort_cascades_but_keeps_thread_total() {
        let (store, effort) = fixture().await;
        store.record(sample(Some("eff1"))).await.unwrap();
        store
            .db
            .call(|conn| conn.execute("DELETE FROM task_effort WHERE id = 1", []))
            .await
            .unwrap();
        // The row referenced the effort, so it cascades away entirely.
        assert!(store.list_for_effort(&effort).await.unwrap().is_empty());
    }
}
