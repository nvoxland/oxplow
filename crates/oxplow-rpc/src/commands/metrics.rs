//! Unified metric substrate read commands (epic tsk213).
//!
//! The successor to effort observations + code-quality scans: a durable,
//! time-anchored typed metric model. These are the read-side cores the Tauri
//! and remote transports share.

use oxplow_app::metric_engine::{SeriesPoint, TimeWindow};
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
    from_ms: Option<i64>,
    to_ms: Option<i64>,
) -> Result<Vec<SeriesPoint>, IpcError> {
    let Some(spec) = svc.fact_store.get_spec(&metric_key).await? else {
        return Ok(vec![]);
    };
    // Bound the read to the caller's visible range (tsk202) so it no longer
    // computes the whole history just to show a window.
    let window = TimeWindow::from_ms(from_ms, to_ms);
    let mut rows = svc
        .metric_engine
        .series_for_spec_in_stream(&spec, group_by.as_deref(), None, window)
        .await?;
    // The engine returns oldest→newest; this read is newest-first, capped.
    rows.reverse();
    let limit = limit.unwrap_or(200).max(0) as usize;
    rows.truncate(limit);
    Ok(rows)
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
