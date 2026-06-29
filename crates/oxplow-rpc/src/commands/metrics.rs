//! Unified metric substrate read commands (epic tsk213).
//!
//! The successor to effort observations + code-quality scans: a durable,
//! time-anchored typed metric model. These are the read-side cores the Tauri
//! and remote transports share.

use oxplow_app::metrics_service::MetricCatalogEntry;
use oxplow_app::Services;
use oxplow_db::{MetricDefinition, MetricDimensionRollup, MetricFinding, MetricSample};

use crate::error::IpcError;

/// The metric catalog — every known definition (built-in / global / project).
/// Optional `language` / `scope` filter.
pub async fn list_metric_definitions(
    svc: &Services,
    language: Option<String>,
    scope: Option<String>,
) -> Result<Vec<MetricDefinition>, IpcError> {
    let mut defs = svc.metric_store.list_definitions().await?;
    if let Some(lang) = language.as_deref() {
        defs.retain(|d| d.language.as_deref() == Some(lang));
    }
    if let Some(scope) = scope.as_deref() {
        defs.retain(|d| d.scope == scope);
    }
    Ok(defs)
}

/// Samples for one metric (by definition `key`), newest-first, capped at
/// `limit` (default 200). Unknown key → empty (UI-friendly, not an error).
pub async fn list_metric_samples(
    svc: &Services,
    metric_key: String,
    limit: Option<i64>,
) -> Result<Vec<MetricSample>, IpcError> {
    let Some(def) = svc.metric_store.get_definition(&metric_key).await? else {
        return Ok(vec![]);
    };
    let mut rows = svc.metric_store.list_samples(def.id).await?;
    let limit = limit.unwrap_or(200).max(0) as usize;
    rows.truncate(limit);
    Ok(rows)
}

/// The per-finding detail rows for one metric run (`run_id` from a sample) —
/// the findings table / test suite-case tree / coverage file-line detail the
/// per-kind Metric detail view drills into (tsk232). Ordered by id.
pub async fn list_metric_findings(
    svc: &Services,
    run_id: i64,
) -> Result<Vec<MetricFinding>, IpcError> {
    Ok(svc.metric_store.list_findings(run_id).await?)
}

/// Roll up a metric's latest per-file samples by a dimension — `"package"`
/// (the file's parent directory) or any per-file `dims_json` key like
/// `"language"` — summed across all streams, largest first. Unknown key →
/// empty (UI-friendly). Backs the Metric Detail **Breakdown** card
/// (tsk328 package / tsk319 language).
pub async fn metric_dimension_rollup(
    svc: &Services,
    metric_key: String,
    dimension: String,
) -> Result<Vec<MetricDimensionRollup>, IpcError> {
    let Some(def) = svc.metric_store.get_definition(&metric_key).await? else {
        return Ok(vec![]);
    };
    Ok(svc
        .metric_store
        .dimension_rollup_for_metric(def.id, None, dimension)
        .await?)
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

/// Scaffold a new project gauge metric: write a starter script + a `metrics:`
/// `key:` entry into `.oxplow/project.yaml`, then reseed. Returns the project-relative
/// script path so the UI can open it (the Catalog "New metric" action, tsk234).
pub async fn scaffold_metric(
    svc: &Services,
    key: String,
    title: Option<String>,
    language: Option<String>,
    glob: Option<String>,
    scope: Option<String>,
) -> Result<String, IpcError> {
    svc.metrics
        .scaffold_metric(&key, title, language, glob, scope)
        .await
        .map_err(IpcError::internal)
}
