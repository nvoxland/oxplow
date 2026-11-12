//! Commands on shared models (`.context/models.md`) that an extension's
//! script composes: `oxplow.model.set_fields`, its own fields on another
//! extension's items. They run as the composing extension and refuse any
//! other caller, in the command's transaction.

use std::sync::Arc;

use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::extension::extension_services::ExtensionServices;
use crate::models::{Cause, FieldsOutput, Models, Writer};

use super::ops::Op;
use super::util::schema;
use super::{Handler, HandlerOutput};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetFields {
    /// The model (`<owner namespace>/<model>`).
    model: String,
    /// The item's URI.
    uri: String,
    /// Field → value, as declared in the composing extension's `fields:`;
    /// a null removes the field.
    values: serde_json::Map<String, Value>,
}

const SET_FIELDS: &str = "oxplow.model.set_fields";

pub fn ops(services: Arc<ExtensionServices>, models: Models) -> Vec<Op> {
    vec![Op::new(
        "model.write",
        "set_fields",
        schema::<SetFields>(),
        false,
        Handler::Tx(Arc::new(move |ctx, input| {
            let input: SetFields =
                serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                    field: None,
                    message: e.to_string(),
                })?;
            let extension = services
                .composer_name(ctx.run, SET_FIELDS)
                .map_err(|reason| CommandError::Denied { reason })?;
            let applied = crate::models::apply_tx(
                ctx.conn,
                &models.current(),
                ctx.events.vocabulary,
                &Writer {
                    extension: &extension,
                    source: &ctx.events.source,
                    cause: ctx.events.cause.clone(),
                    anchors: Default::default(),
                    why: Cause::Trigger,
                },
                &[],
                &[FieldsOutput {
                    model: input.model.clone(),
                    uri: input.uri,
                    values: input.values,
                }],
            )
            .map_err(|message| CommandError::Invalid {
                field: None,
                message,
            })?;
            let changed = applied
                .counts
                .get(&input.model)
                .and_then(|c| c.get("fields"))
                .copied()
                .unwrap_or(0)
                > 0;
            Ok(HandlerOutput {
                result: json!({ "changed": changed }),
                ..HandlerOutput::default()
            })
        })),
    )]
}
