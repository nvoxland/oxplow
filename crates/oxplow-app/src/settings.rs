//! One configuration backend for native and extension settings.

use std::{
    path::Path,
    sync::{Arc, RwLock},
};

use oxplow_config::settings::{SecretStore, SettingsError, SettingsRegistry, SettingsStore};

use crate::extension::extensions::Extension;

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
        if !key.starts_with("oxplow.ai.providerKeys.") {
            return Ok(None);
        }
        for level in oxplow_config::settings::Level::PRECEDENCE {
            let values = oxplow_config::settings::read_layer(&self.project, &self.global, level)
                .map_err(|_| "settings unavailable")?;
            if let Some(providers) = values.get("oxplow.ai.providers") {
                let providers: Vec<oxplow_ai::config::ProviderConfig> =
                    serde_json::from_value(providers.clone()).map_err(|_| "invalid providers")?;
                let provider = providers
                    .iter()
                    .find(|p| crate::ai_service::provider_key_setting(&p.id) == key)
                    .ok_or("unknown provider")?;
                return Ok(Some(crate::ai_service::endpoint_of(Some(provider))));
            }
        }
        Err("unknown provider".into())
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

pub(crate) fn store(
    project: &Path,
    global: &Path,
    secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    extensions: &[Extension],
) -> Result<SettingsStore, SettingsError> {
    let mut registry = SettingsRegistry::default();
    registry.register_many(oxplow_config::core_settings::definitions(
        &project
            .file_name()
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_default(),
    ))?;
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
    if let Some(serde_json::Value::Array(providers)) = store.get("oxplow.ai.providers", None)?.value
    {
        for provider in providers {
            if let Some(id) = provider.get("id").and_then(serde_json::Value::as_str) {
                store
                    .registry
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .register(crate::ai_service::provider_key_definition(id))?;
            }
        }
    }
    let instances = store
        .get("oxplow.extensionInstances", None)?
        .value
        .and_then(|v| v.as_object().cloned())
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
    Ok(store)
}

pub(crate) fn store_for_declaration(
    project: &Path,
    global: &Path,
    secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    namespace: &str,
    declaration: &crate::extension::extensions::config_schema::ConfigurationFile,
) -> Result<SettingsStore, SettingsError> {
    let store = store(project, global, secrets, &[])?;
    let definitions = crate::extension::extension_settings::definitions_for(namespace, declaration)
        .map_err(|_| SettingsError::InvalidDefinition)?;
    store
        .registry
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .register_many(definitions)?;
    Ok(store)
}
