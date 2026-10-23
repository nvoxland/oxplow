//! What the person does with an operation that failed in front of them
//! (`v_op_error`, recorded by `oxplow.ui.report_error`): open its output
//! (`oxplow.op_error.mark_seen`) and dismiss it (`oxplow.op_error.dismiss`,
//! `dismiss_all`). Each is a mark in the event log naming the
//! `ui.op_failed` event, read back as `v_op_error.seen` / `dismissed`, and
//! kept as long as the failure is (the `ui` namespace's window). A
//! person's or a lens's on their behalf, never an agent's: the list is
//! what the person sees.

use std::sync::Arc;

use oxplow_db::op_error_store::{is_op_failed_tx, state_tx, undismissed_tx};
use oxplow_domain::events::schema::{UiOpErrorDismissed, UiOpErrorMarkV1, UiOpErrorSeen};
use oxplow_domain::uri::build::event_uri;
use oxplow_domain::{CommandError, Envelope, EventId};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::util::{parse, schema};
use super::{Handler, HandlerOutput, TxCtx};
use crate::commands::ops::Op;

pub const MARK_SEEN: &str = "oxplow.op_error.mark_seen";
pub const DISMISS: &str = "oxplow.op_error.dismiss";
pub const DISMISS_ALL: &str = "oxplow.op_error.dismiss_all";

/// One failed operation.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OneInput {
    /// The failed operation's event (`v_op_error.event_id`).
    pub event: String,
}

/// Every failed operation the person hasn't dismissed.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AllInput {}

/// The mark of kind `T` on failed operation `event`, anchored where the
/// failure is, so whoever reads the failure reads its marks.
fn mark<T>(ctx: &TxCtx<'_>, event: &str, kind: &str) -> Result<Envelope, CommandError>
where
    T: oxplow_domain::events::schema::EventType<Payload = UiOpErrorMarkV1>,
{
    let anchors = oxplow_db::event_log_store::get_tx(ctx.conn, &EventId(event.to_string()))?
        .map(|e| e.envelope.anchors)
        .unwrap_or_default();
    Ok(ctx
        .events
        .typed::<T>(&UiOpErrorMarkV1 {
            event: event.to_string(),
        })
        .with_anchors(anchors)
        .with_subject([event_uri(event)])
        .with_dedupe_key(format!("ui.op_error.{kind}:{event}")))
}

/// `event`, refused at `/event` unless it is a failed operation.
fn failed_operation(ctx: &TxCtx<'_>, input: OneInput) -> Result<String, CommandError> {
    if is_op_failed_tx(ctx.conn, &input.event)? {
        return Ok(input.event);
    }
    Err(CommandError::Invalid {
        field: Some("/event".into()),
        message: format!("no failed operation {}", input.event),
    })
}

fn mark_seen(ctx: &TxCtx<'_>, input: OneInput) -> Result<HandlerOutput, CommandError> {
    let event = failed_operation(ctx, input)?;
    let result = json!({ "seen": event });
    if state_tx(ctx.conn, &event)?.0 {
        return Ok(HandlerOutput {
            result,
            unchanged: true,
            ..HandlerOutput::default()
        });
    }
    Ok(HandlerOutput {
        result,
        events: vec![mark::<UiOpErrorSeen>(ctx, &event, "seen")?],
        ..HandlerOutput::default()
    })
}

fn dismiss(ctx: &TxCtx<'_>, input: OneInput) -> Result<HandlerOutput, CommandError> {
    let event = failed_operation(ctx, input)?;
    let result = json!({ "dismissed": event });
    if state_tx(ctx.conn, &event)?.1 {
        return Ok(HandlerOutput {
            result,
            unchanged: true,
            ..HandlerOutput::default()
        });
    }
    Ok(HandlerOutput {
        result,
        events: vec![mark::<UiOpErrorDismissed>(ctx, &event, "dismissed")?],
        ..HandlerOutput::default()
    })
}

fn dismiss_all(ctx: &TxCtx<'_>) -> Result<HandlerOutput, CommandError> {
    let events = undismissed_tx(ctx.conn)?
        .iter()
        .map(|event| mark::<UiOpErrorDismissed>(ctx, event, "dismissed"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(HandlerOutput {
        result: json!({ "dismissed": events.len() }),
        unchanged: events.is_empty(),
        events,
        ..HandlerOutput::default()
    })
}

/// Every `op_error.*` command.
pub fn ops() -> Vec<Op> {
    vec![
        Op::new(
            "diagnostics.write",
            "mark_op_error_seen",
            schema::<OneInput>(),
            false,
            Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
                mark_seen(ctx, parse(input)?)
            })),
        ),
        Op::new(
            "diagnostics.write",
            "dismiss_op_error",
            schema::<OneInput>(),
            false,
            Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
                dismiss(ctx, parse(input)?)
            })),
        ),
        Op::new(
            "diagnostics.write",
            "dismiss_all_op_errors",
            schema::<AllInput>(),
            false,
            Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
                let _: AllInput = parse(input)?;
                dismiss_all(ctx)
            })),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ui::REPORT_ERROR;
    use oxplow_domain::Actor;

    async fn report(svc: &crate::Services, label: &str) -> String {
        let out = svc
            .commands
            .run(
                &Actor::Human,
                REPORT_ERROR,
                json!({ "label": label }),
                false,
            )
            .await
            .unwrap();
        out.result["event"].as_str().unwrap().to_string()
    }

    /// `(label, seen, dismissed)` per failed operation, oldest first.
    async fn state(svc: &crate::Services) -> Vec<(String, i64, i64)> {
        let out = svc
            .sql
            .query_sql(
                "SELECT label, seen, dismissed FROM v_op_error ORDER BY at, event_id",
                vec![],
                None,
            )
            .await
            .unwrap();
        out.rows
            .iter()
            .map(|r| {
                let int = |c: &oxplow_db::SqlCell| match c {
                    oxplow_db::SqlCell::Int(n) => *n,
                    other => panic!("{other:?}"),
                };
                let text = match &r[0] {
                    oxplow_db::SqlCell::Text(t) => t.clone(),
                    other => panic!("{other:?}"),
                };
                (text, int(&r[1]), int(&r[2]))
            })
            .collect()
    }

    async fn run(
        svc: &crate::Services,
        actor: &Actor,
        name: &str,
        input: serde_json::Value,
    ) -> Result<oxplow_domain::CommandOutcome, CommandError> {
        svc.commands.run(actor, name, input, false).await
    }

    /// The person opens a failed operation's output, then dismisses it:
    /// each is recorded once, read back from `v_op_error`; doing it again
    /// changes nothing; an id that isn't a failed operation is refused at
    /// the field; an agent can't.
    #[tokio::test]
    async fn a_person_marks_a_failed_operation_seen_and_dismisses_it() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let push = report(svc, "Push to origin").await;
        assert_eq!(state(svc).await, vec![("Push to origin".into(), 0, 0)]);

        let seen = run(svc, &Actor::Human, MARK_SEEN, json!({ "event": push }))
            .await
            .unwrap();
        assert!(seen.audit_id.is_some());
        assert_eq!(state(svc).await, vec![("Push to origin".into(), 1, 0)]);
        let again = run(svc, &Actor::Human, MARK_SEEN, json!({ "event": push }))
            .await
            .unwrap();
        assert!(again.audit_id.is_none(), "nothing recorded: {again:?}");

        run(svc, &Actor::Human, DISMISS, json!({ "event": push }))
            .await
            .unwrap();
        assert_eq!(state(svc).await, vec![("Push to origin".into(), 1, 1)]);
        let again = run(svc, &Actor::Human, DISMISS, json!({ "event": push }))
            .await
            .unwrap();
        assert!(again.audit_id.is_none(), "nothing recorded: {again:?}");

        // The first event in the log is no failed operation.
        let other = fx
            .svc
            .sql
            .query_sql(
                "SELECT id FROM v_event WHERE type <> 'ui.op_failed' LIMIT 1",
                vec![],
                None,
            )
            .await
            .unwrap()
            .rows[0][0]
            .clone();
        let oxplow_db::SqlCell::Text(other) = other else {
            panic!()
        };
        for event in ["nope".to_string(), other] {
            let err = run(svc, &Actor::Human, DISMISS, json!({ "event": event }))
                .await
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), message } if f == "/event" && message.contains("no failed operation")),
                "{err:?}"
            );
        }

        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        // An agent may mark one seen too.
        run(svc, &agent, MARK_SEEN, json!({ "event": push }))
            .await
            .unwrap();
    }

    /// What a failed operation printed is `v_op_error_output`, a row per
    /// failure that captured some; one that captured nothing has none.
    #[tokio::test]
    async fn a_failed_operations_output_is_read_through_its_model() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let out = svc
            .commands
            .run(
                &Actor::Human,
                REPORT_ERROR,
                json!({ "label": "Push", "stderr": "rejected: non-fast-forward", "stdout": "To origin" }),
                false,
            )
            .await
            .unwrap();
        let pushed = out.result["event"].as_str().unwrap().to_string();
        report(svc, "Quiet").await;
        let rows = svc
            .sql
            .query_sql(
                "SELECT event_id, stderr, stdout, truncated FROM v_op_error_output",
                vec![],
                None,
            )
            .await
            .unwrap()
            .rows;
        use oxplow_db::SqlCell::{Int, Text};
        assert_eq!(
            rows,
            vec![vec![
                Text(pushed),
                Text("rejected: non-fast-forward".into()),
                Text("To origin".into()),
                Int(0)
            ]]
        );
    }

    /// Dismiss all dismisses each failed operation not yet dismissed, and
    /// with none left changes nothing.
    #[tokio::test]
    async fn dismiss_all_clears_what_the_person_still_sees() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let first = report(svc, "Merge").await;
        report(svc, "Save").await;
        run(svc, &Actor::Human, DISMISS, json!({ "event": first }))
            .await
            .unwrap();
        report(svc, "Query").await;

        let out = run(svc, &Actor::Human, DISMISS_ALL, json!({}))
            .await
            .unwrap();
        assert_eq!(out.result, json!({ "dismissed": 2 }));
        assert!(state(svc)
            .await
            .iter()
            .all(|(_, seen, dismissed)| *seen == 1 && *dismissed == 1));
        let again = run(svc, &Actor::Human, DISMISS_ALL, json!({}))
            .await
            .unwrap();
        assert!(
            again.audit_id.is_none() && again.result == json!({ "dismissed": 0 }),
            "{again:?}"
        );
    }
}
