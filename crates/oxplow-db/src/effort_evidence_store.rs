//! Per-effort evidence the metric engine computes, stored so lenses can read
//! it (`v_effort_metric_delta`, `v_effort_observation`). Rows are replaced
//! wholesale per effort by core's refresher. See `.context/semantic-layer.md`.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, Timestamp};

use crate::database::map_sql_err;
use crate::{Database, EffortMetricDelta};

/// One effort-review observation: a run the effort claimed, as its evidence
/// (`effort_observation_row`, read as `v_effort_observation`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct EffortObservation {
    /// `test-run` | `diff-coverage` | `static-analysis`.
    pub kind: String,
    /// `observed` (oxplow saw it directly) | `asserted` (agent reported it).
    pub provenance: String,
    /// Free-form origin tag, e.g. `post-tool-bash` / `agent`.
    pub source: String,
    /// Headline numeric (e.g. coverage %); kind-specific, nullable.
    pub metric_value: Option<f64>,
    /// Kind-specific structured payload (parsed by the UI, opaque to Rust).
    pub payload_json: Option<String>,
    /// The snapshot the run was captured against.
    pub local_snapshot_id: Option<i64>,
    pub created_at: Timestamp,
}

fn now() -> String {
    serde_json::to_value(oxplow_domain::Timestamp::now())
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// An effort's claims, as a signature: how many, and the newest's time.
const SIG: &str = "SELECT count(*) || '|' || coalesce(max(a.recorded_at), '') \
                   FROM effort_attribution a WHERE a.effort_id = e.id";

fn attribution_sig_tx(tx: &rusqlite::Connection, effort_id: i64) -> Result<String, DomainError> {
    tx.query_row(
        &format!("SELECT ({SIG}) FROM effort e WHERE e.id = ?1"),
        [effort_id],
        |r| r.get(0),
    )
    .map_err(map_sql_err)
}

#[derive(Clone)]
pub struct SqliteEffortEvidenceStore {
    db: Database,
}

impl SqliteEffortEvidenceStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// What `effort_id`'s evidence is computed from, as a signature of its
    /// claims: how many, and the newest's time (tsk889). A claim added,
    /// moved away or changed moves it. Read it before computing the
    /// evidence, and store it with [`Self::replace`].
    pub async fn attribution_sig(&self, effort_id: i64) -> Result<String, DomainError> {
        self.db
            .read(move |tx| attribution_sig_tx(tx, effort_id))
            .await
    }

    /// Closed efforts whose claims moved since their evidence was stored.
    pub async fn stale_closed(&self) -> Result<Vec<i64>, DomainError> {
        self.db
            .read(|tx| {
                let mut st = tx
                    .prepare(&format!(
                        "SELECT e.id FROM effort e
                          LEFT JOIN effort_evidence_state s ON s.effort_id = e.id
                          WHERE e.ended_at IS NOT NULL
                            AND s.attribution_sig IS NOT ({SIG})"
                    ))
                    .map_err(map_sql_err)?;
                let ids = st
                    .query_map([], |r| r.get(0))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<i64>>>())
                    .map_err(map_sql_err)?;
                Ok(ids)
            })
            .await
    }

    /// Replace `effort_id`'s evidence — its metric deltas and observations —
    /// and record `sig`, the claims it was computed from, in one
    /// transaction.
    pub async fn replace(
        &self,
        effort_id: i64,
        deltas: Vec<EffortMetricDelta>,
        observations: Vec<EffortObservation>,
        sig: String,
    ) -> Result<(), DomainError> {
        let at = now();
        self.db
            .transaction(move |tx| {
                tx.execute("DELETE FROM effort_metric_delta WHERE effort_id = ?1", [effort_id])
                    .map_err(map_sql_err)?;
                for d in &deltas {
                    tx.execute(
                        "INSERT INTO effort_metric_delta (effort_id, key, title, unit, direction, kind, category, language, agg,
                           baseline, current, delta, changed, attributed_files, sample_count, target, warn_at, fail_at,
                           crossing, latest_capture_id, refreshed_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
                        rusqlite::params![
                            effort_id, d.key, d.title, d.unit, d.direction, d.kind, d.category, d.language, d.agg,
                            d.baseline, d.current, d.delta, i64::from(d.changed), d.attributed_files, d.sample_count,
                            d.target, d.warn_at, d.fail_at, d.crossing, d.latest_run_id, at
                        ],
                    )
                    .map_err(map_sql_err)?;
                }
                tx.execute("DELETE FROM effort_observation_row WHERE effort_id = ?1", [effort_id])
                    .map_err(map_sql_err)?;
                for (seq, o) in observations.iter().enumerate() {
                    let created = serde_json::to_value(o.created_at)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default();
                    tx.execute(
                        "INSERT INTO effort_observation_row (effort_id, seq, kind, provenance, source, metric_value,
                           payload_json, local_snapshot_id, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        rusqlite::params![
                            effort_id, seq as i64, o.kind, o.provenance, o.source, o.metric_value,
                            o.payload_json, o.local_snapshot_id, created
                        ],
                    )
                    .map_err(map_sql_err)?;
                }
                tx.execute(
                    "INSERT INTO effort_evidence_state (effort_id, attribution_sig, refreshed_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT (effort_id) DO UPDATE SET
                         attribution_sig = excluded.attribution_sig,
                         refreshed_at = excluded.refreshed_at",
                    rusqlite::params![effort_id, sig, at],
                )
                .map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }

    /// `effort_id`'s stored observations, in their stored order (newest
    /// first); `kind` filters.
    pub async fn list_observations(
        &self,
        effort_id: i64,
        kind: Option<String>,
    ) -> Result<Vec<EffortObservation>, DomainError> {
        self.db
            .read(move |c| {
                let mut stmt = c
                    .prepare(
                        "SELECT kind, provenance, source, metric_value, payload_json,
                                local_snapshot_id, created_at
                           FROM effort_observation_row
                          WHERE effort_id = ?1 AND (?2 IS NULL OR kind = ?2)
                          ORDER BY seq",
                    )
                    .map_err(map_sql_err)?;
                let rows = stmt
                    .query_map(rusqlite::params![effort_id, kind], |r| {
                        Ok((
                            EffortObservation {
                                kind: r.get(0)?,
                                provenance: r.get(1)?,
                                source: r.get(2)?,
                                metric_value: r.get(3)?,
                                payload_json: r.get(4)?,
                                local_snapshot_id: r.get(5)?,
                                created_at: Timestamp::now(),
                            },
                            r.get::<_, String>(6)?,
                        ))
                    })
                    .map_err(map_sql_err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(map_sql_err)?;
                rows.into_iter()
                    .map(|(o, created)| {
                        Ok(EffortObservation {
                            created_at: crate::database::string_to_ts(&created)?,
                            ..o
                        })
                    })
                    .collect()
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SemanticLayer;
    use serde_json::json;

    async fn seeded() -> Database {
        let db = Database::in_memory();
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/tmp/x', '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                   VALUES (1, 1, 'T', 'active', '2026-01-01', '2026-01-01');
                 INSERT INTO task (id, thread_id, title, status, priority, created_by, created_at, updated_at)
                   VALUES (1, 1, 'Task', 'in_progress', 'medium', 'agent', '2026-01-01', '2026-01-01');
                 INSERT INTO effort (id, work_item, thread_id, started_at) VALUES (1, 'work_item:oxplow:tsk1', 1, '2026-01-01');",
            )
        })
        .await
        .unwrap();
        db
    }

    fn delta(key: &str, current: f64, crossing: Option<&str>) -> EffortMetricDelta {
        EffortMetricDelta {
            key: key.into(),
            title: key.into(),
            unit: None,
            direction: "lower-better".into(),
            kind: "gauge".into(),
            category: None,
            language: None,
            agg: "files".into(),
            baseline: Some(1.0),
            current,
            delta: Some(current - 1.0),
            changed: true,
            attributed_files: Some(2),
            sample_count: 3,
            target: None,
            warn_at: Some(2.0),
            fail_at: None,
            crossing: crossing.map(str::to_string),
            latest_run_id: Some(9),
        }
    }

    #[tokio::test]
    async fn deltas_and_observations_replace_per_effort_and_read_through_views() {
        let db = seeded().await;
        let store = SqliteEffortEvidenceStore::new(db.clone());
        store
            .replace(
                1,
                vec![delta("a", 3.0, Some("warn")), delta("b", 1.0, None)],
                Vec::new(),
                String::new(),
            )
            .await
            .unwrap();
        store
            .replace(
                1,
                vec![delta("a", 4.0, Some("warn"))],
                vec![
                    observation("diff-coverage", 72.5),
                    observation("test-run", 1.0),
                ],
                String::new(),
            )
            .await
            .unwrap();
        let sl = SemanticLayer::new(db);
        let q = |sql: &'static str| {
            let sl = sl.clone();
            async move {
                serde_json::to_value(sl.query_sql(sql, vec![], None).await.unwrap().rows).unwrap()
            }
        };
        assert_eq!(
            q("SELECT key, current, delta, crossing, latest_capture_id FROM v_effort_metric_delta").await,
            json!([["a", 4.0, 3.0, "warn", 9]])
        );
        assert_eq!(
            q("SELECT kind, metric_value FROM v_effort_observation WHERE effort_id = 1 ORDER BY seq").await,
            json!([["diff-coverage", 72.5], ["test-run", 1.0]])
        );
        let read = store
            .list_observations(1, Some("diff-coverage".into()))
            .await
            .unwrap();
        assert_eq!(read, vec![observation("diff-coverage", 72.5)]);
        assert_eq!(store.list_observations(1, None).await.unwrap().len(), 2);
    }

    fn observation(kind: &str, value: f64) -> EffortObservation {
        EffortObservation {
            kind: kind.into(),
            provenance: "observed".into(),
            source: "post-tool-bash".into(),
            metric_value: Some(value),
            payload_json: Some("{}".into()),
            local_snapshot_id: None,
            created_at: Timestamp::from_unix_ms(1_790_000_000_000),
        }
    }
}
