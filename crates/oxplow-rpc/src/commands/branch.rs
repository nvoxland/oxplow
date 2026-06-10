//! Cores for the `branch` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::Services;
use oxplow_git::{BranchRef, BranchRefKind};

use crate::error::IpcError;

pub async fn list_branches(svc: &Services) -> Result<Vec<BranchRef>, IpcError> {
    Ok(svc.git.list_branches_project().await)
}

pub async fn get_default_branch(svc: &Services) -> Result<Option<String>, IpcError> {
    Ok(svc.git.detect_default_branch().await)
}

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

/// Filter helper for the UI that wants only locals or only remotes.
pub async fn list_local_branches(svc: &Services) -> Result<Vec<BranchRef>, IpcError> {
    let all = svc.git.list_branches_project().await;
    Ok(all
        .into_iter()
        .filter(|b| b.kind == BranchRefKind::Local)
        .collect())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_branches_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("list_branches", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert!(out.is_array(), "expected a JSON array, got {out}");
    }

    #[tokio::test]
    async fn list_local_branches_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("list_local_branches", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert!(out.is_array(), "expected a JSON array, got {out}");
    }
}
