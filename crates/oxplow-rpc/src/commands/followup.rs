//! Cores for the `followup` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::{Followup, Services};
use oxplow_domain::ThreadId;

use crate::error::IpcError;

pub async fn list_followups(
    svc: &Services,
    thread_id: ThreadId,
) -> Result<Vec<Followup>, IpcError> {
    Ok(svc.followups.list_for_thread(&thread_id))
}

pub async fn add_followup(
    svc: &Services,
    thread_id: ThreadId,
    body: String,
) -> Result<Followup, IpcError> {
    Ok(svc.followups.add(thread_id, body))
}

pub async fn remove_followup(svc: &Services, id: String) -> Result<(), IpcError> {
    svc.followups.remove(&id);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_followups_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_followups",
            serde_json::json!({"threadId": "thr999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn add_followup_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "add_followup",
            serde_json::json!({"threadId": "thr1", "body": "check this later"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_object());
    }
}
