//! `effort.open` / `effort.close` (P2.6.4, tsk456): bracket work on a work
//! item that isn't an oxplow task — `work_item:linear:ENG-12` — so it gets
//! the same snapshots, attribution and review an in_progress task does.
//! An oxplow task's effort follows its status (`work_item.transition`),
//! so these commands refuse `work_item:oxplow:…`: opening one directly
//! would break "in_progress ⟺ one open effort".
//!
//! Both are `Tx` over the effort store's cores, so the effort row, its
//! `effort.opened` / `effort.closed` (caused by the run's
//! `command.executed`) and the audit commit together; the snapshot pin is
//! the effort-lifecycle pump consumer's. Not undoable: closing brackets a
//! snapshot, and a reopened effort is a new one.

use std::sync::Arc;

use oxplow_domain::refs::build::{validate_work_item_ref, work_item_id_of_ref};
use oxplow_domain::work_items::{provider_of, WorkItemsRegistry};
use oxplow_domain::{
    Atomicity, CommandError, CommandSpec, Confirm, EffortId, Invokers, Lifecycle, StreamId,
    ThreadId, Timestamp,
};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Command, Handler, HandlerOutput, TxCtx};

pub const OPEN: &str = "effort.open";
pub const CLOSE: &str = "effort.close";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortOpenInput {
    /// The work item, as a canonical ref: `work_item:linear:ENG-12`. Not an
    /// oxplow task — move the task to `in_progress` instead.
    pub work_item: String,
    /// The thread doing the work (`thr3`); defaults to the caller's. An
    /// agent may name only a thread in its own stream.
    #[serde(default)]
    pub thread: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortCloseInput {
    /// The effort (`eff12`).
    pub effort: String,
    /// What shipped, as the effort's summary.
    #[serde(default)]
    pub summary: Option<String>,
}

fn spec(name: &str, summary: &str, schema: serde_json::Value) -> CommandSpec {
    CommandSpec {
        name: name.into(),
        summary: summary.into(),
        input_schema: schema,
        invokers: Invokers::ALL,
        confirm: Confirm::Never,
        undoable: false,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        effect: oxplow_domain::CommandEffect::Write,
    }
}

fn invalid(field: &str, message: impl Into<String>) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message: message.into(),
    }
}

/// A SQLite error as the bus sees it — a lock blip stays `Busy`, so the
/// run retries.
fn storage(e: rusqlite::Error) -> CommandError {
    CommandError::from(oxplow_db::map_sql_err(e))
}

/// The stream `thread` belongs to; `Invalid` for an unknown thread, or —
/// when `working` — one that isn't its stream's working (writer) thread:
/// a queued or closed thread takes no new effort (closing one is fine).
fn stream_of(
    ctx: &TxCtx<'_>,
    thread: ThreadId,
    field: &str,
    working: bool,
) -> Result<StreamId, CommandError> {
    let (stream, status): (i64, String) = ctx
        .conn
        .query_row(
            "SELECT stream_id, status FROM threads WHERE id = ?1",
            [thread.value()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(storage)?
        .ok_or_else(|| invalid(field, format!("unknown thread `{thread}`")))?;
    let status: oxplow_domain::ThreadStatus =
        serde_json::from_value(serde_json::Value::String(status.clone()))
            .map_err(|_| invalid(field, format!("thread `{thread}` has status `{status}`")))?;
    if working && !status.is_writer() {
        return Err(invalid(
            field,
            format!("thread `{thread}` is {status:?}, not its stream's working thread"),
        ));
    }
    Ok(StreamId::new(stream))
}

/// An agent acts only within its own stream.
fn within_actor_stream(
    ctx: &TxCtx<'_>,
    thread: ThreadId,
    field: &str,
    working: bool,
) -> Result<(), CommandError> {
    let stream = stream_of(ctx, thread, field, working)?;
    match ctx.actor.stream_id() {
        Some(own) if own != stream => Err(invalid(
            field,
            format!("thread `{thread}` is in stream `{stream}`, not the caller's `{own}`"),
        )),
        _ => Ok(()),
    }
}

/// A free-standing effort is for items whose provider doesn't open one
/// itself: refused when `work_item`'s provider is registered and declares
/// `in_progress_opens_effort` (oxplow's tasks — their effort follows their
/// status). An unregistered provider's item (`work_item:linear:ENG-12`
/// with no Linear provider) takes one.
fn opens_its_own_effort(
    registry: &WorkItemsRegistry,
    work_item: &str,
    field: &str,
) -> Result<(), CommandError> {
    let Ok(provider) = provider_of(work_item) else {
        return Ok(());
    };
    match registry.get(provider) {
        Ok(p) if p.features().in_progress_opens_effort => Err(invalid(
            field,
            format!(
                "`{work_item}`'s effort opens and closes with its status ({provider} items \
                 open their own) — run `work_item.transition` on it instead"
            ),
        )),
        _ => Ok(()),
    }
}

pub fn open_command(registry: WorkItemsRegistry) -> Command {
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: EffortOpenInput =
            serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                field: None,
                message: e.to_string(),
            })?;
        validate_work_item_ref(&input.work_item)
            .map_err(|e| invalid("/work_item", e.to_string()))?;
        opens_its_own_effort(&registry, &input.work_item, "/work_item")?;
        let thread = match input.thread.as_deref() {
            Some(raw) => raw
                .parse::<ThreadId>()
                .map_err(|e| invalid("/thread", e.to_string()))?,
            None => ctx
                .actor
                .thread_id()
                .ok_or_else(|| invalid("/thread", "no thread given and the caller has none"))?,
        };
        within_actor_stream(ctx, thread, "/thread", true)?;
        if let Some(open) =
            oxplow_db::effort_store::find_open_for_work_item_tx(ctx.conn, &input.work_item)
                .map_err(storage)?
        {
            return Err(invalid(
                "/work_item",
                format!(
                    "`{}` already has an open effort ({}); close it first",
                    input.work_item, open.id
                ),
            ));
        }
        let effort = oxplow_db::effort_store::start_tx(
            ctx.conn,
            &ctx.events,
            &input.work_item,
            thread,
            None,
            Timestamp::now(),
            false,
        )
        .map_err(CommandError::from)?;
        Ok(HandlerOutput {
            result: json!({
                "effort": effort.to_string(),
                "work_item": input.work_item,
                "thread": thread.to_string(),
                "label": work_item_id_of_ref(&input.work_item),
            }),
            inverse: None,
            events: Vec::new(),
            after_commit: None,
        })
    }));
    Command::new(
        spec(
            OPEN,
            "Open an effort on a work item whose provider doesn't open one itself (not an \
             oxplow task; work_item:<provider>:<id>): its edits are snapshotted and attributed \
             like an in_progress task's. Close it with effort.close.",
            serde_json::to_value(schemars::schema_for!(EffortOpenInput)).expect("schema"),
        ),
        handler,
    )
    .expect("effort.open registers")
}

pub fn close_command(registry: WorkItemsRegistry) -> Command {
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: EffortCloseInput =
            serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                field: None,
                message: e.to_string(),
            })?;
        let id = EffortId::try_from_str(&input.effort).ok_or_else(|| {
            invalid(
                "/effort",
                format!("`{}` is not an effort id (`eff12`)", input.effort),
            )
        })?;
        let row: Option<(String, i64)> = ctx
            .conn
            .query_row(
                "SELECT work_item, thread_id FROM effort WHERE id = ?1",
                [id.value()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(storage)?;
        let (work_item, thread) =
            row.ok_or_else(|| invalid("/effort", format!("no effort `{id}`")))?;
        opens_its_own_effort(&registry, &work_item, "/effort")?;
        within_actor_stream(ctx, ThreadId::new(thread), "/effort", false)?;
        let closed = oxplow_db::effort_store::finish_tx(
            ctx.conn,
            &ctx.events,
            id,
            None,
            input.summary.as_deref(),
            Timestamp::now(),
            false,
        )
        .map_err(CommandError::from)?;
        if !closed {
            return Err(invalid(
                "/effort",
                format!("effort `{id}` is already closed"),
            ));
        }
        Ok(HandlerOutput {
            result: json!({ "effort": id.to_string(), "work_item": work_item }),
            inverse: None,
            events: Vec::new(),
            after_commit: None,
        })
    }));
    Command::new(
        spec(
            CLOSE,
            "Close an effort opened with effort.open, with an optional summary.",
            serde_json::to_value(schemars::schema_for!(EffortCloseInput)).expect("schema"),
        ),
        handler,
    )
    .expect("effort.close registers")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::EffortStore as _;
    use oxplow_domain::Actor;

    fn agent(fx: &crate::test_fixtures::EffortFixture) -> Actor {
        Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        }
    }

    const LINEAR: &str = "work_item:linear:ENG-12";

    #[tokio::test]
    async fn open_and_close_bracket_a_foreign_work_item_with_their_audit() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let bus = &fx.svc.commands;
        let opened = bus
            .run(&agent(&fx), OPEN, json!({ "work_item": LINEAR }), false)
            .await
            .unwrap();
        let effort: EffortId = opened.result["effort"].as_str().unwrap().parse().unwrap();
        let row = fx
            .svc
            .effort_store
            .get_effort(&effort)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.work_item, LINEAR);
        assert_eq!(row.thread_id, fx.thread);
        let executed = opened.event_id.clone().unwrap();
        let events = fx.svc.event_log_store.read_after(0, 100).await.unwrap();
        assert!(events
            .iter()
            .any(|e| e.envelope.event_type == "effort.opened"
                && e.envelope.cause.as_ref() == Some(&executed)));

        // A second open on the same item is refused, naming the open one.
        let again = bus
            .run(&agent(&fx), OPEN, json!({ "work_item": LINEAR }), false)
            .await
            .unwrap_err();
        assert!(again.to_string().contains(&effort.to_string()), "{again}");

        let closed = bus
            .run(
                &agent(&fx),
                CLOSE,
                json!({ "effort": effort.to_string(), "summary": "shipped" }),
                false,
            )
            .await
            .unwrap();
        let row = fx
            .svc
            .effort_store
            .get_effort(&effort)
            .await
            .unwrap()
            .unwrap();
        assert!(row.ended_at.is_some());
        assert_eq!(row.summary.as_deref(), Some("shipped"));
        let events = fx.svc.event_log_store.read_after(0, 100).await.unwrap();
        assert!(events.iter().any(
            |e| e.envelope.event_type == "effort.closed" && e.envelope.cause == closed.event_id
        ));
        let twice = bus
            .run(
                &agent(&fx),
                CLOSE,
                json!({ "effort": effort.to_string() }),
                false,
            )
            .await
            .unwrap_err();
        assert!(twice.to_string().contains("already closed"), "{twice}");
    }

    #[tokio::test]
    async fn open_refuses_bad_input_oxplow_tasks_and_other_streams() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let bus = &fx.svc.commands;
        let refused = |err: CommandError, field: &str| match err {
            CommandError::Invalid { field: Some(f), .. } => assert_eq!(f, field),
            other => panic!("expected Invalid at {field}, got {other:?}"),
        };
        // The schema names the missing field.
        let err = bus
            .run(&agent(&fx), OPEN, json!({}), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("work_item"), "{err}");
        refused(
            bus.run(&agent(&fx), OPEN, json!({ "work_item": "ENG-12" }), false)
                .await
                .unwrap_err(),
            "/work_item",
        );
        let task = oxplow_domain::refs::build::work_item_ref(fx.task);
        let err = bus
            .run(&agent(&fx), OPEN, json!({ "work_item": task }), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("work_item.transition"), "{err}");
        refused(
            bus.run(
                &agent(&fx),
                OPEN,
                json!({ "work_item": LINEAR, "thread": "thr999" }),
                false,
            )
            .await
            .unwrap_err(),
            "/thread",
        );
        // A thread in another stream is out of an agent's reach.
        fx.svc
            .db
            .transaction(|c| {
                c.execute_batch(
                    "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                       VALUES (2, 'worktree', 'b', 'b', 'refs/heads/b', 'main', '/elsewhere',
                               '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z');
                     INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                       VALUES (2, 2, 'other', 'active',
                               '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z');",
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        refused(
            bus.run(
                &agent(&fx),
                OPEN,
                json!({ "work_item": LINEAR, "thread": "thr2" }),
                false,
            )
            .await
            .unwrap_err(),
            "/thread",
        );
        // A thread in the caller's stream that isn't working (queued,
        // closed) can't take an effort.
        fx.svc
            .db
            .transaction(|c| {
                c.execute_batch(
                    "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                       VALUES (3, 1, 'q', 'queued',
                               '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z');",
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        refused(
            bus.run(
                &Actor::Human,
                OPEN,
                json!({ "work_item": LINEAR, "thread": "thr3" }),
                false,
            )
            .await
            .unwrap_err(),
            "/thread",
        );
    }
}
