//! Cores for the `effort` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use std::path::Path;

use oxplow_app::Services;
use oxplow_db::{Effort, EffortAtSnapshot, EffortFile, EffortStore as _};
use oxplow_domain::{EffortId, Timestamp};
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

/// Efforts whose span overlaps `[window_start, window_end]` — the time-range
/// overlay the Metrics Explorer draws as effort bands (tsk233).
pub async fn list_efforts_in_window(
    svc: &Services,
    window_start: Timestamp,
    window_end: Timestamp,
) -> Result<Vec<Effort>, IpcError> {
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
pub async fn get_effort(svc: &Services, effort_id: EffortId) -> Result<Option<Effort>, IpcError> {
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
) -> Result<Vec<Effort>, IpcError> {
    Ok(svc
        .effort_store
        .list_efforts_overlapping_range(range_start, range_end)
        .await?)
}

#[cfg(test)]
mod tests {
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
