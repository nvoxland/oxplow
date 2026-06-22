//! Unified metric substrate read commands (epic tsk213).

use oxplow_app::metrics_service::MetricCatalogEntry;
use oxplow_db::{MetricDefinition, MetricFinding, MetricSample};

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

/// Enable/disable a metric in `oxplow.yaml` (the Catalog toggle).
#[tauri::command]
#[specta::specta]
pub async fn set_metric_enabled(
    state: tauri::State<'_, AppState>,
    key: String,
    enabled: bool,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::metrics::set_metric_enabled(&state, key, enabled).await
}
