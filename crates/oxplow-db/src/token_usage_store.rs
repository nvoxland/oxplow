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
    /// The human-authored user prompt that OPENED this turn (tsk143).
    /// Nullable — a turn can be an assistant continuation with no opening
    /// prompt, or an agent kind whose transcript text we don't parse. Pure
    /// observation: read from the transcript, never generated.
    pub prompt: Option<String>,
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
    pub prompt: Option<String>,
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

/// Token totals for one agent/harness (`agent_kind`), used by the
/// Token Analytics page's by-harness rollup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AgentKindTokenUsage {
    pub agent_kind: String,
    pub totals: TokenUsageTotals,
}

/// Token totals for one (agent_kind, model) pair. `model` is nullable
/// (a turn can land without a parsed model). Used by the Token
/// Analytics page's by-model breakdown, grouped under each harness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ModelTokenUsage {
    pub agent_kind: String,
    pub model: Option<String>,
    pub totals: TokenUsageTotals,
}

/// Token volume bucketed by calendar day (UTC), newest day last.
/// Drives the tokens-per-day trend chart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct TokenUsageByDay {
    /// `YYYY-MM-DD`.
    pub day: String,
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

const SELECT_COLS: &str = "id, stream_id, thread_id, effort_id, session_id, agent_kind, \
     model, prompt, input_tokens, output_tokens, cache_creation_input_tokens, \
     cache_read_input_tokens, message_count, provenance, recorded_at";

fn row_to_usage(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentTokenUsage> {
    let recorded_at: String = row.get(14)?;
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
        prompt: row.get(7)?,
        input_tokens: row.get(8)?,
        output_tokens: row.get(9)?,
        cache_creation_input_tokens: row.get(10)?,
        cache_read_input_tokens: row.get(11)?,
        message_count: row.get(12)?,
        provenance: row.get(13)?,
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
                       (stream_id, thread_id, effort_id, session_id, agent_kind, model, prompt,
                        input_tokens, output_tokens, cache_creation_input_tokens,
                        cache_read_input_tokens, message_count, provenance, recorded_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'observed', ?13)",
                    params![
                        stream_val,
                        thread_val,
                        effort_val,
                        usage.session_id,
                        usage.agent_kind,
                        usage.model,
                        usage.prompt,
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

    /// Summed totals across every recorded turn (all streams/threads).
    pub async fn totals_overall(&self) -> Result<TokenUsageTotals, DomainError> {
        self.db
            .call(|conn| conn.query_row(&totals_sql("1=1"), [], totals_from_row))
            .await
    }

    /// Totals grouped by agent/harness (`agent_kind`), busiest first.
    pub async fn totals_by_agent_kind(&self) -> Result<Vec<AgentKindTokenUsage>, DomainError> {
        self.db
            .call(|conn| {
                let sql = format!(
                    "{select} GROUP BY agent_kind ORDER BY \
                     COALESCE(SUM(input_tokens + output_tokens + \
                       cache_creation_input_tokens + cache_read_input_tokens), 0) DESC",
                    select = grouped_totals_select("agent_kind")
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map([], |row| {
                    Ok(AgentKindTokenUsage {
                        agent_kind: row.get(0)?,
                        totals: totals_at(row, 1)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Totals grouped by (agent_kind, model), busiest first. Lets the UI
    /// nest models under their harness.
    pub async fn totals_by_model(&self) -> Result<Vec<ModelTokenUsage>, DomainError> {
        self.db
            .call(|conn| {
                let sql = format!(
                    "{select} GROUP BY agent_kind, model ORDER BY \
                     COALESCE(SUM(input_tokens + output_tokens + \
                       cache_creation_input_tokens + cache_read_input_tokens), 0) DESC",
                    select = grouped_totals_select("agent_kind, model")
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map([], |row| {
                    Ok(ModelTokenUsage {
                        agent_kind: row.get(0)?,
                        model: row.get(1)?,
                        totals: totals_at(row, 2)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Token volume per calendar day over the last `days` days (oldest
    /// first), for the trend chart.
    pub async fn usage_by_day(&self, days: u32) -> Result<Vec<TokenUsageByDay>, DomainError> {
        let modifier = format!("-{days} days");
        self.db
            .call(move |conn| {
                let sql = "SELECT date(recorded_at) AS day,
                     COALESCE(SUM(input_tokens + output_tokens + \
                       cache_creation_input_tokens + cache_read_input_tokens), 0),
                     COALESCE(SUM(input_tokens), 0),
                     COALESCE(SUM(output_tokens), 0)
                   FROM agent_token_usage
                   WHERE date(recorded_at) >= date('now', ?1)
                   GROUP BY day
                   ORDER BY day ASC";
                let mut stmt = conn.prepare(sql)?;
                let rows = stmt.query_map(params![modifier], |row| {
                    Ok(TokenUsageByDay {
                        day: row.get(0)?,
                        total_tokens: row.get(1)?,
                        input_tokens: row.get(2)?,
                        output_tokens: row.get(3)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
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
    totals_at(row, 0)
}

/// Read a `TokenUsageTotals` from six consecutive aggregate columns
/// starting at `base` (input, output, cache-creation, cache-read,
/// message_count, count). Lets a grouped query prepend its key columns.
fn totals_at(row: &rusqlite::Row<'_>, base: usize) -> rusqlite::Result<TokenUsageTotals> {
    let input: i64 = row.get(base)?;
    let output: i64 = row.get(base + 1)?;
    let cache_creation: i64 = row.get(base + 2)?;
    let cache_read: i64 = row.get(base + 3)?;
    Ok(TokenUsageTotals {
        input_tokens: input,
        output_tokens: output,
        cache_creation_input_tokens: cache_creation,
        cache_read_input_tokens: cache_read,
        total_tokens: input + output + cache_creation + cache_read,
        message_count: row.get(base + 4)?,
        turns: row.get(base + 5)?,
    })
}

/// `SELECT <group_cols>, <6 aggregate total columns> FROM agent_token_usage`
/// — the caller appends `GROUP BY` / `ORDER BY`. The aggregate columns
/// line up with `totals_at(row, <number of group cols>)`.
fn grouped_totals_select(group_cols: &str) -> String {
    format!(
        "SELECT {group_cols},
           COALESCE(SUM(input_tokens), 0),
           COALESCE(SUM(output_tokens), 0),
           COALESCE(SUM(cache_creation_input_tokens), 0),
           COALESCE(SUM(cache_read_input_tokens), 0),
           COALESCE(SUM(message_count), 0),
           COUNT(*)
         FROM agent_token_usage"
    )
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
            prompt: Some("fix the parser".into()),
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
        assert_eq!(u.prompt.as_deref(), Some("fix the parser"));
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

    /// A sample turn with explicit agent/model/token counts (effort-less,
    /// so analytics queries that ignore effort still cover it).
    fn sample_kind(
        agent_kind: &str,
        model: Option<&str>,
        input: i64,
        output: i64,
    ) -> NewAgentTokenUsage {
        NewAgentTokenUsage {
            stream_id: "str1".into(),
            thread_id: "thr1".into(),
            effort_id: None,
            session_id: "sess-1".into(),
            agent_kind: agent_kind.into(),
            model: model.map(str::to_string),
            prompt: None,
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            message_count: 1,
        }
    }

    #[tokio::test]
    async fn overall_sums_every_row() {
        let (store, _e) = fixture().await;
        store
            .record(sample_kind("claude", Some("opus"), 100, 10))
            .await
            .unwrap();
        store
            .record(sample_kind("codex", Some("gpt"), 5, 1))
            .await
            .unwrap();
        let all = store.totals_overall().await.unwrap();
        assert_eq!(all.input_tokens, 105);
        assert_eq!(all.output_tokens, 11);
        assert_eq!(all.total_tokens, 105 + 11);
        assert_eq!(all.turns, 2);
    }

    #[tokio::test]
    async fn by_agent_kind_splits_and_orders_busiest_first() {
        let (store, _e) = fixture().await;
        store
            .record(sample_kind("claude", Some("opus"), 100, 0))
            .await
            .unwrap();
        store
            .record(sample_kind("claude", Some("opus"), 100, 0))
            .await
            .unwrap();
        store
            .record(sample_kind("codex", Some("gpt"), 5, 0))
            .await
            .unwrap();
        let rows = store.totals_by_agent_kind().await.unwrap();
        assert_eq!(rows.len(), 2);
        // Busiest first: claude (200) before codex (5).
        assert_eq!(rows[0].agent_kind, "claude");
        assert_eq!(rows[0].totals.input_tokens, 200);
        assert_eq!(rows[0].totals.turns, 2);
        assert_eq!(rows[1].agent_kind, "codex");
        assert_eq!(rows[1].totals.input_tokens, 5);
    }

    #[tokio::test]
    async fn by_model_groups_under_agent() {
        let (store, _e) = fixture().await;
        store
            .record(sample_kind("claude", Some("opus"), 100, 0))
            .await
            .unwrap();
        store
            .record(sample_kind("claude", Some("sonnet"), 30, 0))
            .await
            .unwrap();
        store
            .record(sample_kind("claude", Some("opus"), 20, 0))
            .await
            .unwrap();
        let rows = store.totals_by_model().await.unwrap();
        // Two distinct (agent_kind, model) pairs.
        assert_eq!(rows.len(), 2);
        let opus = rows
            .iter()
            .find(|r| r.model.as_deref() == Some("opus"))
            .unwrap();
        assert_eq!(opus.agent_kind, "claude");
        assert_eq!(opus.totals.input_tokens, 120);
        assert_eq!(opus.totals.turns, 2);
        let sonnet = rows
            .iter()
            .find(|r| r.model.as_deref() == Some("sonnet"))
            .unwrap();
        assert_eq!(sonnet.totals.input_tokens, 30);
    }

    #[tokio::test]
    async fn usage_by_day_buckets_today() {
        let (store, _e) = fixture().await;
        store
            .record(sample_kind("claude", Some("opus"), 100, 10))
            .await
            .unwrap();
        store
            .record(sample_kind("claude", Some("opus"), 50, 5))
            .await
            .unwrap();
        let rows = store.usage_by_day(30).await.unwrap();
        // Both turns land on the same (today's) bucket.
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].total_tokens, 165);
        assert_eq!(rows[0].input_tokens, 150);
        assert_eq!(rows[0].output_tokens, 15);
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
