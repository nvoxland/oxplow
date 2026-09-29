//! `work_item.transition`: the first command on the bus. Moves one task
//! to a status through `TaskService::update`, which owns the effort
//! lifecycle and logs `work_item.transitioned` in its own transaction —
//! so this handler is `BestEffort` (audited after it returns) until the
//! service's core moves inside the bus's transaction.

use std::sync::Arc;

use oxplow_domain::{
    Atomicity, CommandCall, CommandError, CommandSpec, Confirm, Invokers, Lifecycle, TaskId,
    TaskStatus,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Command, Handler, HandlerOutput};
use crate::task_service::{TaskService, UpdateTaskChanges};

pub const NAME: &str = "work_item.transition";

/// The input: which task, and the status to move it to.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemTransitionInput {
    /// The task id (`tsk42`).
    pub id: String,
    pub to: TaskStatus,
}

pub fn spec() -> CommandSpec {
    CommandSpec {
        name: NAME.into(),
        summary:
            "Move a task to a status (ready, in_progress, blocked, done, canceled, archived); \
                  entering or leaving in_progress opens or closes its effort."
                .into(),
        input_schema: serde_json::to_value(schemars::schema_for!(WorkItemTransitionInput))
            .expect("schema serializes"),
        invokers: Invokers::ALL,
        confirm: Confirm::Never,
        undoable: true,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::BestEffort,
    }
}

pub fn command(tasks: TaskService) -> Command {
    let handler = Handler::BestEffort(Arc::new(move |_actor, input| {
        let tasks = tasks.clone();
        Box::pin(async move {
            let input: WorkItemTransitionInput =
                serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                    field: None,
                    message: e.to_string(),
                })?;
            let id: TaskId = input.id.parse().map_err(|e| CommandError::Invalid {
                field: Some("/id".into()),
                message: format!("{e}"),
            })?;
            let before = tasks.load(id).await.map_err(|e| CommandError::Failed {
                message: e.to_string(),
            })?;
            let after = tasks
                .update(
                    id,
                    UpdateTaskChanges {
                        status: Some(input.to),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| CommandError::Failed {
                    message: e.to_string(),
                })?;
            Ok(HandlerOutput {
                result: serde_json::to_value(&after).expect("Task serializes"),
                inverse: Some(CommandCall {
                    name: NAME.into(),
                    input: serde_json::to_value(WorkItemTransitionInput {
                        id: input.id,
                        to: before.status,
                    })
                    .expect("input serializes"),
                }),
                events: Vec::new(),
                after_commit: None,
            })
        })
    }));
    Command::new(spec(), handler).expect("work_item.transition registers")
}
