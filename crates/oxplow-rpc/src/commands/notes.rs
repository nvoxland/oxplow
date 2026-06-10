//! Cores for the `notes` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.
//!
//! Thread-scoped notes (the per-thread capture pad backing the
//! Explore-subagent findings flow). Per-task notes were retired
//! — task_effort.summary already records what shipped on a
//! task, so a separate note table for the same purpose was duplicative.

use oxplow_app::Services;
use oxplow_domain::stores::{TaskEventStore, TaskNoteStore};
use oxplow_domain::{NoteId, TaskEvent, TaskId, TaskNote, ThreadId};

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

pub async fn delete_work_note(svc: &Services, id: NoteId) -> Result<(), IpcError> {
    Ok(svc.work_note_store.delete(&id).await?)
}

pub async fn list_task_events(
    svc: &Services,
    item_id: Option<TaskId>,
    thread_id: Option<ThreadId>,
) -> Result<Vec<TaskEvent>, IpcError> {
    match (item_id, thread_id) {
        (Some(i), _) => Ok(svc.task_event_store.list_for_item(i).await?),
        (None, Some(t)) => Ok(svc.task_event_store.list_for_thread(&t).await?),
        (None, None) => Ok(vec![]),
    }
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

    #[tokio::test]
    async fn list_task_events_dispatches_with_optional_args() {
        let (svc, _dir) = crate::test_support::services();
        // Both optional args omitted → core returns an empty list.
        let out = crate::dispatch("list_task_events", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(out, serde_json::json!([]));
    }
}
