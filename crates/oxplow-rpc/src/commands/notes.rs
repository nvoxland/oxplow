//! Cores for the `notes` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.
//!
//! Thread-scoped notes (the per-thread capture pad backing the
//! Explore-subagent findings flow). Per-task notes were retired
//! — effort.summary already records what shipped on a
//! task, so a separate note table for the same purpose was duplicative.

use oxplow_app::Services;
use oxplow_domain::stores::TaskNoteStore;
use oxplow_domain::{TaskNote, ThreadId};

use crate::error::IpcError;

pub async fn add_thread_note(
    svc: &Services,
    thread_id: ThreadId,
    body: String,
    author: String,
) -> Result<TaskNote, IpcError> {
    Ok(svc
        .work_note_store
        .add_for_thread(&thread_id, &body, &author)
        .await?)
}

pub async fn list_thread_notes(
    svc: &Services,
    thread_id: ThreadId,
) -> Result<Vec<TaskNote>, IpcError> {
    Ok(svc.work_note_store.list_for_thread(&thread_id).await?)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_thread_notes_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_thread_notes",
            serde_json::json!({"threadId": "thr999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }
}
