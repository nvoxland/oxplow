//! Native operations for an extension's validated configuration layers.
//! Reads leave no audit payload; writes omit private layer values and stamps.

use std::sync::Arc;

use oxplow_domain::CommandError;

use crate::extension_configuration::{
    ConfigurationError, ConfigurationHost, ReadInput, WriteInput,
};

use super::ops::Op;
use super::util::schema;
use super::{Handler, HandlerOutput};

pub fn ops(host: ConfigurationHost) -> Vec<Op> {
    vec![
        snapshot_op(host.clone()),
        inspect_op(host.clone()),
        compare_exchange_op(host),
    ]
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
            let snapshot = host.snapshot(ctx.conn, ctx.run).map_err(command_error)?;
            Ok(HandlerOutput {
                result: serde_json::to_value(snapshot)
                    .expect("a configuration snapshot serializes"),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// Return only the current schema version and layer stamps. This remains
/// available when stored values no longer validate against the declaration.
pub fn inspect_op(host: ConfigurationHost) -> Op {
    Op::new(
        "extension_config.read",
        "inspect",
        schema::<ReadInput>(),
        false,
        Handler::Tx(Arc::new(move |ctx, input| {
            let _: ReadInput =
                serde_json::from_value(input).map_err(|_| CommandError::Invalid {
                    field: None,
                    message: "invalid extension configuration read input".into(),
                })?;
            let stamps = host.inspect(ctx.conn, ctx.run).map_err(command_error)?;
            Ok(HandlerOutput {
                result: serde_json::to_value(stamps).expect("configuration stamps serialize"),
                ..HandlerOutput::default()
            })
        })),
    )
}

pub fn compare_exchange_op(host: ConfigurationHost) -> Op {
    Op::new(
        "extension_config.write",
        "compare_exchange",
        schema::<WriteInput>(),
        false,
        Handler::Tx(Arc::new(move |ctx, input| {
            let input: WriteInput =
                serde_json::from_value(input).map_err(|_| CommandError::Invalid {
                    field: None,
                    message: "invalid extension configuration write input".into(),
                })?;
            let stamps = host.compare_exchange(ctx, &input).map_err(command_error)?;
            Ok(HandlerOutput {
                result: serde_json::to_value(stamps).expect("configuration stamps serialize"),
                ..HandlerOutput::default()
            })
        })),
    )
    .with_unrecorded(&["project", "personal"])
}

fn command_error(error: ConfigurationError) -> CommandError {
    match error {
        ConfigurationError::Store(
            oxplow_db::extension_config_store::ConfigStoreError::Storage(storage),
        ) => CommandError::from(storage),
        ConfigurationError::Denied => CommandError::Denied {
            reason: "extension configuration access denied".into(),
        },
        ConfigurationError::Store(
            oxplow_db::extension_config_store::ConfigStoreError::Conflict { .. },
        ) => CommandError::Invalid {
            field: Some("/expected".into()),
            message: "configuration changed; inspect the current stamps before retrying".into(),
        },
        _ => CommandError::Invalid {
            field: None,
            message: "invalid extension configuration request".into(),
        },
    }
}
