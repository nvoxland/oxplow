//! An extension's declared event types (`event_types:` in its manifest,
//! P8.D2): what the vocabulary registers for each one
//! (`EventSchemaRegistry::register_declared`), including the upcast its
//! Starlark compiles to (`.context/extensions.md`).

use std::sync::Arc;

use oxplow_domain::events::schema::Upcast;
use oxplow_domain::DomainError;
use serde_json::json;

/// How long an upcast may run. It runs where an older row is read — a
/// consumer's delivery inside the pump's transaction — so it is as tight
/// as a command's script.
const UPCAST_BUDGET: oxplow_collect_plugin::SandboxBudget =
    crate::extension_commands::COMMAND_SCRIPT_BUDGET;

/// The upcast of `event_type`'s newest version: the script's
/// `transform({from_v, payload})` returns the payload at that version.
/// It runs sandboxed, with no host (no files, no `ai_*`); the registry
/// validates what it returns against the newest schema.
pub fn starlark_upcast(event_type: &str, script: &str) -> Upcast {
    let (event_type, script) = (event_type.to_string(), script.to_string());
    Arc::new(move |from_v, payload| {
        use oxplow_collect_plugin::runtime::{run_sandboxed, run_starlark};
        let script = script.clone();
        run_sandboxed(&UPCAST_BUDGET, move || {
            run_starlark(&script, &json!({ "from_v": from_v, "payload": payload }))
        })
        .map_err(|e| {
            DomainError::Invalid(format!("the upcast of `{event_type}@{from_v}` failed: {e}"))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::events::schema::{DeclaredEventType, EventSchemaRegistry};

    fn declared(v: u32, schema: serde_json::Value, upcast: Option<Upcast>) -> DeclaredEventType {
        DeclaredEventType {
            event_type: "acme_pr.merged".into(),
            v,
            schema,
            summary: "a pull request merged".into(),
            upcast,
        }
    }

    #[test]
    fn a_starlark_upcast_carries_v1_to_v2() {
        let mut r = EventSchemaRegistry::new();
        r.register_declared(
            "acme-pr",
            declared(
                1,
                json!({"type": "object", "required": ["number"], "additionalProperties": false,
                       "properties": {"number": {"type": "integer"}}}),
                None,
            ),
        )
        .unwrap();
        let script = "def transform(x):\n    return {\"pr\": x[\"payload\"][\"number\"], \"from\": x[\"from_v\"]}\n";
        r.register_declared(
            "acme-pr",
            declared(
                2,
                json!({"type": "object", "required": ["pr"], "additionalProperties": false,
                       "properties": {"pr": {"type": "integer"}, "from": {"type": "integer"}}}),
                Some(starlark_upcast("acme_pr.merged", script)),
            ),
        )
        .unwrap();
        let (v, up) = r
            .upcast_to_latest("acme_pr.merged", 1, json!({"number": 12}))
            .unwrap();
        assert_eq!((v, up), (2, json!({"pr": 12, "from": 1})));
    }

    #[test]
    fn an_upcast_that_fails_or_returns_the_wrong_shape_is_refused() {
        let mut r = EventSchemaRegistry::new();
        r.register_declared("acme-pr", declared(1, json!({"type": "object"}), None))
            .unwrap();
        r.register_declared(
            "acme-pr",
            declared(
                2,
                json!({"type": "object", "required": ["pr"]}),
                Some(starlark_upcast(
                    "acme_pr.merged",
                    "def transform(x):\n    return {\"nope\": 1}\n",
                )),
            ),
        )
        .unwrap();
        let wrong = r
            .upcast_to_latest("acme_pr.merged", 1, json!({}))
            .unwrap_err();
        assert!(wrong.to_string().contains("acme_pr.merged@2"), "{wrong}");
        let mut r = EventSchemaRegistry::new();
        r.register_declared(
            "acme-pr",
            declared(
                2,
                json!({"type": "object"}),
                Some(starlark_upcast(
                    "acme_pr.merged",
                    "def transform(x):\n    fail(\"no\")\n",
                )),
            ),
        )
        .unwrap();
        let failed = r
            .upcast_to_latest("acme_pr.merged", 1, json!({}))
            .unwrap_err();
        assert!(
            failed
                .to_string()
                .contains("upcast of `acme_pr.merged@1` failed"),
            "{failed}"
        );
    }
}
