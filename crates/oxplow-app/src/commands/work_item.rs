//! `work_item.transition`: move a task to a status, as a `Tx` command —
//! the row, the effort open/close it implies, `work_item.transitioned`
//! and the effort's own events commit in the bus's transaction with the
//! audit row, all caused by the run's `command.executed` and carrying the
//! actor's source (`oxplow_db::task_store::set_status_tx`). The effort's
//! snapshot pin is the effort-lifecycle pump consumer's; callers that need
//! it settle the pump (`TaskService::settle_lifecycle`).

use oxplow_domain::{
    Atomicity, CommandCall, CommandError, CommandSpec, Confirm, Invokers, Lifecycle, TaskId,
    TaskStatus, Timestamp,
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
