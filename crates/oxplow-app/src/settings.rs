//! One configuration backend for native and extension settings.

use std::{
    path::Path,
    sync::{Arc, RwLock},
};

use oxplow_config::settings::{SecretStore, SettingsError, SettingsRegistry, SettingsStore};

use crate::extension::extensions::Extension;

pub(crate) fn ai_provider_definitions(id: &str) -> Vec<oxplow_config::settings::SettingDefinition> {
    use serde_json::json;
    let mut key = crate::ai_service::provider_key_definition(id);
    key.title = format!("{id} API key");
    let mut enabled = key.clone();
    enabled.key = format!("oxplow.ai.{id}.enabled");
    enabled.title = format!("Enable {id}");
    enabled.schema = json!({"type":"boolean"});
    enabled.default = Some(json!(false));
    enabled.secret = false;
    let mut url = enabled.clone();
    url.key = format!("oxplow.ai.{id}.baseUrl");
    url.title = format!("{id} base URL");
    url.schema = json!({"type":"string"});
    url.default = Some(json!(""));
    vec![enabled, key, url]
}

pub(crate) fn ai_role_key(role: &str) -> String {
    format!("oxplow.ai.roles.{}", role.replace('/', "."))
}

pub(crate) fn ai_role_definition(role: &str) -> oxplow_config::settings::SettingDefinition {
    let mut definition = crate::ai_service::provider_key_definition(role);
    definition.key = ai_role_key(role);
    definition.title = format!("AI role {role}");
    definition.secret = false;
    definition.schema = serde_json::json!({"type":"object", "additionalProperties":false,
        "required":["provider","model"], "properties":{
        "provider":{"type":"string","minLength":1}, "model":{"type":"string","minLength":1},
        "restrict":{"type":"array","uniqueItems":true,"items":{"enum":["tools","sql","streaming","images"]}}}});
    definition
}

struct SecureSettings {
    backend: Arc<dyn oxplow_ai::secrets::SecretStore>,
    project: std::path::PathBuf,
    global: std::path::PathBuf,
}

impl SecureSettings {
    fn endpoint(&self, name: &str) -> Result<Option<String>, String> {
        let Some(identity) = name.strip_prefix("settings:v1:") else {
            return Ok(None);
        };
        let identity: serde_json::Value =
            serde_json::from_str(identity).map_err(|_| "invalid secure setting")?;
        let Some(key) = identity.get(3).and_then(serde_json::Value::as_str) else {
            return Err("invalid secure setting".into());
        };
        let Some(provider) = key
            .strip_prefix("oxplow.ai.")
            .and_then(|key| key.strip_suffix(".apiKey"))
        else {
            return Ok(None);
        };
        let endpoint_key = format!("oxplow.ai.{provider}.baseUrl");
        for level in oxplow_config::settings::Level::PRECEDENCE {
            let values = oxplow_config::settings::read_layer(&self.project, &self.global, level)
                .map_err(|_| "settings unavailable")?;
            if let Some(value) = values.get(&endpoint_key) {
                let endpoint = value.as_str().ok_or("invalid endpoint")?;
                return Ok(Some(endpoint.trim().trim_end_matches('/').to_owned()));
            }
        }
        Ok(Some(String::new()))
    }
}

impl SecretStore for SecureSettings {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        let raw = self
            .backend
            .get(name)
            .map_err(|_| "secure settings unavailable")?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let Some(endpoint) = self.endpoint(name)? else {
            return Ok(Some(raw));
        };
        let stored: serde_json::Value =
            serde_json::from_str(&raw).map_err(|_| "invalid secure setting")?;
        if stored.get("endpoint").and_then(serde_json::Value::as_str) != Some(endpoint.as_str()) {
            return Err("secure settings endpoint changed".into());
        }
        stored
            .get("value")
            .and_then(serde_json::Value::as_str)
            .map(|value| Some(value.to_owned()))
            .ok_or_else(|| "invalid secure setting".into())
    }
    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        let stored = match self.endpoint(name)? {
            Some(endpoint) => serde_json::json!({"endpoint":endpoint,"value":value}).to_string(),
            None => value.to_owned(),
        };
        self.backend
            .set(name, &stored)
            .map_err(|_| "secure settings unavailable".into())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        self.backend
            .delete(name)
            .map_err(|_| "secure settings unavailable".into())
    }
}

/// Share a host's registry/store until its declarations or files change.
/// Resolved values are never cached: getters still read current override files.
fn cache_key(
    project: &Path,
    global: &Path,
    secrets: &Arc<dyn oxplow_ai::secrets::SecretStore>,
    extensions: &[Extension],
    documents: &[std::collections::BTreeMap<String, serde_json::Value>],
) -> Result<String, SettingsError> {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(
        serde_json::to_vec(&(project, global, extensions))
            .map_err(|_| SettingsError::InvalidDefinition)?,
    );
    digest.update(format!("{:p}", Arc::as_ptr(secrets)).as_bytes());
    digest.update(serde_json::to_vec(documents).map_err(|_| SettingsError::InvalidDocument)?);
    Ok(hex::encode(digest.finalize()))
}

pub(crate) fn store(
    project: &Path,
    global: &Path,
    secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    extensions: &[Extension],
) -> Result<SettingsStore, SettingsError> {
    static STORES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<String, SettingsStore>>,
    > = std::sync::OnceLock::new();
    let cache = STORES.get_or_init(Default::default);
    let documents = oxplow_config::settings::Level::PRECEDENCE
        .into_iter()
        .map(|level| oxplow_config::settings::read_layer(project, global, level))
        .collect::<Result<Vec<_>, _>>()?;
    let key = cache_key(project, global, &secrets, extensions, &documents)?;
    if let Some(store) = cache
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key)
        .cloned()
    {
        return Ok(store);
    }
    let mut registry = SettingsRegistry::default();
    registry.register_many(oxplow_config::core_settings::definitions(
        &project
            .file_name()
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_default(),
    ))?;
    registry.register_many(crate::harnesses::setting_definitions(extensions))?;
    oxplow_config::core_settings::register_dynamic_keys(
        &mut registry,
        documents
            .iter()
            .flat_map(|document| document.keys().cloned()),
    )?;
    for extension in extensions {
        let definition = oxplow_config::core_settings::extension_definition(&extension.name);
        if registry.get(&definition.key).is_none() {
            registry.register(definition)?;
        }
    }
    crate::extension::extension_settings::replace_extensions(&mut registry, extensions)
        .map_err(|_| SettingsError::InvalidDefinition)?;
    for extension in extensions.iter().filter(|e| e.enabled) {
        for name in extension
            .collectors
            .iter()
            .flat_map(|collector| &collector.credentials)
        {
            let definition = crate::extension::extension_settings::credential_definition(
                &extension.namespace,
                name,
            );
            if registry.get(&definition.key).is_none() {
                registry.register(definition)?;
            }
        }
    }
    for extension in extensions.iter().filter(|extension| extension.enabled) {
        for provider in extension
            .implementations
            .iter()
            .filter(|implementation| implementation.capability == "ai_provider")
        {
            for definition in ai_provider_definitions(&provider.id) {
                if registry.get(&definition.key).is_none() {
                    registry.register(definition)?;
                }
            }
        }
        for profile in &extension.ai_profiles {
            let definition = ai_role_definition(&format!("{}/{}", extension.namespace, profile.id));
            if registry.get(&definition.key).is_none() {
                registry.register(definition)?;
            }
        }
    }
    for role in oxplow_ai::config::BuiltinRole::ALL {
        let definition = ai_role_definition(role.name());
        if registry.get(&definition.key).is_none() {
            registry.register(definition)?;
        }
    }
    let store = SettingsStore::new(
        project.to_path_buf(),
        global.to_path_buf(),
        Arc::new(RwLock::new(registry)),
        Arc::new(SecureSettings {
            backend: secrets,
            project: project.to_path_buf(),
            global: global.to_path_buf(),
        }),
    );
    // Registry discovery tolerates invalid dynamic values so registered
    // setters can repair them. Direct consumers still validate every value.
    let configured = |key: &str| documents.iter().find_map(|document| document.get(key));
    let instances = configured("oxplow.extensionInstances")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    for extension in extensions.iter().filter(|e| e.enabled) {
        for provider in &extension.providers {
            let mut ids = vec![provider.id.clone()];
            for (full, config) in &instances {
                let Some((namespace, id)) = full.split_once('/') else {
                    continue;
                };
                if namespace == extension.namespace
                    && config
                        .get("provider")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(id)
                        == provider.id
                {
                    ids.push(id.to_owned());
                }
            }
            ids.sort();
            ids.dedup();
            for id in ids {
                let instance = format!("{}/{}", extension.namespace, id);
                for definition in crate::harnesses::declared_instance_settings(
                    project, extension, provider, &instance,
                ) {
                    let mut registry = store.registry.write().unwrap_or_else(|e| e.into_inner());
                    if registry.get(&definition.key).is_none() {
                        registry.register(definition)?;
                    }
                }
                for credential in &provider.credentials {
                    if credential.oauth.is_some() || credential.secret_ref.is_some() {
                        continue;
                    }
                    let definition = crate::providers::registry::provider_credential_definition(
                        &extension.namespace,
                        &id,
                        &credential.name,
                    );
                    let mut registry = store.registry.write().unwrap_or_else(|e| e.into_inner());
                    if registry.get(&definition.key).is_none() {
                        registry.register(definition)?;
                    }
                }
            }
        }
    }
    let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
    if cache.len() >= 32 {
        cache.clear();
    }
    cache.insert(key, store.clone());
    Ok(store)
}

pub(crate) fn store_for_declaration(
    project: &Path,
    global: &Path,
    secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    namespace: &str,
    declaration: &crate::extension::extensions::config_schema::ConfigurationFile,
) -> Result<SettingsStore, SettingsError> {
    let store = store(project, global, secrets, &[])?.fork_registry();
    let definitions = crate::extension::extension_settings::definitions_for(namespace, declaration)
        .map_err(|_| SettingsError::InvalidDefinition)?;
    store
        .registry
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .register_many(definitions)?;
    Ok(store)
}

#[cfg(test)]
mod registry_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn each_role_inherits_and_resets_independently() {
        use oxplow_config::settings::Level;
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let settings = store(
            project.path(),
            global.path(),
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            &[],
        )
        .unwrap();
        let main = json!({"provider":"openai", "model":"main"});
        let summarize = json!({"provider":"openai", "model":"small"});
        settings
            .set("oxplow.ai.roles.main", main.clone(), Level::Global)
            .unwrap();
        settings
            .set(
                "oxplow.ai.roles.summarize",
                summarize.clone(),
                Level::Global,
            )
            .unwrap();
        settings
            .set(
                "oxplow.ai.roles.summarize",
                json!({"provider":"openai", "model":"project"}),
                Level::Project,
            )
            .unwrap();
        assert_eq!(
            settings.get("oxplow.ai.roles.main", None).unwrap().value,
            Some(main)
        );
        settings
            .unset("oxplow.ai.roles.summarize", Level::Project)
            .unwrap();
        assert_eq!(
            settings
                .get("oxplow.ai.roles.summarize", None)
                .unwrap()
                .value,
            Some(summarize)
        );
        settings
            .unset("oxplow.ai.roles.summarize", Level::Global)
            .unwrap();
        assert_eq!(
            settings
                .get("oxplow.ai.roles.summarize", None)
                .unwrap()
                .value,
            None
        );
    }

    #[test]
    fn ai_provider_settings_exist_before_configuration_and_are_independent() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let settings = store(
            project.path(),
            global.path(),
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            &[],
        )
        .unwrap();
        settings
            .registry
            .write()
            .unwrap()
            .register_many(ai_provider_definitions("openai"))
            .unwrap();
        assert!(
            settings
                .definition("oxplow.ai.openai.apiKey")
                .unwrap()
                .secret
        );
        assert!(settings.definition("oxplow.ai.providers").is_err());
        settings
            .set(
                "oxplow.ai.openai.enabled",
                json!(true),
                oxplow_config::settings::Level::Global,
            )
            .unwrap();
        settings
            .set(
                "oxplow.ai.openai.baseUrl",
                json!("https://one.example/v1"),
                oxplow_config::settings::Level::Project,
            )
            .unwrap();
        settings
            .set(
                "oxplow.ai.openai.apiKey",
                json!("private"),
                oxplow_config::settings::Level::Global,
            )
            .unwrap();
        settings
            .set(
                "oxplow.ai.openai.baseUrl",
                json!("https://two.example/v1"),
                oxplow_config::settings::Level::Project,
            )
            .unwrap();
        assert!(settings.get("oxplow.ai.openai.apiKey", None).is_err());
        assert_eq!(
            settings
                .get("oxplow.ai.openai.enabled", None)
                .unwrap()
                .value,
            Some(json!(true))
        );
    }
}
