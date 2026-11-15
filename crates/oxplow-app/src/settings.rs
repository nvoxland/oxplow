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
    // Registry discovery tolerates invalid dynamic values so registered
    // setters can repair them. Direct consumers still validate every value.
    let configured = |key: &str| documents.iter().find_map(|document| document.get(key));
    if let Some(serde_json::Value::Array(providers)) = configured("oxplow.ai.providers") {
        let mut ids = std::collections::BTreeSet::new();
        for provider in providers {
            if let Some(id) = provider
                .get("id")
                .and_then(serde_json::Value::as_str)
                .filter(|id| {
                    !id.is_empty() && crate::ai_service::provider_key_setting(id).len() <= 256
                })
                .filter(|id| ids.insert((*id).to_owned()))
            {
                store
                    .registry
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .register(crate::ai_service::provider_key_definition(id))?;
            }
        }
    }
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
    fn host_stores_are_shared_and_dynamic_definitions_refresh_after_a_write() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let secrets: Arc<dyn oxplow_ai::secrets::SecretStore> =
            Arc::new(oxplow_ai::secrets::MemorySecrets::default());
        let first = store(project.path(), global.path(), secrets.clone(), &[]).unwrap();
        let second = store(project.path(), global.path(), secrets.clone(), &[]).unwrap();
        assert!(Arc::ptr_eq(&first.registry, &second.registry));
        first
            .set(
                "oxplow.ai.providers",
                json!([{"id":"new","kind":"openai"}]),
                oxplow_config::settings::Level::Project,
            )
            .unwrap();
        let changed = store(project.path(), global.path(), secrets.clone(), &[]).unwrap();
        assert!(changed
            .definition(&crate::ai_service::provider_key_setting("new"))
            .is_ok());
        assert!(!Arc::ptr_eq(&first.registry, &changed.registry));
        let path = project.path().join(".oxplow/project.yaml");
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("new", "old")).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let replaced = store(project.path(), global.path(), secrets.clone(), &[]).unwrap();
        assert!(replaced
            .definition(&crate::ai_service::provider_key_setting("old"))
            .is_ok());
        assert!(replaced
            .definition(&crate::ai_service::provider_key_setting("new"))
            .is_err());

        let isolated = store(
            project.path(),
            global.path(),
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            &[],
        )
        .unwrap();
        assert!(!Arc::ptr_eq(&changed.registry, &isolated.registry));
    }

    #[test]
    fn malformed_dynamic_configuration_can_be_repaired_through_registered_keys() {
        for (key, value, replacement) in [
            ("oxplow.ai.providers", json!({"not":"an array"}), json!([])),
            (
                "oxplow.ai.providers",
                json!([{"id":"", "kind":"openai"}]),
                json!([]),
            ),
            (
                "oxplow.ai.providers",
                json!([{"id":"x".repeat(256), "kind":"openai"}]),
                json!([]),
            ),
            ("oxplow.extensionInstances", json!(["not a map"]), json!({})),
        ] {
            let project = tempfile::tempdir().unwrap();
            let global = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(project.path().join(".oxplow")).unwrap();
            let text = serde_yaml::to_string(&serde_json::json!({(key):value})).unwrap();
            std::fs::write(project.path().join(".oxplow/project.yaml"), text).unwrap();
            let settings = store(
                project.path(),
                global.path(),
                Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
                &[],
            )
            .unwrap();
            settings
                .set(
                    key,
                    replacement.clone(),
                    oxplow_config::settings::Level::Project,
                )
                .unwrap();
            assert_eq!(settings.get(key, None).unwrap().value, Some(replacement));
        }
    }

    #[test]
    fn duplicate_provider_ids_keep_registry_available_for_repair() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join(".oxplow")).unwrap();
        std::fs::write(project.path().join(".oxplow/project.yaml"), "oxplow.ai.providers:\n  - {id: duplicate, kind: openai}\n  - {id: duplicate, kind: openai}\n").unwrap();
        let settings = store(
            project.path(),
            global.path(),
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            &[],
        )
        .unwrap();
        assert!(settings
            .definition(&crate::ai_service::provider_key_setting("duplicate"))
            .is_ok());
        settings
            .set(
                "oxplow.ai.providers",
                json!([]),
                oxplow_config::settings::Level::Project,
            )
            .unwrap();
        assert_eq!(
            settings.get("oxplow.ai.providers", None).unwrap().value,
            Some(json!([]))
        );
    }
}
