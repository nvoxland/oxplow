//! Cores for the `effort` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_domain::SnapshotId;

use oxplow_app::Services;
use oxplow_db::{Effort, EffortAtSnapshot, EffortStore as _};
use oxplow_domain::Timestamp;

use crate::error::IpcError;

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

pub async fn list_efforts_at_snapshots(
    svc: &Services,
    snapshot_ids: Vec<SnapshotId>,
) -> Result<Vec<EffortAtSnapshot>, IpcError> {
    Ok(svc
        .effort_store
        .list_efforts_at_snapshots(snapshot_ids.into_iter().map(SnapshotId::value).collect())
        .await?)
}
