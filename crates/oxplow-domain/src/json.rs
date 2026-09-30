//! Arbitrary JSON at the IPC boundary. specta's own `serde_json::Value`
//! impl describes Value's Rust enum (`{ Bool: … } | { Number: … }`), which
//! is neither what serde puts on the wire nor usable from TypeScript. A
//! value that really is any JSON — a command's input and result — is
//! exported as `unknown`: the caller narrows it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Any JSON value. Serializes as the value itself; exports to TypeScript
/// as `unknown`. Use it as an IPC argument type, or name it on a `Value`
/// field with `#[specta(type = Json)]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Json(pub Value);

impl specta::Type for Json {
    fn definition(_types: &mut specta::Types) -> specta::datatype::DataType {
        specta::datatype::DataType::Reference(specta_typescript::define("unknown"))
    }
}

impl From<Json> for Value {
    fn from(j: Json) -> Value {
        j.0
    }
}
