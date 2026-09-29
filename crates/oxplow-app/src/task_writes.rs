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

/// Insert (id 0) or overwrite a task row; its status only through
/// [`set_status`]. A new row is inserted `ready` first.
pub async fn upsert(svc: &Services, actor: &Actor, mut item: Task) -> Result<Task, CommandError> {
    let wanted = item.status;
    let stored = if item.id.is_placeholder() {
        item.status = TaskStatus::Ready;
        item.completed_at = None;
        item.id = svc.task_store.insert(&item).await?;
        TaskStatus::Ready
    } else {
        let current = svc
            .task_store
            .get(item.id)
            .await?
            .ok_or_else(|| CommandError::Failed {
                message: format!("task {} not found", item.id),
            })?;
        item.status = current.status;
        item.completed_at = current.completed_at;
        svc.task_store.update(&item).await?;
        current.status
    };
    if wanted != stored {
        return set_status(svc, actor, item.id, wanted).await;
    }
    svc.task_store
        .get(item.id)
        .await?
        .ok_or_else(|| CommandError::Failed {
            message: format!("task {} not found", item.id),
        })
}
