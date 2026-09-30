//! Every wire type's JSON Schema, by name — what the goldens under
//! `schemas/` hold (`tests/schemas.rs` keeps them equal, `OXPLOW_BLESS=1`
//! to write a new one) and what [`validate`] checks a message against.

use serde_json::Value;

use crate::codec::notify;
use crate::errors::ErrorObject;
use crate::model::*;

/// `(name, schema)` for every wire type, sorted by name.
pub fn all() -> Vec<(&'static str, Value)> {
    macro_rules! schema {
        ($t:ty) => {
            serde_json::to_value(schemars::schema_for!($t)).expect("schema serializes")
        };
    }
    let mut all = vec![
        ("initialize_params", schema!(InitializeParams)),
        ("initialize_result", schema!(InitializeResult)),
        ("check_params", schema!(CheckParams)),
        ("check_result", schema!(CheckResult)),
        ("discover_params", schema!(DiscoverParams)),
        ("discover_result", schema!(DiscoverResult)),
        ("invoke_params", schema!(InvokeParams)),
        ("invoke_result", schema!(InvokeResult)),
        ("read_params", schema!(ReadParams)),
        ("read_result", schema!(ReadResult)),
        ("cancel", schema!(notify::Cancel)),
        ("progress", schema!(notify::Progress)),
        ("record", schema!(notify::Record)),
        ("state", schema!(notify::State)),
        ("error", schema!(ErrorObject)),
    ];
    all.sort_by_key(|(name, _)| *name);
    all
}

/// The schema a message's payload must match: a method's params or
/// result, or a notification's params.
pub fn for_message(method: &str, is_result: bool) -> Option<&'static str> {
    Some(match (method, is_result) {
        (method::INITIALIZE, false) => "initialize_params",
        (method::INITIALIZE, true) => "initialize_result",
        (method::CHECK, false) => "check_params",
        (method::CHECK, true) => "check_result",
        (method::DISCOVER, false) => "discover_params",
        (method::DISCOVER, true) => "discover_result",
        (method::INVOKE, false) => "invoke_params",
        (method::INVOKE, true) => "invoke_result",
        (method::READ, false) => "read_params",
        (method::READ, true) => "read_result",
        (notify::CANCEL, false) => "cancel",
        (notify::PROGRESS, false) => "progress",
        (notify::RECORD, false) => "record",
        (notify::STATE, false) => "state",
        _ => return None,
    })
}

/// Check `value` against the schema named `name`; the violations, if any.
pub fn validate(name: &str, value: &Value) -> Result<(), Vec<String>> {
    let Some((_, schema)) = all().into_iter().find(|(n, _)| *n == name) else {
        return Err(vec![format!("no wire type `{name}`")]);
    };
    let validator = jsonschema::validator_for(&schema)
        .map_err(|e| vec![format!("schema `{name}` doesn't compile: {e}")])?;
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
