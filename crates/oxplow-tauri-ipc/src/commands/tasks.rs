use oxplow_domain::{Task, TaskId, ThreadId};

pub use oxplow_rpc::commands::tasks::{
    CreateTaskRequest, MoveTaskRequest, ReorderTasksRequest, UpdateTaskRequest,
};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_tasks_for_thread(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<Vec<Task>, IpcError> {
    oxplow_rpc::commands::tasks::list_tasks_for_thread(&state, thread_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_task(
    state: tauri::State<'_, AppState>,
    id: TaskId,
) -> Result<Option<Task>, IpcError> {
    oxplow_rpc::commands::tasks::get_task(&state, id).await
}

/// Insert-or-update a Task. The id field acts as the discriminator —
/// `TaskId::placeholder()` (i.e. 0) means "client doesn't know an id
/// yet, allocate one"; any other value means "update this row in
/// place". On the update path we refetch the stored row so any
/// server-side side effects (e.g. `completed_at` flips, sort_index
/// rewrites a future change might add) appear in the returned shape.
#[tauri::command]
#[specta::specta]
pub async fn upsert_task(state: tauri::State<'_, AppState>, item: Task) -> Result<Task, IpcError> {
    oxplow_rpc::commands::tasks::upsert_task(&state, item).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_task(state: tauri::State<'_, AppState>, id: TaskId) -> Result<(), IpcError> {
    oxplow_rpc::commands::tasks::delete_task(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_task(
    state: tauri::State<'_, AppState>,
    req: CreateTaskRequest,
) -> Result<Task, IpcError> {
    oxplow_rpc::commands::tasks::create_task(&state, req).await
}

#[tauri::command]
#[specta::specta]
pub async fn update_task(
    state: tauri::State<'_, AppState>,
    req: UpdateTaskRequest,
) -> Result<Task, IpcError> {
    oxplow_rpc::commands::tasks::update_task(&state, req).await
}

#[tauri::command]
#[specta::specta]
pub async fn reorder_tasks(
    state: tauri::State<'_, AppState>,
    req: ReorderTasksRequest,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::tasks::reorder_tasks(&state, req).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_task_summaries(
    state: tauri::State<'_, AppState>,
    thread_id: Option<ThreadId>,
) -> Result<Vec<Task>, IpcError> {
    oxplow_rpc::commands::tasks::get_task_summaries(&state, thread_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn move_task(
    state: tauri::State<'_, AppState>,
    req: MoveTaskRequest,
) -> Result<Task, IpcError> {
    oxplow_rpc::commands::tasks::move_task(&state, req).await
}
