//! The shared settings vocabulary and its three override levels. A setting is
//! one atomic value: neither object fields nor arrays merge between levels.
//! Secrets use the same API but never enter a YAML document or an inventory.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use fs2::FileExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_DEPTH: usize = 64;

fn empty_object() -> Value {
    Value::Object(Default::default())
}

/// The registry definition is identical for core and extension settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingDefinition {
    pub key: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub order: i32,
    #[serde(default = "empty_object")]
    pub ui: Value,
    #[serde(default)]
    pub restart_required: bool,
    /// Assigned by the registry host, never accepted as caller authority.
    pub owner: String,
}

/// Which override file (or corresponding secure-store entry) a write targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Local,
    Project,
    Global,
}

impl Level {
    pub const PRECEDENCE: [Self; 3] = [Self::Local, Self::Project, Self::Global];
}

/// The source of one complete effective value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Default,
    Local,
    Project,
    Global,
}

impl From<Level> for Origin {
    fn from(value: Level) -> Self {
        match value {
            Level::Local => Self::Local,
            Level::Project => Self::Project,
            Level::Global => Self::Global,
        }
    }
}

/// A configuration read. Only authorized direct reads contain secret values;
/// use [`SettingsStore::inventory`] for display or queryable metadata.
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SettingValue {
    pub key: String,
    pub value: Option<Value>,
    pub origin: Option<Origin>,
    pub is_set: bool,
    pub secret: bool,
}

// Deliberately omit values: a caller logging an ordinary read must not expose a
// secret merely because its registry definition changed.
impl std::fmt::Debug for SettingValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingValue")
            .field("key", &self.key)
            .field("origin", &self.origin)
            .field("is_set", &self.is_set)
            .field("secret", &self.secret)
            .finish_non_exhaustive()
    }
}

/// A registry definition with its redacted effective value and origin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingInventory {
    #[serde(flatten)]
    pub definition: SettingDefinition,
    pub value: Option<Value>,
    pub origin: Option<Origin>,
    pub is_set: bool,
}

/// Backed by the machine keychain in production and memory in tests. Errors
/// are deliberately discarded at this boundary; backends may name secrets.
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>, String>;
    fn set(&self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("unknown setting")]
    UnknownSetting,
    #[error("duplicate setting key")]
    DuplicateKey,
    #[error("invalid setting definition")]
    InvalidDefinition,
    #[error("invalid setting schema")]
    InvalidSchema,
    #[error("setting value does not satisfy its schema")]
    InvalidValue,
    #[error("settings document is invalid")]
    InvalidDocument,
    #[error("secret settings cannot be stored in YAML")]
    PlaintextSecret,
    #[error("secure settings storage is unavailable")]
    SecureStorage,
    #[error("settings storage is unavailable")]
    Storage,
    #[error("settings YAML cannot be edited without changing unrelated values")]
    UnsafeEdit,
}

#[derive(Clone)]
struct RegisteredSetting {
    definition: SettingDefinition,
    validator: Arc<jsonschema::Validator>,
}

/// One registry for schemas, defaults, ownership, categories and UI metadata.
#[derive(Clone, Default)]
pub struct SettingsRegistry {
    settings: BTreeMap<String, RegisteredSetting>,
}

impl SettingsRegistry {
    pub fn register(&mut self, definition: SettingDefinition) -> Result<(), SettingsError> {
        self.register_many([definition])
    }

    /// Register a batch all-or-nothing. A bad extension leaves the existing
    /// registry and every other owner's definitions intact.
    pub fn register_many(
        &mut self,
        definitions: impl IntoIterator<Item = SettingDefinition>,
    ) -> Result<(), SettingsError> {
        let mut additions = BTreeMap::new();
        for definition in definitions {
            if self.settings.contains_key(&definition.key)
                || additions.contains_key(&definition.key)
            {
                return Err(SettingsError::DuplicateKey);
            }
            let setting = compile_definition(definition)?;
            additions.insert(setting.definition.key.clone(), setting);
        }
        self.settings.extend(additions);
        Ok(())
    }

    /// Publish a new generation of one owner's definitions atomically. Values
    /// remain in their stores when an extension withdraws its declarations.
    pub fn replace_owner(
        &mut self,
        owner: &str,
        definitions: impl IntoIterator<Item = SettingDefinition>,
    ) -> Result<(), SettingsError> {
        let definitions: Vec<_> = definitions.into_iter().collect();
        if definitions
            .iter()
            .any(|definition| definition.owner != owner)
        {
            return Err(SettingsError::InvalidDefinition);
        }
        let mut candidate = self.clone();
        candidate
            .settings
            .retain(|_, setting| setting.definition.owner != owner);
        candidate.register_many(definitions)?;
        *self = candidate;
        Ok(())
    }

    pub fn remove_owner(&mut self, owner: &str) {
        self.settings
            .retain(|_, setting| setting.definition.owner != owner);
    }

    /// Replace participating owners together, so declarations can move between
    /// extensions without a transient collision or partially published batch.
    pub fn replace_owners(
        &mut self,
        owners: &[String],
        definitions: impl IntoIterator<Item = SettingDefinition>,
    ) -> Result<(), SettingsError> {
        let definitions: Vec<_> = definitions.into_iter().collect();
        if definitions
            .iter()
            .any(|definition| !owners.contains(&definition.owner))
        {
            return Err(SettingsError::InvalidDefinition);
        }
        let mut candidate = self.clone();
        candidate
            .settings
            .retain(|_, setting| !owners.contains(&setting.definition.owner));
        candidate.register_many(definitions)?;
        *self = candidate;
        Ok(())
    }

    pub fn definitions(&self) -> impl Iterator<Item = &SettingDefinition> {
        self.settings.values().map(|setting| &setting.definition)
    }

    pub fn get(&self, key: &str) -> Option<&SettingDefinition> {
        self.settings.get(key).map(|setting| &setting.definition)
    }

    pub fn validate(&self, key: &str, value: &Value) -> Result<(), SettingsError> {
        let setting = self
            .settings
            .get(key)
            .ok_or(SettingsError::UnknownSetting)?;
        validate_value(&setting.validator, value)
    }
}

fn compile_definition(definition: SettingDefinition) -> Result<RegisteredSetting, SettingsError> {
    if !valid_key(&definition.key)
        || definition.title.trim().is_empty()
        || definition.category.trim().is_empty()
        || definition.owner.trim().is_empty()
        || !definition.ui.is_object()
        || (definition.secret
            && (definition.default.is_some() || contains_default(&definition.schema)))
    {
        return Err(SettingsError::InvalidDefinition);
    }
    bounded(&definition.schema)?;
    bounded(&definition.ui)?;
    let validator = jsonschema::options()
        .offline()
        .build(&definition.schema)
        .map_err(|_| SettingsError::InvalidSchema)?;
    if let Some(default) = &definition.default {
        validate_value(&validator, default)?;
    }
    Ok(RegisteredSetting {
        definition,
        validator: Arc::new(validator),
    })
}

fn valid_key(key: &str) -> bool {
    key.len() <= 256
        && key.contains('.')
        && key.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-'))
        })
}

fn contains_default(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key("default") || object.values().any(contains_default)
        }
        Value::Array(array) => array.iter().any(contains_default),
        _ => false,
    }
}

fn bounded(value: &Value) -> Result<(), SettingsError> {
    fn depth(value: &Value, level: usize) -> Result<(), SettingsError> {
        if level > MAX_DEPTH {
            return Err(SettingsError::InvalidValue);
        }
        match value {
            Value::Object(object) => {
                for value in object.values() {
                    depth(value, level + 1)?;
                }
            }
            Value::Array(array) => {
                for value in array {
                    depth(value, level + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    struct Counter(u64);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len() as u64;
            if self.0 > MAX_BYTES {
                return Err(std::io::Error::other("settings size limit exceeded"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    depth(value, 0)?;
    serde_json::to_writer(Counter(0), value).map_err(|_| SettingsError::InvalidValue)
}

fn validate_value(validator: &jsonschema::Validator, value: &Value) -> Result<(), SettingsError> {
    bounded(value)?;
    if validator.is_valid(value) {
        Ok(())
    } else {
        Err(SettingsError::InvalidValue)
    }
}

/// The one storage interface. Files are read afresh for every operation, so a
/// global write is visible to another project without a process-local cache.
#[derive(Clone)]
pub struct SettingsStore {
    pub registry: Arc<RwLock<SettingsRegistry>>,
    project_dir: PathBuf,
    global_dir: PathBuf,
    secrets: Arc<dyn SecretStore>,
}

impl SettingsStore {
    pub fn new(
        project_dir: PathBuf,
        global_dir: PathBuf,
        registry: Arc<RwLock<SettingsRegistry>>,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            project_dir,
            global_dir,
            registry,
            secrets,
        }
    }

    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    pub fn global_dir(&self) -> &Path {
        &self.global_dir
    }

    pub fn path(&self, level: Level) -> PathBuf {
        level_path(&self.project_dir, &self.global_dir, level)
    }

    pub fn definition(&self, key: &str) -> Result<SettingDefinition, SettingsError> {
        self.registry
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(key)
            .cloned()
            .ok_or(SettingsError::UnknownSetting)
    }

    pub fn validate(&self, key: &str, value: &Value) -> Result<(), SettingsError> {
        self.registry
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .validate(key, value)
    }

    pub fn get(&self, key: &str, level: Option<Level>) -> Result<SettingValue, SettingsError> {
        let definition = self.definition(key)?;
        self.read(&definition, level, false)
    }

    /// Metadata has the same origin and presence as a direct get, but never
    /// includes secret bytes. The UI and semantic layer must use this read.
    pub fn inspect(&self, key: &str, level: Option<Level>) -> Result<SettingValue, SettingsError> {
        let definition = self.definition(key)?;
        self.read(&definition, level, true)
    }

    fn read(
        &self,
        definition: &SettingDefinition,
        level: Option<Level>,
        redact: bool,
    ) -> Result<SettingValue, SettingsError> {
        let levels: Vec<_> = level.map_or_else(|| Level::PRECEDENCE.to_vec(), |level| vec![level]);
        // Reject plaintext even when it is shadowed by a higher secure value.
        let documents: Vec<_> = levels
            .iter()
            .map(|level| self.layer(*level).map(|values| (*level, values)))
            .collect::<Result<_, _>>()?;
        self.read_documents(definition, &documents, level.is_none(), redact)
    }

    fn read_documents(
        &self,
        definition: &SettingDefinition,
        documents: &[(Level, BTreeMap<String, Value>)],
        include_default: bool,
        redact: bool,
    ) -> Result<SettingValue, SettingsError> {
        if definition.secret
            && documents
                .iter()
                .any(|(_, values)| values.contains_key(&definition.key))
        {
            return Err(SettingsError::PlaintextSecret);
        }
        for (level, values) in documents {
            let value = if definition.secret {
                self.secrets
                    .get(&self.secret_identity(&definition.key, *level))
                    .map_err(|_| SettingsError::SecureStorage)?
                    .map(|secret| {
                        serde_json::from_str(&secret).map_err(|_| SettingsError::SecureStorage)
                    })
                    .transpose()?
            } else {
                values.get(&definition.key).cloned()
            };
            if let Some(value) = value {
                self.validate(&definition.key, &value)?;
                return Ok(SettingValue {
                    key: definition.key.clone(),
                    value: if redact && definition.secret {
                        None
                    } else {
                        Some(value)
                    },
                    origin: Some((*level).into()),
                    is_set: true,
                    secret: definition.secret,
                });
            }
        }
        let value = if include_default {
            definition.default.clone()
        } else {
            None
        };
        Ok(SettingValue {
            key: definition.key.clone(),
            origin: value.as_ref().map(|_| Origin::Default),
            is_set: value.is_some(),
            value,
            secret: definition.secret,
        })
    }

    pub fn inventory(&self) -> Result<Vec<SettingInventory>, SettingsError> {
        let definitions: Vec<_> = self
            .registry
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .definitions()
            .cloned()
            .collect();
        let documents = Level::PRECEDENCE
            .into_iter()
            .map(|level| self.layer(level).map(|values| (level, values)))
            .collect::<Result<Vec<_>, _>>()?;
        definitions
            .into_iter()
            .map(|definition| {
                let value = self.read_documents(&definition, &documents, true, true)?;
                Ok(SettingInventory {
                    definition,
                    value: value.value,
                    origin: value.origin,
                    is_set: value.is_set,
                })
            })
            .collect()
    }

    /// One file's raw overrides. This deliberately keeps unknown keys: a
    /// disabled extension must not lose its values when another key changes.
    pub fn layer(&self, level: Level) -> Result<BTreeMap<String, Value>, SettingsError> {
        read_layer(&self.project_dir, &self.global_dir, level)
    }

    pub fn set(&self, key: &str, value: Value, level: Level) -> Result<(), SettingsError> {
        let definition = self.definition(key)?;
        self.validate(key, &value)?;
        if definition.secret {
            if self.layer(level)?.contains_key(key) {
                return Err(SettingsError::PlaintextSecret);
            }
            let value = serde_json::to_string(&value).map_err(|_| SettingsError::InvalidValue)?;
            self.secrets
                .set(&self.secret_identity(key, level), &value)
                .map_err(|_| SettingsError::SecureStorage)
        } else {
            write_layer_key(&self.path(level), key, Some(&value))
        }
    }

    pub fn unset(&self, key: &str, level: Level) -> Result<(), SettingsError> {
        let definition = self.definition(key)?;
        if definition.secret {
            // A manual plaintext entry is an invalid document, never a second
            // secret backend. Let the person remove it explicitly from YAML.
            if self.layer(level)?.contains_key(key) {
                return Err(SettingsError::PlaintextSecret);
            }
            self.secrets
                .delete(&self.secret_identity(key, level))
                .map_err(|_| SettingsError::SecureStorage)
        } else {
            write_layer_key(&self.path(level), key, None)
        }
    }

    fn secret_identity(&self, key: &str, level: Level) -> String {
        // JSON tuple encoding avoids ambiguous path/key separators. Global
        // values share an identity across projects; other levels never do.
        let project = if level == Level::Global {
            None
        } else {
            Some(
                identity_path(&self.project_dir)
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        format!(
            "settings:v1:{}",
            serde_json::to_string(&(identity_path(&self.global_dir), level, project, key,))
                .expect("identity serializes")
        )
    }
}

/// Canonicalize the nearest existing ancestor before appending missing path
/// components. Creating a configuration directory later must not rename its
/// existing secure entries (notably /var vs /private/var on macOS).
fn identity_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut ancestor = absolute.as_path();
    let mut suffix = Vec::new();
    loop {
        if let Ok(mut resolved) = ancestor.canonicalize() {
            for part in suffix.into_iter().rev() {
                if part == std::ffi::OsStr::new("..") {
                    resolved.pop();
                } else if part != std::ffi::OsStr::new(".") {
                    resolved.push(part);
                }
            }
            return resolved;
        }
        let Some(component) = ancestor.components().next_back() else {
            return absolute;
        };
        suffix.push(component.as_os_str().to_os_string());
        let Some(parent) = ancestor.parent() else {
            return absolute;
        };
        ancestor = parent;
    }
}

pub fn level_path(project_dir: &Path, global_dir: &Path, level: Level) -> PathBuf {
    match level {
        Level::Local => project_dir.join(crate::OXPLOW_STATE_DIR).join("local.yaml"),
        Level::Project => project_dir
            .join(crate::OXPLOW_STATE_DIR)
            .join("project.yaml"),
        Level::Global => global_dir.join("global.yaml"),
    }
}

pub fn read_layer(
    project_dir: &Path,
    global_dir: &Path,
    level: Level,
) -> Result<BTreeMap<String, Value>, SettingsError> {
    let text = read_text(&level_path(project_dir, global_dir, level))?;
    parse_layer(&text)
}

fn read_text(path: &Path) -> Result<String, SettingsError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(_) => return Err(SettingsError::Storage),
    };
    let mut text = String::new();
    file.take(MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| SettingsError::Storage)?;
    if text.len() as u64 > MAX_BYTES {
        return Err(SettingsError::InvalidDocument);
    }
    Ok(text)
}

fn parse_layer(text: &str) -> Result<BTreeMap<String, Value>, SettingsError> {
    let value: serde_yaml::Value =
        serde_yaml::from_str(text).map_err(|_| SettingsError::InvalidDocument)?;
    let map = match value {
        serde_yaml::Value::Null => return Ok(BTreeMap::new()),
        serde_yaml::Value::Mapping(map) => map,
        _ => return Err(SettingsError::InvalidDocument),
    };
    map.into_iter()
        .map(|(key, value)| {
            let key = key
                .as_str()
                .ok_or(SettingsError::InvalidDocument)?
                .to_owned();
            let value = serde_json::to_value(value).map_err(|_| SettingsError::InvalidDocument)?;
            bounded(&value).map_err(|_| SettingsError::InvalidDocument)?;
            Ok((key, value))
        })
        .collect()
}

/// Lock a stable sidecar rather than the document's replaceable inode. Re-read
/// under that lock, so independent processes cannot overwrite each other's keys.
pub fn write_layer_key(path: &Path, key: &str, value: Option<&Value>) -> Result<(), SettingsError> {
    let parent = path.parent().ok_or(SettingsError::Storage)?;
    std::fs::create_dir_all(parent).map_err(|_| SettingsError::Storage)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(crate::atomic::lock_path(path))
        .map_err(|_| SettingsError::Storage)?;
    lock.lock_exclusive().map_err(|_| SettingsError::Storage)?;
    let original = read_text(path)?;
    let mut expected = parse_layer(&original)?;
    match value {
        Some(value) => {
            expected.insert(key.to_string(), value.clone());
        }
        None => {
            expected.remove(key);
        }
    }
    let block = value
        .map(|value| {
            let mut map = serde_yaml::Mapping::new();
            map.insert(
                serde_yaml::Value::String(key.to_string()),
                crate::to_yaml(value),
            );
            serde_yaml::to_string(&map).map_err(|_| SettingsError::InvalidValue)
        })
        .transpose()?;
    let edited = crate::edit_top_level_key(&original, key, block.as_deref());
    if parse_layer(&edited)? != expected {
        return Err(SettingsError::UnsafeEdit);
    }
    if edited.len() as u64 > MAX_BYTES {
        return Err(SettingsError::InvalidDocument);
    }
    crate::atomic::write_atomic(path, edited.as_bytes()).map_err(|_| SettingsError::Storage)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[derive(Default)]
    struct MemorySecrets {
        values: Mutex<BTreeMap<String, String>>,
        fail: bool,
    }

    impl SecretStore for MemorySecrets {
        fn get(&self, name: &str) -> Result<Option<String>, String> {
            if self.fail {
                return Err("backend failure containing token-sensitive-value".into());
            }
            Ok(self.values.lock().unwrap().get(name).cloned())
        }

        fn set(&self, name: &str, value: &str) -> Result<(), String> {
            if self.fail {
                return Err(format!("backend rejected {value}"));
            }
            self.values
                .lock()
                .unwrap()
                .insert(name.into(), value.into());
            Ok(())
        }

        fn delete(&self, name: &str) -> Result<(), String> {
            if self.fail {
                return Err("backend failure".into());
            }
            self.values.lock().unwrap().remove(name);
            Ok(())
        }
    }

    fn definition(key: &str, schema: Value, default: Option<Value>) -> SettingDefinition {
        SettingDefinition {
            key: key.into(),
            title: "Example setting".into(),
            description: "A setting used by the storage contract tests.".into(),
            category: "example".into(),
            schema,
            default,
            secret: false,
            order: 0,
            ui: json!({}),
            restart_required: false,
            owner: "example".into(),
        }
    }

    fn store(definitions: Vec<SettingDefinition>) -> (TempDir, TempDir, SettingsStore) {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let mut registry = SettingsRegistry::default();
        registry.register_many(definitions).unwrap();
        let store = SettingsStore::new(
            project.path().to_path_buf(),
            global.path().to_path_buf(),
            Arc::new(RwLock::new(registry)),
            Arc::new(MemorySecrets::default()),
        );
        (project, global, store)
    }

    fn number_definition() -> SettingDefinition {
        definition(
            "example.limit",
            json!({ "type": "integer", "minimum": 1 }),
            Some(json!(5)),
        )
    }

    #[test]
    fn settings_resolve_each_level_and_unset_reveals_inheritance() {
        let (_project, _global, store) = store(vec![number_definition()]);
        let read = || store.get("example.limit", None).unwrap();
        assert_eq!(read().value, Some(json!(5)));
        assert_eq!(read().origin, Some(Origin::Default));
        assert!(
            !store
                .get("example.limit", Some(Level::Local))
                .unwrap()
                .is_set
        );
        for (level, value, origin) in [
            (Level::Global, 10, Origin::Global),
            (Level::Project, 20, Origin::Project),
            (Level::Local, 30, Origin::Local),
        ] {
            store.set("example.limit", json!(value), level).unwrap();
            assert_eq!(read().value, Some(json!(value)));
            assert_eq!(read().origin, Some(origin));
        }
        assert_eq!(
            store
                .get("example.limit", Some(Level::Global))
                .unwrap()
                .value,
            Some(json!(10))
        );
        for (level, expected, origin) in [
            (Level::Local, 20, Origin::Project),
            (Level::Project, 10, Origin::Global),
            (Level::Global, 5, Origin::Default),
        ] {
            store.unset("example.limit", level).unwrap();
            assert_eq!(read().value, Some(json!(expected)));
            assert_eq!(read().origin, Some(origin));
        }
    }

    #[test]
    fn settings_objects_and_arrays_replace_whole_values() {
        let (_project, _global, store) = store(vec![
            definition(
                "example.object",
                json!({"type":"object"}),
                Some(json!({"a":1,"b":2})),
            ),
            definition(
                "example.array",
                json!({"type":"array"}),
                Some(json!([1, 2])),
            ),
        ]);
        store
            .set("example.object", json!({"c":3}), Level::Global)
            .unwrap();
        store
            .set("example.object", json!({"d":4}), Level::Local)
            .unwrap();
        store
            .set("example.array", json!([3]), Level::Project)
            .unwrap();
        assert_eq!(
            store.get("example.object", None).unwrap().value,
            Some(json!({"d":4}))
        );
        assert_eq!(
            store.get("example.array", None).unwrap().value,
            Some(json!([3]))
        );
        store.unset("example.object", Level::Local).unwrap();
        assert_eq!(
            store.get("example.object", None).unwrap().value,
            Some(json!({"c":3}))
        );
    }

    #[test]
    fn settings_null_is_a_value_not_an_unset_sentinel() {
        let (_project, _global, store) = store(vec![definition(
            "example.nullable",
            json!({"type":["integer","null"]}),
            Some(json!(5)),
        )]);
        store
            .set("example.nullable", Value::Null, Level::Local)
            .unwrap();
        let value = store.get("example.nullable", None).unwrap();
        assert_eq!(value.value, Some(Value::Null));
        assert_eq!(value.origin, Some(Origin::Local));
        store.unset("example.nullable", Level::Local).unwrap();
        assert_eq!(
            store.get("example.nullable", None).unwrap().value,
            Some(json!(5))
        );
    }

    #[test]
    fn settings_schema_validation_never_changes_a_file_on_failure() {
        let (_project, _global, store) = store(vec![number_definition()]);
        store
            .set("example.limit", json!(10), Level::Project)
            .unwrap();
        let before = std::fs::read(store.path(Level::Project)).unwrap();
        assert!(matches!(
            store.set("example.limit", json!(0), Level::Project),
            Err(SettingsError::InvalidValue)
        ));
        assert!(matches!(
            store.set("example.missing", json!(10), Level::Project),
            Err(SettingsError::UnknownSetting)
        ));
        assert_eq!(std::fs::read(store.path(Level::Project)).unwrap(), before);
        assert!(matches!(
            store.get("example.missing", None),
            Err(SettingsError::UnknownSetting)
        ));
    }

    #[test]
    fn settings_registry_registration_is_atomic_and_defaults_are_validated() {
        let mut registry = SettingsRegistry::default();
        registry.register(number_definition()).unwrap();
        let first = definition("example.new", json!({"type":"boolean"}), Some(json!(true)));
        assert!(matches!(
            registry.register_many([first.clone(), number_definition()]),
            Err(SettingsError::DuplicateKey)
        ));
        assert!(registry.get("example.new").is_none());
        let bad = definition(
            "example.bad",
            json!({"type":"integer"}),
            Some(json!("not a number")),
        );
        assert!(matches!(
            registry.register_many([first, bad]),
            Err(SettingsError::InvalidValue)
        ));
        assert_eq!(registry.definitions().count(), 1);
    }

    #[test]
    fn settings_owner_replacement_is_atomic_and_does_not_remove_other_owners() {
        let mut registry = SettingsRegistry::default();
        registry.register(number_definition()).unwrap();
        let mut other = definition("other.flag", json!({"type":"boolean"}), Some(json!(true)));
        other.owner = "other".into();
        registry.register(other.clone()).unwrap();
        assert!(matches!(
            registry.replace_owner("example", [other]),
            Err(SettingsError::InvalidDefinition)
        ));
        assert!(registry.get("example.limit").is_some());
        registry.replace_owner("example", []).unwrap();
        assert!(registry.get("example.limit").is_none());
        assert!(registry.get("other.flag").is_some());
    }

    #[test]
    fn settings_schemas_cannot_fetch_external_references() {
        let mut registry = SettingsRegistry::default();
        let remote = definition(
            "example.remote",
            json!({"$ref":"https://invalid.example/schema"}),
            None,
        );
        assert!(matches!(
            registry.register(remote),
            Err(SettingsError::InvalidSchema)
        ));
        let file = definition("example.file", json!({"$ref":"file:///etc/passwd"}), None);
        assert!(matches!(
            registry.register(file),
            Err(SettingsError::InvalidSchema)
        ));
    }

    #[test]
    fn settings_secret_reads_use_the_same_api_and_inventory_is_redacted() {
        let mut secret = definition("example.token", json!({"type":"string"}), None);
        secret.secret = true;
        let (_project, _global, store) = store(vec![secret]);
        assert!(!store.get("example.token", None).unwrap().is_set);
        store
            .set(
                "example.token",
                json!("token-sensitive-value"),
                Level::Global,
            )
            .unwrap();
        store
            .set("example.token", json!("project-secret"), Level::Project)
            .unwrap();
        let value = store.get("example.token", None).unwrap();
        assert_eq!(value.value, Some(json!("project-secret")));
        assert_eq!(value.origin, Some(Origin::Project));
        assert!(!format!("{value:?}").contains("project-secret"));
        let inventory = store.inventory().unwrap();
        assert!(inventory[0].is_set);
        assert_eq!(inventory[0].value, None);
        assert_eq!(inventory[0].origin, Some(Origin::Project));
        assert!(!serde_json::to_string(&inventory)
            .unwrap()
            .contains("project-secret"));
        for level in Level::PRECEDENCE {
            assert!(
                !store.path(level).exists(),
                "a secret write must not create a YAML file"
            );
        }
        store.unset("example.token", Level::Project).unwrap();
        assert_eq!(
            store.get("example.token", None).unwrap().value,
            Some(json!("token-sensitive-value"))
        );
        store.unset("example.token", Level::Global).unwrap();
        assert!(!store.get("example.token", None).unwrap().is_set);
    }

    #[test]
    fn settings_secrets_are_isolated_by_project_level_and_global_home() {
        let mut secret = definition("example.token", json!({"type":"string"}), None);
        secret.secret = true;
        let (project, global, first) = store(vec![secret]);
        let second_project = tempfile::tempdir().unwrap();
        let second = SettingsStore::new(
            second_project.path().into(),
            global.path().into(),
            first.registry.clone(),
            first.secrets.clone(),
        );
        first
            .set("example.token", json!("mine"), Level::Local)
            .unwrap();
        first
            .set("example.token", json!("ours"), Level::Global)
            .unwrap();
        assert_eq!(
            second.get("example.token", None).unwrap().value,
            Some(json!("ours"))
        );
        assert!(
            !second
                .get("example.token", Some(Level::Local))
                .unwrap()
                .is_set
        );
        let alternate_home = tempfile::tempdir().unwrap();
        let isolated = SettingsStore::new(
            project.path().into(),
            alternate_home.path().into(),
            first.registry.clone(),
            first.secrets.clone(),
        );
        assert!(!isolated.get("example.token", None).unwrap().is_set);
        first.unset("example.token", Level::Local).unwrap();
        assert_eq!(
            first.get("example.token", None).unwrap().value,
            Some(json!("ours"))
        );
    }

    #[test]
    #[cfg(unix)]
    fn settings_secret_identity_survives_creation_of_a_missing_global_directory() {
        let project = tempfile::tempdir().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(directory.path(), &alias).unwrap();
        let mut secret = definition("example.token", json!({"type":"string"}), None);
        secret.secret = true;
        let mut registry = SettingsRegistry::default();
        registry
            .register_many([secret, number_definition()])
            .unwrap();
        let store = SettingsStore::new(
            project.path().into(),
            alias.join("not-yet-created"),
            Arc::new(RwLock::new(registry)),
            Arc::new(MemorySecrets::default()),
        );
        store
            .set("example.token", json!("fixture-token"), Level::Project)
            .unwrap();
        assert!(!store.global_dir().exists());
        store.set("example.limit", json!(8), Level::Global).unwrap();
        assert_eq!(
            store.get("example.token", None).unwrap().value,
            Some(json!("fixture-token"))
        );
        store.unset("example.token", Level::Project).unwrap();
        assert!(!store.get("example.token", None).unwrap().is_set);
    }

    #[test]
    fn settings_secret_backend_failures_never_fall_back_or_echo_values() {
        let mut secret = definition("example.token", json!({"type":"string"}), None);
        secret.secret = true;
        let (_project, _global, mut store) = store(vec![secret]);
        store.secrets = Arc::new(MemorySecrets {
            fail: true,
            ..Default::default()
        });
        for result in [
            store.set(
                "example.token",
                json!("token-sensitive-value"),
                Level::Global,
            ),
            store.unset("example.token", Level::Global),
            store.get("example.token", None).map(|_| ()),
        ] {
            let error = result.unwrap_err();
            assert!(matches!(error, SettingsError::SecureStorage));
            assert!(!error.to_string().contains("token-sensitive-value"));
        }
        assert!(!store.path(Level::Global).exists());
    }

    #[test]
    fn settings_secrets_cannot_have_manifest_defaults_or_plaintext_overrides() {
        let mut secret = definition(
            "example.token",
            json!({"type":"string"}),
            Some(json!("secret-default")),
        );
        secret.secret = true;
        let mut registry = SettingsRegistry::default();
        assert!(matches!(
            registry.register(secret.clone()),
            Err(SettingsError::InvalidDefinition)
        ));
        secret.default = None;
        secret.schema = json!({"type":"string", "default":"secret-default"});
        assert!(matches!(
            registry.register(secret.clone()),
            Err(SettingsError::InvalidDefinition)
        ));
        secret.schema = json!({"type":"string"});
        let (_project, _global, store) = store(vec![secret]);
        write_layer_key(
            &store.path(Level::Global),
            "example.token",
            Some(&json!("plaintext")),
        )
        .unwrap();
        assert!(matches!(
            store.get("example.token", None),
            Err(SettingsError::PlaintextSecret)
        ));
        assert!(matches!(
            store.set("example.token", json!("secure"), Level::Global),
            Err(SettingsError::PlaintextSecret)
        ));
    }

    #[test]
    fn settings_writes_preserve_other_owners_values_comments_and_default_overrides() {
        let (_project, _global, store) = store(vec![number_definition()]);
        let path = store.path(Level::Project);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = "# retained header\nabsent_extension.option: { on: true }\n# keep this comment\nexample.limit: 5\n";
        std::fs::write(&path, original).unwrap();
        store
            .set("example.limit", json!(10), Level::Project)
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(
            "# retained header\nabsent_extension.option: { on: true }\n# keep this comment\n"
        ));
        store
            .set("example.limit", json!(5), Level::Project)
            .unwrap();
        assert_eq!(
            store.get("example.limit", None).unwrap().origin,
            Some(Origin::Project)
        );
        assert_eq!(
            store
                .layer(Level::Project)
                .unwrap()
                .get("absent_extension.option"),
            Some(&json!({"on":true}))
        );
    }

    #[test]
    fn settings_concurrent_writers_preserve_each_others_keys() {
        let definitions: Vec<_> = (0..12)
            .map(|i| {
                definition(
                    &format!("example.value{i}"),
                    json!({"type":"integer"}),
                    None,
                )
            })
            .collect();
        let (_project, _global, store) = store(definitions);
        std::thread::scope(|scope| {
            for i in 0..12 {
                let store = store.clone();
                scope.spawn(move || {
                    store
                        .set(&format!("example.value{i}"), json!(i), Level::Global)
                        .unwrap()
                });
            }
        });
        let values = store.layer(Level::Global).unwrap();
        assert_eq!(values.len(), 12);
        for i in 0..12 {
            assert_eq!(values.get(&format!("example.value{i}")), Some(&json!(i)));
        }
    }

    #[test]
    fn settings_global_changes_are_visible_to_other_project_stores() {
        let (_project, global, first) = store(vec![number_definition()]);
        let second_project = tempfile::tempdir().unwrap();
        let second = SettingsStore::new(
            second_project.path().into(),
            global.path().into(),
            first.registry.clone(),
            first.secrets.clone(),
        );
        first
            .set("example.limit", json!(25), Level::Global)
            .unwrap();
        assert_eq!(
            second.get("example.limit", None).unwrap().value,
            Some(json!(25))
        );
        second
            .set("example.limit", json!(40), Level::Local)
            .unwrap();
        first
            .set("example.limit", json!(30), Level::Global)
            .unwrap();
        assert_eq!(
            second.get("example.limit", None).unwrap().value,
            Some(json!(40))
        );
    }

    #[test]
    fn settings_invalid_documents_are_not_silently_ignored() {
        let (_project, _global, store) = store(vec![number_definition()]);
        let path = store.path(Level::Global);
        std::fs::write(&path, "[not, a, mapping]\n").unwrap();
        assert!(matches!(
            store.get("example.limit", None),
            Err(SettingsError::InvalidDocument)
        ));
        assert!(matches!(
            store.set("example.limit", json!(3), Level::Global),
            Err(SettingsError::InvalidDocument)
        ));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[not, a, mapping]\n"
        );
    }

    #[test]
    fn settings_read_errors_do_not_expose_invalid_secret_values() {
        let mut secret = definition("example.token", json!({"type":"integer"}), None);
        secret.secret = true;
        let (_project, _global, store) = store(vec![secret]);
        store
            .secrets
            .set(
                &store.secret_identity("example.token", Level::Project),
                "\"token-sensitive-value\"",
            )
            .unwrap();
        let error = store.get("example.token", None).unwrap_err();
        assert!(matches!(error, SettingsError::InvalidValue));
        assert!(!format!("{error:?}").contains("token-sensitive-value"));
    }
}
