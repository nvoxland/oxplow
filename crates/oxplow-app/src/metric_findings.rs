//! The findings function (P4.8, `.context/semantic-layer.md` "Metrics in
//! SQL"): a query reads the located items behind a metric as a table —
//!
//! ```sql
//! SELECT path, line, severity, rule, message
//! FROM metric_findings('oxplow.rust.unsafe_blocks')
//! ORDER BY path, line
//! ```
//!
//! `metric_findings('<key>')` is the metric's CURRENT state — what a
//! rescan still finds, never a fixed item's old fact
//! (`MetricEngine::current_facts`); `metric_findings('<key>', <capture>)`
//! is exactly one recording's. Each row is a fact the metric's filter
//! keeps, with its severity: the fact's own (a lint's), else the value
//! against the metric's `warn_at` / `fail_at` in its `direction`.
//!
//! As with `metric_grid()`, the engine computes the rows before any
//! connection is taken and the query runs against them as a temp table:
//! `metric_findings(…)` becomes `temp."metric_findings_1"`.

use oxplow_db::sql_tokens::{calls, string_literal, Call};
use oxplow_db::{SqlCell, TempTable};
use oxplow_domain::DomainError;

use crate::metric_engine::MetricEngine;

/// The findings' temp table.
pub const FINDINGS_TABLE: &str = "metric_findings_1";

/// A query's `metric_findings(…)` call, parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct FindingsPlan {
    pub key: String,
    pub capture: Option<i64>,
    call: Call,
}

fn invalid(msg: impl Into<String>) -> DomainError {
    DomainError::Invalid(msg.into())
}

/// The table's columns, in order.
const COLUMNS: &[&str] = &[
    "subject_kind",
    "subject_ref",
    "path",
    "line",
    "value",
    "severity",
    "rule",
    "message",
    "branch",
    "captured_at",
];

/// The findings plan of `sql`, or `None` when it reads no findings.
pub fn plan(sql: &str) -> Result<Option<FindingsPlan>, DomainError> {
    let found = calls(sql, "metric_findings")?;
    let call = match found.as_slice() {
        [] => return Ok(None),
        [one] => one.clone(),
        _ => return Err(invalid("a query reads one metric_findings()")),
    };
    let usage = "metric_findings() takes a quoted metric key and optionally a capture id, \
                 e.g. metric_findings('oxplow.rust.unsafe_blocks')";
    let (key, capture) = match call.args.as_slice() {
        [k] => (string_literal(k), None),
        [k, c] => (
            string_literal(k),
            Some(c.parse::<i64>().map_err(|_| {
                invalid(format!(
                    "metric_findings(): the capture id is a number (v_capture.id), not `{c}`"
                ))
            })?),
        ),
        _ => return Err(invalid(usage)),
    };
    let key = key.ok_or_else(|| invalid(usage))?;
    Ok(Some(FindingsPlan { key, capture, call }))
}

impl FindingsPlan {
    /// Read the findings and rewrite `sql` to read them. With `with_rows`
    /// false only the metric is resolved (a check).
    pub async fn materialize(
        &self,
        sql: &str,
        engine: &MetricEngine,
        with_rows: bool,
    ) -> Result<crate::metric_grid::Materialized, DomainError> {
        let key = &self.key;
        let spec = engine.spec(key).await?.ok_or_else(|| {
            invalid(format!(
                "metric_findings('{key}'): {}",
                crate::metric_engine::missing_metric(key)
            ))
        })?;
        let measure = spec.source_measure.clone().ok_or_else(|| {
            invalid(format!(
                "metric_findings('{key}'): `{key}` has no facts of its own (a formula or entity \
                 metric); read its view or its inputs instead"
            ))
        })?;
        let findings = if with_rows {
            engine.findings_for_spec(&spec, self.capture).await?
        } else {
            Vec::new()
        };
        let text = |v: Option<String>| v.map_or(SqlCell::Null(()), SqlCell::Text);
        let rows = findings
            .into_iter()
            .map(|f| {
                vec![
                    text(f.subject_kind),
                    text(f.subject_ref),
                    text(f.path),
                    f.line.map_or(SqlCell::Null(()), SqlCell::Int),
                    SqlCell::Real(f.value),
                    text(f.severity),
                    text(f.rule),
                    text(f.message),
                    text(f.branch),
                    SqlCell::Text(f.captured_at.to_text()),
                ]
            })
            .collect();
        let mut sql = sql.to_string();
        sql.replace_range(
            self.call.start..self.call.end,
            &format!("temp.{}", crate::metric_grid::quote(FINDINGS_TABLE)),
        );
        Ok(crate::metric_grid::Materialized {
            sql,
            table: TempTable {
                name: FINDINGS_TABLE.into(),
                columns: COLUMNS.iter().map(|c| c.to_string()).collect(),
                rows,
            },
            measures: vec![measure],
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use oxplow_db::{NewFact, NewMetricCapture, NewMetricSpec};

    use super::*;

    fn ts(s: &str) -> oxplow_domain::Timestamp {
        serde_json::from_str(&format!("\"{s}\"")).unwrap()
    }

    /// A lower-better metric (warn at 10, fail at 20) over a complete
    /// measure, scanned twice: the second scan no longer finds `a.rs`. (A
    /// per-path measure would keep `a.rs`: its scans restate only the
    /// paths they read.)
    async fn services() -> (Arc<crate::Services>, tempfile::TempDir, i64) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = Arc::new(crate::Services::in_memory(dir.path()).unwrap());
        svc.streams.ensure_primary().await.unwrap();
        let facts = &svc.fact_store;
        // A complete measure: each scan restates the whole population.
        let measure = facts
            .upsert_measure(oxplow_db::NewMeasure::new("acme.complexity", "Complexity"))
            .await
            .unwrap();
        let mut spec = NewMetricSpec::base("acme.complex", "Complex", "acme.complexity", "count");
        spec.direction = "lower-better".into();
        spec.warn_at = Some(10.0);
        spec.fail_at = Some(20.0);
        spec.filter_json = Some(r#"{"min_value":5.0}"#.into());
        facts.upsert_spec(spec).await.unwrap();
        let fact = |path: &str, value: f64| NewFact {
            subject_ref: Some(path.into()),
            path: Some(path.into()),
            line: Some(3),
            ..NewFact::new(measure, value)
        };
        let scan = |at: &str| NewMetricCapture {
            captured_at: Some(ts(at)),
            ..NewMetricCapture::done(1, "complexity", "builtin")
        };
        let first = facts
            .record_facts(
                scan("2026-09-01T10:00:00Z"),
                vec![fact("a.rs", 25.0), fact("b.rs", 12.0), fact("c.rs", 1.0)],
            )
            .await
            .unwrap();
        facts
            .record_facts(
                scan("2026-09-02T10:00:00Z"),
                vec![fact("b.rs", 12.0), fact("c.rs", 6.0)],
            )
            .await
            .unwrap();
        (svc, dir, first)
    }

    fn rows(out: &oxplow_db::SqlQueryResult) -> Vec<(String, Option<String>)> {
        out.rows
            .iter()
            .map(|r| match (&r[0], &r[1]) {
                (SqlCell::Text(p), SqlCell::Text(s)) => (p.clone(), Some(s.clone())),
                (SqlCell::Text(p), SqlCell::Null(())) => (p.clone(), None),
                other => panic!("{other:?}"),
            })
            .collect()
    }

    /// P4.8 (tsk516): the current state only — `a.rs` was fixed — each
    /// with its threshold severity; the filter drops `c.rs`'s 1 (first
    /// scan) but keeps its 6; the query reports the measure it read.
    #[tokio::test]
    async fn findings_are_the_current_offenders_with_their_severity() {
        let (svc, _dir, first) = services().await;
        let out = svc
            .sql
            .query_sql(
                "SELECT path, severity FROM metric_findings('acme.complex') ORDER BY path",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            rows(&out),
            vec![("b.rs".into(), Some("warn".into())), ("c.rs".into(), None)]
        );
        assert_eq!(out.reads.measures, vec!["acme.complexity".to_string()]);
        // One recording's, pinned.
        let out = svc
            .sql
            .query_sql(
                &format!(
                    "SELECT path, severity FROM metric_findings('acme.complex', {first}) ORDER BY path"
                ),
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            rows(&out),
            vec![
                ("a.rs".into(), Some("fail".into())),
                ("b.rs".into(), Some("warn".into()))
            ]
        );
        // It checks like any query.
        assert!(svc
            .sql
            .check("SELECT path, line, value, rule, message, subject_ref, captured_at FROM metric_findings('acme.complex')")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn bad_findings_calls_are_named() {
        let (svc, _dir, _) = services().await;
        for (sql, says) in [
            ("SELECT * FROM metric_findings('nope')", "no metric `nope`"),
            ("SELECT * FROM metric_findings(x)", "quoted metric key"),
            ("SELECT * FROM metric_findings('acme.complex', 'x')", "capture id"),
            (
                "SELECT * FROM metric_findings('acme.complex') JOIN metric_findings('acme.complex')",
                "one metric_findings()",
            ),
        ] {
            let err = svc.sql.query_sql(sql, vec![], None).await.unwrap_err();
            assert!(err.to_string().contains(says), "{sql}: {err}");
        }
    }
}
