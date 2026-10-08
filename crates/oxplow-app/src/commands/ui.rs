//! What the person saw go wrong in the app (tsk1072): `oxplow.ui.report_error`
//! records an operation that failed in front of them — a merge, a save,
//! a query — as `ui.op_failed@1`, so the agent can read what they read
//! (`v_op_error`, the output through `read_event_content`). A person's
//! only: the app reports what it showed, and an agent never reports one.
//! The `ui` namespace expires like the agent's (30 days, output 14).

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_db::event_content_store::put_json_tx;
use oxplow_db::event_log_store::anchors_for_thread_tx;
use oxplow_domain::events::schema::{UiOpFailed, UiOpFailedV1};
use oxplow_domain::refs::build::thread_ref;
use oxplow_domain::{Anchors, CommandError, Invokers, StreamId, ThreadId};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::util::{parse, ref_id, schema};
use super::{Handler, HandlerOutput, TxCtx};

pub const REPORT_ERROR: &str = "oxplow.ui.report_error";

/// `oxplow.ui.report_error`: an operation failed in front of the person.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportErrorInput {
    /// What the person was doing ("Merge bugfixes into current").
    pub label: String,
    /// The command it ran, shell-style ("git merge bugfixes").
    #[serde(default)]
    pub command: Option<String>,
    /// The error message, when there was no captured output.
    #[serde(default)]
    pub message: Option<String>,
    /// The process's exit code.
    #[serde(default)]
    pub exit_code: Option<i64>,
    /// The thread it was started from (`thread:thr3`); absent, none.
    #[serde(default)]
    pub thread: Option<String>,
    /// The stream the app was showing (`stream:str1`), for an error from no
    /// thread: its agent reads the output. A thread names its own stream,
    /// so not with `thread` (tsk1079).
    #[serde(default)]
    pub stream: Option<String>,
    /// The signal that killed the process (`SIGKILL`).
    #[serde(default)]
    pub signal: Option<String>,
    /// How long it ran, in milliseconds.
    #[serde(default)]
    pub duration_ms: Option<i64>,
    /// Captured stderr: stored as the event's `output` body.
    #[serde(default)]
    pub stderr: Option<String>,
    /// Captured stdout: stored as the event's `output` body.
    #[serde(default)]
    pub stdout: Option<String>,
}

/// An empty string is nothing to show.
fn given(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

fn report(ctx: &TxCtx<'_>, input: ReportErrorInput) -> Result<HandlerOutput, CommandError> {
    let thread = input
        .thread
        .as_deref()
        .map(|raw| ref_id::<ThreadId>(raw, "thread", "/thread"))
        .transpose()?;
    let stream = match input.stream.as_deref() {
        Some(_) if thread.is_some() => {
            return Err(CommandError::Invalid {
                field: Some("/stream".into()),
                message: "a thread names its own stream: give one or the other".into(),
            });
        }
        Some(raw) => Some(ref_id::<StreamId>(raw, "stream", "/stream")?),
        None => None,
    };
    let mut body = Map::new();
    for (key, text) in [
        ("stderr", given(input.stderr)),
        ("stdout", given(input.stdout)),
    ] {
        if let Some(text) = text {
            body.insert(key.into(), Value::String(text));
        }
    }
    let output = if body.is_empty() {
        None
    } else {
        Some(put_json_tx(ctx.conn, "ui", &Value::Object(body))?)
    };
    let mut event = ctx.events.typed::<UiOpFailed>(&UiOpFailedV1 {
        label: input.label,
        command: given(input.command),
        message: given(input.message),
        exit_code: input.exit_code,
        thread: thread.map(|t| t.to_string()),
        signal: given(input.signal),
        duration_ms: input.duration_ms,
        output,
    });
    if let Some(thread) = thread {
        event = event
            .with_anchors(anchors_for_thread_tx(ctx.conn, thread)?)
            .with_subject([thread_ref(thread)]);
    } else if let Some(stream) = stream {
        event = event.with_anchors(Anchors {
            stream_id: Some(stream),
            ..Anchors::default()
        });
    }
    Ok(HandlerOutput {
        result: json!({ "event": event.id.as_str() }),
        events: vec![event],
        ..HandlerOutput::default()
    })
}

/// Every `ui.*` command.
pub fn ops() -> Vec<Op> {
    vec![Op::new(
        "diagnostics.write",
        "report_error",
        schema::<ReportErrorInput>(),
        false,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            report(ctx, parse::<ReportErrorInput>(input)?)
        })),
    )
    .open_to(Invokers::HUMAN_ONLY)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_bodies::{self, EventBodyKey};
    use oxplow_domain::{Actor, CommandError, ThreadId};
    use serde_json::{json, Value};

    async fn op_errors(svc: &crate::Services) -> Vec<Value> {
        let out = svc
            .sql
            .query_sql(
                "SELECT event_id, at, stream_id, thread, label, command, message, exit_code, \
                 signal, duration_ms, output_size, payload_expired_at FROM v_op_error",
                vec![],
                None,
            )
            .await
            .unwrap();
        out.rows
            .iter()
            .map(|row| {
                Value::Object(
                    out.columns
                        .iter()
                        .cloned()
                        .zip(row.iter().map(|c| serde_json::to_value(c).unwrap()))
                        .collect(),
                )
            })
            .collect()
    }

    /// tsk1072: an error the person saw is in `v_op_error` for the agent to
    /// read through `query_sql`, and its captured output is the event's
    /// `output` body, readable by an agent of the thread's stream.
    #[tokio::test]
    async fn a_reported_error_is_readable_by_an_agent_through_v_op_error() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                REPORT_ERROR,
                json!({
                    "label": "List data",
                    "command": "query_sql",
                    "message": "IpcCallError: query_sql: timed out after 5s",
                    "exit_code": 1,
                    "thread": oxplow_domain::refs::build::thread_ref(fx.thread),
                    "signal": "SIGTERM",
                    "duration_ms": 5012,
                    "stderr": "timed out after 5s",
                    "stdout": "partial rows",
                }),
                false,
            )
            .await
            .unwrap();
        assert!(out.audit_id.is_some(), "recorded like any person's run");

        let rows = op_errors(&fx.svc).await;
        assert_eq!(rows.len(), 1, "{rows:?}");
        let row = &rows[0];
        assert_eq!(row["label"], "List data");
        assert_eq!(row["command"], "query_sql");
        assert_eq!(
            row["message"],
            "IpcCallError: query_sql: timed out after 5s"
        );
        assert_eq!(row["exit_code"], 1);
        assert_eq!(row["thread"], fx.thread.to_string());
        assert_eq!(row["signal"], "SIGTERM");
        assert_eq!(row["duration_ms"], 5012);
        assert!(row["output_size"].as_i64().unwrap() > 0);
        assert!(row["payload_expired_at"].is_null());
        assert_eq!(row["event_id"], out.result["event"]);

        let event_id = row["event_id"].as_str().unwrap();
        let stream = fx
            .svc
            .event_log_store
            .get(oxplow_domain::EventId(event_id.to_string()))
            .await
            .unwrap()
            .unwrap()
            .envelope
            .anchors
            .stream_id
            .expect("anchored to the thread's stream");
        assert_eq!(row["stream_id"], stream.value());
        let body = event_bodies::read(&fx.svc, event_id, EventBodyKey::Output, Some(stream))
            .await
            .unwrap()
            .expect("the output is the event's body");
        let body: Value = serde_json::from_str(&body.text).unwrap();
        assert_eq!(
            body,
            json!({ "stderr": "timed out after 5s", "stdout": "partial rows" })
        );
    }

    /// tsk1072: a report with nothing captured has no body, and one from no
    /// thread has no anchors.
    #[tokio::test]
    async fn a_report_without_output_or_thread_has_neither() {
        let fx = crate::test_fixtures::services_with_effort().await;
        fx.svc
            .commands
            .run(
                &Actor::Human,
                REPORT_ERROR,
                json!({ "label": "Save note", "stderr": "", "stdout": "" }),
                false,
            )
            .await
            .unwrap();
        let rows = op_errors(&fx.svc).await;
        assert_eq!(rows.len(), 1);
        assert!(rows[0]["output_size"].is_null());
        assert!(rows[0]["thread"].is_null());
        assert!(rows[0]["stream_id"].is_null());
        assert!(rows[0]["command"].is_null());
    }

    /// tsk1079: an error from no thread names the stream the app showed, so
    /// that stream's agent can read its output; a thread names its own, so
    /// both together are refused.
    #[tokio::test]
    async fn a_report_from_no_thread_is_anchored_to_its_stream() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let stream = oxplow_domain::StreamId::new(1);
        fx.svc
            .commands
            .run(
                &Actor::Human,
                REPORT_ERROR,
                json!({ "label": "List data", "stream": oxplow_domain::refs::build::stream_ref(stream), "stderr": "timed out" }),
                false,
            )
            .await
            .unwrap();
        let rows = op_errors(&fx.svc).await;
        assert_eq!(rows[0]["stream_id"], stream.value());
        assert!(rows[0]["thread"].is_null());
        let event_id = rows[0]["event_id"].as_str().unwrap();
        let body = event_bodies::read(&fx.svc, event_id, EventBodyKey::Output, Some(stream))
            .await
            .unwrap()
            .expect("the stream's agent reads the output");
        assert!(body.text.contains("timed out"));

        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                REPORT_ERROR,
                json!({
                    "label": "x",
                    "thread": oxplow_domain::refs::build::thread_ref(fx.thread),
                    "stream": oxplow_domain::refs::build::stream_ref(stream),
                }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/stream"),
            "{err:?}"
        );
    }

    /// tsk1072: once retention expires the payload (`{}`), the error stays a
    /// row — when it happened, where — with its details NULL.
    #[tokio::test]
    async fn an_expired_report_keeps_its_row_without_details() {
        let fx = crate::test_fixtures::services_with_effort().await;
        fx.svc
            .commands
            .run(
                &Actor::Human,
                REPORT_ERROR,
                json!({ "label": "Push", "thread": oxplow_domain::refs::build::thread_ref(fx.thread), "stderr": "denied" }),
                false,
            )
            .await
            .unwrap();
        fx.svc
            .db
            .transaction(|tx| {
                tx.execute(
                    "UPDATE event_log SET payload = '{}', payload_expired_at = at
                      WHERE type = 'ui.op_failed'",
                    [],
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let rows = op_errors(&fx.svc).await;
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert!(row["label"].is_null() && row["thread"].is_null() && row["output_size"].is_null());
        assert!(row["stream_id"].is_i64(), "the anchors outlive the payload");
        assert!(row["payload_expired_at"].is_string());
    }

    /// tsk1072: only the person's app reports what it showed — an agent, or
    /// a lens (acting for anyone), is refused and nothing is logged.
    #[tokio::test]
    async fn an_agent_may_not_report_a_ui_error() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let agent = Actor::Agent {
            thread_id: Some(ThreadId::new(fx.thread.value())),
            stream_id: None,
        };
        let lens = Actor::Lens {
            lens_id: "acme/errors".into(),
            on_behalf_of: Box::new(Actor::Human),
        };
        for actor in [agent, lens] {
            let err = fx
                .svc
                .commands
                .run(
                    &actor,
                    REPORT_ERROR,
                    json!({ "label": "forged", "stderr": "x" }),
                    false,
                )
                .await
                .unwrap_err();
            assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        }
        assert!(op_errors(&fx.svc).await.is_empty());
        let agent_can_list = fx
            .svc
            .commands
            .list(&Actor::Agent {
                thread_id: Some(ThreadId::new(fx.thread.value())),
                stream_id: None,
            })
            .into_iter()
            .any(|spec| spec.id == REPORT_ERROR);
        assert!(!agent_can_list, "not offered to an agent");
    }
}
