//! Unified metric substrate read commands (epic tsk213).

use oxplow_app::metric_engine::{FactFinding, RollupRow, SeriesPoint};
use oxplow_app::metrics_service::MetricCatalogEntry;
use oxplow_db::MetricSpec;

use crate::error::IpcError;
use crate::state::AppState;

/// The metric catalog — every known metric SPEC. Optional `language` / `scope`
/// filter. Drives the Catalog / Explorer measure picker.
#[tauri::command]
#[specta::specta]
pub async fn list_metric_definitions(
    state: tauri::State<'_, AppState>,
    language: Option<String>,
    scope: Option<String>,
) -> Result<Vec<MetricSpec>, IpcError> {
    oxplow_rpc::commands::metrics::list_metric_definitions(&state, language, scope).await
}

/// Time series for one metric (by spec `key`), newest-first — one point per
/// capture over the metric's source-measure facts. `group_by` slices by a
/// conformed dimension (`subject` / `branch` / `oxplow.model` / …).
#[tauri::command]
#[specta::specta]
pub async fn list_metric_samples(
    state: tauri::State<'_, AppState>,
    metric_key: String,
    limit: Option<i64>,
    group_by: Option<String>,
) -> Result<Vec<SeriesPoint>, IpcError> {
    oxplow_rpc::commands::metrics::list_metric_samples(&state, metric_key, limit, group_by).await
}

/// Roll up a metric (by spec `key`) by a dimension (`"package"` or a `dims_json`
/// key like `"language"`), largest first — the Metric Detail Breakdown card +
/// subject breakdown (tsk328/tsk319).
#[tauri::command]
#[specta::specta]
pub async fn metric_dimension_rollup(
    state: tauri::State<'_, AppState>,
    metric_key: String,
    dimension: String,
) -> Result<Vec<RollupRow>, IpcError> {
    oxplow_rpc::commands::metrics::metric_dimension_rollup(&state, metric_key, dimension).await
}

/// The located items behind one metric (by spec `key`) — the read-time finding
/// view over its filtered facts. `capture_id` scopes to one recording's
/// drill-in (findings table / per-file coverage / per-case tests).
#[tauri::command]
#[specta::specta]
pub async fn list_metric_findings(
    state: tauri::State<'_, AppState>,
    metric_key: String,
    capture_id: Option<i64>,
) -> Result<Vec<FactFinding>, IpcError> {
    oxplow_rpc::commands::metrics::list_metric_findings(&state, metric_key, capture_id).await
}

/// Time series for a MEASURE, aggregated per capture over its atomic facts —
/// the measure-level read (vs `list_metric_samples`'s spec ergonomics).
#[tauri::command]
#[specta::specta]
pub async fn metric_series(
    state: tauri::State<'_, AppState>,
    measure_key: String,
    aggregation: String,
    group_by: Option<String>,
    min_value: Option<f64>,
    severity: Option<String>,
) -> Result<Vec<SeriesPoint>, IpcError> {
    oxplow_rpc::commands::metrics::metric_series(
        &state,
        measure_key,
        aggregation,
        group_by,
        min_value,
        severity,
    )
    .await
}

/// By-dimension rollup for a MEASURE over its atomic facts — the measure-level
/// breakdown (vs `metric_dimension_rollup`'s spec ergonomics).
#[tauri::command]
#[specta::specta]
pub async fn metric_rollup(
    state: tauri::State<'_, AppState>,
    measure_key: String,
    dimension: Option<String>,
) -> Result<Vec<RollupRow>, IpcError> {
    oxplow_rpc::commands::metrics::metric_rollup(&state, measure_key, dimension).await
}

/// The available catalog (built-in ∪ global ∪ project) + enabled flags — the
/// Catalog page's browse read.
#[tauri::command]
#[specta::specta]
pub async fn list_metric_catalog(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<MetricCatalogEntry>, IpcError> {
    oxplow_rpc::commands::metrics::list_metric_catalog(&state).await
}

/// Enable/disable a metric in `.oxplow/project.yaml` (the Catalog toggle).
#[tauri::command]
#[specta::specta]
pub async fn set_metric_enabled(
    state: tauri::State<'_, AppState>,
    key: String,
    enabled: bool,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::metrics::set_metric_enabled(&state, key, enabled).await
}

/// Enable/disable many metrics in one write (the per-section Enable/Disable-all,
/// tsk32).
#[tauri::command]
#[specta::specta]
pub async fn set_metrics_enabled(
    state: tauri::State<'_, AppState>,
    keys: Vec<String>,
    enabled: bool,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::metrics::set_metrics_enabled(&state, keys, enabled).await
}

/// Set a metric's `target` override in `.oxplow/project.yaml` (the Catalog inline edit,
/// tsk233). `trigger` is inherent to the definition, not overridable (tsk290).
#[tauri::command]
#[specta::specta]
pub async fn set_metric_override(
    state: tauri::State<'_, AppState>,
    key: String,
    target: Option<f64>,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::metrics::set_metric_override(&state, key, target).await
}
