//! Hint commands: a person dismisses a hint raised to them in Alerts
//! (`.context/extensions.md` "Advisories"). `Tx` over the nudge store's
//! `dismiss_tx`; read back through `v_agent_nudge`, where a dismissed
//! hint has its `delivered_at`. An agent can't dismiss what was raised to
//! the person.

use crate::commands::ops::Op;
use oxplow_domain::Invokers;
use std::sync::Arc;

use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::util::{parse, schema};
use super::{Handler, HandlerOutput, TxCtx};

pub const DISMISS: &str = "oxplow.hint.dismiss";

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
    .open_to(Invokers::NO_AGENT)
}

/// The hint commands, for the bus.
pub fn ops() -> Vec<Op> {
    vec![dismiss_op()]
}
