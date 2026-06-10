//! Thread-scoped notes (the per-thread capture pad backing the
//! Explore-subagent findings flow). Per-task notes were retired
//! — task_effort.summary already records what shipped on a
//! task, so a separate note table for the same purpose was duplicative.

use oxplow_domain::{NoteId, TaskEvent, TaskId, TaskNote, ThreadId};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn add_thread_note(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
    body: String,
    author: String,
) -> Result<TaskNote, IpcError> {
    oxplow_rpc::commands::notes::add_thread_note(&state, thread_id, body, author).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_thread_notes(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<Vec<TaskNote>, IpcError> {
    oxplow_rpc::commands::notes::list_thread_notes(&state, thread_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_work_note(
    state: tauri::State<'_, AppState>,
    id: NoteId,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::notes::delete_work_note(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_task_events(
    state: tauri::State<'_, AppState>,
    item_id: Option<TaskId>,
    thread_id: Option<ThreadId>,
) -> Result<Vec<TaskEvent>, IpcError> {
    oxplow_rpc::commands::notes::list_task_events(&state, item_id, thread_id).await
}
