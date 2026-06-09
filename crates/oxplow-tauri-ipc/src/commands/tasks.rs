use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::{CreateTaskInput, OxplowEvent, UpdateTaskChanges};
use oxplow_domain::stores::TaskStore;
use oxplow_domain::{Task, TaskId, ThreadId};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_tasks_for_thread(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<Vec<Task>, IpcError> {
    Ok(state.task_store.list_for_thread(&thread_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_task(
    state: tauri::State<'_, AppState>,
    id: TaskId,
) -> Result<Option<Task>, IpcError> {
    Ok(state.task_store.get(id).await?)
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
    let thread_id = item.thread_id;
    let result = if item.id.is_placeholder() {
        let mut new_item = item;
        let id = state.task_store.insert(&new_item).await?;
        new_item.id = id;
        new_item
    } else {
        let id = item.id;
        state.task_store.update(&item).await?;
        state
            .task_store
            .get(id)
            .await?
            .ok_or_else(IpcError::not_found)?
    };
    state.events.emit(OxplowEvent::TasksChanged { thread_id });
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_task(state: tauri::State<'_, AppState>, id: TaskId) -> Result<(), IpcError> {
    let thread_id = state.task_store.get(id).await?.and_then(|i| i.thread_id);
    state.task_store.soft_delete(id).await?;
    state.events.emit(OxplowEvent::TasksChanged { thread_id });
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CreateTaskRequest {
    #[serde(rename = "threadId")]
    pub thread_id: Option<ThreadId>,
    pub input: CreateTaskInput,
}

#[tauri::command]
#[specta::specta]
pub async fn create_task(
    state: tauri::State<'_, AppState>,
    req: CreateTaskRequest,
) -> Result<Task, IpcError> {
    let item = state.tasks.create(req.thread_id, req.input).await?;
    state.events.emit(OxplowEvent::TasksChanged {
        thread_id: req.thread_id,
    });
    Ok(item)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UpdateTaskRequest {
    pub id: TaskId,
    pub changes: UpdateTaskChanges,
}

#[tauri::command]
#[specta::specta]
pub async fn update_task(
    state: tauri::State<'_, AppState>,
    req: UpdateTaskRequest,
) -> Result<Task, IpcError> {
    let item = state.tasks.update(req.id, req.changes).await?;
    state.events.emit(OxplowEvent::TasksChanged {
        thread_id: item.thread_id,
    });
    Ok(item)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReorderTasksRequest {
    #[serde(rename = "threadId")]
    pub thread_id: Option<ThreadId>,
    pub order: Vec<TaskId>,
}

#[tauri::command]
#[specta::specta]
pub async fn reorder_tasks(
    state: tauri::State<'_, AppState>,
    req: ReorderTasksRequest,
) -> Result<(), IpcError> {
    state
        .tasks
        .reorder(req.thread_id.as_ref(), &req.order)
        .await?;
    state.events.emit(OxplowEvent::TasksChanged {
        thread_id: req.thread_id,
    });
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MoveTaskRequest {
    pub id: TaskId,
    /// Destination thread, or `None` to move onto the backlog.
    #[serde(rename = "threadId")]
    pub thread_id: Option<ThreadId>,
}

#[tauri::command]
#[specta::specta]
pub async fn get_task_summaries(
    state: tauri::State<'_, AppState>,
    thread_id: Option<ThreadId>,
) -> Result<Vec<Task>, IpcError> {
    Ok(match thread_id {
        Some(t) => state.task_store.list_for_thread(&t).await?,
        None => state.task_store.list_backlog().await?,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn move_task(
    state: tauri::State<'_, AppState>,
    req: MoveTaskRequest,
) -> Result<Task, IpcError> {
    let origin_thread_id = state
        .task_store
        .get(req.id)
        .await?
        .and_then(|i| i.thread_id);
    let item = state.tasks.move_to(req.id, req.thread_id).await?;
    // Notify both buckets so the renderer refetches the source and
    // destination. When origin == destination it's a noop reorder and
    // a single event is enough.
    state.events.emit(OxplowEvent::TasksChanged {
        thread_id: origin_thread_id,
    });
    if origin_thread_id != req.thread_id {
        state.events.emit(OxplowEvent::TasksChanged {
            thread_id: req.thread_id,
        });
    }
    Ok(item)
}
