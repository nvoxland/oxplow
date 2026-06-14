//! Lenient parameter extraction for MCP tools.
//!
//! rmcp's stock [`rmcp::handler::server::wrapper::Parameters`]
//! deserializes the raw JSON `arguments` object straight into `P`, so a
//! weak model (opencode / GPT-5-mini) that sends camelCase keys
//! (`touchedFiles`) against our snake_case params (`touched_files`) hits
//! an opaque `-32602 missing field "touched_files"` transport error it
//! can't act on. The camelCase priors come from training; our tool
//! *outputs* are camelCase too, which reinforces the wrong guess.
//!
//! This module is a drop-in replacement [`Parameters`] (same name, so
//! the `#[tool]` macro still recognizes it in handler signatures and
//! derives the schema from the inner `P`). Before deserializing it
//! case-folds incoming object keys to snake_case **additively** — it
//! inserts a snake_case copy of any camelCase/kebab key, never removing
//! the original — so:
//!
//! - snake_case input is left byte-for-byte untouched (canonical form),
//! - a reasonable camelCase call just works,
//! - free-form nested values keep their original keys.
//!
//! snake_case stays the advertised/canonical name: the JSON schema is
//! still derived from `P` unchanged; aliases are only tolerated input.
//! Genuine failures (a field that's truly missing under either casing)
//! surface as a clear, self-describing [`McpError`] rather than a raw
//! transport error.

use rmcp::handler::server::common::FromContextPart;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::JsonObject;
use rmcp::ErrorData as McpError;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Drop-in replacement for `rmcp::handler::server::wrapper::Parameters`
/// that tolerates camelCase (and kebab-case) aliases of our snake_case
/// param fields. See the module docs for the rationale.
pub struct Parameters<P>(pub P);

impl<P: JsonSchema> JsonSchema for Parameters<P> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        P::schema_name()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        P::json_schema(generator)
    }
}

impl<S, P> FromContextPart<ToolCallContext<'_, S>> for Parameters<P>
where
    P: DeserializeOwned,
{
    fn from_context_part(context: &mut ToolCallContext<S>) -> Result<Self, McpError> {
        let arguments = context.arguments.take().unwrap_or_default();
        let value = lenient_from_object::<P>(arguments)?;
        Ok(Parameters(value))
    }
}

/// Deserialize `P` from a raw MCP `arguments` object, first augmenting
/// it with snake_case aliases of any camelCase/kebab keys so a
/// reasonable wrong-casing call just works. On failure returns a clear,
/// self-describing [`McpError`] (`invalid_params`) instead of an opaque
/// transport error.
pub fn lenient_from_object<P: DeserializeOwned>(arguments: JsonObject) -> Result<P, McpError> {
    let mut value = Value::Object(arguments);
    add_snake_case_aliases(&mut value);
    serde_json::from_value(value).map_err(|e| {
        McpError::invalid_params(
            format!(
                "Invalid parameters for this tool: {e}. Field names are snake_case \
                 (camelCase aliases are also accepted); see the tool's input schema \
                 for the required fields.",
            ),
            None,
        )
    })
}

/// Recursively walk a JSON value and, for every object key whose
/// snake_case form differs and isn't already present, insert a
/// snake_case copy of the entry (without removing the original).
/// Additive: snake_case input is untouched, and free-form nested values
/// keep their original keys.
fn add_snake_case_aliases(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let aliases: Vec<(String, Value)> = map
                .iter()
                .filter_map(|(k, v)| {
                    let snake = to_snake_case(k);
                    if snake != *k && !map.contains_key(&snake) {
                        Some((snake, v.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            for (k, v) in aliases {
                map.insert(k, v);
            }
            for v in map.values_mut() {
                add_snake_case_aliases(v);
            }
        }
        Value::Array(items) => {
            for v in items {
                add_snake_case_aliases(v);
            }
        }
        _ => {}
    }
}

/// camelCase / kebab-case → snake_case (ASCII). Already-snake keys pass
/// through unchanged. A `_` is inserted before each uppercase letter
/// that follows a lowercase letter or digit; runs of uppercase
/// (acronyms) are not split.
fn to_snake_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    let mut prev_lower_or_digit = false;
    for ch in key.chars() {
        if ch == '-' {
            out.push('_');
            prev_lower_or_digit = false;
        } else if ch.is_ascii_uppercase() {
            if prev_lower_or_digit {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_lower_or_digit = false;
        } else {
            out.push(ch);
            prev_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompleteTaskParams, CreateTaskMcpParams, ReorderTasksParams, UpdateTaskMcpParams};

    fn obj(v: Value) -> JsonObject {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn to_snake_case_handles_camel_kebab_and_acronyms() {
        assert_eq!(to_snake_case("touchedFiles"), "touched_files");
        assert_eq!(to_snake_case("threadId"), "thread_id");
        assert_eq!(to_snake_case("orderedItemIds"), "ordered_item_ids");
        assert_eq!(to_snake_case("durationMs"), "duration_ms");
        // Already snake: untouched.
        assert_eq!(to_snake_case("touched_files"), "touched_files");
        assert_eq!(to_snake_case("id"), "id");
        // Kebab folds to snake too.
        assert_eq!(to_snake_case("thread-id"), "thread_id");
        // Trailing acronym is not split.
        assert_eq!(to_snake_case("threadID"), "thread_id");
    }

    #[test]
    fn add_snake_case_aliases_is_additive_and_recursive() {
        let mut v = serde_json::json!({
            "touchedFiles": ["a.rs"],
            "id": "tsk1",
            "impacts": [{ "targetKind": "wiki", "id": "x" }],
        });
        add_snake_case_aliases(&mut v);
        let map = v.as_object().unwrap();
        // Snake alias added, original camel key preserved.
        assert_eq!(map["touched_files"], serde_json::json!(["a.rs"]));
        assert_eq!(map["touchedFiles"], serde_json::json!(["a.rs"]));
        // Single-word key untouched, no spurious alias.
        assert_eq!(map["id"], "tsk1");
        // Recurses into array elements.
        let impact = &map["impacts"][0];
        assert_eq!(impact["target_kind"], "wiki");
    }

    #[test]
    fn camel_case_completes_task() {
        // The headline failure from the task: complete_task with
        // camelCase `touchedFiles` must deserialize, not -32602.
        let p: CompleteTaskParams = lenient_from_object(obj(serde_json::json!({
            "id": "tsk1",
            "summary": "did the thing",
            "touchedFiles": ["src/lib.rs", "src/main.rs"],
        })))
        .unwrap();
        assert_eq!(p.id, "tsk1");
        assert_eq!(p.touched_files.unwrap(), vec!["src/lib.rs", "src/main.rs"]);
    }

    #[test]
    fn snake_case_still_works_and_is_canonical() {
        let p: CompleteTaskParams = lenient_from_object(obj(serde_json::json!({
            "id": "tsk1",
            "summary": "did the thing",
            "touched_files": ["src/lib.rs"],
        })))
        .unwrap();
        assert_eq!(p.touched_files.unwrap(), vec!["src/lib.rs"]);
    }

    #[test]
    fn camel_case_create_task_with_many_multiword_fields() {
        let p: CreateTaskMcpParams = lenient_from_object(obj(serde_json::json!({
            "threadId": "thr5",
            "title": "t",
            "description": "d",
            "parentId": "tsk9",
            "touchedFiles": ["a"],
        })))
        .unwrap();
        assert_eq!(p.thread_id.as_deref(), Some("thr5"));
        assert_eq!(p.parent_id.as_deref(), Some("tsk9"));
        assert_eq!(p.touched_files.unwrap(), vec!["a"]);
    }

    #[test]
    fn camel_case_update_and_reorder() {
        let u: UpdateTaskMcpParams = lenient_from_object(obj(serde_json::json!({
            "id": "tsk1",
            "parentId": "tsk2",
        })))
        .unwrap();
        assert_eq!(u.parent_id.as_deref(), Some("tsk2"));

        let r: ReorderTasksParams = lenient_from_object(obj(serde_json::json!({
            "threadId": "thr5",
            "orderedItemIds": ["tsk1", "tsk2"],
        })))
        .unwrap();
        assert_eq!(r.thread_id.as_deref(), Some("thr5"));
        assert_eq!(r.ordered_item_ids, vec!["tsk1", "tsk2"]);
    }

    #[test]
    fn truly_missing_field_yields_clear_error() {
        // `summary` is genuinely required and absent under any casing.
        let err = lenient_from_object::<CompleteTaskParams>(obj(serde_json::json!({
            "id": "tsk1",
        })))
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("summary"), "should name the field: {msg}");
        assert!(
            msg.contains("snake_case") && msg.contains("camelCase"),
            "should explain accepted casing: {msg}",
        );
    }
}
