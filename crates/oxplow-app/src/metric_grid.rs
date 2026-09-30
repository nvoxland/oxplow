//! The metric function (P4.5, `.context/semantic-layer.md` "Metrics in
//! SQL"): a query reads metrics as columns of a grid —
//!
//! ```sql
//! SELECT bucket, zone, MEASURE('oxplow.coverage.abs_pct')
//! FROM metric_grid('day', 'zone')
//! ```
//!
//! The bucket `'capture'` keeps one row per capture instead: `bucket` is
//! the capture's time and `capture_id` joins `v_capture` for its branch,
//! provenance and git version.
//!
//! The engine stays the one authority on what a metric means (its
//! aggregation, temporal fold, filters, scale, the cube): each
//! `MEASURE('<key>')` is that metric's series, read through
//! `MetricEngine::series_for_spec_read` bucketed by the grid's bucket and
//! grouped by its dimension, **before** any connection is taken (the engine
//! holds its own pool permits). The points land in a temp table the query
//! runs against: `metric_grid(…)` becomes `temp."metric_grid_1"` and each
//! `MEASURE('<key>')` its column `"measure:<key>"`.

use std::collections::BTreeMap;

use oxplow_db::sql_tokens::{calls, string_literal, Call};
use oxplow_db::{SqlCell, TempTable};
use oxplow_domain::DomainError;

use crate::metric_bucket::TimeBucket;
use crate::metric_engine::{MetricEngine, SeriesRead};

/// The grid's temp table.
pub const GRID_TABLE: &str = "metric_grid_1";

fn invalid(msg: impl Into<String>) -> DomainError {
    DomainError::Invalid(msg.into())
}

/// A query's `metric_grid(…)` and `MEASURE(…)` calls, parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct GridPlan {
    /// `None`: one row per capture (`'capture'`).
    pub bucket: Option<TimeBucket>,
    /// The dimension each row is grouped by, when given.
    pub dim: Option<String>,
    /// The metric keys, in first-use order, distinct.
    pub keys: Vec<String>,
    grid: Call,
    measures: Vec<(Call, String)>,
}

/// The grid plan of `sql`, or `None` when it reads no metric. `MEASURE()`
/// without `metric_grid()`, or more than one grid, is an error.
pub fn plan(sql: &str) -> Result<Option<GridPlan>, DomainError> {
    let grids = calls(sql, "metric_grid")?;
    let measure_calls = calls(sql, "MEASURE")?;
    let grid = match grids.as_slice() {
        [] if measure_calls.is_empty() => return Ok(None),
        [] => {
            return Err(invalid(
                "MEASURE() reads a metric from `FROM metric_grid('<bucket>'[, '<dimension>'])`, which this query doesn't have",
            ))
        }
        [one] => one.clone(),
        _ => return Err(invalid("a query reads one metric_grid()")),
    };
    let literal = |arg: &str, what: &str| {
        string_literal(arg)
            .ok_or_else(|| invalid(format!("metric_grid(): the {what} is a quoted string")))
    };
    let (bucket, dim) = match grid.args.as_slice() {
        [b] => (literal(b, "bucket")?, None),
        [b, d] => (literal(b, "bucket")?, Some(literal(d, "dimension")?)),
        _ => return Err(invalid(
            "metric_grid() takes a bucket ('day', 'week', 'month' or 'capture') and optionally a dimension",
        )),
    };
    let bucket = match bucket.as_str() {
        "capture" => None,
        b => Some(TimeBucket::parse(b).ok_or_else(|| {
            invalid(format!(
                "metric_grid('{bucket}'): the bucket is 'day', 'week', 'month' or 'capture'"
            ))
        })?),
    };
    let mut keys: Vec<String> = Vec::new();
    let mut measures = Vec::new();
    for call in measure_calls {
        let key = match call.args.as_slice() {
            [arg] => string_literal(arg),
            _ => None,
        }
        .ok_or_else(|| {
            invalid(
                "MEASURE() takes one quoted metric key, e.g. MEASURE('oxplow.coverage.abs_pct')",
            )
        })?;
        if !keys.contains(&key) {
            keys.push(key.clone());
        }
        measures.push((call, key));
    }
    if keys.is_empty() {
        return Err(invalid(
            "metric_grid() is read through MEASURE('<key>') columns",
        ));
    }
    Ok(Some(GridPlan {
        bucket,
        dim,
        keys,
        grid,
        measures,
    }))
}

/// `"name"` — a quoted identifier.
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// A grid made ready to run: the query rewritten to read the temp table,
/// the table itself, and the measures behind its metrics.
#[derive(Debug, Clone)]
pub struct Materialized {
    pub sql: String,
    pub table: TempTable,
    pub measures: Vec<String>,
}

impl GridPlan {
    /// The grid's column for a metric key.
    pub fn column(key: &str) -> String {
        format!("measure:{key}")
    }

    /// Read every metric's series and build the grid. With `with_rows`
    /// false only the metrics are resolved (a check: the columns are what
    /// the query needs to compile).
    pub async fn materialize(
        &self,
        sql: &str,
        engine: &MetricEngine,
        stream: Option<i64>,
        with_rows: bool,
    ) -> Result<Materialized, DomainError> {
        // (bucket, capture, group) → one value per key, in key order.
        type Key = (String, Option<i64>, Option<String>);
        let mut grid: BTreeMap<Key, Vec<Option<f64>>> = BTreeMap::new();
        let mut measures = Vec::new();
        for (i, key) in self.keys.iter().enumerate() {
            let named = |e: DomainError| match e {
                DomainError::Invalid(m) => invalid(format!("MEASURE('{key}'): {m}")),
                other => other,
            };
            let spec = engine.spec(key).await?.ok_or_else(|| {
                invalid(format!(
                    "MEASURE('{key}'): no metric `{key}` (see v_metric_spec)"
                ))
            })?;
            let entity = crate::entity_metrics::entity_of(&spec);
            match (&spec.source_measure, &entity) {
                (Some(m), _) => measures.push(m.clone()),
                (None, Some(e)) => measures.push(e.view.clone()),
                (None, None) => {
                    return Err(invalid(format!(
                        "MEASURE('{key}'): `{key}` is a formula metric; read its inputs' measures instead"
                    )))
                }
            }
            let read = SeriesRead {
                group_by: self.dim.clone(),
                stream,
                bucket: self.bucket,
                ..SeriesRead::default()
            };
            let points = if with_rows {
                engine
                    .series_for_spec_read(&spec, &read)
                    .await
                    .map_err(named)?
            } else {
                // Still ask whether the dimension can slice it.
                engine
                    .series_for_spec_read(
                        &spec,
                        &SeriesRead {
                            // A window no capture falls in: nothing read.
                            window: Some(crate::metric_engine::TimeWindow {
                                from: Some(oxplow_domain::Timestamp::from_unix_ms(0)),
                                to: Some(oxplow_domain::Timestamp::from_unix_ms(0)),
                            }),
                            ..read
                        },
                    )
                    .await
                    .map_err(named)?
            };
            for p in points {
                let key = match self.bucket {
                    Some(_) => (
                        p.captured_at.to_text().chars().take(10).collect(),
                        None,
                        p.group.clone(),
                    ),
                    None => (p.captured_at.to_text(), Some(p.capture_id), p.group.clone()),
                };
                let row = grid
                    .entry(key)
                    .or_insert_with(|| vec![None; self.keys.len()]);
                row[i] = Some(p.value);
            }
        }
        let mut columns = vec!["bucket".to_string()];
        if self.bucket.is_none() {
            columns.push("capture_id".to_string());
        }
        if let Some(d) = &self.dim {
            columns.push(d.clone());
        }
        columns.extend(self.keys.iter().map(|k| Self::column(k)));
        let rows = grid
            .into_iter()
            .map(|((bucket, capture, group), values)| {
                let mut row = vec![SqlCell::Text(bucket)];
                if self.bucket.is_none() {
                    row.push(capture.map_or(SqlCell::Null(()), SqlCell::Int));
                }
                if self.dim.is_some() {
                    row.push(group.map_or(SqlCell::Null(()), SqlCell::Text));
                }
                row.extend(
                    values
                        .into_iter()
                        .map(|v| v.map_or(SqlCell::Null(()), SqlCell::Real)),
                );
                row
            })
            .collect();
        // Rewrite from the end so earlier offsets hold.
        let mut edits: Vec<(usize, usize, String)> = vec![(
            self.grid.start,
            self.grid.end,
            format!("temp.{}", quote(GRID_TABLE)),
        )];
        for (call, key) in &self.measures {
            edits.push((call.start, call.end, quote(&Self::column(key))));
        }
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        let mut sql = sql.to_string();
        for (start, end, with) in edits {
            sql.replace_range(start..end, &with);
        }
        measures.sort();
        measures.dedup();
        Ok(Materialized {
            sql,
            table: TempTable {
                name: GRID_TABLE.into(),
                columns,
                rows,
            },
            measures,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grid_query_is_planned_and_bad_ones_are_named() {
        let p = plan(
            "SELECT bucket, zone, MEASURE('a.b') AS a, measure('c') FROM metric_grid('week', 'zone') WHERE MEASURE('a.b') > 0",
        )
        .unwrap()
        .unwrap();
        assert_eq!(p.bucket, Some(TimeBucket::Week));
        assert_eq!(p.dim.as_deref(), Some("zone"));
        assert_eq!(p.keys, vec!["a.b".to_string(), "c".to_string()]);
        assert_eq!(plan("SELECT 1 FROM v_task").unwrap(), None);
        for (sql, says) in [
            ("SELECT MEASURE('a') FROM v_task", "doesn't have"),
            (
                "SELECT MEASURE('a') FROM metric_grid('hour')",
                "'day', 'week', 'month' or 'capture'",
            ),
            (
                "SELECT MEASURE(x) FROM metric_grid('day')",
                "one quoted metric key",
            ),
            ("SELECT bucket FROM metric_grid('day')", "MEASURE('<key>')"),
            (
                "SELECT MEASURE('a') FROM metric_grid('day') JOIN metric_grid('week')",
                "one metric_grid()",
            ),
        ] {
            let err = plan(sql).unwrap_err().to_string();
            assert!(err.contains(says), "{sql}: {err}");
        }
    }

    use crate::metric_engine::{MetricEngine, SeriesRead};
    use crate::sql_gateway::SqlGateway;
    use oxplow_db::{Database, NewFact, NewMetricCapture, NewMetricSpec, SqliteFactStore};

    fn ts(s: &str) -> oxplow_domain::Timestamp {
        serde_json::from_str(&format!("\"{s}\"")).unwrap()
    }

    /// Facts on three days in two zones, and three metrics over them.
    async fn fixture() -> (SqlGateway, MetricEngine, SqliteFactStore) {
        use oxplow_domain::stores::StreamStore;
        let db = Database::in_memory();
        oxplow_db::SqliteStreamStore::new(db.clone())
            .upsert(&oxplow_domain::Stream {
                id: oxplow_domain::StreamId::new(1),
                kind: oxplow_domain::StreamKind::Primary,
                title: "t".into(),
                branch: "main".into(),
                branch_ref: "refs/heads/main".into(),
                branch_source: "main".into(),
                worktree_path: "/r".into(),
                working_pane: String::new(),
                talking_pane: String::new(),
                working_session_id: String::new(),
                talking_session_id: String::new(),
                custom_prompt: None,
                created_at: ts("2026-01-01T00:00:00Z"),
                updated_at: ts("2026-01-01T00:00:00Z"),
                archived_at: None,
            })
            .await
            .unwrap();
        let facts = SqliteFactStore::new(db.clone());
        let m = facts
            .upsert_measure(oxplow_db::NewMeasure::new("acme.size", "Size"))
            .await
            .unwrap();
        for (day, values) in [
            ("2026-03-02T10:00:00Z", [3.0, 4.0]),
            ("2026-03-03T10:00:00Z", [5.0, 1.0]),
            ("2026-03-05T09:00:00Z", [2.0, 8.0]),
        ] {
            let cap = NewMetricCapture {
                captured_at: Some(ts(day)),
                ..NewMetricCapture::done(1, "metrics", "builtin")
            };
            let fs = [("api", values[0]), ("ui", values[1])]
                .into_iter()
                .map(|(zone, v)| NewFact {
                    dims_json: Some(format!("{{\"zone\":\"{zone}\"}}")),
                    ..NewFact::new(m, v)
                })
                .collect();
            facts.record_facts(cap, fs).await.unwrap();
        }
        for (key, agg) in [
            ("acme.size_sum", "sum"),
            ("acme.size_max", "max"),
            ("acme.size_count", "count"),
        ] {
            facts
                .upsert_spec(NewMetricSpec::base(key, key, "acme.size", agg))
                .await
                .unwrap();
        }
        let mut formula = NewMetricSpec::base("acme.ratio", "Ratio", "acme.size", "sum");
        formula.source_measure = None;
        formula.formula =
            Some(r#"{"op":"div","left":"acme.size_sum","right":"acme.size_count"}"#.into());
        facts.upsert_spec(formula).await.unwrap();
        let engine = MetricEngine::new(facts.clone());
        (
            SqlGateway::new(db).with_engine(engine.clone()),
            engine,
            facts,
        )
    }

    fn day(p: &crate::metric_engine::SeriesPoint) -> String {
        p.captured_at.to_text().chars().take(10).collect()
    }

    /// P4.5 (tsk490): a grid's rows are the engine's own series — for every
    /// metric, by bucket and by a dimension — and it runs on the one-
    /// connection database (the series are read before the query takes it).
    #[tokio::test]
    async fn grid_rows_equal_the_engine_series() {
        let (gateway, engine, facts) = fixture().await;
        for key in ["acme.size_sum", "acme.size_max", "acme.size_count"] {
            let spec = facts.get_spec(key).await.unwrap().unwrap();
            for dim in [None, Some("zone")] {
                let expected: Vec<Vec<serde_json::Value>> = engine
                    .series_for_spec_read(
                        &spec,
                        &SeriesRead {
                            group_by: dim.map(str::to_string),
                            bucket: Some(TimeBucket::Day),
                            ..SeriesRead::default()
                        },
                    )
                    .await
                    .unwrap()
                    .iter()
                    .map(|p| {
                        let mut row = vec![serde_json::json!(day(p))];
                        if dim.is_some() {
                            row.push(serde_json::json!(p.group));
                        }
                        row.push(serde_json::json!(p.value));
                        row
                    })
                    .collect();
                let sql = match dim {
                    None => format!("SELECT bucket, MEASURE('{key}') FROM metric_grid('day') ORDER BY bucket"),
                    Some(d) => format!(
                        "SELECT bucket, {d}, MEASURE('{key}') FROM metric_grid('day', '{d}') ORDER BY bucket, {d}"
                    ),
                };
                let out = gateway.query_sql(&sql, vec![], None).await.unwrap();
                let got: Vec<Vec<serde_json::Value>> =
                    serde_json::from_value(serde_json::to_value(&out.rows).unwrap()).unwrap();
                assert!(!expected.is_empty());
                assert_eq!(got, expected, "{key} by {dim:?}");
                assert_eq!(out.reads.measures, vec!["acme.size".to_string()]);
                assert!(out.reads.tables.iter().all(|t| t.starts_with("temp.")));
            }
        }
        // Several metrics side by side, filtered and aggregated in SQL. The
        // measure is complete (each capture restates the population), so the
        // week's sum is its last capture's, 2 + 8 — the engine's semantics,
        // not SQL's.
        let out = gateway
            .query_sql(
                "SELECT count(*), sum(MEASURE('acme.size_sum')), max(MEASURE('acme.size_max'))
                   FROM metric_grid('week') WHERE MEASURE('acme.size_count') > 0",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            serde_json::json!([[1, 10.0, 8.0]])
        );
        // A check resolves the metrics without reading them.
        let reads = gateway
            .check("SELECT bucket, MEASURE('acme.size_sum') FROM metric_grid('day')")
            .await
            .unwrap();
        assert_eq!(reads.measures, vec!["acme.size".to_string()]);
    }

    /// One row per capture, joinable to `v_capture` — what the metric pages
    /// read for their recordings.
    #[tokio::test]
    async fn a_capture_grid_has_a_row_per_capture() {
        let (gateway, engine, facts) = fixture().await;
        let spec = facts.get_spec("acme.size_sum").await.unwrap().unwrap();
        let series = engine
            .series_for_spec_read(&spec, &SeriesRead::default())
            .await
            .unwrap();
        let out = gateway
            .query_sql(
                "SELECT g.capture_id, MEASURE('acme.size_sum') AS v, c.provenance
                   FROM metric_grid('capture') g JOIN v_capture c ON c.id = g.capture_id
                  ORDER BY g.bucket",
                vec![],
                None,
            )
            .await
            .unwrap();
        let got: Vec<(i64, f64)> = out
            .rows
            .iter()
            .map(|r| match (&r[0], &r[1]) {
                (SqlCell::Int(id), SqlCell::Real(v)) => (*id, *v),
                other => panic!("{other:?}"),
            })
            .collect();
        let want: Vec<(i64, f64)> = series.iter().map(|p| (p.capture_id, p.value)).collect();
        assert_eq!(got, want);
        assert!(out.reads.models.contains(&"v_capture".to_string()));
    }

    #[tokio::test]
    async fn a_bad_measure_is_named() {
        let (gateway, _, _) = fixture().await;
        for (sql, says) in [
            (
                "SELECT MEASURE('nope') FROM metric_grid('day')",
                "MEASURE('nope'): no metric `nope`",
            ),
            (
                "SELECT MEASURE('acme.ratio') FROM metric_grid('day')",
                "MEASURE('acme.ratio'): `acme.ratio` is a formula metric",
            ),
        ] {
            let err = gateway
                .query_sql(sql, vec![], None)
                .await
                .unwrap_err()
                .to_string();
            assert!(err.contains(says), "{sql}: {err}");
        }
        let plain = SqlGateway::new(Database::in_memory());
        let err = plain
            .query_sql("SELECT MEASURE('x') FROM metric_grid('day')", vec![], None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no metric engine"), "{err}");
    }
}
