//! task effort tracking commands.

use oxplow_db::{
    AgentKindTokenUsage, AgentNudge, AgentTokenUsage, EffortAtSnapshot, EffortChangedPaths,
    EffortFile, EffortObservation, ModelTokenUsage, TaskEffort, TokenUsageByDay, TokenUsageTotals,
};
use oxplow_domain::{EffortId, TaskId, ThreadId};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_task_efforts(
    state: tauri::State<'_, AppState>,
    item_id: TaskId,
) -> Result<Vec<TaskEffort>, IpcError> {
    oxplow_rpc::commands::effort::list_task_efforts(&state, item_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_effort_files(
    state: tauri::State<'_, AppState>,
    effort_id: EffortId,
) -> Result<Vec<EffortFile>, IpcError> {
    oxplow_rpc::commands::effort::get_effort_files(&state, effort_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_efforts_at_snapshots(
    state: tauri::State<'_, AppState>,
    snapshot_ids: Vec<i64>,
) -> Result<Vec<EffortAtSnapshot>, IpcError> {
    oxplow_rpc::commands::effort::list_efforts_at_snapshots(&state, snapshot_ids).await
}

/// All distinct file paths whose `file_snapshot` rows fall inside
/// this effort's snapshot bracket — the "all changes during this
/// effort" reference list. Returns empty when the effort has no
/// start/end snapshot pin yet. Drives the reference view shown
/// alongside the canonical `task_effort_file` list on
/// `SnapshotDetailPage`.
#[tauri::command]
#[specta::specta]
pub async fn list_changed_paths_for_effort(
    state: tauri::State<'_, AppState>,
    effort_id: EffortId,
) -> Result<EffortChangedPaths, IpcError> {
    oxplow_rpc::commands::effort::list_changed_paths_for_effort(&state, effort_id).await
}

/// Collection observations (test-run / diff-coverage) for an effort,
/// newest-first. Optional `kind` filter. Drives the effort-review
/// coverage badge + tests-run list on `TaskPage`.
#[tauri::command]
#[specta::specta]
pub async fn list_effort_observations(
    state: tauri::State<'_, AppState>,
    effort_id: EffortId,
    kind: Option<String>,
) -> Result<Vec<EffortObservation>, IpcError> {
    oxplow_rpc::commands::effort::list_effort_observations(&state, effort_id, kind).await
}

/// Persisted agent nudges (report-less-run / commit-hygiene) for an effort,
/// newest-first. Drives the collapsed "Agent nudges" debug sub-view on
/// `TaskPage`.
#[tauri::command]
#[specta::specta]
pub async fn list_nudges_for_effort(
    state: tauri::State<'_, AppState>,
    effort_id: EffortId,
) -> Result<Vec<AgentNudge>, IpcError> {
    oxplow_rpc::commands::effort::list_nudges_for_effort(&state, effort_id).await
}

/// Per-turn agent token-usage rows for an effort, newest-first (tsk104).
#[tauri::command]
#[specta::specta]
pub async fn list_token_usage_for_effort(
    state: tauri::State<'_, AppState>,
    effort_id: EffortId,
) -> Result<Vec<AgentTokenUsage>, IpcError> {
    oxplow_rpc::commands::effort::list_token_usage_for_effort(&state, effort_id).await
}

/// Summed token totals for one effort.
#[tauri::command]
#[specta::specta]
pub async fn get_effort_token_totals(
    state: tauri::State<'_, AppState>,
    effort_id: EffortId,
) -> Result<TokenUsageTotals, IpcError> {
    oxplow_rpc::commands::effort::get_effort_token_totals(&state, effort_id).await
}

/// Summed token totals for a whole thread (Work panel running total).
#[tauri::command]
#[specta::specta]
pub async fn get_thread_token_totals(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<TokenUsageTotals, IpcError> {
    oxplow_rpc::commands::effort::get_thread_token_totals(&state, thread_id).await
}

/// Summed token totals across every recorded turn (Token Analytics page).
#[tauri::command]
#[specta::specta]
pub async fn token_totals_overall(
    state: tauri::State<'_, AppState>,
) -> Result<TokenUsageTotals, IpcError> {
    oxplow_rpc::commands::effort::token_totals_overall(&state).await
}

/// Token totals grouped by agent/harness, busiest first.
#[tauri::command]
#[specta::specta]
pub async fn token_usage_by_agent(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AgentKindTokenUsage>, IpcError> {
    oxplow_rpc::commands::effort::token_usage_by_agent(&state).await
}

/// Token totals grouped by (agent_kind, model), busiest first.
#[tauri::command]
#[specta::specta]
pub async fn token_usage_by_model(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ModelTokenUsage>, IpcError> {
    oxplow_rpc::commands::effort::token_usage_by_model(&state).await
}

/// Token volume bucketed by day over the last `days` days (trend chart).
#[tauri::command]
#[specta::specta]
pub async fn token_usage_by_day(
    state: tauri::State<'_, AppState>,
    days: u32,
) -> Result<Vec<TokenUsageByDay>, IpcError> {
    oxplow_rpc::commands::effort::token_usage_by_day(&state, days).await
}
