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
    enabled.description = format!("Allow direct AI calls through {id}");
    enabled.schema = json!({"type":"boolean"});
    enabled.default = Some(json!(false));
    enabled.secret = false;
    let mut url = enabled.clone();
    url.key = format!("oxplow.ai.{id}.baseUrl");
    url.title = format!("{id} base URL");
    url.description = format!("Endpoint {id} is called at; empty uses its default");
    url.schema = json!({"type":"string"});
    url.default = Some(json!(""));
    vec![enabled, key, url]
}

pub(crate) use oxplow_config::core_settings::{
    role_definition as ai_role_definition, role_key as ai_role_key,
};

struct SecureSettings {
    backend: Arc<dyn oxplow_ai::secrets::SecretStore>,
    project: std::path::PathBuf,
    global: std::path::PathBuf,
}

impl SecureSettings {
    fn endpoint(&self, name: &str) -> Result<Option<String>, String> {
        let documents =
            Documents::read(&self.project, &self.global).map_err(|_| "settings unavailable")?;
        self.endpoint_from(name, &documents)
    }

    fn endpoint_from(&self, name: &str, documents: &Documents) -> Result<Option<String>, String> {
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
        match documents.first(&endpoint_key) {
            Some((_, value)) => {
                let endpoint = value.as_str().ok_or("invalid endpoint")?;
                Ok(Some(endpoint.trim().trim_end_matches('/').to_owned()))
            }
            None => Ok(Some(String::new())),
        }
    }
}

impl SecureSettings {
    fn read(&self, name: &str, documents: Option<&Documents>) -> Result<Option<String>, String> {
        let raw = self
            .backend
            .get(name)
            .map_err(|_| "secure settings unavailable")?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let Some(endpoint) = (match documents {
            Some(documents) => self.endpoint_from(name, documents)?,
            None => self.endpoint(name)?,
        }) else {
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
}

impl SecretStore for SecureSettings {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        self.read(name, None)
    }
    fn get_with_documents(
        &self,
        name: &str,
        documents: &Documents,
    ) -> Result<Option<String>, String> {
        self.read(name, Some(documents))
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

use oxplow_config::settings::Documents;

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    modified: Option<std::time::SystemTime>,
    len: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed: (i64, i64),
}

fn file_stamp(path: &Path) -> Result<Option<FileStamp>, SettingsError> {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            #[cfg(unix)]
            use std::os::unix::fs::MetadataExt;
            Ok(Some(FileStamp {
                modified: metadata.modified().ok(),
                len: metadata.len(),
                #[cfg(unix)]
                inode: metadata.ino(),
                #[cfg(unix)]
                changed: (metadata.ctime(), metadata.ctime_nsec()),
            }))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(SettingsError::Storage),
    }
}

/// Unchanged files need metadata checks only. Retain immutable parsed documents,
/// never secrets; a new inode, timestamp, size or permission change invalidates.
pub(crate) fn documents(project: &Path, global: &Path) -> Result<Documents, SettingsError> {
    type CachedDocuments = ([Option<FileStamp>; 3], Documents);
    type DocumentCache = std::sync::Mutex<
        std::collections::BTreeMap<(std::path::PathBuf, std::path::PathBuf), CachedDocuments>,
    >;
    static CACHE: std::sync::OnceLock<DocumentCache> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let paths = oxplow_config::settings::Level::PRECEDENCE
        .map(|level| oxplow_config::settings::level_path(project, global, level));
    let stamps: [Option<FileStamp>; 3] = paths
        .iter()
        .map(|path| file_stamp(path))
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .expect("three paths");
    let key = (project.to_owned(), global.to_owned());
    if let Some((old, documents)) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        if old == &stamps {
            return Ok(documents.clone());
        }
    }
    let documents = Documents::read(project, global)?;
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= 32 {
        cache.clear();
    }
    cache.insert(key, (stamps, documents.clone()));
    Ok(documents)
}

/// Only settings declarations, dynamic key identities and instance topology
/// affect the registry. Ordinary value edits reuse its compiled validators.
fn cache_key(
    project: &Path,
    global: &Path,
    secrets: &Arc<dyn oxplow_ai::secrets::SecretStore>,
    extensions: &[Extension],
    documents: &Documents,
) -> Result<String, SettingsError> {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(
        serde_json::to_vec(&(project, global)).map_err(|_| SettingsError::InvalidDefinition)?,
    );
    digest.update(format!("{:p}", Arc::as_ptr(secrets)).as_bytes());
    for extension in extensions {
        let credentials = extension
            .collectors
            .iter()
            .map(|collector| &collector.credentials)
            .collect::<Vec<_>>();
        digest.update(
            serde_json::to_vec(&(
                &extension.name,
                &extension.namespace,
                &extension.path,
                extension.enabled,
                &extension.configuration,
                &extension.implementations,
                &extension.providers,
                &extension.ai_profiles,
                credentials,
            ))
            .map_err(|_| SettingsError::InvalidDefinition)?,
        );
        for provider in &extension.providers {
            let path = project.join(&extension.path).join(&provider.declarations);
            digest.update(format!("{:?}", file_stamp(&path)).as_bytes());
        }
    }
    let keys = documents
        .levels()
        .flat_map(|(_, document)| document.keys())
        .filter(|key| key.starts_with("oxplow.extensions.") && key.ends_with(".enabled"))
        .collect::<std::collections::BTreeSet<_>>();
    digest.update(serde_json::to_vec(&keys).map_err(|_| SettingsError::InvalidDocument)?);
    let instances = documents
        .first("oxplow.extensionInstances")
        .map(|(_, value)| value);
    digest.update(serde_json::to_vec(&instances).map_err(|_| SettingsError::InvalidDocument)?);
    Ok(hex::encode(digest.finalize()))
}

pub(crate) fn store(
    project: &Path,
    global: &Path,
    secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    extensions: &[Extension],
) -> Result<SettingsStore, SettingsError> {
    let documents = documents(project, global)?;
    store_with_documents(project, global, secrets, extensions, &documents)
}

pub(crate) fn store_with_documents(
    project: &Path,
    global: &Path,
    secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    extensions: &[Extension],
    documents: &Documents,
) -> Result<SettingsStore, SettingsError> {
    static STORES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<String, SettingsStore>>,
    > = std::sync::OnceLock::new();
    let cache = STORES.get_or_init(Default::default);
    let key = cache_key(project, global, &secrets, extensions, documents)?;
    if let Some(store) = cache
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key)
        .cloned()
    {
        return Ok(store.fork_registry());
    }
    let mut registry = SettingsRegistry::default();
    registry.register_many(oxplow_config::core_settings::definitions(
        &project
            .file_name()
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_default(),
    ))?;
    registry.register_categories(oxplow_config::core_settings::categories())?;
    registry.register_many(crate::harnesses::setting_definitions(extensions))?;
    // A loaded extension's own switch first (a required one is `const:
    // true`); the switches the files name for other extensions after.
    for extension in extensions {
        let mut definition = oxplow_config::core_settings::extension_definition(&extension.name);
        if crate::bundled_extensions::is_required(&extension.name) {
            definition.schema = serde_json::json!({"type":"boolean", "const":true});
        }
        if registry.get(&definition.key).is_none() {
            registry.register(definition)?;
        }
    }
    oxplow_config::core_settings::register_dynamic_keys(
        &mut registry,
        documents
            .levels()
            .flat_map(|(_, document)| document.keys().cloned()),
    )?;
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
    let configured = |key: &str| documents.first(key).map(|(_, value)| value);
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
    Ok(store.fork_registry())
}

pub(crate) fn store_for_declaration(
    project: &Path,
    global: &Path,
    secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    namespace: &str,
    declaration: &crate::extension::extensions::config_schema::ConfigurationFile,
) -> Result<SettingsStore, SettingsError> {
    // `store` already hands each caller its own registry fork.
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

#[cfg(test)]
mod registry_tests {
    use super::*;
    use serde_json::json;

    /// Each provider setting says what it is: only the key is a credential.
    #[test]
    fn provider_settings_each_describe_themselves() {
        let definitions = ai_provider_definitions("openai");
        let described = |suffix: &str| {
            definitions
                .iter()
                .find(|d| d.key.ends_with(suffix))
                .unwrap()
                .description
                .clone()
        };
        assert!(described(".apiKey").contains("credential"));
        assert!(
            !described(".enabled").contains("credential"),
            "{}",
            described(".enabled")
        );
        assert!(
            !described(".baseUrl").contains("credential"),
            "{}",
            described(".baseUrl")
        );
    }

    #[test]
    fn provider_declaration_file_edits_invalidate_registry_identity() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let secrets: Arc<dyn oxplow_ai::secrets::SecretStore> =
            Arc::new(oxplow_ai::secrets::MemorySecrets::default());
        let mut extension =
            crate::extension::extensions::empty_extension("acme", "acme", "project");
        extension.path = "provider-source".into();
        extension.providers.push(
            serde_json::from_value(
                json!({"id":"coder", "capability":"agent_harness", "declarations":"provider.json"}),
            )
            .unwrap(),
        );
        let folder = project.path().join(&extension.path);
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("provider.json");
        std::fs::write(&path, "original").unwrap();
        let documents = documents(project.path(), global.path()).unwrap();
        let before = cache_key(
            project.path(),
            global.path(),
            &secrets,
            std::slice::from_ref(&extension),
            &documents,
        )
        .unwrap();
        std::fs::write(&path, "changed declarations").unwrap();
        let after = cache_key(
            project.path(),
            global.path(),
            &secrets,
            &[extension],
            &documents,
        )
        .unwrap();
        assert_ne!(before, after);
    }

    #[test]
    fn cached_registry_is_isolated_and_value_edits_reuse_declarations() {
        use oxplow_config::settings::Level;
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let secrets: Arc<dyn oxplow_ai::secrets::SecretStore> =
            Arc::new(oxplow_ai::secrets::MemorySecrets::default());
        let first = store(project.path(), global.path(), secrets.clone(), &[]).unwrap();
        first
            .set("oxplow.snapshots.retentionDays", json!(10), Level::Project)
            .unwrap();
        let before = documents(project.path(), global.path()).unwrap();
        let key_before = cache_key(project.path(), global.path(), &secrets, &[], &before).unwrap();
        first
            .registry
            .write()
            .unwrap()
            .register_many(ai_provider_definitions("caller_only"))
            .unwrap();
        let second = store(project.path(), global.path(), secrets.clone(), &[]).unwrap();
        assert!(second.definition("oxplow.ai.caller_only.enabled").is_err());
        assert!(before.shares(&documents(project.path(), global.path()).unwrap()));
        second
            .set("oxplow.snapshots.retentionDays", json!(11), Level::Project)
            .unwrap();
        let after = documents(project.path(), global.path()).unwrap();
        assert!(!before.shares(&after));
        assert_eq!(
            key_before,
            cache_key(project.path(), global.path(), &secrets, &[], &after).unwrap()
        );
        assert_eq!(
            second
                .snapshot()
                .and_then(|snapshot| snapshot.get("oxplow.snapshots.retentionDays", None))
                .unwrap()
                .value,
            Some(json!(11))
        );
    }

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
            settings
                .snapshot()
                .and_then(|snapshot| snapshot.get("oxplow.ai.roles.main", None))
                .unwrap()
                .value,
            Some(main)
        );
        settings
            .unset("oxplow.ai.roles.summarize", Level::Project)
            .unwrap();
        assert_eq!(
            settings
                .snapshot()
                .and_then(|snapshot| snapshot.get("oxplow.ai.roles.summarize", None))
                .unwrap()
                .value,
            Some(summarize)
        );
        settings
            .unset("oxplow.ai.roles.summarize", Level::Global)
            .unwrap();
        assert_eq!(
            settings
                .snapshot()
                .and_then(|snapshot| snapshot.get("oxplow.ai.roles.summarize", None))
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
        assert!(settings
            .snapshot()
            .and_then(|snapshot| snapshot.get("oxplow.ai.openai.apiKey", None))
            .is_err());
        assert_eq!(
            settings
                .snapshot()
                .and_then(|snapshot| snapshot.get("oxplow.ai.openai.enabled", None))
                .unwrap()
                .value,
            Some(json!(true))
        );
    }
}
