//! `work_item.transition`: move a task to a status, as a `Tx` command —
//! the row, the effort open/close it implies, `work_item.transitioned`
//! and the effort's own events commit in the bus's transaction with the
//! audit row, all caused by the run's `command.executed` and carrying the
//! actor's source (`oxplow_db::task_store::set_status_tx`). The effort's
//! snapshot pin is the effort-lifecycle pump consumer's; callers that need
//! it settle the pump (`TaskService::settle_lifecycle`).

use oxplow_domain::{
    Atomicity, CommandCall, CommandError, CommandSpec, Confirm, Invokers, Lifecycle, TaskId,
    TaskPriority, TaskStatus, Timestamp,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::{Command, Handler, HandlerOutput, TxCtx};

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
        atomicity: Atomicity::Tx,
        effect: oxplow_domain::CommandEffect::Write,
    }
}

/// The handler is pure: it runs inside a transaction the bus may retry.
pub fn command() -> Command {
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: WorkItemTransitionInput =
            serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                field: None,
                message: e.to_string(),
            })?;
        let id: TaskId = input.id.parse().map_err(|e| CommandError::Invalid {
            field: Some("/id".into()),
            message: format!("{e}"),
        })?;
        let change = oxplow_db::task_store::set_status_tx(
            ctx.conn,
            &ctx.events,
            id,
            input.to,
            Timestamp::now(),
        )
        .map_err(|e| match e {
            oxplow_domain::DomainError::NotFound => CommandError::Failed {
                message: format!("task {id} not found"),
            },
            other => CommandError::from(other),
        })?;
        Ok(HandlerOutput {
            result: serde_json::to_value(&change.after).expect("Task serializes"),
            inverse: Some(CommandCall {
                name: NAME.into(),
                input: serde_json::to_value(WorkItemTransitionInput {
                    id: input.id,
                    to: change.before.status,
                })
                .expect("input serializes"),
            }),
            events: Vec::new(),
            after_commit: None,
        })
    }));
    Command::new(spec(), handler).expect("work_item.transition registers")
}

pub const UPDATE: &str = "work_item.update";

/// Edit a task's fields and, optionally, its status — one transaction.
/// Absent fields are left alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemUpdateInput {
    /// The task id (`tsk42`).
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<TaskPriority>,
    /// The parent task (`tsk7`), or `""` to detach.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
}

pub fn update_spec() -> CommandSpec {
    CommandSpec {
        name: UPDATE.into(),
        summary: "Edit a task's title, description, priority or parent and, optionally, move \
                  it to a status — all in one transaction (see work_item.transition for the \
                  status's effects)."
            .into(),
        input_schema: serde_json::to_value(schemars::schema_for!(WorkItemUpdateInput))
            .expect("schema serializes"),
        invokers: Invokers::ALL,
        confirm: Confirm::Never,
        undoable: true,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        effect: oxplow_domain::CommandEffect::Write,
    }
}

/// The fields and status change as one `Tx` run over
/// `oxplow_db::task_store::update_with_status_tx`: `work_item.edited` for
/// the fields, then the status move with everything it implies, all
/// caused by the run. The inverse restores exactly what was given.
pub fn update_command() -> Command {
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: WorkItemUpdateInput =
            serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                field: None,
                message: e.to_string(),
            })?;
        let id: TaskId = input.id.parse().map_err(|e| CommandError::Invalid {
            field: Some("/id".into()),
            message: format!("{e}"),
        })?;
        let parent = match input.parent_id.as_deref() {
            None => None,
            Some("") => Some(None),
            Some(raw) => Some(Some(raw.parse::<TaskId>().map_err(|e| {
                CommandError::Invalid {
                    field: Some("/parent_id".into()),
                    message: format!("{e}"),
                }
            })?)),
        };
        let not_found = |e: oxplow_domain::DomainError| match e {
            oxplow_domain::DomainError::NotFound => CommandError::Failed {
                message: format!("task {id} not found"),
            },
            other => CommandError::from(other),
        };
        let before = oxplow_db::task_store::get_task_tx(ctx.conn, id)
            .map_err(not_found)?
            .ok_or_else(|| not_found(oxplow_domain::DomainError::NotFound))?;
        let now = Timestamp::now();
        let mut item = before.clone();
        if let Some(t) = &input.title {
            item.title = t.clone();
        }
        if let Some(d) = &input.description {
            item.description = d.clone();
        }
        if let Some(p) = input.priority {
            item.priority = p;
        }
        if let Some(p) = parent {
            item.parent_id = p;
        }
        item.updated_at = now;
        let after = oxplow_db::task_store::update_with_status_tx(
            ctx.conn,
            &ctx.events,
            &item,
            input.status,
            now,
        )
        .map_err(not_found)?;
        let inverse = WorkItemUpdateInput {
            id: input.id.clone(),
            title: input.title.as_ref().map(|_| before.title.clone()),
            description: input
                .description
                .as_ref()
                .map(|_| before.description.clone()),
            priority: input.priority.map(|_| before.priority),
            parent_id: input
                .parent_id
                .as_ref()
                .map(|_| before.parent_id.map(|p| p.to_string()).unwrap_or_default()),
            status: input.status.map(|_| before.status),
        };
        Ok(HandlerOutput {
            result: serde_json::to_value(&after).expect("Task serializes"),
            inverse: Some(CommandCall {
                name: UPDATE.into(),
                input: serde_json::to_value(inverse).expect("input serializes"),
            }),
            events: Vec::new(),
            after_commit: None,
        })
    }));
    Command::new(update_spec(), handler).expect("work_item.update registers")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::EffortStore as _;
    use oxplow_domain::{Actor, StreamId};
    use serde_json::json;

    /// P2.6.3 (tsk455): a transition is one transaction with its audit —
    /// the status, the effort close, `work_item.transitioned` and
    /// `effort.closed` all carry the actor's source and are caused by the
    /// run's `command.executed`.
    #[tokio::test]
    async fn a_transition_commits_with_its_audit_and_names_its_cause() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        };
        assert_eq!(spec().atomicity, Atomicity::Tx);
        let outcome = fx
            .svc
            .commands
            .run(
                &agent,
                NAME,
                json!({ "id": fx.task.to_string(), "to": "done" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(outcome.result["status"], "done");
        let executed = outcome.event_id.expect("a write is recorded");

        let events = fx.svc.event_log_store.read_after(0, 50).await.unwrap();
        let caused: Vec<(&str, &str)> = events
            .iter()
            .filter(|e| e.envelope.cause.as_ref() == Some(&executed))
            .map(|e| (e.envelope.event_type.as_str(), e.envelope.source.as_str()))
            .collect();
        let source = format!("agent:{}", fx.thread);
        assert_eq!(
            caused,
            vec![
                ("effort.closed", source.as_str()),
                ("work_item.transitioned", source.as_str()),
            ]
        );
        assert!(events
            .iter()
            .any(|e| e.envelope.id == executed && e.envelope.event_type == "command.executed"));
        let effort = fx
            .svc
            .effort_store
            .get_effort(&fx.effort)
            .await
            .unwrap()
            .unwrap();
        assert!(effort.ended_at.is_some());
    }

    /// `work_item.update`: fields and status commit together, audited and
    /// undoable — an undo restores both.
    #[tokio::test]
    async fn an_update_edits_fields_and_status_atomically_and_undoes() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let bus = &fx.svc.commands;
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        };
        let out = bus
            .run(
                &agent,
                UPDATE,
                json!({ "id": fx.task.to_string(), "title": "renamed", "status": "blocked" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["title"], "renamed");
        assert_eq!(out.result["status"], "blocked");
        let executed = out.event_id.clone().unwrap();
        let events = fx.svc.event_log_store.read_after(0, 100).await.unwrap();
        let caused: Vec<&str> = events
            .iter()
            .filter(|e| e.envelope.cause.as_ref() == Some(&executed))
            .map(|e| e.envelope.event_type.as_str())
            .collect();
        assert_eq!(
            caused,
            vec![
                "work_item.edited",
                "effort.closed",
                "work_item.transitioned"
            ]
        );

        bus.undo(&agent, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        use oxplow_domain::stores::TaskStore as _;
        let back = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(back.title, "t");
        assert_eq!(back.status, oxplow_domain::TaskStatus::InProgress);
    }

    /// A refused run writes nothing — not the fields either.
    #[tokio::test]
    async fn a_denied_update_changes_nothing() {
        let fx = crate::test_fixtures::services_with_effort().await;
        fx.svc
            .db
            .transaction(|c| {
                c.execute_batch(
                    "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                       VALUES (9, 1, 'q', 'queued',
                               '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z');",
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        let queued = Actor::Agent {
            thread_id: Some(oxplow_domain::ThreadId::new(9)),
            stream_id: Some(StreamId::new(1)),
        };
        let err = fx
            .svc
            .commands
            .run(
                &queued,
                UPDATE,
                json!({ "id": fx.task.to_string(), "title": "renamed", "status": "done" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        use oxplow_domain::stores::TaskStore as _;
        let row = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(
            (row.title.as_str(), row.status),
            ("t", oxplow_domain::TaskStatus::InProgress)
        );
    }

    /// An unknown task fails the run and writes nothing but the error audit.
    #[tokio::test]
    async fn a_failed_transition_logs_nothing() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let before = fx
            .svc
            .event_log_store
            .read_after(0, 50)
            .await
            .unwrap()
            .len();
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                NAME,
                json!({ "id": "tsk999", "to": "done" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
        let after = fx
            .svc
            .event_log_store
            .read_after(0, 50)
            .await
            .unwrap()
            .len();
        assert_eq!(before, after);
    }
}
