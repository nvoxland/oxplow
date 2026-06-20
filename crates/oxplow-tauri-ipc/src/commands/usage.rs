use oxplow_db::{UsageEvent, UsageRollup};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn record_usage(
    state: tauri::State<'_, AppState>,
    kind: String,
    payload_json: String,
) -> Result<UsageEvent, IpcError> {
    oxplow_rpc::commands::usage::record_usage(&state, kind, payload_json).await
}

/// Per-key rollup of recent usage events of a single `kind`. Returns
/// the most-recently-touched keys (file paths, note slugs, task
/// ids, …) along with how many times each has been touched. Drives
/// "recent files" / "recent notes" affordances in the renderer.
#[tauri::command]
#[specta::specta]
pub async fn list_recent_usage_rollup(
    state: tauri::State<'_, AppState>,
    kind: String,
    stream_id: Option<String>,
    limit: u32,
) -> Result<Vec<UsageRollup>, IpcError> {
    oxplow_rpc::commands::usage::list_recent_usage_rollup(&state, kind, stream_id, limit).await
}
