//! The `.oxplow/project.yaml` key registry (`.context/commands.md`,
//! "config.*"). One source of truth for what the file may contain:
//! the JSON Schema generated from the file's own shape (`RawConfig`),
//! whose top-level properties are the keys. `write_project_config` uses
//! it to know which keys it owns, and the `config.*` commands use it to
//! list, describe and validate keys — so a field added to `RawConfig` is
//! automatically managed, documented and settable, and can never again be
//! the "unknown extra" that re-inserted a stale value over a fresh one.

use serde_json::Value;

use crate::{
    basename, parse_project_config, render_project_config, ConfigError, OxplowConfig, RawConfig,
};

/// Keys only a person may set. Each either runs a program (`lsp`,
/// `collection`, `acpAgents`, `extensionInstances`, `agents`, `gauges`), chooses the model that
/// reads the project (`ai`, `agentModels`), enables code (`extensions`),
/// or steers every agent (`agentPromptAppend` — an agent setting it could
/// persist instructions into all threads): an agent asking to change one
/// gets `NeedsConfirmation` and the person decides. Everything else is the
/// agent's to set through `config.set`. A key whose doc says it runs
/// programs or steers agents must be listed here (a test enforces it).
pub const HUMAN_ONLY_KEYS: &[&str] = &[
    "agents",
    "agentModels",
    "acpAgents",
    "extensionInstances",
    "ai",
    "lsp",
    "collection",
    "extensions",
    "gauges",
    "agentPromptAppend",
];

/// One key of `.oxplow/project.yaml`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ConfigKey {
    pub key: String,
    /// What the key is for, from the schema's field doc.
    pub doc: String,
    /// Only a person may set it (see [`HUMAN_ONLY_KEYS`]).
    pub human_only: bool,
    /// JSON Schema for the key's value (with the document's `$defs`).
    pub schema: Value,
}

/// The whole file's JSON Schema (draft 2020-12), from `RawConfig`.
pub fn project_config_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(RawConfig)).expect("schema serializes")
}

/// Every key, in the schema's order, with its doc, schema and whether it
/// is human-only.
pub fn config_keys() -> Vec<ConfigKey> {
    let root = project_config_schema();
    let defs = root.get("$defs").cloned();
    let Some(props) = root.get("properties").and_then(|p| p.as_object()) else {
        return Vec::new();
    };
    let mut keys: Vec<ConfigKey> = props
        .iter()
        .map(|(key, prop)| {
            let doc = prop
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            let mut schema = prop.clone();
            if let (Some(defs), Some(obj)) = (defs.as_ref(), schema.as_object_mut()) {
                obj.insert("$defs".into(), defs.clone());
                obj.remove("description");
            }
            ConfigKey {
                human_only: HUMAN_ONLY_KEYS.contains(&key.as_str()),
                key: key.clone(),
                doc,
                schema,
            }
        })
        .collect();
    keys.sort_by(|a, b| a.key.cmp(&b.key));
    keys
}

/// Whether `key` is one the file schema knows.
pub fn is_config_key(key: &str) -> bool {
    config_keys().iter().any(|k| k.key == key)
}

pub fn config_key(key: &str) -> Option<ConfigKey> {
    config_keys().into_iter().find(|k| k.key == key)
}

/// The value `key` has in the document `config` renders to: `None` when
/// the key is at its default (and so absent from the file).
pub fn key_value(config: &OxplowConfig, project_dir: &std::path::Path, key: &str) -> Option<Value> {
    let doc = render_project_config(config, &basename(project_dir));
    doc.get(key).map(yaml_to_json)
}

/// YAML → JSON through text. Serializing a `serde_json::Value` directly
/// into another format breaks when a dependency turns on serde_json's
/// `arbitrary_precision` (numbers become a private map); JSON text is
/// valid YAML, so the text is the safe bridge both ways.
fn yaml_to_json(v: &serde_yaml::Value) -> Value {
    let text = serde_json::to_string(v).expect("yaml value serializes as json");
    serde_json::from_str(&text).expect("json text parses")
}

fn json_to_yaml(v: &Value) -> Result<serde_yaml::Value, ConfigError> {
    let text = serde_json::to_string(v).expect("json value serializes");
    Ok(serde_yaml::from_str(&text)?)
}

/// `config` with `key` set to `value` (or removed when `None`), taken
/// through the same document → validation path a hand edit takes, so a
/// command's write can't produce a file the loader would reject.
pub fn with_key(
    config: &OxplowConfig,
    project_dir: &std::path::Path,
    key: &str,
    value: Option<&Value>,
) -> Result<OxplowConfig, ConfigError> {
    if !is_config_key(key) {
        return Err(ConfigError::Invalid(format!(
            "`{key}` is not a project.yaml key; see config.list_keys"
        )));
    }
    let fallback = basename(project_dir);
    let mut doc = render_project_config(config, &fallback);
    match value {
        Some(v) => {
            doc.insert(serde_yaml::Value::String(key.to_string()), json_to_yaml(v)?);
        }
        None => {
            doc.remove(serde_yaml::Value::String(key.to_string()));
        }
    }
    parse_project_config(serde_yaml::Value::Mapping(doc), &fallback)
}

#[cfg(test)]
mod tests {

    /// P6.H1: the effective-config view lists every key the file schema
    /// knows, each with a value (its default when unset).
    #[test]
    fn every_schema_key_has_an_effective_value() {
        let config = crate::default_config("demo".into());
        let entries: std::collections::BTreeSet<&str> = crate::config_entries(&config, "demo")
            .iter()
            .map(|e| e.key)
            .collect();
        let keys: std::collections::BTreeSet<String> =
            config_keys().into_iter().map(|k| k.key).collect();
        let entries: std::collections::BTreeSet<String> =
            entries.into_iter().map(str::to_string).collect();
        assert_eq!(entries, keys);
        let retention = crate::config_entries(&config, "demo")
            .into_iter()
            .find(|e| e.key == "snapshotRetentionDays")
            .unwrap();
        assert!(!retention.set);
        assert_eq!(
            retention.value,
            serde_yaml::Value::from(crate::DEFAULT_SNAPSHOT_RETENTION_DAYS)
        );
    }
    use super::*;
    use serde_json::json;

    #[test]
    fn every_file_key_is_registered_documented_and_the_human_only_list_is_real() {
        let keys = config_keys();
        let names: Vec<&str> = keys.iter().map(|k| k.key.as_str()).collect();
        for expected in [
            "agents",
            "projectName",
            "lsp",
            "metricRetentionDays",
            "metricDetailMaxPerProducer",
            "metricDetailRetentionDays",
            "iconTint",
            "zones",
            "ai",
            "extensions",
        ] {
            assert!(
                names.contains(&expected),
                "{expected} missing from {names:?}"
            );
        }
        for k in &keys {
            assert!(!k.doc.is_empty(), "`{}` has no doc", k.key);
            assert!(
                k.schema.get("$defs").is_some(),
                "`{}` schema lacks $defs",
                k.key
            );
        }
        for h in HUMAN_ONLY_KEYS {
            assert!(names.contains(h), "human-only key `{h}` is not a file key");
        }
        assert!(config_key("ai").unwrap().human_only);
        assert!(!config_key("zones").unwrap().human_only);
        assert!(!is_config_key("nope"));
    }

    #[test]
    fn every_key_that_runs_programs_or_steers_agents_is_human_only() {
        for k in config_keys() {
            let dangerous = k.doc.contains("Runs programs") || k.doc.contains("Steers every agent");
            if dangerous {
                assert!(
                    k.human_only,
                    "`{}` runs programs or steers agents: {}",
                    k.key, k.doc
                );
            }
        }
        assert!(config_key("gauges").unwrap().human_only);
        assert!(config_key("agentPromptAppend").unwrap().human_only);
    }

    #[test]
    fn with_key_sets_validates_and_unsets_through_the_loader_path() {
        let dir = std::path::Path::new("/tmp/proj");
        let base = crate::default_config("proj".into());
        assert_eq!(key_value(&base, dir, "zones"), None);
        let with_zones = with_key(
            &base,
            dir,
            "zones",
            Some(&json!([{ "match": "src/**", "zone": "core" }])),
        )
        .unwrap();
        assert_eq!(with_zones.zones.len(), 1);
        assert_eq!(with_zones.zones[0].zone, "core");
        assert_eq!(
            key_value(&with_zones, dir, "zones").unwrap()[0]["zone"],
            "core"
        );
        // The loader's validation applies: a reserved label is refused.
        let err = with_key(
            &base,
            dir,
            "zones",
            Some(&json!([{ "match": "src/**", "zone": "other" }])),
        )
        .unwrap_err();
        assert!(err.to_string().contains("reserved"), "{err}");
        // A scalar key, then back to its default.
        let days = with_key(&base, dir, "metricRetentionDays", Some(&json!(30))).unwrap();
        assert_eq!(days.metric_retention_days, 30);
        let reset = with_key(&days, dir, "metricRetentionDays", None).unwrap();
        assert_eq!(reset.metric_retention_days, base.metric_retention_days);
        assert!(with_key(&base, dir, "nope", Some(&json!(1))).is_err());
        assert!(with_key(&base, dir, "metricRetentionDays", Some(&json!("x"))).is_err());
    }
}
