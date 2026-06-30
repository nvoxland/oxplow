//! The metric aggregation engine (epic tsk12, child tsk15) — the heart of
//! "metrics are definitions computed over facts."
//!
//! A metric is NOT a stored row stream; it's a spec (a source measure + an
//! aggregation + an optional filter) evaluated over the durable [`FactRow`]s. The
//! engine turns those facts into a time series (one point per CAPTURE, aggregated
//! ACROSS the subjects measured in that capture) and into a by-dimension rollup.
//!
//! Additivity is respected at the two seams:
//!   * WITHIN a capture, subjects combine per the [`Aggregation`] (sum the
//!     per-file counts, count the matching functions, Σnum/Σden for a ratio).
//!   * ACROSS time, a measure's `temporal_semantics` governs how the series
//!     collapses to one number ([`range_value`]): semi-additive snapshots take the
//!     LAST capture; additive events SUM the captures; ratios re-derive Σnum/Σden.
//!
//! Built additively (no behaviour change): the producers (tsk14) feed facts, the
//! read surface (tsk16) exposes these results. Materialisation into a series cache
//! + derived-metric formulas are follow-ups under tsk15.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use oxplow_db::{FactRow, SqliteFactStore};
use oxplow_domain::{DomainError, Timestamp};

/// How the subjects measured in a single capture combine into the capture's
/// value. (Cross-time collapse is [`range_value`], driven by temporal semantics.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aggregation {
    /// Number of facts (e.g. count of functions over a complexity threshold).
    Count,
    /// Σ of the fact values (e.g. total complexity; a per-file count headline).
    Sum,
    /// Mean of the fact values.
    Avg,
    Min,
    Max,
    /// Value of the last fact in the capture (single-subject/tree metrics).
    Last,
    /// Σnumerator / Σdenominator across the capture's facts (coverage %, pass rate).
    Ratio,
}

impl Aggregation {
    /// Parse the spec string (`sum` | `avg` | `last` | `min` | `max` | `count` |
    /// `ratio`). Unknown ⇒ `None`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "count" => Self::Count,
            "sum" => Self::Sum,
            "avg" => Self::Avg,
            "min" => Self::Min,
            "max" => Self::Max,
            "last" => Self::Last,
            "ratio" => Self::Ratio,
            _ => return None,
        })
    }
}

/// `additive` | `semi-additive` | `non-additive` — a measure's additivity OVER
/// TIME (the BI distinction that governs how a series collapses to one number).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Temporal {
    Additive,
    SemiAdditive,
    NonAdditive,
}

impl Temporal {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "additive" => Self::Additive,
            "semi-additive" => Self::SemiAdditive,
            "non-additive" => Self::NonAdditive,
            _ => return None,
        })
    }
}

/// A simple conjunctive predicate over a fact. Covers the common metric filters
/// (count-over-threshold; severity/dimension equality). Richer predicates land
/// with the config layer (tsk17).
#[derive(Debug, Clone, Default)]
pub struct FactFilter {
    /// Keep facts with `value >= min_value` (e.g. complexity ≥ threshold).
    pub min_value: Option<f64>,
    /// Keep facts whose reported `severity` equals this (e.g. `error`).
    pub severity: Option<String>,
    /// Keep facts whose `dims_json[key]` equals this value.
    pub dim_eq: Option<(String, String)>,
}

impl FactFilter {
    pub fn matches(&self, f: &FactRow) -> bool {
        if let Some(min) = self.min_value {
            if f.value < min {
                return false;
            }
        }
        if let Some(sev) = &self.severity {
            if f.severity.as_deref() != Some(sev.as_str()) {
                return false;
            }
        }
        if let Some((key, val)) = &self.dim_eq {
            if dim_value(f, key).as_deref() != Some(val.as_str()) {
                return false;
            }
        }
        true
    }
}

/// One point in a metric's time series: a single capture's aggregated value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesPoint {
    pub capture_id: i64,
    pub captured_at: Timestamp,
    pub value: f64,
    /// Carried for ratio metrics so downstream roll-ups re-aggregate correctly.
    pub numerator: Option<f64>,
    pub denominator: Option<f64>,
    /// The group-by dimension value, when the series is sliced by a dimension.
    pub group: Option<String>,
}

/// One row of a by-dimension rollup (the metric's "breakdown" card).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RollupRow {
    pub key: String,
    pub value: f64,
    pub subject_count: i64,
}

/// The package (parent directory, repo-relative) of a path; root files ⇒ `"."`.
fn package_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => dir.to_string(),
        _ => ".".to_string(),
    }
}

/// Read a conformed-dimension value off a fact: the reported `severity`/`rule`
/// columns for those keys, `package` derived from the path, otherwise the
/// `dims_json` entry under the (namespaced) dimension key.
fn dim_value(f: &FactRow, dimension: &str) -> Option<String> {
    match dimension {
        "oxplow.severity" => f.severity.clone(),
        "oxplow.rule" => f.rule.clone(),
        "oxplow.package" | "package" => {
            let path = f.path.as_deref().or(f.subject_ref.as_deref())?;
            Some(package_of(path))
        }
        key => {
            let parsed: serde_json::Value = serde_json::from_str(f.dims_json.as_deref()?).ok()?;
            match parsed.get(key)? {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Null => None,
                other => Some(other.to_string()),
            }
        }
    }
}

/// Combine the facts measured in one capture into a `(value, numerator,
/// denominator)` triple per `agg`. `facts` is non-empty.
fn aggregate_facts(facts: &[&FactRow], agg: Aggregation) -> (f64, Option<f64>, Option<f64>) {
    match agg {
        Aggregation::Count => (facts.len() as f64, None, None),
        Aggregation::Sum => (facts.iter().map(|f| f.value).sum(), None, None),
        Aggregation::Avg => {
            let sum: f64 = facts.iter().map(|f| f.value).sum();
            (sum / facts.len() as f64, None, None)
        }
        Aggregation::Min => (
            facts.iter().map(|f| f.value).fold(f64::INFINITY, f64::min),
            None,
            None,
        ),
        Aggregation::Max => (
            facts
                .iter()
                .map(|f| f.value)
                .fold(f64::NEG_INFINITY, f64::max),
            None,
            None,
        ),
        Aggregation::Last => (facts.last().map(|f| f.value).unwrap_or(0.0), None, None),
        Aggregation::Ratio => {
            let mut num = 0.0;
            let mut den = 0.0;
            for f in facts {
                if let (Some(n), Some(d)) = (f.numerator, f.denominator) {
                    num += n;
                    den += d;
                }
            }
            let value = if den != 0.0 { num / den } else { 0.0 };
            (value, Some(num), Some(den))
        }
    }
}

/// Aggregate facts into a time series: one [`SeriesPoint`] per capture (optionally
/// per capture × dimension value), aggregated across the subjects in that capture.
/// Facts are expected oldest-first (as the store returns them), so the series is
/// time-ascending without a re-sort. Facts failing `filter` are dropped; when
/// `group_by` is set, facts missing that dimension are dropped (they can't be
/// placed on an axis).
pub fn aggregate_series(
    facts: &[FactRow],
    agg: Aggregation,
    filter: &FactFilter,
    group_by: Option<&str>,
) -> Vec<SeriesPoint> {
    // Preserve first-seen (time-ascending) order while grouping.
    let mut order: Vec<(i64, Option<String>)> = Vec::new();
    let mut index: HashMap<(i64, Option<String>), usize> = HashMap::new();
    let mut buckets: Vec<Vec<&FactRow>> = Vec::new();

    for f in facts {
        if !filter.matches(f) {
            continue;
        }
        let group = match group_by {
            Some(dim) => match dim_value(f, dim) {
                Some(g) => Some(g),
                None => continue, // can't place a fact missing the slice dim
            },
            None => None,
        };
        let key = (f.capture_id, group);
        match index.get(&key) {
            Some(&i) => buckets[i].push(f),
            None => {
                index.insert(key.clone(), buckets.len());
                order.push(key);
                buckets.push(vec![f]);
            }
        }
    }

    order
        .into_iter()
        .enumerate()
        .map(|(i, (capture_id, group))| {
            let fs = &buckets[i];
            let captured_at = fs[0].captured_at;
            let (value, numerator, denominator) = aggregate_facts(fs, agg);
            SeriesPoint {
                capture_id,
                captured_at,
                value,
                numerator,
                denominator,
                group,
            }
        })
        .collect()
}

/// Collapse a time series to a single in-range number, respecting additivity OVER
/// TIME: a semi-additive snapshot takes the LAST capture (summing snapshots across
/// time double-counts); an additive event SUMs the captures; a non-additive ratio
/// re-derives Σnumerator / Σdenominator across the points (never averages ratios).
/// `None` for an empty series.
pub fn range_value(series: &[SeriesPoint], temporal: Temporal) -> Option<f64> {
    if series.is_empty() {
        return None;
    }
    Some(match temporal {
        Temporal::SemiAdditive => series.last().map(|p| p.value).unwrap_or(0.0),
        Temporal::Additive => series.iter().map(|p| p.value).sum(),
        Temporal::NonAdditive => {
            let num: f64 = series.iter().filter_map(|p| p.numerator).sum();
            let den: f64 = series.iter().filter_map(|p| p.denominator).sum();
            if den != 0.0 {
                num / den
            } else {
                0.0
            }
        }
    })
}

/// Roll a metric's LATEST value per subject up by a dimension, summing across
/// subjects — the "breakdown" card (which package / language holds the most).
/// Largest first; ties broken on key for determinism. Facts are expected
/// oldest-first, so the last fact seen per subject is its latest.
pub fn compute_rollup(facts: &[FactRow], dimension: &str) -> Vec<RollupRow> {
    // Latest fact per subject (oldest-first ⇒ last write wins).
    let mut latest: HashMap<String, &FactRow> = HashMap::new();
    for f in facts {
        let Some(subject) = f.subject_ref.as_deref().or(f.path.as_deref()) else {
            continue;
        };
        latest.insert(subject.to_string(), f);
    }

    let mut by_key: std::collections::BTreeMap<String, (f64, i64)> =
        std::collections::BTreeMap::new();
    for f in latest.values() {
        let Some(key) = dim_value(f, dimension) else {
            continue;
        };
        let entry = by_key.entry(key).or_insert((0.0, 0));
        entry.0 += f.value;
        entry.1 += 1;
    }

    let mut out: Vec<RollupRow> = by_key
        .into_iter()
        .map(|(key, (value, subject_count))| RollupRow {
            key,
            value,
            subject_count,
        })
        .collect();
    out.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.key.cmp(&b.key))
    });
    out
}

/// The engine over a fact store: fetches the right facts and applies the pure
/// aggregation. A metric's facts are fetched by its `source_measure`.
#[derive(Clone)]
pub struct MetricEngine {
    facts: SqliteFactStore,
}

impl MetricEngine {
    pub fn new(facts: SqliteFactStore) -> Self {
        Self { facts }
    }

    /// The time series for a measure under an aggregation + filter, optionally
    /// sliced by a conformed dimension. Empty when the measure is unknown.
    pub async fn series(
        &self,
        measure_key: &str,
        agg: Aggregation,
        filter: &FactFilter,
        group_by: Option<&str>,
    ) -> Result<Vec<SeriesPoint>, DomainError> {
        let Some(measure) = self.facts.get_measure(measure_key).await? else {
            return Ok(Vec::new());
        };
        let facts = self.facts.facts_for_measure(measure.id).await?;
        Ok(aggregate_series(&facts, agg, filter, group_by))
    }

    /// The by-dimension rollup (breakdown) for a measure. Empty when unknown.
    pub async fn rollup(
        &self,
        measure_key: &str,
        dimension: &str,
    ) -> Result<Vec<RollupRow>, DomainError> {
        let Some(measure) = self.facts.get_measure(measure_key).await? else {
            return Ok(Vec::new());
        };
        let facts = self.facts.facts_for_measure(measure.id).await?;
        Ok(compute_rollup(&facts, dimension))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        serde_json::from_str(&format!("\"{s}\"")).unwrap()
    }

    /// A fact in `capture_id` captured at `captured_at` with `value`; override the
    /// remaining fields with struct-update syntax in each test as needed.
    fn fact(capture_id: i64, captured_at: &str, value: f64) -> FactRow {
        FactRow {
            id: 0,
            capture_id,
            measure_id: 1,
            value,
            numerator: None,
            denominator: None,
            subject_kind: None,
            subject_ref: None,
            path: None,
            line: None,
            severity: None,
            rule: None,
            detail: None,
            dims_json: None,
            captured_at: ts(captured_at),
            branch: None,
            closest_git_version: None,
            git_version_exact: false,
            basis_ref: None,
            snapshot_id: None,
            stream_id: 1,
            thread_id: None,
            effort_id: None,
            provenance: "observed".into(),
            source: "test".into(),
        }
    }

    #[test]
    fn count_series_counts_facts_over_threshold_per_capture() {
        // Capture 1 (10:00): three functions, two over threshold 10.
        // Capture 2 (11:00): one over threshold.
        let facts = vec![
            fact(1, "2026-06-30T10:00:00Z", 14.0),
            fact(1, "2026-06-30T10:00:00Z", 11.0),
            fact(1, "2026-06-30T10:00:00Z", 3.0),
            fact(2, "2026-06-30T11:00:00Z", 20.0),
            fact(2, "2026-06-30T11:00:00Z", 2.0),
        ];
        let series = aggregate_series(
            &facts,
            Aggregation::Count,
            &FactFilter {
                min_value: Some(10.0),
                ..Default::default()
            },
            None,
        );
        // One point per capture, time-ascending: 2 over-threshold, then 1.
        assert_eq!(
            series.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![2.0, 1.0]
        );
        // A count metric is semi-additive: the in-range value is the LAST capture.
        assert_eq!(range_value(&series, Temporal::SemiAdditive), Some(1.0));
    }

    #[test]
    fn sum_series_totals_per_capture_subjects() {
        let facts = vec![
            fact(1, "2026-06-30T10:00:00Z", 14.0),
            fact(1, "2026-06-30T10:00:00Z", 6.0),
        ];
        let series = aggregate_series(&facts, Aggregation::Sum, &FactFilter::default(), None);
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].value, 20.0);
    }

    #[test]
    fn ratio_series_reaggregates_components_not_naive_average() {
        // File A 1/1 (100%), File B 0/3 (0%). True combined = 1/4 = 25%, not 50%.
        let facts = vec![
            FactRow {
                numerator: Some(1.0),
                denominator: Some(1.0),
                subject_ref: Some("a.rs".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 1.0)
            },
            FactRow {
                numerator: Some(0.0),
                denominator: Some(3.0),
                subject_ref: Some("b.rs".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 0.0)
            },
        ];
        let series = aggregate_series(&facts, Aggregation::Ratio, &FactFilter::default(), None);
        assert_eq!(series.len(), 1);
        assert!(
            (series[0].value - 0.25).abs() < 1e-9,
            "got {}",
            series[0].value
        );
        // Non-additive across time too: Σnum/Σden, never an average of ratios.
        assert_eq!(range_value(&series, Temporal::NonAdditive), Some(0.25));
    }

    #[test]
    fn additive_series_sums_across_time_in_range() {
        let facts = vec![
            fact(1, "2026-06-30T10:00:00Z", 100.0),
            fact(2, "2026-06-30T11:00:00Z", 250.0),
        ];
        let series = aggregate_series(&facts, Aggregation::Sum, &FactFilter::default(), None);
        assert_eq!(
            series.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![100.0, 250.0]
        );
        // Additive over time: the in-range value SUMS the captures (350), unlike a
        // semi-additive snapshot which would take just the last (250).
        assert_eq!(range_value(&series, Temporal::Additive), Some(350.0));
        assert_eq!(range_value(&series, Temporal::SemiAdditive), Some(250.0));
    }

    #[test]
    fn series_grouped_by_dimension_splits_per_value() {
        let facts = vec![
            FactRow {
                dims_json: Some("{\"oxplow.language\":\"rust\"}".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 10.0)
            },
            FactRow {
                dims_json: Some("{\"oxplow.language\":\"rust\"}".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 4.0)
            },
            FactRow {
                dims_json: Some("{\"oxplow.language\":\"typescript\"}".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 7.0)
            },
        ];
        let mut series = aggregate_series(
            &facts,
            Aggregation::Sum,
            &FactFilter::default(),
            Some("oxplow.language"),
        );
        series.sort_by(|a, b| a.group.cmp(&b.group));
        let by_group: Vec<(Option<String>, f64)> =
            series.iter().map(|p| (p.group.clone(), p.value)).collect();
        assert_eq!(
            by_group,
            vec![
                (Some("rust".to_string()), 14.0),
                (Some("typescript".to_string()), 7.0),
            ]
        );
    }

    #[test]
    fn rollup_sums_latest_per_subject_by_package() {
        // a.rs measured twice — only the latest (3.0) counts.
        let facts = vec![
            FactRow {
                subject_ref: Some("src/app/a.rs".into()),
                path: Some("src/app/a.rs".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 9.0)
            },
            FactRow {
                subject_ref: Some("src/app/a.rs".into()),
                path: Some("src/app/a.rs".into()),
                ..fact(2, "2026-06-30T11:00:00Z", 3.0)
            },
            FactRow {
                subject_ref: Some("src/app/b.rs".into()),
                path: Some("src/app/b.rs".into()),
                ..fact(2, "2026-06-30T11:00:00Z", 4.0)
            },
            FactRow {
                subject_ref: Some("src/util/c.rs".into()),
                path: Some("src/util/c.rs".into()),
                ..fact(2, "2026-06-30T11:00:00Z", 20.0)
            },
        ];
        let rollup = compute_rollup(&facts, "package");
        let rows: Vec<(String, f64, i64)> = rollup
            .iter()
            .map(|r| (r.key.clone(), r.value, r.subject_count))
            .collect();
        // src/util (20) > src/app (3 latest a.rs + 4 b.rs = 7).
        assert_eq!(
            rows,
            vec![
                ("src/util".to_string(), 20.0, 1),
                ("src/app".to_string(), 7.0, 2),
            ]
        );
    }
}
