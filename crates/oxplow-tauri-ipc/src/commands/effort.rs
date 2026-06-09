//! task effort tracking commands.

use std::path::Path;

use oxplow_db::{
    EffortAtSnapshot, EffortFile, EffortObservation, TaskEffort, TaskEffortStore as _,
};
use oxplow_domain::{EffortId, TaskId};
use oxplow_fs_watch::WorkspaceFilter;

use crate::error::IpcError;
use crate::state::AppState;

fn current_filter(state: &AppState) -> WorkspaceFilter {
    let cfg = state.config.read();
    cfg.as_ref()
        .map(|c| WorkspaceFilter::with_user_entries(&c.generated))
        .unwrap_or_default()
}

#[tauri::command]
#[specta::specta]
pub async fn list_task_efforts(
    state: tauri::State<'_, AppState>,
    item_id: TaskId,
) -> Result<Vec<TaskEffort>, IpcError> {
    Ok(state.effort_store.list_for_item(item_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_effort_files(
    state: tauri::State<'_, AppState>,
    effort_id: EffortId,
) -> Result<Vec<EffortFile>, IpcError> {
    let filter = current_filter(&state);
    let rows = state.effort_store.list_files(&effort_id).await?;
    Ok(rows
        .into_iter()
        .filter(|f| !filter.ignore(Path::new(&f.path)))
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn list_efforts_at_snapshots(
    state: tauri::State<'_, AppState>,
    snapshot_ids: Vec<i64>,
) -> Result<Vec<EffortAtSnapshot>, IpcError> {
    Ok(state
        .effort_store
        .list_efforts_at_snapshots(snapshot_ids)
        .await?)
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
) -> Result<Vec<String>, IpcError> {
    let filter = current_filter(&state);
    let paths = state
        .effort_store
        .list_changed_paths_for_effort(&effort_id)
        .await?;
    Ok(paths
        .into_iter()
        .filter(|p| !filter.ignore(Path::new(p)))
        .collect())
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
    Ok(state
        .observation_store
        .list_for_effort(&effort_id.to_string(), kind.as_deref())
        .await?)
}
