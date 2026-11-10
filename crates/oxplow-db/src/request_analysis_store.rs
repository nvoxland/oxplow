//! A request's stored analysis on one stream (`request_analysis`,
//! `request_metric_delta`, `request_observation`; V22): what the metric
//! engine computes over the request's window, stored so pages can read it.
//! `.context/collection.md` "A request's analysis".

use rusqlite::{params, Connection};

use oxplow_domain::DomainError;

use crate::database::{map_sql_err, Database};
use crate::effort_evidence_store::EffortObservation;
use crate::fact_store::EffortMetricDelta;

/// What a request's analysis on a stream is computed from, as a signature:
/// its turns' runs (how many, and the newest), its files (how many) and its
/// end (when its last turn there ended, or that one runs). Bound `?1`
/// request and `?2` stream.
const SIG: &str = "SELECT (SELECT count(*) || '|' || coalesce(max(c.id), '')
                             FROM metric_capture c
                            WHERE c.trigger = 'on-report'
                              AND c.turn_id IN (SELECT t.id FROM agent_turn t
                                                  JOIN threads th ON th.id = t.thread_id
                                                 WHERE t.request_id = ?1 AND th.stream_id = ?2))
                       || '|' || (SELECT count(*) FROM turn_file f
                                   JOIN agent_turn t ON t.id = f.turn_id
                                  WHERE t.request_id = ?1 AND f.stream_id = ?2)
                       || '|' || (SELECT coalesce(max(t.ended_at), '') || '|' || max(t.ended_at IS NULL)
                                    FROM agent_turn t JOIN threads th ON th.id = t.thread_id
                                   WHERE t.request_id = ?1 AND th.stream_id = ?2)";

fn sig_tx(c: &Connection, request: i64, stream: i64) -> Result<String, DomainError> {
    c.query_row(SIG, params![request, stream], |r| r.get(0))
        .map_err(map_sql_err)
}

/// The window an analysis was computed over, stored beside it.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisSpan {
    pub started_at: String,
    pub ended_at: Option<String>,
    pub start_snapshot_id: Option<i64>,
    pub end_snapshot_id: Option<i64>,
}

#[derive(Clone)]
pub struct SqliteRequestAnalysisStore {
    db: Database,
}

impl SqliteRequestAnalysisStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// What `request`'s analysis on `stream` is computed from now. Read it
    /// before computing, and store it with [`Self::replace`]: an input that
    /// lands while it computes leaves it stale.
    pub async fn sig(&self, request: i64, stream: i64) -> Result<String, DomainError> {
        self.db.read(move |c| sig_tx(c, request, stream)).await
    }

    /// The (request, stream) pairs to compute: each with a turn running on
    /// the stream, each whose stored analysis is stale, and each with a turn
    /// that ended there since `since` and none stored yet. Older requests
    /// are analyzed when asked for.
    pub async fn due(&self, since: String) -> Result<Vec<(i64, i64)>, DomainError> {
        self.db
            .read(move |c| {
                let mut stmt = c
                    .prepare(&format!(
                        "WITH candidate AS (
                           SELECT DISTINCT t.request_id AS request_id, th.stream_id AS stream_id
                             FROM agent_turn t JOIN threads th ON th.id = t.thread_id
                            WHERE t.request_id IS NOT NULL
                              AND (t.ended_at IS NULL OR t.ended_at >= ?1)
                           UNION
                           SELECT request_id, stream_id FROM request_analysis
                         )
                         SELECT k.request_id, k.stream_id FROM candidate k
                           LEFT JOIN request_analysis a
                             ON a.request_id = k.request_id AND a.stream_id = k.stream_id
                          WHERE a.sig IS NOT (
                                  {}
                                )
                          ORDER BY k.request_id, k.stream_id",
                        SIG.replace("?1", "k.request_id")
                            .replace("?2", "k.stream_id")
                    ))
                    .map_err(map_sql_err)?;
                let rows = stmt
                    .query_map(params![since], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(map_sql_err)?;
                Ok(rows)
            })
            .await
    }

    /// Replace `request`'s analysis on `stream` — its span, metric deltas
    /// and observations — and record `sig`, what it was computed from, in
    /// one transaction.
    pub async fn replace(
        &self,
        request: i64,
        stream: i64,
        span: AnalysisSpan,
        deltas: Vec<EffortMetricDelta>,
        observations: Vec<EffortObservation>,
        sig: String,
    ) -> Result<(), DomainError> {
        let at = crate::database::ts_to_string(oxplow_domain::Timestamp::now());
        self.db
            .transaction(move |tx| {
                tx.execute(
                    "INSERT INTO request_analysis (request_id, stream_id, started_at, ended_at,
                       start_snapshot_id, end_snapshot_id, sig, refreshed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT (request_id, stream_id) DO UPDATE SET
                       started_at = excluded.started_at, ended_at = excluded.ended_at,
                       start_snapshot_id = excluded.start_snapshot_id,
                       end_snapshot_id = excluded.end_snapshot_id,
                       sig = excluded.sig, refreshed_at = excluded.refreshed_at",
                    params![
                        request,
                        stream,
                        span.started_at,
                        span.ended_at,
                        span.start_snapshot_id,
                        span.end_snapshot_id,
                        sig,
                        at
                    ],
                )
                .map_err(map_sql_err)?;
                tx.execute(
                    "DELETE FROM request_metric_delta WHERE request_id = ?1 AND stream_id = ?2",
                    params![request, stream],
                )
                .map_err(map_sql_err)?;
                for d in &deltas {
                    tx.execute(
                        "INSERT INTO request_metric_delta (request_id, stream_id, key, title, unit,
                           direction, kind, category, language, agg, baseline, current, delta, changed,
                           attributed_files, sample_count, target, warn_at, fail_at, crossing,
                           latest_capture_id)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                                 ?16, ?17, ?18, ?19, ?20, ?21)",
                        params![
                            request, stream, d.key, d.title, d.unit, d.direction, d.kind,
                            d.category, d.language, d.agg, d.baseline, d.current, d.delta,
                            i64::from(d.changed), d.attributed_files, d.sample_count, d.target,
                            d.warn_at, d.fail_at, d.crossing, d.latest_capture_id
                        ],
                    )
                    .map_err(map_sql_err)?;
                }
                tx.execute(
                    "DELETE FROM request_observation WHERE request_id = ?1 AND stream_id = ?2",
                    params![request, stream],
                )
                .map_err(map_sql_err)?;
                for (seq, o) in observations.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO request_observation (request_id, stream_id, seq, kind,
                           provenance, source, metric_value, payload_json, local_snapshot_id,
                           created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![
                            request,
                            stream,
                            seq as i64,
                            o.kind,
                            o.provenance,
                            o.source,
                            o.metric_value,
                            o.payload_json,
                            o.local_snapshot_id,
                            crate::database::ts_to_string(o.created_at)
                        ],
                    )
                    .map_err(map_sql_err)?;
                }
                Ok(())
            })
            .await
    }
}
