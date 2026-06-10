//! Cores for the `effort` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use std::path::Path;

use oxplow_app::Services;
use oxplow_db::{
    EffortAtSnapshot, EffortFile, EffortObservation, TaskEffort, TaskEffortStore as _,
};
use oxplow_domain::{EffortId, TaskId};
use oxplow_fs_watch::WorkspaceFilter;

use crate::error::IpcError;

fn current_filter(svc: &Services) -> WorkspaceFilter {
    let cfg = svc.config.read();
    cfg.as_ref()
        .map(|c| WorkspaceFilter::with_user_entries(&c.generated))
        .unwrap_or_default()
}

pub async fn list_task_efforts(
    svc: &Services,
    item_id: TaskId,
) -> Result<Vec<TaskEffort>, IpcError> {
    Ok(svc.effort_store.list_for_item(item_id).await?)
}

pub async fn get_effort_files(
    svc: &Services,
    effort_id: EffortId,
) -> Result<Vec<EffortFile>, IpcError> {
    let filter = current_filter(svc);
    let rows = svc.effort_store.list_files(&effort_id).await?;
    Ok(rows
        .into_iter()
        .filter(|f| !filter.ignore(Path::new(&f.path)))
        .collect())
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

/// All distinct file paths whose `file_snapshot` rows fall inside
/// this effort's snapshot bracket — the "all changes during this
/// effort" reference list. Returns empty when the effort has no
/// start/end snapshot pin yet. Drives the reference view shown
/// alongside the canonical `task_effort_file` list on
/// `SnapshotDetailPage`.
pub async fn list_changed_paths_for_effort(
    svc: &Services,
    effort_id: EffortId,
) -> Result<Vec<String>, IpcError> {
    let filter = current_filter(svc);
    let paths = svc
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
pub async fn list_effort_observations(
    svc: &Services,
    effort_id: EffortId,
    kind: Option<String>,
) -> Result<Vec<EffortObservation>, IpcError> {
    Ok(svc
        .observation_store
        .list_for_effort(&effort_id.to_string(), kind.as_deref())
        .await?)
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
    async fn list_effort_observations_dispatches_with_optional_kind() {
        let (svc, _dir) = crate::test_support::services();
        // `kind` omitted → None.
        let out = crate::dispatch(
            "list_effort_observations",
            serde_json::json!({"effortId": "eff999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }
}
