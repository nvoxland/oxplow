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

/// A 64-bit integer the exporter would refuse (`i64`, `u64`, `usize`) that
/// is a row id, count, size or timestamp, far inside JavaScript's exact
/// integers: exported as `number`. Name it on a field with
/// `#[specta(type = oxplow_domain::JsNumber)]` (`Option<JsNumber>` for an
/// option). A marker for the exporter only; it is never constructed.
/// (`f64` would export as `number | null`, `NaN` being null in JSON.)
pub struct JsNumber;

impl specta::Type for JsNumber {
    fn definition(_types: &mut specta::Types) -> specta::datatype::DataType {
        specta::datatype::DataType::Reference(specta_typescript::define("number"))
    }
}
