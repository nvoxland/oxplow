//! Cores for the `effort` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use std::path::Path;

use oxplow_app::Services;
use oxplow_db::{
    EffortAtSnapshot, EffortChangedPaths, EffortFile, TaskEffort, TaskEffortStore as _,
};
use oxplow_domain::{EffortId, TaskId, Timestamp};
use oxplow_fs_watch::WorkspaceFilter;

use crate::error::IpcError;

fn current_filter(svc: &Services) -> WorkspaceFilter {
    let cfg = svc.config.read();
    cfg.as_ref()
        .map(|c| {
            WorkspaceFilter::for_project(
                &svc.layout.project_dir,
                &c.generated.exclude,
                &c.generated.include,
            )
        })
        .unwrap_or_default()
}

pub async fn list_task_efforts(
    svc: &Services,
    item_id: TaskId,
) -> Result<Vec<TaskEffort>, IpcError> {
    Ok(svc.effort_store.list_for_item(item_id).await?)
}

/// Efforts whose span overlaps `[window_start, window_end]` — the time-range
/// overlay the Metrics Explorer draws as effort bands (tsk233).
pub async fn list_efforts_in_window(
    svc: &Services,
    window_start: Timestamp,
    window_end: Timestamp,
) -> Result<Vec<TaskEffort>, IpcError> {
    Ok(svc
        .effort_store
        .list_in_window(window_start, window_end)
        .await?)
}

pub async fn get_effort_files(
    svc: &Services,
    effort_id: EffortId,
) -> Result<Vec<EffortFile>, IpcError> {
    let filter = current_filter(svc);
    let rows = svc.effort_store.list_files(&effort_id).await?;
    Ok(rows
        .into_iter()
        .filter(|f| !filter.ignore(Path::new(&f.path), false))
        .collect())
}

/// One effort by id — its snapshot bracket (`start_snapshot_id` /
/// `end_snapshot_id`), task id, and lifecycle stamps. Lets the diff
/// view resolve an `effortDiffRef(effortId)` into the (start, end)
/// snapshot endpoints it diffs, including after a cold history reopen
/// where only the effort id survives. `null` when the id is unknown.
pub async fn get_effort(
    svc: &Services,
    effort_id: EffortId,
) -> Result<Option<TaskEffort>, IpcError> {
    Ok(svc.effort_store.get_effort(&effort_id).await?)
}

pub async fn list_efforts_at_snapshots(
    svc: &Services,
    snapshot_ids: Vec<i64>,
) -> Result<Vec<EffortAtSnapshot>, IpcError> {
    Ok(svc
        .effort_store
        .list_efforts_at_snapshots(snapshot_ids)
        .await?)
}

/// Every effort whose snapshot window overlaps the half-open range
/// `(range_start, range_end]` — incl. efforts that merely started or
/// ended inside it, contain it, or are still open. Drives the diff
/// view's "other efforts that overlapped this range" roster.
pub async fn list_efforts_overlapping_range(
    svc: &Services,
    range_start: i64,
    range_end: i64,
) -> Result<Vec<TaskEffort>, IpcError> {
    Ok(svc
        .effort_store
        .list_efforts_overlapping_range(range_start, range_end)
        .await?)
}

/// All distinct file paths whose `file_snapshot` rows fall inside
/// this effort's snapshot bracket — the "all changes during this
/// effort" reference list. Returns empty when the effort has no
/// start/end snapshot pin yet. Drives the reference view shown
/// alongside the canonical `task_effort_file` list on
/// `SnapshotDetailPage`.
pub async fn list_changed_paths_for_effort(
    svc: &Services,
    effort_id: EffortId,
) -> Result<EffortChangedPaths, IpcError> {
    let filter = current_filter(svc);
    let split = svc
        .effort_store
        .list_changed_paths_for_effort(&effort_id)
        .await?;
    let keep = |paths: Vec<String>| -> Vec<String> {
        paths
            .into_iter()
            .filter(|p| !filter.ignore(Path::new(p), false))
            .collect()
    };
    Ok(EffortChangedPaths {
        claimed: keep(split.claimed),
        unclaimed: keep(split.unclaimed),
    })
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_task_efforts_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_task_efforts",
            serde_json::json!({"itemId": "tsk999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn get_effort_dispatches_and_returns_null_for_missing() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "get_effort",
            serde_json::json!({"effortId": "eff999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_null(), "missing effort → null, got {out}");
    }
}
