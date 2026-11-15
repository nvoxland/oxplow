//! Authorized nested read projection for extension scripts and native aliases.
//! Writes use the shared config.set/config.unset commands.

use std::sync::Arc;

use oxplow_domain::CommandError;

use crate::extension::extension_configuration::{ConfigurationError, ConfigurationHost, ReadInput};

use super::ops::Op;
use super::util::schema;
use super::{Handler, HandlerOutput};

pub fn ops(host: ConfigurationHost) -> Vec<Op> {
    vec![snapshot_op(host)]
}

pub fn snapshot_op(host: ConfigurationHost) -> Op {
    Op::new(
        "extension_config.read",
        "snapshot",
        schema::<ReadInput>(),
        false,
        Handler::Tx(Arc::new(move |ctx, input| {
            let _: ReadInput =
                serde_json::from_value(input).map_err(|_| CommandError::Invalid {
                    field: None,
                    message: "invalid extension configuration read input".into(),
                })?;
            let snapshot = host.snapshot(ctx.run).map_err(command_error)?;
            Ok(HandlerOutput {
                result: serde_json::to_value(snapshot)
                    .expect("a configuration snapshot serializes"),
                ..HandlerOutput::default()
            })
        })),
    )
}

fn command_error(error: ConfigurationError) -> CommandError {
    match error {
        ConfigurationError::Denied => CommandError::Denied {
            reason: "extension configuration access denied".into(),
        },
        _ => CommandError::Invalid {
            field: None,
            message: "invalid extension configuration request".into(),
        },
    }
}
