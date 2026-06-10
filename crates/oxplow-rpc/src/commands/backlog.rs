//! Cores for the `backlog` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::{BacklogState, Services};
use oxplow_domain::stores::TaskStore;
use oxplow_domain::Task;

use crate::error::IpcError;

pub async fn list_backlog(svc: &Services) -> Result<Vec<Task>, IpcError> {
    Ok(svc.task_store.list_backlog().await?)
}

/// Bucketed backlog view: ready/blocked/in_progress/done.
pub async fn get_backlog_state(svc: &Services) -> Result<BacklogState, IpcError> {
    let rows = svc.tasks.list_backlog().await?;
    Ok(BacklogState::from_rows(rows))
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_backlog_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("list_backlog", serde_json::json!(null), &svc)
            .await
            .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn get_backlog_state_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("get_backlog_state", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert!(out.is_object());
    }
}
