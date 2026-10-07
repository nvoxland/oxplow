//! Cores for the `notes` command module: the read of a thread's notes
//! (writes are `oxplow.knowledge.add_note` / `oxplow.knowledge.update_note`).
//!
//! Thread-scoped notes (the per-thread capture pad backing the
//! Explore-subagent findings flow). Per-task notes were retired
//! — effort.summary already records what shipped on a
//! task, so a separate note table for the same purpose was duplicative.

use oxplow_app::Services;
use oxplow_domain::stores::ThreadNoteStore;
use oxplow_domain::{ThreadId, ThreadNote};

use crate::error::IpcError;

pub async fn list_thread_notes(
    svc: &Services,
    thread_id: ThreadId,
) -> Result<Vec<ThreadNote>, IpcError> {
    Ok(svc.thread_note_store.list_for_thread(&thread_id).await?)
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
