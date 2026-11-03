//! Hint commands: a person dismisses a hint raised to them in Alerts, one
//! or all (`.context/extensions-commands.md` "Advisories"). `Tx` over the
//! nudge store's `dismiss_tx` / `dismiss_all_tx`; read back through
//! `v_agent_nudge`, where a dismissed hint has its `delivered_at`. An agent
//! can't dismiss what was raised to the person.

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::util::{parse, schema};
use super::{Handler, HandlerOutput, TxCtx};

pub const DISMISS: &str = "oxplow.hint.dismiss";
pub const DISMISS_ALL: &str = "oxplow.hint.dismiss_all";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DismissInput {
    /// The hint's nudge (`v_agent_nudge.id`).
    pub nudge: i64,
}

/// `hint.dismiss { nudge }`.
pub fn dismiss_op() -> Op {
    Op::new(
        "hints.write",
        "dismiss",
        schema::<DismissInput>(),
        false,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: DismissInput = parse(input)?;
            let result = json!({ "dismissed": input.nudge });
            if oxplow_db::agent_nudge_store::dismiss_tx(ctx.conn, input.nudge)? {
                return Ok(HandlerOutput {
                    result,
                    ..HandlerOutput::default()
                });
            }
            // Dismissed already (a double click): nothing to record.
            if oxplow_db::agent_nudge_store::person_hint_tx(ctx.conn, input.nudge)? {
                return Ok(HandlerOutput {
                    result,
                    unchanged: true,
                    ..HandlerOutput::default()
                });
            }
            Err(CommandError::Invalid {
                field: Some("/nudge".into()),
                message: format!("no hint {} was raised to the person", input.nudge),
            })
        })),
    )
}

/// Every hint still raised to the person.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DismissAllInput {}

/// `hint.dismiss_all {}`: Clear notices. With none raised, nothing to
/// record.
pub fn dismiss_all_op() -> Op {
    Op::new(
        "hints.write",
        "dismiss_all",
        schema::<DismissAllInput>(),
        false,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let _: DismissAllInput = parse(input)?;
            let n = oxplow_db::agent_nudge_store::dismiss_all_tx(ctx.conn)?;
            Ok(HandlerOutput {
                result: json!({ "dismissed": n }),
                unchanged: n == 0,
                ..HandlerOutput::default()
            })
        })),
    )
}

/// The hint commands, for the bus.
pub fn ops() -> Vec<Op> {
    vec![dismiss_op(), dismiss_all_op()]
}

#[cfg(test)]
mod tests {
    use oxplow_db::agent_nudge_store::{Audience, NewAgentNudge};
    use oxplow_domain::Actor;
    use serde_json::json;

    fn raised(thread: String, audience: Audience, message: &str) -> NewAgentNudge {
        NewAgentNudge {
            thread_id: thread,
            effort_id: None,
            kind: "x/muted".into(),
            message: message.into(),
            trigger: None,
            turn_id: None,
            cause: None,
            audience,
        }
    }

    /// Clear notices dismisses every hint raised to the person and none
    /// raised to an agent; with none left it records nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn dismiss_all_puts_away_every_hint_raised_to_the_person() {
        let fx = crate::test_fixtures::services_with_effort().await;
        for (audience, m) in [
            (Audience::Person, "one"),
            (Audience::Person, "two"),
            (Audience::Agent, "theirs"),
        ] {
            fx.svc
                .nudge_store
                .record(raised(fx.thread.to_string(), audience, m))
                .await
                .unwrap();
        }
        let out = fx
            .svc
            .commands
            .run(&Actor::Human, super::DISMISS_ALL, json!({}), false)
            .await
            .unwrap();
        assert_eq!(out.result["dismissed"], json!(2));
        let left: i64 = fx
            .svc
            .db
            .transaction(|tx| {
                tx.query_row(
                    "SELECT count(*) FROM agent_nudge WHERE delivered_at IS NULL",
                    [],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(left, 1, "the agent's hint still waits for it");
        let again = fx
            .svc
            .commands
            .run(&Actor::Human, super::DISMISS_ALL, json!({}), false)
            .await
            .unwrap();
        assert_eq!(again.result["dismissed"], json!(0));
    }
}
