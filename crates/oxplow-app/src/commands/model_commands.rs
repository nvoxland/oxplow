//! Commands on shared models (`.context/models.md`):
//! `oxplow.model.set_fields`, an extension's own fields on another's items
//! (composed by its script, run as it, refused to any other caller), and
//! `oxplow.refs.verify`, re-pinning a row's pinned refs to now (anyone may).

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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct VerifyRefs {
    /// What pins them: a shared model row's URI.
    uri: String,
    /// The files to re-pin; every file it pins when left out.
    #[serde(default)]
    files: Option<Vec<String>>,
}

pub fn ops(services: Arc<ExtensionServices>, models: Models) -> Vec<Op> {
    let verify = Op::new(
        "model.write",
        "verify_refs",
        schema::<VerifyRefs>(),
        false,
        Handler::Tx(Arc::new(|ctx, input| {
            let input: VerifyRefs =
                serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                    field: None,
                    message: e.to_string(),
                })?;
            let verified = oxplow_db::model_store::verify_pins_tx(
                ctx.conn,
                &input.uri,
                input.files.as_deref(),
            )
            .map_err(|e| CommandError::Invalid {
                field: Some("/files".into()),
                message: e.to_string(),
            })?;
            Ok(HandlerOutput {
                result: json!({ "verified": verified }),
                ..HandlerOutput::default()
            })
        })),
    );
    vec![
        verify,
        Op::new(
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
                        anchors: {
                            // The command's, with its agent's open turn.
                            let mut anchors = ctx.run.anchors().clone();
                            if let (Some(thread), None) = (anchors.thread_id, anchors.turn_id) {
                                anchors.turn_id = oxplow_db::agent_stores::activity_anchors_tx(
                                    ctx.conn,
                                    thread,
                                    anchors.agent_session_id,
                                )
                                .map_err(|e| CommandError::Failed {
                                    message: e.to_string(),
                                })?
                                .turn_id;
                            }
                            anchors
                        },
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
        ),
    ]
}
