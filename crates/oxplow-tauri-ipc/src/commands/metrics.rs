//! Unified metric substrate read commands (epic tsk213).

use oxplow_app::metrics_service::MetricCatalogEntry;
use oxplow_db::{MetricDefinition, MetricDimensionRollup, MetricFinding, MetricSample};

use crate::error::IpcError;
use crate::state::AppState;

/// The metric catalog — every known definition. Optional `language` / `scope`
/// filter. Drives the Catalog / Explorer measure picker.
#[tauri::command]
#[specta::specta]
pub async fn list_metric_definitions(
    state: tauri::State<'_, AppState>,
    language: Option<String>,
    scope: Option<String>,
) -> Result<Vec<MetricDefinition>, IpcError> {
    oxplow_rpc::commands::metrics::list_metric_definitions(&state, language, scope).await
}

/// Durable samples for one metric (by definition `key`), newest-first.
#[tauri::command]
#[specta::specta]
pub async fn list_metric_samples(
    state: tauri::State<'_, AppState>,
    metric_key: String,
    limit: Option<i64>,
) -> Result<Vec<MetricSample>, IpcError> {
    oxplow_rpc::commands::metrics::list_metric_samples(&state, metric_key, limit).await
}

/// Roll up a metric's per-file samples by a dimension (`"package"` or a
/// `dims_json` key like `"language"`), largest first — the Metric Detail
/// Breakdown card (tsk328/tsk319).
#[tauri::command]
#[specta::specta]
pub async fn metric_dimension_rollup(
    state: tauri::State<'_, AppState>,
    metric_key: String,
    dimension: String,
) -> Result<Vec<MetricDimensionRollup>, IpcError> {
    oxplow_rpc::commands::metrics::metric_dimension_rollup(&state, metric_key, dimension).await
}

/// Per-finding detail rows for one metric run — the per-kind Metric detail
/// drill-in (findings table / test tree / coverage heat).
#[tauri::command]
#[specta::specta]
pub async fn list_metric_findings(
    state: tauri::State<'_, AppState>,
    run_id: i64,
) -> Result<Vec<MetricFinding>, IpcError> {
    oxplow_rpc::commands::metrics::list_metric_findings(&state, run_id).await
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

/// Scaffold a new project gauge metric (script + `metrics:` entry); returns the
/// project-relative script path. The Catalog "New metric" action (tsk234).
#[tauri::command]
#[specta::specta]
pub async fn scaffold_metric(
    state: tauri::State<'_, AppState>,
    key: String,
    title: Option<String>,
    language: Option<String>,
    glob: Option<String>,
    scope: Option<String>,
) -> Result<String, IpcError> {
    oxplow_rpc::commands::metrics::scaffold_metric(&state, key, title, language, glob, scope).await
}
