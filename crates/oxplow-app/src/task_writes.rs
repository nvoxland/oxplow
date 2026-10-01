//! Task writes made on someone's behalf (MCP, RPC): oxplow's tasks
//! through the `work_item.*` commands (the one write surface,
//! `.context/commands.md`), run as the actor — so each is audited, logs
//! its events caused by `command.executed`, and opens or closes the
//! effort in the same transaction. The callers speak oxplow's task shape
//! (a status, a priority, a thread); this module builds the commands'
//! `v_work_item`-shaped inputs from it (a status is oxplow's
//! `native_state` — a transition names its canonical state too; thread
//! and priority ride under `native`).
//! (P2.6.3, tsk455, P7.A1.)

use oxplow_domain::refs::build::work_item_ref;
use oxplow_domain::stores::TaskStore as _;
use oxplow_domain::{Actor, CommandError, Task, TaskId, TaskStatus};
use serde_json::json;

use crate::commands::work_item::{self, state_pair, WorkItemCreateInput, WorkItemUpdateInput};
use crate::Services;

/// The `state` / `native_state` pair for moving `id` to `to` (an
/// `archived` task stays done or canceled as it was).
async fn pair_for(
    svc: &Services,
    id: TaskId,
    to: TaskStatus,
) -> Result<(oxplow_domain::work_items::CanonicalState, String), CommandError> {
    let completed = match to {
        TaskStatus::Archived => svc
            .task_store
            .get(id)
            .await?
            .is_some_and(|t| t.completed_at.is_some()),
        _ => false,
    };
    Ok(state_pair(to, completed))
}

fn status_name(status: TaskStatus) -> String {
    state_pair(status, false).1
}

fn task_of(result: serde_json::Value, command: &str) -> Result<Task, CommandError> {
    serde_json::from_value(result).map_err(|e| CommandError::Failed {
        message: format!("{command} result: {e}"),
    })
}

/// Move `id` to `to` as `actor`, then settle the pump so the effort's
/// snapshot pin is in place for whatever the caller reads next.
pub async fn set_status(
    svc: &Services,
    actor: &Actor,
    id: TaskId,
    to: TaskStatus,
) -> Result<Task, CommandError> {
    let (state, native_state) = pair_for(svc, id, to).await?;
    let outcome = svc
        .work_items_client()
        .transition(actor, &work_item_ref(id), state, Some(&native_state))
        .await?;
    svc.tasks.settle_lifecycle().await;
    task_of(outcome.result, work_item::NAME)
}

/// File a task as `actor` (`work_item.create` on oxplow): audited, and
/// filed straight into `in_progress` it opens the effort in the same run
/// — then settles the pump so the effort's start snapshot is pinned.
pub async fn create(
    svc: &Services,
    actor: &Actor,
    thread: Option<oxplow_domain::ThreadId>,
    input: crate::task_service::CreateTaskInput,
) -> Result<Task, CommandError> {
    let moves_status = input.status.is_some_and(|s| s != TaskStatus::Ready);
    let mut native = json!({});
    if let Some(t) = thread {
        native["thread"] = t.to_string().into();
    }
    if let Some(p) = input.priority {
        native["priority"] = serde_json::to_value(p).expect("priority serializes");
    }
    let args = WorkItemCreateInput {
        provider: Some(crate::work_items::PROVIDER.into()),
        title: input.title,
        body: input.description,
        parent_ref: input.parent_id.map(work_item_ref),
        state: None,
        // oxplow's status is its native state (the canonical one follows).
        native_state: input.status.map(status_name),
        native: Some(native),
    };
    let outcome = svc
        .commands
        .run(
            actor,
            work_item::CREATE,
            serde_json::to_value(args).expect("input serializes"),
            false,
        )
        .await?;
    if moves_status {
        svc.tasks.settle_lifecycle().await;
    }
    task_of(outcome.result, work_item::CREATE)
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
    let input = WorkItemUpdateInput {
        item_ref: work_item_ref(id),
        title: changes.title,
        body: changes.description,
        parent_ref: changes
            .parent_id
            .map(|p| p.map(work_item_ref).unwrap_or_default()),
        state: None,
        native_state: changes.status.map(status_name),
        native: changes.priority.map(|p| json!({ "priority": p })),
    };
    let moves_status = input.native_state.is_some();
    let outcome = svc.work_items_client().update(actor, input).await?;
    if moves_status {
        svc.tasks.settle_lifecycle().await;
    }
    task_of(outcome.result, work_item::UPDATE)
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
