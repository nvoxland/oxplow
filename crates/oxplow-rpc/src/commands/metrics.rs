//! Unified metric substrate read commands (epic tsk213).
//!
//! The successor to effort observations + code-quality scans: a durable,
//! time-anchored typed metric model. These are the read-side cores the Tauri
//! and remote transports share.

use oxplow_app::metric_engine::{Aggregation, FactFilter, FactFinding, RollupRow, SeriesPoint};
use oxplow_app::metrics_service::MetricCatalogEntry;
use oxplow_app::Services;
use oxplow_db::MetricSpec;

use crate::error::IpcError;

/// The metric catalog — every known metric SPEC (built-in / global / project).
/// A metric is an aggregation defined OVER a measure (epic tsk12), not a second
/// store of rows. Optional `language` / `scope` filter.
pub async fn list_metric_definitions(
    svc: &Services,
    language: Option<String>,
    scope: Option<String>,
) -> Result<Vec<MetricSpec>, IpcError> {
    let mut specs = svc.fact_store.list_specs().await?;
    if let Some(lang) = language.as_deref() {
        specs.retain(|s| s.language.as_deref() == Some(lang));
    }
    if let Some(scope) = scope.as_deref() {
        specs.retain(|s| s.scope == scope);
    }
    Ok(specs)
}

/// Time series for one metric (by spec `key`) — one point per capture,
/// aggregated over the metric's source-measure facts (epic tsk12): value
/// (+numerator/denominator), captured_at, branch, provenance. Newest-first,
/// capped at `limit` (default 200). `group_by` slices by a conformed dimension
/// (`subject` / `branch` / `oxplow.model` / …), one series-point per
/// (capture × group). Unknown key → empty (UI-friendly, not an error).
pub async fn list_metric_samples(
    svc: &Services,
    metric_key: String,
    limit: Option<i64>,
    group_by: Option<String>,
) -> Result<Vec<SeriesPoint>, IpcError> {
    let Some(spec) = svc.fact_store.get_spec(&metric_key).await? else {
        return Ok(vec![]);
    };
    let mut rows = svc
        .metric_engine
        .series_for_spec(&spec, group_by.as_deref())
        .await?;
    // The engine returns oldest→newest; this read is newest-first, capped.
    rows.reverse();
    let limit = limit.unwrap_or(200).max(0) as usize;
    rows.truncate(limit);
    Ok(rows)
}

/// The located items behind one metric (by spec `key`) — the read-time finding
/// view over the metric's filtered facts (epic tsk12): subject, path/line,
/// severity (reported for lint, else derived from thresholds × direction),
/// rule, message, value. `capture_id` scopes to one recording's drill-in (omit
/// for every matching fact). Unknown key → empty (UI-friendly).
pub async fn list_metric_findings(
    svc: &Services,
    metric_key: String,
    capture_id: Option<i64>,
) -> Result<Vec<FactFinding>, IpcError> {
    let Some(spec) = svc.fact_store.get_spec(&metric_key).await? else {
        return Ok(vec![]);
    };
    Ok(svc
        .metric_engine
        .findings_for_spec(&spec, capture_id)
        .await?)
}

/// Roll up one metric (by spec `key`) by a dimension — its source-measure
/// facts, additivity-aware per the measure's temporal semantics (level gauges:
/// latest per subject; events: every fact; ratios: per-group Σnum/Σden),
/// largest first, with the contributing subject count (epic tsk12). `dimension`
/// is `"package"`, a conformed dim (`oxplow.severity` / …), or any `dims_json`
/// key. Unknown key → empty (UI-friendly). Backs the Metric Detail
/// **Breakdown** card + the subject breakdown (tsk328 package / tsk319 language).
pub async fn metric_dimension_rollup(
    svc: &Services,
    metric_key: String,
    dimension: String,
) -> Result<Vec<RollupRow>, IpcError> {
    let Some(spec) = svc.fact_store.get_spec(&metric_key).await? else {
        return Ok(vec![]);
    };
    Ok(svc.metric_engine.rollup_for_spec(&spec, &dimension).await?)
}

/// Time SERIES for a MEASURE, aggregated per capture over its atomic facts
/// (epic tsk12) — the measure-level read (vs `list_metric_samples`'s spec-key
/// ergonomics). `aggregation` is count|sum|avg|min|max|last|ratio; `group_by`
/// slices by a conformed dimension; `min_value` keeps facts ≥ a threshold;
/// `severity` keeps one lint severity. Empty when the measure is unknown.
pub async fn metric_series(
    svc: &Services,
    measure_key: String,
    aggregation: String,
    group_by: Option<String>,
    min_value: Option<f64>,
    severity: Option<String>,
) -> Result<Vec<SeriesPoint>, IpcError> {
    let Some(agg) = Aggregation::parse(&aggregation) else {
        return Err(IpcError::invalid(
            "aggregation must be one of count|sum|avg|min|max|last|ratio",
        ));
    };
    let filter = FactFilter {
        min_value,
        max_value: None,
        severity,
        dim_eq: None,
    };
    Ok(svc
        .metric_engine
        .series(&measure_key, agg, &filter, group_by.as_deref())
        .await?)
}

/// By-dimension ROLLUP (breakdown) for a MEASURE over its atomic facts (epic
/// tsk12), additivity-aware per its temporal semantics (level gauges: latest
/// per subject; events: every fact; ratios: per-group Σnum/Σden), largest
/// first, with the contributing subject count. `dimension` is `oxplow.package`
/// (default), a conformed dim, or any `dims_json` key. Empty when unknown.
pub async fn metric_rollup(
    svc: &Services,
    measure_key: String,
    dimension: Option<String>,
) -> Result<Vec<RollupRow>, IpcError> {
    let dimension = dimension.unwrap_or_else(|| "oxplow.package".to_string());
    Ok(svc.metric_engine.rollup(&measure_key, &dimension).await?)
}

/// The available catalog (built-in ∪ global ∪ project) with each entry's
/// enabled-in-this-project flag — drives the Catalog page (tsk219).
pub async fn list_metric_catalog(svc: &Services) -> Result<Vec<MetricCatalogEntry>, IpcError> {
    Ok(svc.metrics.catalog().await)
}

/// Enable (add a `use:`) or disable (remove) a metric in `.oxplow/project.yaml`, then
/// reseed. The Catalog toggle.
pub async fn set_metric_enabled(
    svc: &Services,
    key: String,
    enabled: bool,
) -> Result<(), IpcError> {
    svc.metrics
        .set_metric_enabled(&key, enabled)
        .await
        .map_err(IpcError::internal)
}

/// Enable or disable MANY metrics in one config write + reseed — the per-section
/// "Enable all / Disable all" action (tsk32).
pub async fn set_metrics_enabled(
    svc: &Services,
    keys: Vec<String>,
    enabled: bool,
) -> Result<(), IpcError> {
    svc.metrics
        .set_metrics_enabled(&keys, enabled)
        .await
        .map_err(IpcError::internal)
}

/// Set a metric's `target` override in `.oxplow/project.yaml` (enabling it if needed;
/// `None` clears that override), then reseed. The Catalog inline edit (tsk233).
/// `trigger` is inherent to the definition and is not overridable (tsk290).
pub async fn set_metric_override(
    svc: &Services,
    key: String,
    target: Option<f64>,
) -> Result<(), IpcError> {
    svc.metrics
        .set_metric_override(&key, target)
        .await
        .map_err(IpcError::internal)
}
