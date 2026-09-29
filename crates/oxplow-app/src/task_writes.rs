//! Task writes made on someone's behalf (MCP, RPC). A status change is
//! always the `work_item.transition` command, run as the actor — so it is
//! audited, logs `work_item.transitioned` caused by `command.executed`,
//! and opens or closes the effort in the same transaction. Other fields
//! are plain row writes. (P2.6.3, tsk455; `.context/commands.md`.)

use oxplow_domain::stores::TaskStore as _;
use oxplow_domain::{Actor, CommandError, Task, TaskId, TaskStatus};

use crate::Services;

/// Move `id` to `to` as `actor`, then settle the pump so the effort's
/// snapshot pin is in place for whatever the caller reads next.
pub async fn set_status(
    svc: &Services,
    actor: &Actor,
    id: TaskId,
    to: TaskStatus,
) -> Result<Task, CommandError> {
    let outcome = svc
        .commands
        .run(
            actor,
            crate::commands::work_item::NAME,
            serde_json::json!({ "id": id.to_string(), "to": to }),
            // Not pre-confirmed: a transition that ever needs a person's
            // confirmation comes back as NEEDS_CONFIRMATION.
            false,
        )
        .await?;
    svc.tasks.settle_lifecycle().await;
    serde_json::from_value(outcome.result).map_err(|e| CommandError::Failed {
        message: format!("work_item.transition result: {e}"),
    })
}

/// File a task as `actor` (`work_item.create`): audited, and filed
/// straight into `in_progress` it opens the effort in the same run —
/// then settles the pump so the effort's start snapshot is pinned.
pub async fn create(
    svc: &Services,
    actor: &Actor,
    thread: Option<oxplow_domain::ThreadId>,
    input: crate::task_service::CreateTaskInput,
) -> Result<Task, CommandError> {
    let moves_status = input.status.is_some_and(|s| s != TaskStatus::Ready);
    let args = crate::commands::work_item::WorkItemCreateInput {
        title: input.title,
        description: input.description,
        parent_id: input.parent_id.map(|p| p.to_string()),
        status: input.status,
        priority: input.priority,
        thread: thread.map(|t| t.to_string()),
    };
    let outcome = svc
        .commands
        .run(
            actor,
            crate::commands::work_item::CREATE,
            serde_json::to_value(args).expect("input serializes"),
            false,
        )
        .await?;
    if moves_status {
        svc.tasks.settle_lifecycle().await;
    }
    serde_json::from_value(outcome.result).map_err(|e| CommandError::Failed {
        message: format!("work_item.create result: {e}"),
    })
}

/// Edit `id`'s fields and move its status, as `actor`, in one audited
/// transaction (`work_item.update`); settles the pump when the status
/// changed so the effort's snapshot pin is in place.
pub async fn update(
    svc: &Services,
    actor: &Actor,
    id: TaskId,
    changes: crate::task_service::UpdateTaskChanges,
) -> Result<Task, CommandError> {
    let input = crate::commands::work_item::WorkItemUpdateInput {
        id: id.to_string(),
        title: changes.title,
        description: changes.description,
        priority: changes.priority,
        parent_id: changes
            .parent_id
            .map(|p| p.map(|p| p.to_string()).unwrap_or_default()),
        status: changes.status,
    };
    let moves_status = input.status.is_some();
    let outcome = svc
        .commands
        .run(
            actor,
            crate::commands::work_item::UPDATE,
            serde_json::to_value(input).expect("input serializes"),
            false,
        )
        .await?;
    if moves_status {
        svc.tasks.settle_lifecycle().await;
    }
    serde_json::from_value(outcome.result).map_err(|e| CommandError::Failed {
        message: format!("work_item.update result: {e}"),
    })
}

/// Insert (id 0) or edit a task. A new row goes through the create path
/// (`insert_logged`: filing straight into a status opens its effort and
/// logs it, in the insert's transaction). An existing row's title,
/// description, priority, parent and status change through
/// `work_item.update`, as `actor`, atomically; its other columns (thread,
/// sort position, authorship) aren't upsert's to change.
pub async fn upsert(svc: &Services, actor: &Actor, item: Task) -> Result<Task, CommandError> {
    if item.id.is_placeholder() {
        let (id, _effort) = svc.task_store.insert_logged(&item).await?;
        if item.status != TaskStatus::Ready {
            svc.tasks.settle_lifecycle().await;
        }
        return svc
            .task_store
            .get(id)
            .await?
            .ok_or_else(|| CommandError::Failed {
                message: format!("task {id} not found"),
            });
    }
    update(
        svc,
        actor,
        item.id,
        crate::task_service::UpdateTaskChanges {
            title: Some(item.title),
            description: Some(item.description),
            parent_id: Some(item.parent_id),
            status: Some(item.status),
            priority: Some(item.priority),
        },
    )
    .await
}
