use oxplow_domain::{StreamId, Thread, ThreadId};

pub use oxplow_rpc::commands::threads::{
    CreateThreadRequest, RenameThreadRequest, ReorderThreadQueueRequest, SelectThreadRequest,
    SetThreadPromptRequest, ThreadState, ThreadWorkState,
};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_threads(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<Vec<Thread>, IpcError> {
    oxplow_rpc::commands::threads::list_threads(&state, stream_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_thread(
    state: tauri::State<'_, AppState>,
    req: CreateThreadRequest,
) -> Result<Thread, IpcError> {
    oxplow_rpc::commands::threads::create_thread(&state, req).await
}

#[tauri::command]
#[specta::specta]
pub async fn rename_thread(
    state: tauri::State<'_, AppState>,
    req: RenameThreadRequest,
) -> Result<Thread, IpcError> {
    oxplow_rpc::commands::threads::rename_thread(&state, req).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_thread_prompt(
    state: tauri::State<'_, AppState>,
    req: SetThreadPromptRequest,
) -> Result<Thread, IpcError> {
    oxplow_rpc::commands::threads::set_thread_prompt(&state, req).await
}

#[tauri::command]
#[specta::specta]
pub async fn promote_thread(
    state: tauri::State<'_, AppState>,
    id: ThreadId,
) -> Result<Thread, IpcError> {
    oxplow_rpc::commands::threads::promote_thread(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn close_thread(
    state: tauri::State<'_, AppState>,
    id: ThreadId,
) -> Result<Thread, IpcError> {
    oxplow_rpc::commands::threads::close_thread(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn reopen_thread(
    state: tauri::State<'_, AppState>,
    id: ThreadId,
) -> Result<Thread, IpcError> {
    oxplow_rpc::commands::threads::reopen_thread(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_closed_threads(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<Vec<Thread>, IpcError> {
    oxplow_rpc::commands::threads::list_closed_threads(&state, stream_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn reorder_thread_queue(
    state: tauri::State<'_, AppState>,
    req: ReorderThreadQueueRequest,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::threads::reorder_thread_queue(&state, req).await
}

/// Aggregate "what threads exist on this stream and what's selected/active".
#[tauri::command]
#[specta::specta]
pub async fn get_thread_state(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<ThreadState, IpcError> {
    oxplow_rpc::commands::threads::get_thread_state(&state, stream_id).await
}

/// Bucketed task view for the Work panel.
#[tauri::command]
#[specta::specta]
pub async fn get_thread_work_state(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<ThreadWorkState, IpcError> {
    oxplow_rpc::commands::threads::get_thread_work_state(&state, thread_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn select_thread(
    state: tauri::State<'_, AppState>,
    req: SelectThreadRequest,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::threads::select_thread(&state, req).await
}
