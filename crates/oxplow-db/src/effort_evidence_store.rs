//! Per-effort evidence the metric engine computes, stored so lenses can read
//! it (`v_effort_metric_delta`, `v_effort_observation`). Rows are replaced
//! wholesale per effort by core's refresher. See `.context/semantic-layer.md`.

use oxplow_domain::DomainError;

use crate::database::map_sql_err;
use crate::{Database, EffortMetricDelta, EffortObservation};

fn now() -> String {
    serde_json::to_value(oxplow_domain::Timestamp::now())
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

#[derive(Clone)]
pub struct SqliteEffortEvidenceStore {
    db: Database,
}

impl SqliteEffortEvidenceStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Replace `effort_id`'s metric deltas.
    pub async fn replace_metric_deltas(
        &self,
        effort_id: i64,
        deltas: Vec<EffortMetricDelta>,
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
                Ok(())
            })
            .await
    }

    /// Replace `effort_id`'s observations.
    pub async fn replace_observations(
        &self,
        effort_id: i64,
        observations: Vec<EffortObservation>,
    ) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| {
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
                Ok(())
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
                 INSERT INTO task_effort (id, task_id, thread_id, started_at) VALUES (1, 1, 1, '2026-01-01');",
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
            .replace_metric_deltas(
                1,
                vec![delta("a", 3.0, Some("warn")), delta("b", 1.0, None)],
            )
            .await
            .unwrap();
        store
            .replace_metric_deltas(1, vec![delta("a", 4.0, Some("warn"))])
            .await
            .unwrap();
        store
            .replace_observations(
                1,
                vec![EffortObservation {
                    id: 0,
                    stream_id: "1".into(),
                    effort_id: "eff1".into(),
                    kind: "diff-coverage".into(),
                    provenance: "observed".into(),
                    source: "post-tool-bash".into(),
                    metric_value: Some(72.5),
                    payload_json: Some("{}".into()),
                    local_snapshot_id: None,
                    closest_git_version: None,
                    git_version_exact: false,
                    created_at: oxplow_domain::Timestamp::now(),
                }],
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
            q("SELECT kind, metric_value FROM v_effort_observation WHERE effort_id = 1").await,
            json!([["diff-coverage", 72.5]])
        );
    }
}
