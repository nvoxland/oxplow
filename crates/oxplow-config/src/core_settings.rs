//! Core's declarations in the shared settings registry.
//!
//! Persisted documents use registered, fully qualified keys. `RawConfig`
//! remains the runtime projection of those values, not a storage format.
//! Fixed object fields are independent settings; maps and arrays remain
//! indivisible values, so an override never accidentally merges two levels.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::settings::{Origin, SettingDefinition, SettingsError, SettingsStore};

/// A core setting and its path in the runtime configuration document.
/// Settings with no runtime path are consumed directly through the service.
#[derive(Clone, Debug)]
pub struct CoreBinding {
    pub definition: SettingDefinition,
    pub path: Vec<String>,
}

/// The namespaced key corresponding to an old runtime-document path.
/// This is a projection helper, not an alias accepted by config commands.
pub fn canonical_key(path: &str) -> String {
    let suffix = match path {
        "snapshotRetentionDays" => "snapshots.retentionDays",
        "snapshotMaxFileBytes" => "snapshots.maxFileBytes",
        "snapshotTurnBudgetMs" => "snapshots.turnBudgetMs",
        "metricRetentionDays" => "metrics.retentionDays",
        "metricDetailMaxPerProducer" => "metrics.detailMaxPerProducer",
        "metricDetailRetentionDays" => "metrics.detailRetentionDays",
        other => other,
    };
    format!("oxplow.{suffix}")
}

/// All core definitions, with defaults evaluated for this project's name.
pub fn definitions(fallback_name: &str) -> Vec<SettingDefinition> {
    bindings(fallback_name)
        .into_iter()
        .map(|binding| binding.definition)
        .collect()
}

/// Core fields and their declaration metadata, generated from the runtime
/// schema so a new field cannot silently become an unregistered setting.
pub fn bindings(fallback_name: &str) -> Vec<CoreBinding> {
    let fallback_name = if fallback_name.trim().is_empty() {
        "Project"
    } else {
        fallback_name
    };
    let root_schema = crate::keys::project_config_schema();
    let defaults = default_document(fallback_name);
    let mut out = Vec::new();
    for (name, schema) in root_schema["properties"]
        .as_object()
        .expect("the core schema has properties")
    {
        collect(
            &root_schema,
            schema,
            &defaults,
            vec![name.clone()],
            None,
            &mut out,
        );
    }
    out.push(extra(
        "newThreadSession",
        "New thread session",
        "What a new thread starts with: ask, none, or an agent harness key.",
        "agents",
        json!({ "type": "string", "minLength": 1 }),
        Some(json!("ask")),
    ));
    out.push(extra(
        "enableExtensions",
        "Additional extensions",
        "Extensions enabled in addition to the configured extension list.",
        "extensions",
        json!({ "type": "array", "items": { "type": "string", "minLength": 1 }, "uniqueItems": true }),
        Some(json!([])),
    ));
    out.push(extra(
        "ai.providers",
        "AI providers",
        "Connections to model providers. API keys are separate secret settings.",
        "ai",
        json!({
            "type": "array",
            "items": {
                "type": "object", "additionalProperties": false,
                "required": ["id", "kind"],
                "properties": {
                    "id": { "type": "string", "minLength": 1 },
                    "kind": { "type": "string", "minLength": 1 },
                    "baseUrl": { "type": ["string", "null"] }
                }
            }
        }),
        Some(json!([])),
    ));
    out.sort_by(|a, b| a.definition.key.cmp(&b.definition.key));
    out
}

/// Build the internal runtime document from resolved non-default values.
/// Unknown and extension keys are ignored by this core-only projection.
/// The caller must first validate values through the shared registry.
pub fn document_from_values(
    fallback_name: &str,
    values: &BTreeMap<String, Value>,
) -> serde_yaml::Value {
    let mut document = json!({});
    for binding in bindings(fallback_name) {
        if binding.path.is_empty() {
            continue;
        }
        if let Some(value) = values.get(&binding.definition.key) {
            insert_path(&mut document, &binding.path, value.clone());
        }
    }
    crate::to_yaml(&document)
}

/// Resolve explicit overrides and project them into the core runtime shape.
/// Built-in defaults remain the runtime loader's defaults: notably, an absent
/// agents list enables registered harnesses, whereas an explicit [] is invalid.
pub fn project_document(store: &SettingsStore) -> Result<serde_yaml::Value, SettingsError> {
    let fallback = store
        .project_dir()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut values = BTreeMap::new();
    for binding in bindings(&fallback) {
        if binding.path.is_empty() {
            continue;
        }
        let setting = store.get(&binding.definition.key, None)?;
        if setting.origin != Some(Origin::Default) {
            if let Some(value) = setting.value {
                values.insert(binding.definition.key, value);
            }
        }
    }
    Ok(document_from_values(&fallback, &values))
}

/// Split an internal runtime-document field into its registered values.
/// This lets existing domain operations write their individual settings
/// through the shared service without persisting an old-format object.
pub fn values_for_runtime_field(
    fallback_name: &str,
    field: &str,
    value: &Value,
) -> BTreeMap<String, Value> {
    let mut document = json!({});
    document[field] = value.clone();
    bindings(fallback_name)
        .into_iter()
        .filter(|binding| binding.path.first().is_some_and(|first| first == field))
        .filter_map(|binding| {
            value_at(&document, &binding.path)
                .cloned()
                .map(|value| (binding.definition.key, value))
        })
        .collect()
}

fn default_document(fallback_name: &str) -> Value {
    let config = crate::default_config(fallback_name.to_string());
    let mut document = json!({});
    for entry in crate::config_entries(&config, fallback_name) {
        document[entry.key] = serde_json::from_str(
            &serde_json::to_string(&entry.value).expect("a config value serializes"),
        )
        .expect("a config value is JSON");
    }
    // config_entries intentionally omits default-valued fields inside these
    // objects when rendering YAML. Declarations still need their real defaults.
    document["scripts"] = json!({
        "maxConcurrent": config.scripts.max_concurrent,
        "memoryMb": config.scripts.memory_bytes / (1024 * 1024),
    });
    document["generated"] = json!({"include": [], "exclude": []});
    document["lsp"] = json!({"servers":[]});
    document["testing"] = json!({
        "runPatterns": [], "analysisPatterns": [], "testPaths": [],
    });
    document
}

fn collect(
    root_schema: &Value,
    schema: &Value,
    defaults: &Value,
    path: Vec<String>,
    parent_description: Option<&str>,
    out: &mut Vec<CoreBinding>,
) {
    let schema = resolved_schema(root_schema, schema);
    let description = schema
        .get("description")
        .and_then(Value::as_str)
        .or(parent_description)
        .unwrap_or("");
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        if !properties.is_empty() {
            for (name, property) in properties {
                let mut child_path = path.clone();
                child_path.push(name.clone());
                collect(
                    root_schema,
                    property,
                    defaults,
                    child_path,
                    Some(description),
                    out,
                );
            }
            return;
        }
    }

    let joined = path.join(".");
    let mut default = value_at(defaults, &path).filter(|v| !v.is_null()).cloned();
    // An absent agents list means all registered harnesses. [] as an explicit
    // value would disable all of them and is rejected by the runtime contract.
    if joined == "agents" {
        default = None;
    }
    let mut value_schema = schema.clone();
    if let Some(object) = value_schema.as_object_mut() {
        object.remove("description");
        object.remove("default");
        if let Some(defs) = root_schema.get("$defs") {
            object.insert("$defs".into(), defs.clone());
        }
    }
    strengthen_schema(&joined, &mut value_schema);
    out.push(CoreBinding {
        definition: SettingDefinition {
            key: canonical_key(&joined),
            title: title(&joined),
            description: if description.is_empty() {
                title(&joined)
            } else {
                description.to_string()
            },
            category: category(&joined).to_string(),
            schema: value_schema,
            default,
            secret: false,
            order: 0,
            ui: if matches!(joined.as_str(), "agentPromptAppend" | "testing.agentHint") {
                json!({ "editor": "multiline" })
            } else {
                json!({})
            },
            restart_required: joined == "enableBundledExtensions",
            owner: "core".into(),
        },
        path,
    });
}

fn resolved_schema<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        if let Some(pointer) = reference.strip_prefix('#') {
            if let Some(resolved) = root.pointer(pointer) {
                return resolved_schema(root, resolved);
            }
        }
    }
    // RawConfig's optional fields use anyOf(T, null). An optional fixed
    // object is flattened into its independent fields; scalar optional
    // schemas retain null as a valid value.
    if let Some(variants) = schema.get("anyOf").and_then(Value::as_array) {
        if let Some(object) = variants
            .iter()
            .map(|variant| resolved_schema(root, variant))
            .find(|variant| variant.get("properties").is_some())
        {
            return object;
        }
    }
    schema
}

fn value_at<'a>(root: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(root, |value, key| value.get(key))
}

fn insert_path(root: &mut Value, path: &[String], value: Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut current = root;
    for parent in parents {
        current = current
            .as_object_mut()
            .expect("runtime config paths contain objects")
            .entry(parent)
            .or_insert_with(|| json!({}));
    }
    current[last] = value;
}

fn extra(
    suffix: &str,
    title: &str,
    description: &str,
    category: &str,
    schema: Value,
    default: Option<Value>,
) -> CoreBinding {
    CoreBinding {
        definition: SettingDefinition {
            key: canonical_key(suffix),
            title: title.into(),
            description: description.into(),
            category: category.into(),
            schema,
            default,
            secret: false,
            order: 0,
            ui: json!({}),
            restart_required: false,
            owner: "core".into(),
        },
        path: Vec::new(),
    }
}

fn category(path: &str) -> &'static str {
    match path.split('.').next().unwrap_or(path) {
        "agents" | "agentConfig" | "agentPromptAppend" | "injectSessionContext" => "agents",
        "ai" => "ai",
        "generated" => "files",
        "lsp" => "languageServers",
        "testing" => "testing",
        "scripts" => "scripts",
        "extensions"
        | "enableExtensions"
        | "disableExtensions"
        | "extensionSources"
        | "enableBundledExtensions" => "extensions",
        "activeProviders" | "extensionInstances" | "replacements" => "integrations",
        "eventRetention" => "data",
        key if key.starts_with("snapshot") => "snapshots",
        key if key.starts_with("metric") => "metrics",
        _ => "project",
    }
}

fn title(path: &str) -> String {
    let field = path.rsplit('.').next().unwrap_or(path);
    let mut out = String::new();
    for (index, ch) in field.chars().enumerate() {
        if index == 0 {
            out.extend(ch.to_uppercase());
        } else if ch.is_uppercase() {
            out.push(' ');
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn strengthen_schema(path: &str, schema: &mut Value) {
    let integer_minimum = match path {
        "snapshotRetentionDays"
        | "metricRetentionDays"
        | "metricDetailMaxPerProducer"
        | "metricDetailRetentionDays" => Some(0),
        "snapshotMaxFileBytes" => Some(1024),
        "snapshotTurnBudgetMs" => Some(crate::MIN_SNAPSHOT_TURN_BUDGET_MS),
        "scripts.maxConcurrent" => Some(1),
        "scripts.memoryMb" => Some(64),
        _ => None,
    };
    if let Some(minimum) = integer_minimum {
        *schema = json!({ "type": "integer", "minimum": minimum });
    } else if path == "agents" {
        *schema = json!({ "type": "array", "minItems": 1, "uniqueItems": true, "items": { "type": "string", "minLength": 1 } });
    } else if path == "projectName" {
        *schema = json!({ "type": "string", "minLength": 1, "pattern": "\\S" });
    } else if path == "iconTint" {
        *schema =
            json!({ "type": ["string", "null"], "pattern": "^#?([a-fA-F0-9]{3}|[a-fA-F0-9]{6})$" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_fields_are_registered_individually_but_dynamic_maps_are_atomic() {
        let all = definitions("demo");
        let find = |key: &str| all.iter().find(|definition| definition.key == key).unwrap();
        assert_eq!(
            find("oxplow.snapshots.retentionDays").default,
            Some(json!(7))
        );
        assert_eq!(find("oxplow.scripts.memoryMb").default, Some(json!(512)));
        assert_eq!(find("oxplow.generated.exclude").default, Some(json!([])));
        assert_eq!(find("oxplow.ai.roles").default, Some(json!({})));
        assert_eq!(find("oxplow.ai.roles").category, "ai");
        assert!(all
            .iter()
            .all(|definition| definition.key != "oxplow.scripts"));
        assert!(all.iter().all(|definition| definition.key != "oxplow.ai"));
        assert!(all.iter().all(|definition| !definition.title.is_empty()));
        assert!(all.iter().all(|definition| !definition.category.is_empty()));
    }

    #[test]
    fn runtime_projection_reconstitutes_independent_fields_without_merging_values() {
        let values = BTreeMap::from([
            ("oxplow.snapshots.retentionDays".into(), json!(14)),
            ("oxplow.testing.command".into(), json!("make test")),
            ("oxplow.testing.runPatterns".into(), json!(["pytest"])),
            ("oxplow.scripts.memoryMb".into(), json!(256)),
            (
                "oxplow.ai.roles".into(),
                json!({"main": {"provider": "local", "model": "a"}}),
            ),
            ("oxtask.tasks.workflow".into(), json!("agent_tracks")),
        ]);
        let document = document_from_values("demo", &values);
        let config = crate::parse_project_config(document, "demo").unwrap();
        assert_eq!(config.snapshot_retention_days, 14);
        assert_eq!(config.testing.command.as_deref(), Some("make test"));
        assert_eq!(config.testing.run_patterns, ["pytest"]);
        assert_eq!(config.scripts.memory_bytes, 256 * 1024 * 1024);
        assert_eq!(config.ai_roles["main"].model, "a");
        assert!(config.agents.is_empty());
    }

    #[test]
    fn existing_runtime_objects_split_into_their_registered_options() {
        let values = values_for_runtime_field(
            "demo",
            "testing",
            &json!({ "command": "make test", "runPatterns": ["pytest"] }),
        );
        assert_eq!(values.len(), 2);
        assert_eq!(values["oxplow.testing.command"], "make test");
        assert_eq!(values["oxplow.testing.runPatterns"], json!(["pytest"]));
        let maps =
            values_for_runtime_field("demo", "activeProviders", &json!({"snapshots":"hashes"}));
        assert_eq!(maps.len(), 1);
        assert_eq!(
            maps["oxplow.activeProviders"],
            json!({"snapshots":"hashes"})
        );
    }

    #[test]
    fn every_project_schema_property_has_a_projection_binding() {
        let all = bindings("demo");
        for key in crate::keys::config_keys() {
            assert!(
                all.iter()
                    .any(|binding| binding.path.first() == Some(&key.key)),
                "{}",
                key.key
            );
        }
        let unique: std::collections::BTreeSet<_> =
            all.iter().map(|binding| &binding.definition.key).collect();
        assert_eq!(unique.len(), all.len());
    }

    #[test]
    fn every_core_default_validates_against_its_registered_schema() {
        let mut registry = crate::settings::SettingsRegistry::default();
        registry.register_many(definitions("demo")).unwrap();
    }
}
