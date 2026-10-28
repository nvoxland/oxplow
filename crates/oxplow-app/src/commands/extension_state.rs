//! Native operations for the calling contribution's private namespace. Reads
//! are unaudited; writes audit only metadata and return the resulting stamp.

use std::sync::Arc;

use oxplow_domain::CommandError;

use crate::extension::extension_state::{ReadInput, StateHost, StateHostError, WriteInput};

use super::ops::Op;
use super::util::schema;
use super::{Handler, HandlerOutput};

pub fn ops(host: StateHost) -> Vec<Op> {
    vec![snapshot_op(host.clone()), compare_exchange_op(host)]
}

pub fn snapshot_op(host: StateHost) -> Op {
    Op::new(
        "state.read",
        "snapshot",
        schema::<ReadInput>(),
        false,
        Handler::Tx(Arc::new(move |ctx, input| {
            let input = serde_json::from_value(input).map_err(|_| CommandError::Invalid {
                field: None,
                message: "state snapshot takes { scope, area }".into(),
            })?;
            let snapshot =
                host.snapshot(ctx.conn, ctx.run, input)
                    .map_err(|error| match error {
                        StateHostError::Denied(reason) => CommandError::Denied {
                            reason: reason.into(),
                        },
                        StateHostError::State(
                            oxplow_domain::extension_state::StateError::Storage(error),
                        ) => CommandError::from(error),
                        other => CommandError::Failed {
                            message: other.to_string(),
                        },
                    })?;
            Ok(HandlerOutput {
                result: serde_json::to_value(snapshot).expect("a state snapshot serializes"),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// Extension declarations bind this operation to their own contribution. No
/// owner/namespace selector, script execution or external wait is accepted.
pub fn compare_exchange_op(host: StateHost) -> Op {
    Op::new(
        "state.write",
        "compare_exchange",
        schema::<WriteInput>(),
        false,
        Handler::Tx(Arc::new(move |ctx, input| {
            let input: WriteInput =
                serde_json::from_value(input).map_err(|_| CommandError::Invalid {
                    field: None,
                    message: "invalid state compare-and-swap input".into(),
                })?;
            let snapshot = host.compare_exchange(ctx, &input).map_err(|error| {
                use oxplow_domain::extension_state::StateError;
                match error {
                    StateHostError::Denied(reason) => CommandError::Denied {
                        reason: reason.into(),
                    },
                    StateHostError::State(StateError::Storage(error)) => CommandError::from(error),
                    StateHostError::State(StateError::Conflict { .. }) => CommandError::Invalid {
                        field: Some("/expected".into()),
                        message: "state changed; read its current stamp before retrying".into(),
                    },
                    StateHostError::State(StateError::Invalid(_)) => CommandError::Invalid {
                        field: None,
                        message: "invalid state values or schema version".into(),
                    },
                }
            })?;
            Ok(HandlerOutput {
                result: serde_json::to_value(snapshot.stamp).expect("a state stamp serializes"),
                ..HandlerOutput::default()
            })
        })),
    )
    .with_unrecorded(&["values"])
}
