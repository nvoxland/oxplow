//! Commands on shared models (`.context/models.md`):
//! `oxplow.model.set_fields`, an extension's own fields on another's items
//! (composed by its script, run as it, refused to any other caller), and
//! `oxplow.refs.verify`, re-pinning a row's pinned refs to now (anyone may).

use std::sync::Arc;

use oxplow_domain::CommandError;
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::extension::extension_services::ExtensionServices;
use crate::models::{Cause, FieldsOutput, Models, Writer};

use super::ops::Op;
use super::util::schema;
use super::{CommandCall, Handler, HandlerOutput};

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
    /// Whose fields these are, written only by an undo's replay of the
    /// inverse this command recorded (its run has no composer then); a
    /// script's own run names no one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extension: Option<String>,
}

pub(crate) const SET_FIELDS: &str = "oxplow.model.set_fields";

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
            true,
            Handler::Tx(Arc::new(move |ctx, input| {
                let input: SetFields =
                    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                        field: None,
                        message: e.to_string(),
                    })?;
                let extension = match services.composer_name(ctx.run, SET_FIELDS) {
                    Ok(name) => name,
                    Err(_) if ctx.run.is_replaying_inverse() && input.extension.is_some() => {
                        input.extension.clone().unwrap_or_default()
                    }
                    Err(reason) => return Err(CommandError::Denied { reason }),
                };
                let registry = models.current();
                // The undo: the fields it names, as they stood (null where
                // there was none), written back by the same command.
                let inverse = previous_values(
                    ctx.conn,
                    &registry,
                    &extension,
                    &input.model,
                    &input.uri,
                    input.values.keys(),
                )?
                .map(|values| CommandCall {
                    name: SET_FIELDS.into(),
                    input: json!({
                        "model": input.model,
                        "uri": input.uri,
                        "values": values,
                        "extension": extension,
                    }),
                });
                let applied = crate::models::apply_tx(
                    ctx.conn,
                    &registry,
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
                    inverse: if changed { inverse } else { None },
                    ..HandlerOutput::default()
                })
            })),
        ),
    ]
}

/// The values `names` hold now in the composing extension's namespace of
/// `model`'s fields on `uri` (JSON; null for a field with none). `None`
/// when the extension declares no fields there: the write is refused later,
/// with its message.
fn previous_values<'a>(
    conn: &rusqlite::Connection,
    registry: &crate::models::ModelRegistry,
    extension: &str,
    model: &str,
    uri: &str,
    names: impl Iterator<Item = &'a String>,
) -> Result<Option<serde_json::Map<String, Value>>, CommandError> {
    let Some(decls) = registry
        .fields
        .iter()
        .find(|d| d.extension == extension && d.model == model)
    else {
        return Ok(None);
    };
    let failed = |e: rusqlite::Error| CommandError::Failed {
        message: e.to_string(),
    };
    let mut values = serde_json::Map::new();
    for name in names {
        let stored: Option<String> = conn
            .query_row(
                "SELECT value FROM model_field_value \
                 WHERE model = ?1 AND uri = ?2 AND namespace = ?3 AND name = ?4",
                rusqlite::params![model, uri, decls.namespace, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(failed)?;
        let value = stored
            .and_then(|v| serde_json::from_str::<Value>(&v).ok())
            .unwrap_or(Value::Null);
        values.insert(name.clone(), value);
    }
    Ok(Some(values))
}
