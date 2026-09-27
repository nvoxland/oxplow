//! Entity metrics (tsk322): metrics computed over a semantic-layer view
//! (`v_task`, an extension's `v_*`, …) instead of a measure's facts, sliced by
//! entity dimensions (a SQL expression over the same view, with an optional
//! join).
//!
//! Two kinds:
//! - an **event** metric has a `time` expression: rows bucket by when they
//!   happened and the series is computed live (this module);
//! - a **state** metric has none: its current value is captured as a fact on
//!   a synthesized measure (the `entity-metric` producer in
//!   `metrics_service`), so its history reads through the fact path. Only its
//!   grouped reads come from here, as the current value per group.
//!
//! Everything aggregates in SQL — one row per (bucket, group) comes back, never
//! the entity's rows — so the semantic layer's row cap can't truncate a
//! metric. Median / p90 use a `ROW_NUMBER()` window (p90 is nearest-rank).
//! Queries run through [`SemanticLayer`], so they're read-only by
//! construction. The metric's `where` / `time` / `value` are evaluated over
//! its view alone (aliased `e`); only a dimension's `expr` sees its `join`. See `.context/metrics.md`.

use oxplow_config::{EntityDimensionSpec, EntitySpec};
use oxplow_db::{Dimension, MetricSpec, SemanticLayer, SqlCell};
use oxplow_domain::{DomainError, Timestamp};

use crate::metric_bucket::TimeBucket;
use crate::metric_engine::{RollupRow, SeriesPoint, TimeWindow};

/// The entity half of a stored spec, if it is an entity metric.
pub fn entity_of(spec: &MetricSpec) -> Option<EntitySpec> {
    serde_json::from_str(spec.entity_json.as_deref()?).ok()
}

/// The entity half of a stored dimension, if it is an entity dimension.
pub fn entity_dim_of(d: &Dimension) -> Option<EntityDimensionSpec> {
    serde_json::from_str(d.entity_json.as_deref()?).ok()
}

/// Whether an aggregation adds up across buckets (so a missing bucket is a
/// real zero, and a headline is the total).
pub fn additive(aggregation: &str) -> bool {
    matches!(aggregation, "count" | "count_distinct" | "sum")
}

/// One aggregate read over an entity.
pub struct EntityRead<'a> {
    pub spec: &'a EntitySpec,
    pub dim: Option<&'a EntityDimensionSpec>,
    pub bucket: Option<TimeBucket>,
    pub window: Option<TimeWindow>,
}

/// The SQL for a read: columns `b` (bucket start date or NULL), `g` (group or
/// NULL), `value`, `n` (rows aggregated), ordered by bucket then group.
pub fn build_sql(read: &EntityRead) -> (String, Vec<(String, SqlCell)>) {
    let s = read.spec;
    let time = s.time.as_deref();
    let bucket = match (read.bucket, time) {
        (Some(TimeBucket::Day), Some(t)) => format!("date({t})"),
        (Some(TimeBucket::Week), Some(t)) => format!("date({t}, '-6 days', 'weekday 1')"),
        (Some(TimeBucket::Month), Some(t)) => format!("date({t}, 'start of month')"),
        _ => "NULL".into(),
    };
    let group = read
        .dim
        .map(|d| format!("({})", d.expr))
        .unwrap_or_else(|| "NULL".into());
    let value = s
        .value
        .as_deref()
        .map(|v| format!("({v})"))
        .unwrap_or_else(|| "1".into());
    let join = read.dim.and_then(|d| d.join.as_deref()).unwrap_or("");
    let mut conds = vec![s
        .where_
        .as_deref()
        .map(|w| format!("({w})"))
        .unwrap_or_else(|| "1".into())];
    let mut params = Vec::new();
    if let Some(t) = time {
        if read.bucket.is_some() {
            conds.push(format!("({t}) IS NOT NULL"));
        }
        if let Some(w) = read.window {
            let text = |ts: Timestamp| {
                SqlCell::Text(
                    serde_json::to_value(ts)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default(),
                )
            };
            if let Some(from) = w.from {
                conds.push(format!("julianday({t}) >= julianday(:from)"));
                params.push(("from".to_string(), text(from)));
            }
            if let Some(to) = w.to {
                conds.push(format!("julianday({t}) <= julianday(:to)"));
                params.push(("to".to_string(), text(to)));
            }
        }
    }
    // The metric's own fragments see only its view; the dimension's join
    // applies outside, so a bare column in `where` can't turn ambiguous.
    let base = format!(
        "SELECT e.__b AS b, {group} AS g, e.__v AS v
         FROM (SELECT e.*, {bucket} AS __b, {value} AS __v FROM {view} e WHERE {conds}) e {join}",
        view = s.view,
        conds = conds.join(" AND ")
    );
    let pick = match s.aggregation.as_str() {
        "median" => Some("rn IN ((cnt + 1) / 2, (cnt + 2) / 2)"),
        "p90" => Some("rn = (9 * cnt + 9) / 10"),
        _ => None,
    };
    let sql = match pick {
        Some(cond) => format!(
            "WITH base AS ({base}), ranked AS (
               SELECT b, g, v,
                      ROW_NUMBER() OVER (PARTITION BY b, g ORDER BY v) AS rn,
                      COUNT(*) OVER (PARTITION BY b, g) AS cnt
               FROM base WHERE v IS NOT NULL)
             SELECT b, g, AVG(v) AS value, MAX(cnt) AS n FROM ranked WHERE {cond}
             GROUP BY b, g ORDER BY b, g"
        ),
        None => {
            let agg = match s.aggregation.as_str() {
                "count_distinct" => "COUNT(DISTINCT v)",
                "sum" => "TOTAL(v)",
                "avg" => "AVG(v)",
                "min" => "MIN(v)",
                "max" => "MAX(v)",
                _ => "COUNT(*)",
            };
            format!(
                "WITH base AS ({base})
                 SELECT b, g, {agg} AS value, COUNT(*) AS n FROM base
                 GROUP BY b, g ORDER BY b, g"
            )
        }
    };
    (sql, params)
}

/// Check that a metric (and optionally a dimension over it) compiles against
/// the current schema, without running it.
pub async fn check(
    layer: &SemanticLayer,
    spec: &EntitySpec,
    dim: Option<&EntityDimensionSpec>,
) -> Result<(), DomainError> {
    let (sql, _) = build_sql(&EntityRead {
        spec,
        dim,
        bucket: spec.time.as_ref().map(|_| TimeBucket::Day),
        window: None,
    });
    layer.check_sql(&sql).await
}

/// One aggregated row of a read.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityRow {
    pub bucket: Option<String>,
    pub group: Option<String>,
    pub value: Option<f64>,
    pub rows: i64,
}

fn text(c: &SqlCell) -> Option<String> {
    match c {
        SqlCell::Null(()) => None,
        SqlCell::Text(s) => Some(s.clone()),
        SqlCell::Int(i) => Some(i.to_string()),
        SqlCell::Real(r) => Some(r.to_string()),
        SqlCell::Bool(b) => Some(b.to_string()),
    }
}

fn number(c: &SqlCell) -> Option<f64> {
    match c {
        SqlCell::Int(i) => Some(*i as f64),
        SqlCell::Real(r) => Some(*r),
        SqlCell::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

/// Run a read.
pub async fn run(
    layer: &SemanticLayer,
    read: &EntityRead<'_>,
) -> Result<Vec<EntityRow>, DomainError> {
    let (sql, params) = build_sql(read);
    let out = layer
        .query_sql_named(&sql, params, Some(oxplow_db::semantic_layer::MAX_ROW_LIMIT))
        .await?;
    Ok(out
        .rows
        .iter()
        .map(|r| EntityRow {
            bucket: text(&r[0]),
            group: text(&r[1]),
            value: number(&r[2]),
            rows: number(&r[3]).unwrap_or(0.0) as i64,
        })
        .collect())
}

fn day_start(date: &str) -> Option<Timestamp> {
    serde_json::from_value(serde_json::Value::String(format!("{date}T00:00:00Z"))).ok()
}

/// The next bucket start after `at`.
fn next_bucket(at: Timestamp, bucket: TimeBucket) -> Timestamp {
    let date = at.0.date();
    let next = match bucket {
        TimeBucket::Day => date.next_day().unwrap_or(date),
        TimeBucket::Week => date + time::Duration::days(7),
        TimeBucket::Month => {
            let (y, m) = (date.year(), date.month());
            let (y, m) = if m == time::Month::December {
                (y + 1, m.next())
            } else {
                (y, m.next())
            };
            time::Date::from_calendar_date(y, m, 1).unwrap_or(date)
        }
    };
    Timestamp(next.midnight().assume_utc())
}

/// An event metric's series: one point per bucket (and group). An ungrouped
/// additive series (count / sum) is zero-filled across the window, or across
/// the span it covers when unwindowed, so a quiet day reads 0 rather than
/// vanishing.
pub async fn series(
    layer: &SemanticLayer,
    spec: &EntitySpec,
    dim: Option<&EntityDimensionSpec>,
    bucket: TimeBucket,
    window: Option<TimeWindow>,
) -> Result<Vec<SeriesPoint>, DomainError> {
    let rows = run(
        layer,
        &EntityRead {
            spec,
            dim,
            bucket: Some(bucket),
            window,
        },
    )
    .await?;
    let mut points: Vec<SeriesPoint> = rows
        .into_iter()
        .filter_map(|r| {
            Some(point(
                day_start(r.bucket.as_deref()?)?,
                r.value.unwrap_or(0.0),
                r.group,
            ))
        })
        .collect();
    if dim.is_none() && additive(&spec.aggregation) {
        let first = window
            .and_then(|w| w.from)
            .map(|f| bucket.start_of(f))
            .or_else(|| points.first().map(|p| p.captured_at));
        let last = window
            .and_then(|w| w.to)
            .map(|t| bucket.start_of(t))
            .or_else(|| points.last().map(|p| p.captured_at));
        if let (Some(mut at), Some(last)) = (first, last) {
            let mut filled = Vec::new();
            let mut have = points.into_iter().peekable();
            while at <= last {
                match have.peek() {
                    Some(p) if p.captured_at == at => filled.push(have.next().expect("peeked")),
                    _ => filled.push(point(at, 0.0, None)),
                }
                at = next_bucket(at, bucket);
            }
            filled.extend(have);
            points = filled;
        }
    }
    Ok(points)
}

fn point(at: Timestamp, value: f64, group: Option<String>) -> SeriesPoint {
    SeriesPoint {
        capture_id: 0,
        captured_at: at,
        value,
        numerator: None,
        denominator: None,
        group,
        branch: None,
        provenance: Some("observed".into()),
        git_version: None,
        source: Some("entity".into()),
    }
}

/// The value right now (a state metric's capture), or per group.
pub async fn current(
    layer: &SemanticLayer,
    spec: &EntitySpec,
    dim: Option<&EntityDimensionSpec>,
) -> Result<Vec<EntityRow>, DomainError> {
    // A state read ignores `time`: it's the whole entity as it stands.
    let state = EntitySpec {
        time: None,
        ..spec.clone()
    };
    run(
        layer,
        &EntityRead {
            spec: &state,
            dim,
            bucket: None,
            window: None,
        },
    )
    .await
}

/// A by-dimension breakdown: the metric over every row, per group, largest first.
pub async fn rollup(
    layer: &SemanticLayer,
    spec: &EntitySpec,
    dim: &EntityDimensionSpec,
) -> Result<Vec<RollupRow>, DomainError> {
    let mut rows: Vec<RollupRow> = current(layer, spec, Some(dim))
        .await?
        .into_iter()
        .map(|r| RollupRow {
            key: r.group.unwrap_or_else(|| "(none)".into()),
            value: r.value.unwrap_or(0.0),
            subject_count: r.rows,
        })
        .collect();
    rows.sort_by(|a, b| b.value.total_cmp(&a.value).then_with(|| a.key.cmp(&b.key)));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;

    async fn layer() -> SemanticLayer {
        let db = Database::in_memory();
        db.transaction(|c| {
            c.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/r', '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                   VALUES (1, 1, 'T', 'active', '2026-01-01', '2026-01-01');
                 INSERT INTO task (id, thread_id, title, status, priority, sort_index, created_by, created_at, updated_at, completed_at) VALUES
                   (1, 1, 'a', 'done', 'high', 10, 'agent', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', '2026-09-21T09:00:00Z'),
                   (2, 1, 'b', 'done', 'low', 20, 'agent', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', '2026-09-21T17:30:00Z'),
                   (3, 1, 'c', 'done', 'high', 30, 'agent', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', '2026-09-23T12:00:00Z'),
                   (4, 1, 'd', 'ready', 'high', 40, 'agent', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', NULL),
                   (5, 1, 'e', 'in_progress', 'medium', 50, 'agent', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', NULL);",
            )
            .map_err(|e| DomainError::Invalid(e.to_string()))
        })
        .await
        .unwrap();
        SemanticLayer::new(db)
    }

    fn done() -> EntitySpec {
        EntitySpec {
            view: "v_task".into(),
            where_: Some("status = 'done'".into()),
            time: Some("completed_at".into()),
            value: None,
            aggregation: "count".into(),
        }
    }

    fn priority() -> EntityDimensionSpec {
        EntityDimensionSpec {
            view: "v_task".into(),
            expr: "e.priority".into(),
            join: None,
        }
    }

    fn day(p: &SeriesPoint) -> String {
        serde_json::to_value(p.captured_at)
            .unwrap()
            .as_str()
            .unwrap()[..10]
            .to_string()
    }

    #[tokio::test]
    async fn an_event_metric_counts_per_day_and_zero_fills_quiet_days() {
        let l = layer().await;
        let s = series(&l, &done(), None, TimeBucket::Day, None)
            .await
            .unwrap();
        let got: Vec<_> = s.iter().map(|p| (day(p), p.value)).collect();
        assert_eq!(
            got,
            vec![
                ("2026-09-21".to_string(), 2.0),
                ("2026-09-22".to_string(), 0.0),
                ("2026-09-23".to_string(), 1.0)
            ]
        );
        let weekly = series(&l, &done(), None, TimeBucket::Week, None)
            .await
            .unwrap();
        assert_eq!(
            weekly.iter().map(|p| (day(p), p.value)).collect::<Vec<_>>(),
            vec![("2026-09-21".to_string(), 3.0)]
        );
        // A window bounds the rows and the fill.
        let w = TimeWindow {
            from: Some(serde_json::from_str("\"2026-09-22T00:00:00Z\"").unwrap()),
            to: Some(serde_json::from_str("\"2026-09-24T12:00:00Z\"").unwrap()),
        };
        let windowed = series(&l, &done(), None, TimeBucket::Day, Some(w))
            .await
            .unwrap();
        assert_eq!(
            windowed
                .iter()
                .map(|p| (day(p), p.value))
                .collect::<Vec<_>>(),
            vec![
                ("2026-09-22".to_string(), 0.0),
                ("2026-09-23".to_string(), 1.0),
                ("2026-09-24".to_string(), 0.0)
            ]
        );
    }

    #[tokio::test]
    async fn grouped_by_an_entity_dimension() {
        let l = layer().await;
        let s = series(&l, &done(), Some(&priority()), TimeBucket::Day, None)
            .await
            .unwrap();
        let got: Vec<_> = s
            .iter()
            .map(|p| (day(p), p.group.clone().unwrap(), p.value))
            .collect();
        assert_eq!(
            got,
            vec![
                ("2026-09-21".to_string(), "high".to_string(), 1.0),
                ("2026-09-21".to_string(), "low".to_string(), 1.0),
                ("2026-09-23".to_string(), "high".to_string(), 1.0)
            ]
        );
        // With a join: the dimension may read another view.
        let by_thread = EntityDimensionSpec {
            view: "v_task".into(),
            expr: "t.title".into(),
            join: Some("LEFT JOIN v_thread t ON t.id = e.thread_id".into()),
        };
        let rows = rollup(&l, &done(), &by_thread).await.unwrap();
        assert_eq!(
            (rows[0].key.as_str(), rows[0].value, rows[0].subject_count),
            ("T", 3.0, 3)
        );
    }

    #[tokio::test]
    async fn state_values_and_order_statistics() {
        let l = layer().await;
        let open = EntitySpec {
            where_: Some("status IN ('ready', 'in_progress', 'blocked')".into()),
            time: None,
            ..done()
        };
        let now = current(&l, &open, None).await.unwrap();
        assert_eq!(now[0].value, Some(2.0));
        let stat = |agg: &str| EntitySpec {
            where_: None,
            value: Some("e.sort_index".into()),
            aggregation: agg.into(),
            ..done()
        };
        let one = |agg: &'static str| {
            let l = l.clone();
            async move {
                current(&l, &stat(agg), None).await.unwrap()[0]
                    .value
                    .unwrap()
            }
        };
        // sort_index 10..50: median 30, nearest-rank p90 of 5 = 5th = 50.
        assert_eq!(one("median").await, 30.0);
        assert_eq!(one("p90").await, 50.0);
        assert_eq!(one("sum").await, 150.0);
        assert_eq!(one("avg").await, 30.0);
        let by_prio = rollup(&l, &stat("count_distinct"), &priority())
            .await
            .unwrap();
        assert_eq!(by_prio[0].key, "high");
        assert_eq!(by_prio[0].value, 3.0);
    }

    #[tokio::test]
    async fn a_bad_fragment_is_caught_without_running() {
        let l = layer().await;
        check(&l, &done(), Some(&priority())).await.unwrap();
        let bad = EntitySpec {
            where_: Some("no_such_column = 1".into()),
            ..done()
        };
        assert!(check(&l, &bad, None).await.is_err());
    }
}
