//! Hint commands: a person dismisses a hint raised to them in Alerts
//! (`.context/extensions.md` "Advisories"). `Tx` over the nudge store's
//! `dismiss_tx`; read back through `v_agent_nudge`, where a dismissed
//! hint has its `delivered_at`. An agent can't dismiss what was raised to
//! the person.

use std::sync::Arc;

use oxplow_domain::{
    Atomicity, CommandEffect, CommandError, CommandSpec, Confirm, Invokers, Lifecycle,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Command, Handler, HandlerOutput, TxCtx};

pub const DISMISS: &str = "oxplow.hint.dismiss";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DismissInput {
    /// The hint's nudge (`v_agent_nudge.id`).
    pub nudge: i64,
}

/// `hint.dismiss { nudge }`.
pub fn dismiss_command() -> Command {
    Command::new(
        CommandSpec {
            id: DISMISS.into(),
            summary: "Dismiss a hint raised to the person (a `v_agent_nudge` row with audience `person`)."
                .into(),
            input_schema: serde_json::to_value(schemars::schema_for!(DismissInput))
                .expect("schema serializes"),
            invokers: Invokers {
                human: true,
                agent: false,
                lens: true,
            },
            confirm: Confirm::Never,
            undoable: false,
            lifecycle: Lifecycle::Stable,
            atomicity: Atomicity::Tx,
            effect: CommandEffect::Record,
            needs: Vec::new(),
        },
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: DismissInput =
                serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                    field: None,
                    message: e.to_string(),
                })?;
            if !oxplow_db::agent_nudge_store::dismiss_tx(ctx.conn, input.nudge)? {
                return Err(CommandError::Invalid {
                    field: Some("/nudge".into()),
                    message: format!(
                        "no hint {} raised to the person and not yet dismissed",
                        input.nudge
                    ),
                });
            }
            Ok(HandlerOutput {
                result: json!({ "dismissed": input.nudge }),
                ..HandlerOutput::default()
            })
        })),
    )
    .expect("oxplow.hint.dismiss is a valid command")
}

/// The hint commands, for the bus.
pub fn commands() -> Vec<Command> {
    vec![dismiss_command()]
}
