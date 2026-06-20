//! Cores for the `tasks` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::{CreateTaskInput, OxplowEvent, Services, UpdateTaskChanges};
use oxplow_domain::stores::TaskStore;
use oxplow_domain::{Task, TaskId, ThreadId};

use crate::error::IpcError;

pub async fn get_task(svc: &Services, id: TaskId) -> Result<Option<Task>, IpcError> {
    Ok(svc.task_store.get(id).await?)
}

/// Insert-or-update a Task. The id field acts as the discriminator —
/// `TaskId::placeholder()` (i.e. 0) means "client doesn't know an id
/// yet, allocate one"; any other value means "update this row in
/// place". On the update path we refetch the stored row so any
/// server-side side effects (e.g. `completed_at` flips, sort_index
/// rewrites a future change might add) appear in the returned shape.
pub async fn upsert_task(svc: &Services, item: Task) -> Result<Task, IpcError> {
    let thread_id = item.thread_id;
    let result = if item.id.is_placeholder() {
        let mut new_item = item;
        let id = svc.task_store.insert(&new_item).await?;
        new_item.id = id;
        new_item
    } else {
        let id = item.id;
        svc.task_store.update(&item).await?;
        svc.task_store
            .get(id)
            .await?
            .ok_or_else(IpcError::not_found)?
    };
    svc.events.emit(OxplowEvent::TasksChanged { thread_id });
    Ok(result)
}

pub async fn delete_task(svc: &Services, id: TaskId) -> Result<(), IpcError> {
    let thread_id = svc.task_store.get(id).await?.and_then(|i| i.thread_id);
    svc.task_store.soft_delete(id).await?;
    svc.events.emit(OxplowEvent::TasksChanged { thread_id });
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CreateTaskRequest {
    #[serde(rename = "threadId")]
    pub thread_id: Option<ThreadId>,
    pub input: CreateTaskInput,
}

pub async fn create_task(svc: &Services, req: CreateTaskRequest) -> Result<Task, IpcError> {
    let item = svc.tasks.create(req.thread_id, req.input).await?;
    svc.events.emit(OxplowEvent::TasksChanged {
        thread_id: req.thread_id,
    });
    Ok(item)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UpdateTaskRequest {
    pub id: TaskId,
    pub changes: UpdateTaskChanges,
}

pub async fn update_task(svc: &Services, req: UpdateTaskRequest) -> Result<Task, IpcError> {
    let item = svc.tasks.update(req.id, req.changes).await?;
    svc.events.emit(OxplowEvent::TasksChanged {
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

pub async fn reorder_tasks(svc: &Services, req: ReorderTasksRequest) -> Result<(), IpcError> {
    svc.tasks
        .reorder(req.thread_id.as_ref(), &req.order)
        .await?;
    svc.events.emit(OxplowEvent::TasksChanged {
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

pub async fn move_task(svc: &Services, req: MoveTaskRequest) -> Result<Task, IpcError> {
    let origin_thread_id = svc.task_store.get(req.id).await?.and_then(|i| i.thread_id);
    let item = svc.tasks.move_to(req.id, req.thread_id).await?;
    // Notify both buckets so the renderer refetches the source and
    // destination. When origin == destination it's a noop reorder and
    // a single event is enough.
    svc.events.emit(OxplowEvent::TasksChanged {
        thread_id: origin_thread_id,
    });
    if origin_thread_id != req.thread_id {
        svc.events.emit(OxplowEvent::TasksChanged {
            thread_id: req.thread_id,
        });
    }
    Ok(item)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn get_task_dispatches_for_missing_returns_null() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("get_task", serde_json::json!({"id": "tsk999999"}), &svc)
            .await
            .unwrap();
        assert!(out.is_null());
    }
}
