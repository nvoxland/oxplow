//! An extension's declared event types (`event_types:` in its manifest,
//! P8.D2): what the vocabulary registers for each one
//! (`EventSchemaRegistry::register_declared`), including the upcast its
//! Starlark compiles to (`.context/extensions.md`).

use std::sync::Arc;

use oxplow_domain::events::schema::{DeclaredEventType, EventSchemaRegistry, Upcast};
use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::extensions::manifest_v2::{at, entry_line, key_line, line_under};

/// An extension's `event_types:` as loaded: its valid types, and the
/// retention window it declares for its namespace.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EventTypes {
    pub types: Vec<EventTypeDecl>,
    /// Shorter than the plugin default (`event_retention::check_declared`);
    /// `None`: the default.
    pub retention: Option<EventRetention>,
}

/// How long its events' payloads and large content are kept, in days.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EventRetention {
    pub payload_days: u32,
    pub content_days: u32,
}

/// One `type@v` an extension declares, as loaded: the schema and the
/// upcast's script read from its folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EventTypeDecl {
    pub event_type: String,
    pub v: u32,
    /// The payload's JSON Schema (from the declared file).
    #[specta(type = oxplow_domain::Json)]
    pub schema: Value,
    pub summary: String,
    /// The upcast script's path in the folder, required past v1.
    pub upcast: Option<String>,
    /// Its source.
    pub upcast_source: Option<String>,
    /// `file:line` of the declaration, for what's wrong with it later (a
    /// schema changed at a recorded version).
    pub declared_at: String,
}

impl EventTypeDecl {
    /// What the vocabulary registers for it.
    pub fn declared(&self) -> DeclaredEventType {
        DeclaredEventType {
            event_type: self.event_type.clone(),
            v: self.v,
            schema: self.schema.clone(),
            summary: self.summary.clone(),
            upcast: self
                .upcast_source
                .as_deref()
                .map(|script| starlark_upcast(&self.event_type, script)),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventTypesFile {
    #[serde(default)]
    types: Vec<serde_yaml::Value>,
    #[serde(default)]
    retention: Option<RetentionFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetentionFile {
    payload_days: Option<u32>,
    content_days: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TypeFile {
    #[serde(rename = "type")]
    event_type: String,
    v: u32,
    /// A JSON file in the folder.
    schema: String,
    summary: String,
    upcast: Option<String>,
}

/// Parse an `event_types:` block. Each type is checked the way the
/// vocabulary will register it (namespace, schema, version, upcast) and
/// its files read through `read`; a retention window may only be shorter
/// than the default; what's wrong is `file:line`.
pub fn parse_event_types(
    extension: &str,
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
    read: &dyn Fn(&str) -> Option<String>,
) -> (EventTypes, Vec<String>) {
    let block_line = key_line(manifest, "event_types");
    let block: EventTypesFile = match serde_yaml::from_value(value.clone()) {
        Ok(b) => b,
        Err(e) => {
            return (
                EventTypes::default(),
                vec![at(file, block_line, format!("event_types: {e}"))],
            )
        }
    };
    let mut errors = Vec::new();
    let retention = block.retention.and_then(|r| {
        let default = oxplow_domain::events::retention::PLUGIN_DEFAULT;
        let (p, c) = (default.payload_days, default.content_days);
        let window = EventRetention {
            payload_days: r.payload_days.unwrap_or(p as u32),
            content_days: r.content_days.unwrap_or(c as u32),
        };
        match oxplow_db::event_retention::check_declared(
            window.payload_days.into(),
            window.content_days.into(),
        ) {
            Ok(()) => Some(window),
            Err(e) => {
                let line = line_under(manifest, "event_types", "retention").or(block_line);
                errors.push(at(file, line, format!("event_types.retention: {e}")));
                None
            }
        }
    });
    // Registering into a scratch registry is the check: what it refuses,
    // the running vocabulary would.
    let mut scratch = EventSchemaRegistry::new();
    let mut out = Vec::new();
    for item in block.types {
        let t: TypeFile = match serde_yaml::from_value(item) {
            Ok(t) => t,
            Err(e) => {
                errors.push(at(file, block_line, format!("event type: {e}")));
                continue;
            }
        };
        let line = entry_line(manifest, "event_types", "type", &t.event_type).or(block_line);
        match decl_of(extension, t, file, line, read, &mut scratch) {
            Ok(d) => out.push(d),
            Err(e) => errors.push(at(file, line, e)),
        }
    }
    (
        EventTypes {
            types: out,
            retention,
        },
        errors,
    )
}

fn decl_of(
    extension: &str,
    t: TypeFile,
    file: &str,
    line: Option<usize>,
    read: &dyn Fn(&str) -> Option<String>,
    scratch: &mut EventSchemaRegistry,
) -> Result<EventTypeDecl, String> {
    let name = format!("event type `{}@{}`", t.event_type, t.v);
    let text = read(&t.schema)
        .ok_or_else(|| format!("{name}: schema `{}` isn't in the extension", t.schema))?;
    let schema: Value = serde_json::from_str(&text)
        .map_err(|e| format!("{name}: schema `{}` isn't JSON: {e}", t.schema))?;
    let upcast_source = match &t.upcast {
        None => None,
        Some(path) => {
            let script = read(path)
                .ok_or_else(|| format!("{name}: upcast `{path}` isn't in the extension"))?;
            oxplow_collect_plugin::runtime::check_starlark(path, &script)
                .map_err(|e| format!("{name}: upcast `{path}` {e}"))?;
            Some(script)
        }
    };
    let decl = EventTypeDecl {
        event_type: t.event_type,
        v: t.v,
        schema,
        summary: t.summary,
        upcast: t.upcast,
        upcast_source,
        declared_at: at(file, line, "").trim_end_matches(": ").to_string(),
    };
    scratch
        .register_declared(extension, decl.declared())
        .map_err(|e| e.to_string())?;
    Ok(decl)
}

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

    const FILE: &str = "oxplow/extensions/acme-pr/extension.yaml";

    fn parse(manifest: &str, files: &[(&str, &str)]) -> (EventTypes, Vec<String>) {
        let doc: serde_yaml::Value = serde_yaml::from_str(manifest).unwrap();
        let files: Vec<(String, String)> = files
            .iter()
            .map(|(p, b)| (p.to_string(), b.to_string()))
            .collect();
        parse_event_types("acme-pr", &doc["event_types"], FILE, manifest, &|rel| {
            files.iter().find(|(p, _)| p == rel).map(|(_, b)| b.clone())
        })
    }

    const SCHEMA: &str = r#"{"type": "object", "required": ["number"], "properties": {"number": {"type": "integer"}}}"#;

    #[test]
    fn a_declared_type_loads_with_its_schema() {
        let manifest = "name: acme-pr\nevent_types:\n  types:\n    - type: acme_pr.merged\n      v: 1\n      schema: merged.json\n      summary: A pull request merged.\n";
        let (declared, errors) = parse(manifest, &[("merged.json", SCHEMA)]);
        assert!(errors.is_empty(), "{errors:?}");
        let types = declared.types;
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].schema["required"], json!(["number"]));
        assert_eq!(types[0].declared_at, format!("{FILE}:4"));
    }

    #[test]
    fn a_malformed_declaration_is_an_error_at_its_line() {
        let manifest = "name: acme-pr\nevent_types:\n  types:\n    - type: acme_pr.merged\n      v: 1\n      schema: merged.json\n      summary: ok\n    - type: work_item.stolen\n      v: 1\n      schema: merged.json\n      summary: no\n    - type: acme_pr.closed\n      v: 1\n      schema: missing.json\n      summary: no\n    - type: acme_pr.opened\n      v: 2\n      schema: merged.json\n      summary: no\n";
        let (declared, errors) = parse(manifest, &[("merged.json", SCHEMA)]);
        assert_eq!(declared.types.len(), 1);
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(
            errors[0].starts_with(&format!("{FILE}:8:")) && errors[0].contains("core namespace"),
            "{}",
            errors[0]
        );
        assert!(
            errors[1].starts_with(&format!("{FILE}:12:")) && errors[1].contains("missing.json"),
            "{}",
            errors[1]
        );
        assert!(
            errors[2].starts_with(&format!("{FILE}:16:")) && errors[2].contains("upcast"),
            "{}",
            errors[2]
        );
        let (_, bad) = parse("name: acme-pr\nevent_types:\n  nope: 1\n", &[]);
        assert!(bad[0].starts_with(&format!("{FILE}:2:")), "{bad:?}");
    }

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
    fn a_retention_window_may_only_be_shorter_than_the_default() {
        let (declared, errors) = parse(
            "name: acme-pr\nevent_types:\n  retention: { payload_days: 7 }\n",
            &[],
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            declared.retention,
            Some(EventRetention {
                payload_days: 7,
                content_days: 14
            })
        );
        let (declared, errors) = parse(
            "name: acme-pr\nevent_types:\n  types: []\n  retention: { payload_days: 60 }\n",
            &[],
        );
        assert_eq!(declared.retention, None);
        assert!(
            errors.len() == 1
                && errors[0].starts_with(&format!("{FILE}:4:"))
                && errors[0].contains("60"),
            "{errors:?}"
        );
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
