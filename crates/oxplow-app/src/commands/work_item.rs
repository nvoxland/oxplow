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

use oxplow_db::task_store::EffortTransition;

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
        effect: oxplow_domain::CommandEffect::Record,
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
        ctx.claim(
            matches!(change.effort, EffortTransition::Opened(_)),
            &format!("moving {id} to in_progress"),
        )?;
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

pub const CREATE: &str = "work_item.create";

/// File a task on a thread (or the backlog), optionally straight into a
/// status.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemCreateInput {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The parent task (`tsk7`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// `ready` when absent; `in_progress` opens the effort in the same run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<TaskPriority>,
    /// The thread (`thr3`); absent files onto the project-wide backlog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

pub fn create_spec() -> CommandSpec {
    CommandSpec {
        name: CREATE.into(),
        summary: "File a task on a thread or the backlog, optionally straight into a status \
                  (in_progress opens its effort in the same transaction)."
            .into(),
        input_schema: serde_json::to_value(schemars::schema_for!(WorkItemCreateInput))
            .expect("schema serializes"),
        invokers: Invokers::ALL,
        confirm: Confirm::Never,
        // Undoing a filing would be deleting a task — not what undo is for.
        undoable: false,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        effect: oxplow_domain::CommandEffect::Record,
    }
}

/// `oxplow_db::task_store::insert_logged_tx` as a `Tx` run: the row (at
/// the end of its list), `work_item.created`, and the effort when filed
/// `in_progress`, caused by the run. An agent's task is authored `agent`.
pub fn create_command() -> Command {
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: WorkItemCreateInput =
            serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                field: None,
                message: e.to_string(),
            })?;
        let parent_id = input
            .parent_id
            .as_deref()
            .map(|raw| {
                raw.parse::<TaskId>().map_err(|e| CommandError::Invalid {
                    field: Some("/parent_id".into()),
                    message: format!("{e}"),
                })
            })
            .transpose()?;
        let thread = input
            .thread
            .as_deref()
            .map(|raw| {
                raw.parse::<oxplow_domain::ThreadId>()
                    .map_err(|e| CommandError::Invalid {
                        field: Some("/thread".into()),
                        message: format!("{e}"),
                    })
            })
            .transpose()?;
        let now = Timestamp::now();
        let status = input.status.unwrap_or(TaskStatus::Ready);
        let item = oxplow_domain::Task {
            id: TaskId::placeholder(),
            thread_id: thread,
            parent_id,
            title: input.title,
            description: input.description.unwrap_or_default(),
            status,
            priority: input.priority.unwrap_or(TaskPriority::Medium),
            sort_index: oxplow_db::task_store::next_sort_index_tx(ctx.conn, thread)
                .map_err(CommandError::from)?,
            created_by: oxplow_domain::TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: (status == TaskStatus::Done).then_some(now),
            deleted_at: None,
            note_count: 0,
            author: Some(if ctx.actor.is_agent_driven() {
                oxplow_domain::TaskAuthor::Agent
            } else {
                oxplow_domain::TaskAuthor::User
            }),
        };
        let (id, effort) = oxplow_db::task_store::insert_logged_tx(ctx.conn, &ctx.events, &item)
            .map_err(CommandError::from)?;
        ctx.claim(effort.is_some(), "filing a task in_progress")?;
        let row = oxplow_db::task_store::get_task_tx(ctx.conn, id)
            .map_err(CommandError::from)?
            .ok_or_else(|| CommandError::Failed {
                message: format!("task {id} vanished"),
            })?;
        Ok(HandlerOutput {
            result: serde_json::to_value(&row).expect("Task serializes"),
            inverse: None,
            events: Vec::new(),
            after_commit: None,
        })
    }));
    Command::new(create_spec(), handler).expect("work_item.create registers")
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
        effect: oxplow_domain::CommandEffect::Record,
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
        let (after, effort) = oxplow_db::task_store::update_with_status_tx(
            ctx.conn,
            &ctx.events,
            &item,
            input.status,
            now,
        )
        .map_err(not_found)?;
        ctx.claim(
            matches!(effort, EffortTransition::Opened(_)),
            &format!("moving {id} to in_progress"),
        )?;
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

    /// A queued thread (tsk466) may edit a task and move it anywhere but
    /// `in_progress`: task bookkeeping isn't a claim on the worktree.
    #[tokio::test]
    async fn a_queued_thread_edits_and_finishes_tasks() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let queued = queued_agent(&fx).await;
        let out = fx
            .svc
            .commands
            .run(
                &queued,
                UPDATE,
                json!({ "id": fx.task.to_string(), "title": "renamed", "status": "done" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["status"], "done");
        use oxplow_domain::stores::TaskStore as _;
        let row = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(
            (row.title.as_str(), row.status),
            ("renamed", oxplow_domain::TaskStatus::Done)
        );
    }

    /// Moving a task to `in_progress` opens an effort — a claim — which
    /// only the stream's writer thread may take. A refused run writes
    /// nothing, not the fields either, and is audited as denied.
    #[tokio::test]
    async fn a_queued_thread_cannot_claim() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let queued = queued_agent(&fx).await;
        let filed = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({ "title": "later", "thread": fx.thread.to_string() }),
                false,
            )
            .await
            .unwrap();
        let later = filed.result["id"].as_str().unwrap().to_string();
        for (name, input) in [
            (
                UPDATE,
                json!({ "id": later, "title": "renamed", "status": "in_progress" }),
            ),
            (NAME, json!({ "id": later, "to": "in_progress" })),
        ] {
            let err = fx
                .svc
                .commands
                .run(&queued, name, input, false)
                .await
                .unwrap_err();
            assert!(
                matches!(err, CommandError::Denied { .. }),
                "{name}: {err:?}"
            );
        }
        let id: TaskId = later.parse().unwrap();
        use oxplow_domain::stores::TaskStore as _;
        let row = fx.svc.task_store.get(id).await.unwrap().unwrap();
        assert_eq!(
            (row.title.as_str(), row.status),
            ("later", oxplow_domain::TaskStatus::Ready)
        );
        let recent = fx.svc.commands.audit_store().list_recent(10).await.unwrap();
        let denied = recent
            .iter()
            .filter(|r| r.outcome == oxplow_domain::events::schema::CommandOutcome::Denied)
            .count();
        assert_eq!(denied, 2, "{recent:?}");
    }

    /// `work_item.create` (tsk463): filing a task is audited to the actor;
    /// filed straight into `in_progress` it opens the effort in the same
    /// run, and the body's mentions are projected by the pump.
    #[tokio::test]
    async fn a_create_is_audited_and_opens_its_effort() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        };
        let out = fx
            .svc
            .commands
            .run(
                &agent,
                CREATE,
                json!({
                    "title": "filed",
                    "description": "see [[tsk1]]",
                    "status": "in_progress",
                    "thread": fx.thread.to_string(),
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["status"], "in_progress");
        assert_eq!(out.result["author"], "agent");
        let id: TaskId = out.result["id"].as_str().unwrap().parse().unwrap();
        let executed = out.event_id.clone().unwrap();
        let events = fx.svc.event_log_store.read_after(0, 100).await.unwrap();
        let caused: Vec<&str> = events
            .iter()
            .filter(|e| e.envelope.cause.as_ref() == Some(&executed))
            .map(|e| e.envelope.event_type.as_str())
            .collect();
        assert_eq!(caused, vec!["effort.opened", "work_item.created"]);
        use oxplow_db::EffortStore as _;
        assert!(fx
            .svc
            .effort_store
            .find_open_for_work_item(&oxplow_domain::refs::build::work_item_ref(id))
            .await
            .unwrap()
            .is_some());
        fx.svc.event_pump.run_once().await.unwrap();
        let out_refs = fx
            .svc
            .page_ref_store
            .list_outbound(
                oxplow_db::page_ref_projections::KIND_WORK_ITEM,
                &oxplow_db::page_ref_projections::work_item_id(id),
                None,
            )
            .await
            .unwrap();
        assert!(!out_refs.is_empty(), "the body mention was projected");
    }

    /// A queued thread files tasks (tsk466), but not straight into
    /// `in_progress`: that would open an effort.
    #[tokio::test]
    async fn a_queued_thread_files_but_cannot_file_a_claim() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let queued = queued_agent(&fx).await;
        let out = fx
            .svc
            .commands
            .run(
                &queued,
                CREATE,
                json!({ "title": "noted", "thread": "thr9" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["status"], "ready");
        let err = fx
            .svc
            .commands
            .run(
                &queued,
                CREATE,
                json!({ "title": "mine now", "thread": "thr9", "status": "in_progress" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let n: i64 = fx
            .svc
            .db
            .transaction(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM task WHERE title = 'mine now'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    /// Thread 9, queued, in stream 1, as its agent.
    async fn queued_agent(fx: &crate::test_fixtures::EffortFixture) -> Actor {
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
        Actor::Agent {
            thread_id: Some(oxplow_domain::ThreadId::new(9)),
            stream_id: Some(StreamId::new(1)),
        }
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
