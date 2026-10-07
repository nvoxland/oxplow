//! The effort commands: open, close, link and retitle an effort — a span of
//! one thread's work (`.context/work-tracking.md`). oxplow opens and closes
//! efforts itself through an effort policy; these are what that policy, a
//! person, an agent or a lens runs, over the effort store's cores.
//!
//! - `effort.open { thread?, work_item?, title?, adopt_since? }` opens one
//!   on a thread (the caller's by default). The thread's current effort
//!   closes first; a late open adopts the thread's un-efforted activity
//!   back to `adopt_since`.
//! - `effort.close { effort | thread, as_of?, summary?, reason? }` closes
//!   one, optionally as of a past point.
//! - `effort.link { effort | thread, work_item }` links it to a work item,
//!   or unlinks it (`null`).
//! - `effort.update { effort, title }` sets or clears its own title.
//!
//! Efforts are records, not claims on the worktree, so any thread may run
//! them; an agent only within its own stream. All are `Tx`, so the effort
//! row, its event and the audit commit together. Opening and closing aren't
//! undoable (a reopened effort is a new one); linking and retitling are.

use std::sync::Arc;

use oxplow_db::effort_store::{
    finish_tx, link_tx, open_for_thread_tx, retitle_tx, start_tx, ClosedBy, EffortEnd, EffortStart,
};
use oxplow_domain::refs::build::validate_work_item_ref;
use oxplow_domain::work_items::WorkItemsRegistry;

use super::work_item::with_loose_refs;
use oxplow_domain::{
    Actor, Atomicity, CommandCall, CommandError, CommandSpec, Confirm, EffortId, Invokers,
    Lifecycle, StreamId, ThreadId, Timestamp,
};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Command, Handler, HandlerOutput, TxCtx};

pub const OPEN: &str = "oxplow.effort.open";
pub const CLOSE: &str = "oxplow.effort.close";
pub const LINK: &str = "oxplow.effort.link";
pub const UPDATE: &str = "oxplow.effort.update";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortOpenInput {
    /// The thread doing the work (`thr3`); defaults to the caller's. An
    /// agent may name only a thread in its own stream.
    #[serde(default)]
    pub thread: Option<String>,
    /// The work item it's for, as a canonical ref (`work_item:oxplow:tsk42`,
    /// `work_item:issues:ENG-12`); unlinked when absent.
    #[serde(default)]
    pub work_item: Option<String>,
    /// Its own title; absent, it shows its item's, else its first prompt's.
    #[serde(default)]
    pub title: Option<String>,
    /// RFC 3339: adopt the thread's un-efforted activity back to here
    /// (never before its previous effort ended).
    #[serde(default)]
    pub adopt_since: Option<String>,
}

/// Which effort: by id, or the thread's open one.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortCloseInput {
    /// The effort (`eff12`).
    #[serde(default)]
    pub effort: Option<String>,
    /// Or the thread (`thr3`) whose open effort it is.
    #[serde(default)]
    pub thread: Option<String>,
    /// RFC 3339: close it as of this point; what came after is left to the
    /// thread's next effort. Now when absent.
    #[serde(default)]
    pub as_of: Option<String>,
    /// What shipped, as the effort's summary.
    #[serde(default)]
    pub summary: Option<String>,
    /// Why it closed: `commit` (a commit landed its work) or `switch` (the
    /// thread moved on). Absent, it's the caller's own close.
    #[serde(default)]
    pub reason: Option<CloseReason>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    Commit,
    Switch,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortLinkInput {
    /// The effort (`eff12`).
    #[serde(default)]
    pub effort: Option<String>,
    /// Or the thread (`thr3`) whose open effort it is.
    #[serde(default)]
    pub thread: Option<String>,
    /// The work item to link it to; `null` unlinks it.
    pub work_item: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortUpdateInput {
    /// The effort (`eff12`).
    pub effort: String,
    /// Its own title; `null` clears it back to the default.
    pub title: Option<String>,
}

fn spec(name: &str, summary: &str, schema: serde_json::Value, undoable: bool) -> CommandSpec {
    CommandSpec {
        id: name.into(),
        summary: summary.into(),
        input_schema: schema,
        invokers: Invokers::ALL,
        confirm: Confirm::Never,
        undoable,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        effect: oxplow_domain::CommandEffect::Record,
        needs: Vec::new(),
        ui: None,
    }
}

fn parse<T: serde::de::DeserializeOwned>(input: serde_json::Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
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

fn timestamp(raw: &str, field: &str) -> Result<Timestamp, CommandError> {
    Timestamp::parse(raw).map_err(|e| invalid(field, format!("`{raw}` isn't RFC 3339: {e}")))
}

fn thread_id(raw: &str, field: &str) -> Result<ThreadId, CommandError> {
    raw.parse::<ThreadId>()
        .map_err(|e| invalid(field, e.to_string()))
}

/// `thread`, after checking it exists and — for an agent — that it's in
/// the agent's own stream.
fn reachable_thread(ctx: &TxCtx<'_>, thread: ThreadId, field: &str) -> Result<(), CommandError> {
    let stream: i64 = ctx
        .conn
        .query_row(
            "SELECT stream_id FROM threads WHERE id = ?1",
            [thread.value()],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?
        .ok_or_else(|| invalid(field, format!("unknown thread `{thread}`")))?;
    match ctx.actor.stream_id() {
        Some(own) if own != StreamId::new(stream) => Err(invalid(
            field,
            format!("thread `{thread}` is in stream `str{stream}`, not the caller's `{own}`"),
        )),
        _ => Ok(()),
    }
}

/// The effort named by `effort`, or `thread`'s open one (exactly one of
/// them given), with its thread.
fn target_effort(
    ctx: &TxCtx<'_>,
    effort: Option<&str>,
    thread: Option<&str>,
) -> Result<(EffortId, ThreadId), CommandError> {
    let (id, thread) = match (effort, thread) {
        (Some(raw), None) => {
            let id = EffortId::try_from_str(raw).ok_or_else(|| {
                invalid("/effort", format!("`{raw}` isn't an effort id (`eff12`)"))
            })?;
            let thread: i64 = ctx
                .conn
                .query_row(
                    "SELECT thread_id FROM effort WHERE id = ?1",
                    [id.value()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage)?
                .ok_or_else(|| invalid("/effort", format!("no effort `{id}`")))?;
            (id, ThreadId::new(thread))
        }
        (None, Some(raw)) => {
            let thread = thread_id(raw, "/thread")?;
            let id = open_for_thread_tx(ctx.conn, thread)
                .map_err(storage)?
                .ok_or_else(|| {
                    invalid("/thread", format!("thread `{thread}` has no open effort"))
                })?;
            (id, thread)
        }
        _ => {
            return Err(invalid(
                "/effort",
                "name exactly one of `effort` or `thread`",
            ))
        }
    };
    reachable_thread(ctx, thread, "/effort")?;
    Ok((id, thread))
}

fn output(result: serde_json::Value, inverse: Option<CommandCall>) -> HandlerOutput {
    HandlerOutput {
        result,
        inverse,
        ..HandlerOutput::default()
    }
}

pub fn open_command(work_items: WorkItemsRegistry) -> Command {
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: EffortOpenInput = parse(with_loose_refs(&work_items, input)?)?;
        if let Some(w) = &input.work_item {
            validate_work_item_ref(w).map_err(|e| invalid("/work_item", e.to_string()))?;
        }
        let thread = match input.thread.as_deref() {
            Some(raw) => thread_id(raw, "/thread")?,
            None => ctx
                .actor
                .thread_id()
                .ok_or_else(|| invalid("/thread", "no thread given and the caller has none"))?,
        };
        reachable_thread(ctx, thread, "/thread")?;
        let adopt_since = input
            .adopt_since
            .as_deref()
            .map(|t| timestamp(t, "/adopt_since"))
            .transpose()?;
        let effort = start_tx(
            ctx.conn,
            &ctx.events,
            &EffortStart {
                work_item: input.work_item.as_deref(),
                adopt_since,
                ..EffortStart::at(thread, Timestamp::now())
            },
        )?;
        if input.title.is_some() {
            retitle_tx(ctx.conn, &ctx.events, effort, input.title.as_deref())?;
        }
        Ok(output(
            json!({
                "effort": effort.to_string(),
                "thread": thread.to_string(),
                "work_item": input.work_item,
            }),
            None,
        ))
    }));
    Command::new(
        spec(
            OPEN,
            "Open an effort — a span of a thread's work — on a thread (yours by default), \
             optionally linked to a work item. The thread's current effort closes first. \
             oxplow opens efforts itself; this is for policies, people and corrections.",
            serde_json::to_value(schemars::schema_for!(EffortOpenInput)).expect("schema"),
            false,
        ),
        handler,
    )
    .expect("effort.open registers")
}

pub fn close_command() -> Command {
    let handler = Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
        let input: EffortCloseInput = parse(input)?;
        let (id, _) = target_effort(ctx, input.effort.as_deref(), input.thread.as_deref())?;
        let at = match input.as_of.as_deref() {
            Some(raw) => timestamp(raw, "/as_of")?,
            None => Timestamp::now(),
        };
        let started: String = ctx
            .conn
            .query_row(
                "SELECT started_at FROM effort WHERE id = ?1",
                [id.value()],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if Timestamp::parse(&started).is_ok_and(|s| at < s) {
            return Err(invalid(
                "/as_of",
                format!("effort `{id}` started after `{at}`"),
            ));
        }
        let closed_by = match (input.reason, ctx.actor) {
            (Some(CloseReason::Commit), _) => ClosedBy::Commit,
            (Some(CloseReason::Switch), _) => ClosedBy::Switch,
            (None, Actor::Agent { .. }) => ClosedBy::Agent,
            (None, Actor::Effect { .. }) => ClosedBy::System,
            (None, _) => ClosedBy::Person,
        };
        let closed = finish_tx(
            ctx.conn,
            &ctx.events,
            id,
            &EffortEnd {
                summary: input.summary.as_deref(),
                ..EffortEnd::at(at, closed_by)
            },
        )?;
        if !closed {
            return Err(invalid(
                "/effort",
                format!("effort `{id}` is already closed"),
            ));
        }
        Ok(output(json!({ "effort": id.to_string() }), None))
    }));
    Command::new(
        spec(
            CLOSE,
            "Close an effort (by id, or a thread's open one), optionally as of a past point and \
             with a summary.",
            serde_json::to_value(schemars::schema_for!(EffortCloseInput)).expect("schema"),
            false,
        ),
        handler,
    )
    .expect("effort.close registers")
}

pub fn link_command(work_items: WorkItemsRegistry) -> Command {
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: EffortLinkInput = parse(with_loose_refs(&work_items, input)?)?;
        if let Some(w) = &input.work_item {
            validate_work_item_ref(w).map_err(|e| invalid("/work_item", e.to_string()))?;
        }
        let (id, _) = target_effort(ctx, input.effort.as_deref(), input.thread.as_deref())?;
        let before = link_tx(ctx.conn, &ctx.events, id, input.work_item.as_deref())?;
        Ok(output(
            json!({ "effort": id.to_string(), "work_item": input.work_item }),
            Some(CommandCall {
                name: LINK.into(),
                input: json!({ "effort": id.to_string(), "work_item": before }),
            }),
        ))
    }));
    Command::new(
        spec(
            LINK,
            "Link an effort (by id, or a thread's open one) to a work item, or unlink it with \
             `work_item: null`.",
            serde_json::to_value(schemars::schema_for!(EffortLinkInput)).expect("schema"),
            true,
        ),
        handler,
    )
    .expect("effort.link registers")
}

pub fn update_command() -> Command {
    let handler = Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
        let input: EffortUpdateInput = parse(input)?;
        let (id, _) = target_effort(ctx, Some(&input.effort), None)?;
        let before = retitle_tx(ctx.conn, &ctx.events, id, input.title.as_deref())?;
        Ok(output(
            json!({ "effort": id.to_string(), "title": input.title }),
            Some(CommandCall {
                name: UPDATE.into(),
                input: json!({ "effort": id.to_string(), "title": before }),
            }),
        ))
    }));
    Command::new(
        spec(
            UPDATE,
            "Set an effort's own title, or clear it back to its default with `title: null`.",
            serde_json::to_value(schemars::schema_for!(EffortUpdateInput)).expect("schema"),
            true,
        ),
        handler,
    )
    .expect("effort.update registers")
}

/// The effort commands, for the bus. A loose id in `work_item` is the
/// active work list's (`work_item::with_loose_refs`).
pub fn commands(work_items: WorkItemsRegistry) -> Vec<Command> {
    vec![
        open_command(work_items.clone()),
        close_command(),
        link_command(work_items),
        update_command(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::EffortStore as _;

    fn agent(fx: &crate::test_fixtures::EffortFixture) -> Actor {
        Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        }
    }

    const ISSUES: &str = "work_item:issues:ENG-12";

    /// An agent opens an unlinked effort on its own thread, links it,
    /// titles it and closes it by thread; link and title undo.
    #[tokio::test]
    async fn an_effort_opens_unlinked_and_is_linked_titled_and_closed() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let bus = &fx.svc.commands;
        let opened = bus.run(&agent(&fx), OPEN, json!({}), false).await.unwrap();
        let effort: EffortId = opened.result["effort"].as_str().unwrap().parse().unwrap();
        let row = |fx: &crate::test_fixtures::EffortFixture| {
            let svc = fx.svc.clone();
            async move { svc.effort_store.get_effort(&effort).await.unwrap().unwrap() }
        };
        assert_eq!(row(&fx).await.work_item, None);
        let linked = bus
            .run(
                &agent(&fx),
                LINK,
                json!({ "thread": fx.thread.to_string(), "work_item": ISSUES }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(row(&fx).await.work_item.as_deref(), Some(ISSUES));
        bus.undo(&Actor::Human, linked.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(row(&fx).await.work_item, None);
        bus.run(
            &Actor::Human,
            UPDATE,
            json!({ "effort": effort.to_string(), "title": "Login page" }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(row(&fx).await.title.as_deref(), Some("Login page"));
        bus.run(
            &agent(&fx),
            CLOSE,
            json!({ "thread": fx.thread.to_string(), "summary": "done" }),
            false,
        )
        .await
        .unwrap();
        let closed = row(&fx).await;
        assert!(closed.ended_at.is_some());
        assert_eq!(closed.closed_by.as_deref(), Some("agent"));
        assert_eq!(closed.summary.as_deref(), Some("done"));
    }

    /// Any thread in the caller's stream may take an effort — queued ones
    /// included — but not a thread in another stream; bad refs and an
    /// as-of before the start are refused.
    #[tokio::test]
    async fn open_and_close_refuse_bad_input_and_other_streams() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let bus = &fx.svc.commands;
        let field = |err: CommandError| match err {
            CommandError::Invalid { field: Some(f), .. } => f,
            other => panic!("expected Invalid, got {other:?}"),
        };
        assert_eq!(
            field(
                bus.run(&agent(&fx), OPEN, json!({ "work_item": "ENG-12" }), false)
                    .await
                    .unwrap_err()
            ),
            "/work_item"
        );
        fx.svc
            .db
            .transaction(|c| {
                c.execute_batch(
                    "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                       VALUES (2, 'worktree', 'b', 'b', 'refs/heads/b', 'main', '/elsewhere',
                               '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z');
                     INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                       VALUES (2, 2, 'other', 'active',
                               '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z'),
                              (3, 1, 'q', 'queued',
                               '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z');",
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        assert_eq!(
            field(
                bus.run(&agent(&fx), OPEN, json!({ "thread": "thr2" }), false)
                    .await
                    .unwrap_err()
            ),
            "/thread"
        );
        bus.run(&agent(&fx), OPEN, json!({ "thread": "thr3" }), false)
            .await
            .unwrap();
        assert_eq!(
            field(
                bus.run(
                    &agent(&fx),
                    CLOSE,
                    json!({ "thread": "thr3", "as_of": "2020-01-01T00:00:00Z" }),
                    false
                )
                .await
                .unwrap_err()
            ),
            "/as_of"
        );
        assert_eq!(
            field(
                bus.run(&agent(&fx), CLOSE, json!({}), false)
                    .await
                    .unwrap_err()
            ),
            "/effort"
        );
    }
}
