//! Effort-scoped collection store: structured, agent-or-tool-reported
//! observations attached to a `task_effort` (test runs, diff coverage).
//!
//! Modeled on `code_quality_finding` (kind + metric + payload) plus the
//! `page_ref` freshness-pin columns. See migration
//! `V26__effort_observation.sql` for the schema and the provenance /
//! freshness rationale, and `.context/collection.md` for the subsystem.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, Timestamp};

use crate::database::Database;

/// How many observations to keep per `(effort_id, kind)`. Older ones are
/// pruned in the same transaction as each insert, mirroring
/// `SqliteCodeQualityStore`'s scan retention.
const KEEP_LAST: i64 = 10;

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

/// One persisted observation row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct EffortObservation {
    pub id: i64,
    pub stream_id: String,
    pub effort_id: String,
    /// Well-known kind: `test-run` | `diff-coverage` (open-ended).
    pub kind: String,
    /// `observed` (oxplow saw it directly) | `asserted` (agent reported it).
    pub provenance: String,
    /// Free-form origin tag, e.g. `post-tool-bash` / `agent`.
    pub source: String,
    /// Headline numeric (e.g. coverage %); kind-specific, nullable.
    pub metric_value: Option<f64>,
    /// Kind-specific structured payload (parsed by the UI, opaque to Rust).
    pub payload_json: Option<String>,
    /// Freshness pin — the snapshot this was captured against.
    pub local_snapshot_id: Option<i64>,
    pub closest_git_version: Option<String>,
    pub git_version_exact: bool,
    pub created_at: Timestamp,
}

/// Write-side input — `id` and `created_at` are assigned by the store.
#[derive(Debug, Clone)]
pub struct NewEffortObservation {
    pub stream_id: String,
    pub effort_id: String,
    pub kind: String,
    pub provenance: String,
    pub source: String,
    pub metric_value: Option<f64>,
    pub payload_json: Option<String>,
    pub local_snapshot_id: Option<i64>,
    pub closest_git_version: Option<String>,
    pub git_version_exact: bool,
}

fn row_to_observation(row: &rusqlite::Row<'_>) -> rusqlite::Result<EffortObservation> {
    let created_at: String = row.get(10)?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(EffortObservation {
        id: row.get(0)?,
        stream_id: row.get(1)?,
        effort_id: row.get(2)?,
        kind: row.get(3)?,
        provenance: row.get(4)?,
        source: row.get(5)?,
        metric_value: row.get(6)?,
        payload_json: row.get(7)?,
        local_snapshot_id: row.get(8)?,
        closest_git_version: row.get(9)?,
        git_version_exact: row.get::<_, i64>(11)? != 0,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
    })
}

const SELECT_COLS: &str = "id, stream_id, effort_id, kind, provenance, source, \
     metric_value, payload_json, local_snapshot_id, closest_git_version, \
     created_at, git_version_exact";

#[derive(Clone)]
pub struct SqliteEffortObservationStore {
    db: Database,
}

impl SqliteEffortObservationStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Insert an observation and prune older rows beyond [`KEEP_LAST`] for
    /// the same `(effort_id, kind)`, atomically. Returns the new row id.
    pub async fn record(&self, obs: NewEffortObservation) -> Result<i64, DomainError> {
        self.db
            .call_mut(move |conn| {
                let sql_err = |e: rusqlite::Error| DomainError::Invalid(format!("sql: {e}"));
                let tx = conn.transaction().map_err(sql_err)?;
                let now = ts_to_string(Timestamp::now());
                tx.execute(
                    "INSERT INTO effort_observation
                       (stream_id, effort_id, kind, provenance, source, metric_value,
                        payload_json, local_snapshot_id, closest_git_version,
                        git_version_exact, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        obs.stream_id,
                        obs.effort_id,
                        obs.kind,
                        obs.provenance,
                        obs.source,
                        obs.metric_value,
                        obs.payload_json,
                        obs.local_snapshot_id,
                        obs.closest_git_version,
                        obs.git_version_exact,
                        now,
                    ],
                )
                .map_err(sql_err)?;
                let id = tx.last_insert_rowid();
                // The just-inserted row is always within the newest N, so
                // it survives the prune.
                tx.execute(
                    "DELETE FROM effort_observation
                      WHERE effort_id = ?1 AND kind = ?2
                        AND id NOT IN (
                          SELECT id FROM effort_observation
                           WHERE effort_id = ?1 AND kind = ?2
                           ORDER BY created_at DESC, id DESC
                           LIMIT ?3)",
                    params![obs.effort_id, obs.kind, KEEP_LAST],
                )
                .map_err(sql_err)?;
                tx.commit().map_err(sql_err)?;
                Ok(id)
            })
            .await
    }

    /// Observations for an effort, newest-first. Pass `kind` to filter to
    /// one well-known kind.
    pub async fn list_for_effort(
        &self,
        effort_id: &str,
        kind: Option<&str>,
    ) -> Result<Vec<EffortObservation>, DomainError> {
        let effort_id = effort_id.to_string();
        let kind = kind.map(str::to_string);
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {SELECT_COLS} FROM effort_observation
                      WHERE effort_id = ?1 AND (?2 IS NULL OR kind = ?2)
                      ORDER BY created_at DESC, id DESC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![effort_id, kind], row_to_observation)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// The most recent observation of a given kind for an effort, if any.
    pub async fn latest_for_effort(
        &self,
        effort_id: &str,
        kind: &str,
    ) -> Result<Option<EffortObservation>, DomainError> {
        let effort_id = effort_id.to_string();
        let kind = kind.to_string();
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {SELECT_COLS} FROM effort_observation
                      WHERE effort_id = ?1 AND kind = ?2
                      ORDER BY created_at DESC, id DESC LIMIT 1"
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut rows = stmt.query_map(params![effort_id, kind], row_to_observation)?;
                match rows.next() {
                    Some(r) => Ok(Some(r?)),
                    None => Ok(None),
                }
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal stream → thread → task → effort chain so FK-on
    /// inserts of observations succeed. Returns `(store, effort_id)`.
    async fn fixture() -> (SqliteEffortObservationStore, String) {
        let db = Database::in_memory();
        let db2 = db.clone();
        tokio::task::spawn_blocking(move || {
            db2.with_conn(|conn| {
                let now = "2026-05-26T00:00:00Z";
                conn.execute(
                    "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                     VALUES ('s-1', 'primary', 'p', 'main', 'refs/heads/main', 'main', '/r', ?1, ?1)",
                    [now],
                )?;
                conn.execute(
                    "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                     VALUES ('b-1', 's-1', 't', 'active', ?1, ?1)",
                    [now],
                )?;
                conn.execute(
                    "INSERT INTO task (thread_id, title, status, priority, created_by, created_at, updated_at)
                     VALUES ('b-1', 't', 'in_progress', 'medium', 'user', ?1, ?1)",
                    [now],
                )?;
                let task_id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO task_effort (id, task_id, thread_id, started_at)
                     VALUES ('ef-1', ?1, 'b-1', ?2)",
                    params![task_id, now],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
        .unwrap();
        (SqliteEffortObservationStore::new(db), "ef-1".to_string())
    }

    fn sample(kind: &str, source: &str, metric: Option<f64>) -> NewEffortObservation {
        NewEffortObservation {
            stream_id: "s-1".into(),
            effort_id: "ef-1".into(),
            kind: kind.into(),
            provenance: "observed".into(),
            source: source.into(),
            metric_value: metric,
            payload_json: Some("{}".into()),
            local_snapshot_id: None,
            closest_git_version: None,
            git_version_exact: false,
        }
    }

    #[tokio::test]
    async fn record_then_list_round_trips_fields() {
        let (store, effort) = fixture().await;
        let id = store
            .record(NewEffortObservation {
                metric_value: Some(62.5),
                local_snapshot_id: Some(7),
                closest_git_version: Some("abc1234".into()),
                git_version_exact: true,
                ..sample("diff-coverage", "agent", Some(62.5))
            })
            .await
            .unwrap();
        let got = store.list_for_effort(&effort, None).await.unwrap();
        assert_eq!(got.len(), 1);
        let o = &got[0];
        assert_eq!(o.id, id);
        assert_eq!(o.kind, "diff-coverage");
        assert_eq!(o.metric_value, Some(62.5));
        assert_eq!(o.local_snapshot_id, Some(7));
        assert_eq!(o.closest_git_version.as_deref(), Some("abc1234"));
        assert!(o.git_version_exact);
    }

    #[tokio::test]
    async fn list_filters_by_kind() {
        let (store, effort) = fixture().await;
        store
            .record(sample("test-run", "post-tool-bash", None))
            .await
            .unwrap();
        store
            .record(sample("diff-coverage", "agent", Some(80.0)))
            .await
            .unwrap();
        let runs = store
            .list_for_effort(&effort, Some("test-run"))
            .await
            .unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].kind, "test-run");
        let all = store.list_for_effort(&effort, None).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn latest_for_effort_returns_newest_of_kind() {
        let (store, effort) = fixture().await;
        for pct in [10.0, 20.0, 30.0] {
            store
                .record(sample("diff-coverage", "agent", Some(pct)))
                .await
                .unwrap();
        }
        let latest = store
            .latest_for_effort(&effort, "diff-coverage")
            .await
            .unwrap()
            .expect("a diff-coverage observation exists");
        assert_eq!(latest.metric_value, Some(30.0));
        assert!(store
            .latest_for_effort(&effort, "test-run")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn record_prunes_beyond_keep_last_per_kind() {
        let (store, effort) = fixture().await;
        for i in 0..(KEEP_LAST + 5) {
            store
                .record(sample("test-run", "post-tool-bash", Some(i as f64)))
                .await
                .unwrap();
        }
        // Other kinds are unaffected by the test-run prune.
        store
            .record(sample("diff-coverage", "agent", Some(99.0)))
            .await
            .unwrap();

        let runs = store
            .list_for_effort(&effort, Some("test-run"))
            .await
            .unwrap();
        assert_eq!(
            runs.len() as i64,
            KEEP_LAST,
            "kept only the newest N test-runs"
        );
        // Newest survives, oldest pruned.
        assert_eq!(
            runs.first().unwrap().metric_value,
            Some((KEEP_LAST + 4) as f64)
        );
        assert_eq!(runs.last().unwrap().metric_value, Some(5.0));
        assert_eq!(
            store
                .list_for_effort(&effort, Some("diff-coverage"))
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn deleting_effort_cascades_to_observations() {
        let (store, effort) = fixture().await;
        store
            .record(sample("test-run", "post-tool-bash", None))
            .await
            .unwrap();
        // Deleting the parent effort removes its observations (ON DELETE CASCADE).
        store
            .db
            .call(|conn| conn.execute("DELETE FROM task_effort WHERE id = 'ef-1'", []))
            .await
            .unwrap();
        assert!(store
            .list_for_effort(&effort, None)
            .await
            .unwrap()
            .is_empty());
    }
}
