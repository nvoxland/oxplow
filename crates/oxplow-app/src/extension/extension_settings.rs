//! Extension declarations projected onto the shared settings registry.
//!
//! The declaration remains a convenient nested schema for `config('/pointer')`
//! consumers. Storage and precedence are per registered key: no object merge
//! happens here, and SQL/condition projections never resolve secret values.

use std::collections::BTreeMap;

use oxplow_config::settings::{Origin, SettingDefinition, SettingsRegistry, SettingsStore};
use serde_json::{Map, Value};

use super::extensions::{config_schema::ConfigurationFile, Extension};

const MAX_DEPTH: usize = 64;
const OWNER_PREFIX: &str = "extension/";

#[derive(Debug, thiserror::Error)]
pub(crate) enum ExtensionSettingsError {
    #[error("invalid extension setting declaration: {0}")]
    Declaration(&'static str),
    #[error("extension settings could not be resolved")]
    Resolution,
    #[error("extension settings conflict with another registered definition")]
    Registry,
}

/// A declared collector credential is a normal secret setting at every level.
pub(crate) fn credential_definition(namespace: &str, name: &str) -> SettingDefinition {
    SettingDefinition {
        key: format!("{namespace}.credentials.{}", hex::encode(name)),
        title: name.to_owned(),
        description: "Credential used by the extension's collectors.".to_owned(),
        category: namespace.to_owned(),
        schema: serde_json::json!({"type":"string"}),
        default: None,
        secret: true,
        order: 0,
        ui: serde_json::json!({}),
        restart_required: false,
        owner: format!("{OWNER_PREFIX}{namespace}"),
    }
}

struct Binding {
    definition: SettingDefinition,
    path: Vec<String>,
}

/// Translate fixed schema fields into independent settings. A schema node with
/// `x-atomic: true`, a dynamic object, or an array is one complete setting.
/// Metadata annotations are intentionally distinct from validation keywords.
pub(crate) fn definitions(
    extension: &Extension,
) -> Result<Vec<SettingDefinition>, ExtensionSettingsError> {
    let Some(declaration) = &extension.configuration else {
        return Ok(Vec::new());
    };
    definitions_for(&extension.namespace, declaration)
}

pub(crate) fn definitions_for(
    namespace: &str,
    declaration: &ConfigurationFile,
) -> Result<Vec<SettingDefinition>, ExtensionSettingsError> {
    Ok(bindings_for(namespace, declaration)?
        .into_iter()
        .map(|binding| binding.definition)
        .collect())
}

/// Replace the extension-owned portion of a registry in one swap. Shipped
/// extensions may share a namespace and therefore a merged declaration; those
/// identical definitions count once. Core definitions keep their own owner.
pub(crate) fn replace_extensions(
    registry: &mut SettingsRegistry,
    extensions: &[Extension],
) -> Result<(), ExtensionSettingsError> {
    let mut entries = BTreeMap::new();
    for extension in extensions.iter().filter(|extension| extension.enabled) {
        for definition in definitions(extension)? {
            if let Some(previous) = entries.get(&definition.key) {
                if serde_json::to_value(previous).ok() != serde_json::to_value(&definition).ok() {
                    return Err(ExtensionSettingsError::Registry);
                }
            } else {
                entries.insert(definition.key.clone(), definition);
            }
        }
    }
    let mut candidate = registry.clone();
    let owners: Vec<String> = candidate
        .definitions()
        .filter(|definition| definition.owner.starts_with(OWNER_PREFIX))
        .map(|definition| definition.owner.clone())
        .collect();
    for owner in owners {
        candidate.remove_owner(&owner);
    }
    candidate
        .register_many(entries.into_values())
        .map_err(|_| ExtensionSettingsError::Registry)?;
    *registry = candidate;
    Ok(())
}

/// Reconstruct the extension's nested view from independently resolved values.
/// With `public`, omit secret settings before any secure-store read: SQL and
/// contribution conditions must never resolve a secret into a published value.
/// Only an authorized host consumer should request the secret-bearing view.
pub(crate) fn values_for(
    namespace: &str,
    declaration: &ConfigurationFile,
    store: &SettingsStore,
    public: bool,
) -> Result<Value, ExtensionSettingsError> {
    let mut root = Value::Object(Map::new());
    let bindings: Vec<_> = bindings_for(namespace, declaration)?
        .into_iter()
        .filter(|binding| !(public && binding.definition.secret))
        .collect();
    let keys: Vec<_> = bindings
        .iter()
        .map(|binding| binding.definition.key.as_str())
        .collect();
    let settings = store
        .get_many(&keys)
        .map_err(|_| ExtensionSettingsError::Resolution)?;
    for (binding, setting) in bindings.iter().zip(settings) {
        if let Some(value) = setting.value {
            insert(&mut root, &binding.path, value)?;
        }
    }
    if !public {
        let schema = declaration
            .prepare()
            .map_err(|_| ExtensionSettingsError::Resolution)?;
        if !schema.validate_resolved(&root).is_empty() {
            return Err(ExtensionSettingsError::Resolution);
        }
    }
    Ok(root)
}

/// Origins use the declaration's JSON pointers, matching the public SQL and
/// condition projection. Objects/arrays have one origin at their setting root.
pub(crate) fn origins_for(
    namespace: &str,
    declaration: &ConfigurationFile,
    store: &SettingsStore,
    public: bool,
) -> Result<BTreeMap<String, Origin>, ExtensionSettingsError> {
    let mut origins = BTreeMap::new();
    let bindings: Vec<_> = bindings_for(namespace, declaration)?
        .into_iter()
        .filter(|binding| !(public && binding.definition.secret))
        .collect();
    let keys: Vec<_> = bindings
        .iter()
        .map(|binding| binding.definition.key.as_str())
        .collect();
    let settings = store
        .inspect_many(&keys)
        .map_err(|_| ExtensionSettingsError::Resolution)?;
    for (binding, setting) in bindings.iter().zip(settings) {
        if let Some(origin) = setting.origin {
            origins.insert(pointer(&binding.path), origin);
        }
    }
    Ok(origins)
}

/// Preserve ordinary segments, reserving one prefix for reversible UTF-8 hex
/// escapes. Encoding the entire unusual segment keeps dots distinct from paths.
use oxplow_config::settings::key_segment;

fn insert(root: &mut Value, path: &[String], value: Value) -> Result<(), ExtensionSettingsError> {
    let Some((key, parents)) = path.split_last() else {
        return Err(ExtensionSettingsError::Declaration("empty key"));
    };
    let mut at = root;
    for parent in parents {
        at = at
            .as_object_mut()
            .ok_or(ExtensionSettingsError::Declaration("overlapping keys"))?
            .entry(parent.clone())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    at.as_object_mut()
        .ok_or(ExtensionSettingsError::Declaration("overlapping keys"))?
        .insert(key.clone(), value);
    Ok(())
}

fn bindings_for(
    namespace: &str,
    declaration: &ConfigurationFile,
) -> Result<Vec<Binding>, ExtensionSettingsError> {
    let mut out = Vec::new();
    let properties = declaration
        .schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or(ExtensionSettingsError::Declaration(
            "object properties required",
        ))?;
    for (key, schema) in properties {
        collect(
            namespace,
            declaration,
            schema,
            &mut vec![key.clone()],
            None,
            &mut out,
        )?;
    }
    Ok(out)
}

fn collect(
    namespace: &str,
    declaration: &ConfigurationFile,
    schema: &Value,
    path: &mut Vec<String>,
    category: Option<&str>,
    out: &mut Vec<Binding>,
) -> Result<(), ExtensionSettingsError> {
    if path.len() > MAX_DEPTH {
        return Err(ExtensionSettingsError::Declaration("nesting limit"));
    }
    let schema = dereference(&declaration.schema, schema)?;
    let category = text(&schema, "x-category").or(category);
    let secret = flag(&schema, "x-secret")?;
    let properties = schema.get("properties").and_then(Value::as_object);
    let dynamic = schema
        .get("additionalProperties")
        .is_some_and(|additional| additional != &Value::Bool(false));
    if schema.get("type").and_then(Value::as_str) == Some("object")
        && !dynamic
        && !secret
        && !flag(&schema, "x-atomic")?
    {
        if let Some(properties) = properties.filter(|properties| !properties.is_empty()) {
            for (key, property) in properties {
                path.push(key.clone());
                collect(namespace, declaration, property, path, category, out)?;
                path.pop();
            }
            return Ok(());
        }
    }

    let mut schema_with_defs = schema.clone();
    if let Some(definitions) = declaration.schema.get("$defs") {
        schema_with_defs
            .as_object_mut()
            .ok_or(ExtensionSettingsError::Declaration(
                "schema must be an object",
            ))?
            .insert("$defs".into(), definitions.clone());
    }
    let pointer = pointer(path);
    let default = declaration.defaults.pointer(&pointer).cloned();
    if secret && default.is_some() {
        return Err(ExtensionSettingsError::Declaration(
            "secret defaults are forbidden",
        ));
    }
    let title = text(&schema, "title")
        .map(str::to_owned)
        .unwrap_or_else(|| humanize(path.last().expect("a setting has a path")));
    let order = match schema.get("x-order") {
        None => 0,
        Some(value) => value
            .as_i64()
            .and_then(|number| i32::try_from(number).ok())
            .ok_or(ExtensionSettingsError::Declaration("invalid ordering"))?,
    };
    let ui = schema
        .get("x-ui")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    if !ui.is_object() {
        return Err(ExtensionSettingsError::Declaration(
            "UI metadata must be an object",
        ));
    }
    out.push(Binding {
        definition: SettingDefinition {
            key: format!(
                "{}.{}",
                namespace,
                path.iter()
                    .map(|segment| key_segment(segment))
                    .collect::<Vec<_>>()
                    .join(".")
            ),
            title,
            description: text(&schema, "description").unwrap_or_default().to_owned(),
            category: category.unwrap_or(namespace).to_owned(),
            schema: schema_with_defs,
            default,
            secret,
            order,
            ui,
            restart_required: flag(&schema, "x-restart-required")?,
            owner: format!("{OWNER_PREFIX}{namespace}"),
        },
        path: path.clone(),
    });
    Ok(())
}

fn dereference(root: &Value, schema: &Value) -> Result<Value, ExtensionSettingsError> {
    let mut schema = schema.clone();
    for _ in 0..MAX_DEPTH {
        let Some(reference) = schema.get("$ref").and_then(Value::as_str) else {
            return Ok(schema);
        };
        let reference = reference
            .strip_prefix('#')
            .filter(|reference| reference.starts_with('/'))
            .ok_or(ExtensionSettingsError::Declaration(
                "only local schema references are supported",
            ))?;
        let referenced = root.pointer(reference).and_then(Value::as_object).ok_or(
            ExtensionSettingsError::Declaration("unresolved schema reference"),
        )?;
        let mut resolved = referenced.clone();
        for (key, value) in schema
            .as_object()
            .ok_or(ExtensionSettingsError::Declaration(
                "schema must be an object",
            ))?
        {
            if key != "$ref" {
                resolved.insert(key.clone(), value.clone());
            }
        }
        schema = Value::Object(resolved);
    }
    Err(ExtensionSettingsError::Declaration(
        "schema reference depth limit",
    ))
}

fn text<'a>(schema: &'a Value, key: &str) -> Option<&'a str> {
    schema.get(key).and_then(Value::as_str)
}

fn pointer(path: &[String]) -> String {
    path.iter()
        .map(|segment| format!("/{}", segment.replace('~', "~0").replace('/', "~1")))
        .collect()
}

fn flag(schema: &Value, key: &str) -> Result<bool, ExtensionSettingsError> {
    match schema.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(ExtensionSettingsError::Declaration(
            "invalid boolean metadata",
        )),
    }
}

fn humanize(key: &str) -> String {
    let mut title = String::new();
    for (position, ch) in key.chars().enumerate() {
        if ch.is_uppercase() && position > 0 {
            title.push(' ');
        }
        if position == 0 {
            title.extend(ch.to_uppercase());
        } else {
            title.extend(ch.to_lowercase());
        }
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_config::settings::{Level, SecretStore};
    use serde_json::json;
    use std::sync::{Arc, RwLock};

    struct NoSecrets;

    impl SecretStore for NoSecrets {
        fn get(&self, _: &str) -> Result<Option<String>, String> {
            panic!("a public projection must never read the keychain")
        }

        fn set(&self, _: &str, _: &str) -> Result<(), String> {
            panic!("these tests never store a secret")
        }

        fn delete(&self, _: &str) -> Result<(), String> {
            panic!("these tests never delete a secret")
        }
    }

    fn extension(schema: Value, defaults: Value) -> Extension {
        let mut extension = super::super::extensions::empty_extension("example", "", "project");
        extension.enabled = true;
        extension.configuration = Some(ConfigurationFile {
            version: 1,
            schema,
            defaults,
        });
        extension
    }

    fn projected(extension: &Extension, store: &SettingsStore, public: bool) -> Value {
        values_for(
            &extension.namespace,
            extension.configuration.as_ref().unwrap(),
            store,
            public,
        )
        .unwrap()
    }

    #[test]
    fn fixed_fields_are_independent_but_structured_values_remain_atomic() {
        let extension = extension(
            json!({"type":"object", "properties": {
                "tasks": {"type":"object", "additionalProperties":false,
                    "x-category":"tasks.workflow", "properties": {
                        "workflow": {"type":"string", "enum":["off","agent_tracks"],
                            "title":"Task workflow", "description":"How tasks are tracked.",
                            "x-order":4,"x-ui":{"editor":"select"}},
                        "done": {"type":"string", "x-ui":{"multiline":true}}
                    }},
                "ordered": {"type":"array", "items":{"type":"string"}},
                "servers": {"type":"object", "additionalProperties":{"type":"string"}},
                "point": {"type":"object", "x-atomic":true,"properties": {
                    "x":{"type":"number"},"y":{"type":"number"}
                }}
            }}),
            json!({"tasks":{"workflow":"agent_tracks","done":""},"ordered":[],"servers":{},"point":{"x":0,"y":0}}),
        );
        let definitions = definitions(&extension).unwrap();
        assert_eq!(definitions.len(), 5);
        let workflow = definitions
            .iter()
            .find(|setting| setting.key == "example.tasks.workflow")
            .unwrap();
        assert_eq!(workflow.title, "Task workflow");
        assert_eq!(workflow.category, "tasks.workflow");
        assert_eq!(workflow.default, Some(json!("agent_tracks")));
        assert_eq!(workflow.order, 4);
        assert_eq!(workflow.ui, json!({"editor":"select"}));
        assert!(definitions
            .iter()
            .any(|setting| setting.key == "example.point"));
    }

    #[test]
    fn local_schema_references_keep_metadata_and_nested_defaults() {
        let extension = extension(
            json!({"type":"object", "$defs":{"choice":{"type":"string","enum":["a","b"]}},
                "properties":{"choice":{"$ref":"#/$defs/choice", "title":"Choose"}}}),
            json!({"choice":"b"}),
        );
        let definition = definitions(&extension).unwrap().remove(0);
        assert_eq!(definition.title, "Choose");
        assert_eq!(definition.schema["enum"], json!(["a", "b"]));
        assert_eq!(definition.default, Some(json!("b")));
    }

    #[test]
    fn secrets_have_no_manifest_defaults() {
        let declaration = json!({"type":"object","properties":{
            "token":{"type":"string","x-secret":true}
        }});
        let extension = extension(declaration.clone(), json!({}));
        assert!(definitions(&extension).unwrap()[0].secret);
        let mut invalid = extension;
        invalid.configuration.as_mut().unwrap().defaults = json!({"token":"do not retain"});
        assert!(definitions(&invalid).is_err());
    }

    #[test]
    fn unusual_property_names_have_distinct_canonical_keys_and_original_pointers() {
        let extension = extension(
            json!({"type":"object","properties": {
                "api/url":{"type":"string"}, "my key":{"type":"integer"},
                "a.b":{"type":"string"}, "a":{"type":"object","properties":{"b":{"type":"string"}}},
                "__hex_612e62":{"type":"string"}
            }}),
            json!({"api/url":"old", "my key":1, "a.b":"flat", "a":{"b":"nested"}, "__hex_612e62":"literal"}),
        );
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let mut registry = SettingsRegistry::default();
        replace_extensions(&mut registry, std::slice::from_ref(&extension)).unwrap();
        let store = SettingsStore::new(
            project.path().into(),
            global.path().into(),
            Arc::new(RwLock::new(registry)),
            Arc::new(NoSecrets),
        );
        store
            .set(
                "example.__hex_6170692f75726c",
                json!("new"),
                oxplow_config::settings::Level::Project,
            )
            .unwrap();
        store
            .set(
                "example.__hex_6d79206b6579",
                json!(9),
                oxplow_config::settings::Level::Local,
            )
            .unwrap();
        let values = projected(&extension, &store, false);
        assert_eq!(values["api/url"], "new");
        assert_eq!(values["my key"], 9);
        assert_eq!(values["a.b"], "flat");
        assert_eq!(values["a"]["b"], "nested");
        assert_eq!(values["__hex_612e62"], "literal");
        let origins = origins_for(
            &extension.namespace,
            extension.configuration.as_ref().unwrap(),
            &store,
            false,
        )
        .unwrap();
        assert_eq!(origins["/api~1url"], Origin::Project);
        assert_eq!(origins["/my key"], Origin::Local);
    }

    #[test]
    fn pointer_projection_escapes_names_without_splitting_stored_keys() {
        let extension = extension(
            json!({"type":"object","properties":{"a/b":{"type":"integer"}}}),
            json!({"a/b":3}),
        );
        let bindings = bindings_for(
            &extension.namespace,
            extension.configuration.as_ref().unwrap(),
        )
        .unwrap();
        assert_eq!(bindings[0].definition.default, Some(json!(3)));
        let mut projected = json!({});
        insert(&mut projected, &bindings[0].path, json!(5)).unwrap();
        assert_eq!(projected, json!({"a/b":5}));
    }

    #[test]
    fn public_projection_never_reads_secret_settings() {
        let extension = extension(
            json!({"type":"object","properties":{
                "token":{"type":"string","x-secret":true},
                "enabled":{"type":"boolean"}
            }}),
            json!({"enabled":true}),
        );
        let mut registry = SettingsRegistry::default();
        replace_extensions(&mut registry, std::slice::from_ref(&extension)).unwrap();
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(
            project.path().into(),
            global.path().into(),
            Arc::new(RwLock::new(registry)),
            Arc::new(NoSecrets),
        );
        assert_eq!(projected(&extension, &store, true), json!({"enabled":true}));
        assert_eq!(
            origins_for(
                "example",
                extension.configuration.as_ref().unwrap(),
                &store,
                true
            )
            .unwrap(),
            BTreeMap::from([("/enabled".into(), Origin::Default)])
        );
    }

    #[test]
    fn projections_keep_atomic_objects_and_per_setting_origins() {
        let extension = extension(
            json!({"type":"object","additionalProperties":false,"properties":{
                "tasks":{"type":"object","additionalProperties":false,"properties":{
                    "enabled":{"type":"boolean"},"limit":{"type":"integer"}
                }},
                "headers":{"type":"object","additionalProperties":{"type":"string"}}
            }}),
            json!({"tasks":{"enabled":true,"limit":5},"headers":{"first":"a","second":"b"}}),
        );
        let mut registry = SettingsRegistry::default();
        replace_extensions(&mut registry, std::slice::from_ref(&extension)).unwrap();
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(
            project.path().into(),
            global.path().into(),
            Arc::new(RwLock::new(registry)),
            Arc::new(NoSecrets),
        );
        store
            .set("example.tasks.limit", json!(9), Level::Global)
            .unwrap();
        store
            .set("example.headers", json!({"local":"only"}), Level::Local)
            .unwrap();
        assert_eq!(
            projected(&extension, &store, false),
            json!({
                "tasks":{"enabled":true,"limit":9},"headers":{"local":"only"}
            })
        );
        assert_eq!(
            origins_for(
                "example",
                extension.configuration.as_ref().unwrap(),
                &store,
                false
            )
            .unwrap(),
            BTreeMap::from([
                ("/headers".into(), Origin::Local),
                ("/tasks/enabled".into(), Origin::Default),
                ("/tasks/limit".into(), Origin::Global),
            ])
        );
    }

    #[test]
    fn shared_namespace_registration_deduplicates_without_dropping_core() {
        let extension = extension(
            json!({"type":"object","properties":{"enabled":{"type":"boolean"}}}),
            json!({"enabled":true}),
        );
        let mut sibling = extension.clone();
        sibling.name = "sibling".into();
        let mut registry = SettingsRegistry::default();
        let mut core = definitions(&extension).unwrap().remove(0);
        core.key = "oxplow.enabled".into();
        core.owner = "core".into();
        registry.register_many([core.clone()]).unwrap();
        replace_extensions(&mut registry, &[extension, sibling]).unwrap();
        assert_eq!(registry.definitions().count(), 2);
        replace_extensions(&mut registry, &[]).unwrap();
        assert_eq!(registry.definitions().count(), 1);
        assert_eq!(registry.get("oxplow.enabled"), Some(&core));
    }
}
