//! Unified metric substrate read commands (epic tsk213).

use oxplow_db::{MetricDefinition, MetricSample};

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
