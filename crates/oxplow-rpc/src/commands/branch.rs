//! Cores for the `branch` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use crate::error::IpcError;
use oxplow_app::Services;

pub async fn rename_branch(svc: &Services, from: String, to: String) -> Result<(), IpcError> {
    svc.git
        .rename_branch(from, to)
        .await
        .map_err(|e| IpcError::invalid(e.to_string()))
}

pub async fn delete_branch(svc: &Services, branch: String, force: bool) -> Result<(), IpcError> {
    svc.git
        .delete_branch(branch, force)
        .await
        .map_err(|e| IpcError::invalid(e.to_string()))
}
