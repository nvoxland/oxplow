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
    #[serde(default)]
    pub ui: SettingUi,
    #[serde(default)]
    pub restart_required: bool,
    /// Assigned by the registry host, never accepted as caller authority.
    pub owner: String,
}

/// A Settings category: what a group of settings is called, what it is for,
/// and where it sits among the others. Core declares its own; an extension
/// declares its own in `configuration: categories:`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingCategory {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Lower first; then title.
    #[serde(default)]
    pub order: i32,
    /// `core` or `extension/<namespace>`; empty for an undeclared one.
    #[serde(default)]
    pub owner: String,
}

/// Where a category nobody declared sits: after the declared ones.
pub const UNDECLARED_CATEGORY_ORDER: i32 = 1000;

impl SettingCategory {
    /// A category a setting names but nobody declared: its title from its id
    /// (`languageServers` is "Language servers").
    pub fn undeclared(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            title: words_of(id),
            description: String::new(),
            order: UNDECLARED_CATEGORY_ORDER,
            owner: String::new(),
        }
    }
}

/// `languageServers.extra_tools` as "Language servers extra tools".
fn words_of(id: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    for part in id
        .split(['.', '_', '-', ' '])
        .filter(|part| !part.is_empty())
    {
        let mut word = String::new();
        for c in part.chars() {
            if c.is_uppercase() && !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            word.extend(c.to_lowercase());
        }
        words.push(word);
    }
    let text = words.join(" ");
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// How the Settings editor draws a setting beyond what its schema says:
/// one fixed vocabulary (an extension's `x-ui`), so a hint is never a silent
/// no-op. A choice comes from the schema (`enum`, `boolean`), a secret is
/// `x-secret`, Markdown is `contentMediaType: text/markdown`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingUi {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<SettingEditor>,
}

/// A text control other than the one-line input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SettingEditor {
    /// A text area: prompt text, one item per line.
    Multiline,
}

pub use oxplow_domain::settings::Level;

/// The three override files, read once, in [`Level::PRECEDENCE`] order.
/// Cheap to clone. Every read of settings goes through one, so a page or a
/// command sees one state of the files.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Documents(Arc<[BTreeMap<String, Value>; 3]>);

fn index(level: Level) -> usize {
    match level {
        Level::Local => 0,
        Level::Project => 1,
        Level::Global => 2,
    }
}

impl Documents {
    /// Read the three files. A file that cannot be read or parsed fails.
    pub fn read(project_dir: &Path, global_dir: &Path) -> Result<Self, SettingsError> {
        let [local, project, global] =
            Level::PRECEDENCE.map(|level| read_layer(project_dir, global_dir, level));
        Ok(Self::new([local?, project?, global?]))
    }

    /// Read the three files; one that cannot be read or parsed is empty.
    pub fn read_or_empty(project_dir: &Path, global_dir: &Path) -> Self {
        Self::new(
            Level::PRECEDENCE
                .map(|level| read_layer(project_dir, global_dir, level).unwrap_or_default()),
        )
    }

    /// The documents in [`Level::PRECEDENCE`] order.
    pub fn new(levels: [BTreeMap<String, Value>; 3]) -> Self {
        Self(Arc::new(levels))
    }

    /// One level's raw overrides, unknown keys included.
    pub fn at(&self, level: Level) -> &BTreeMap<String, Value> {
        &self.0[index(level)]
    }

    /// Each level with its overrides, in precedence order.
    pub fn levels(&self) -> impl Iterator<Item = (Level, &BTreeMap<String, Value>)> {
        Level::PRECEDENCE
            .into_iter()
            .map(move |level| (level, self.at(level)))
    }

    /// The same capture (not only equal documents): a cache hit.
    pub fn shares(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// The first level that sets `key`, and its value there.
    pub fn first(&self, key: &str) -> Option<(Level, &Value)> {
        self.levels()
            .find_map(|(level, values)| values.get(key).map(|value| (level, value)))
    }
}

/// The source of one complete effective value: a level, or the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Default,
    Local,
    Project,
    Global,
}

impl Origin {
    /// The level that set the value; `None` for the default.
    pub fn level(self) -> Option<Level> {
        match self {
            Self::Default => None,
            Self::Local => Some(Level::Local),
            Self::Project => Some(Level::Project),
            Self::Global => Some(Level::Global),
        }
    }
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
/// use [`SettingsSnapshot::inspect`] or [`SettingsSnapshot::row`] for display.
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
    /// Resolution failed; presence/origin are unknown, never reported as an
    /// inherited value or accompanied by secure-backend details.
    pub unavailable: bool,
}

/// One setting's Settings row: its redacted effective value and, per level
/// in [`Level::PRECEDENCE`] order, its ordinary override and status.
#[derive(Debug)]
pub struct SettingRow {
    /// The setting's category, declared or [`SettingCategory::undeclared`].
    pub category: SettingCategory,
    pub setting: SettingInventory,
    pub overrides: [Option<Value>; 3],
    pub statuses: [&'static str; 3],
}

/// Backed by the machine keychain in production and memory in tests. Errors
/// are deliberately discarded at this boundary; backends may name secrets.
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>, String>;
    /// Endpoint-bound backends can use the same ordinary snapshot as the read.
    fn get_with_documents(
        &self,
        name: &str,
        _documents: &Documents,
    ) -> Result<Option<String>, String> {
        self.get(name)
    }
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
    /// The value's schema refused it: where, and why.
    #[error("{}", refused_text(.0))]
    ValueRefused(Vec<ValueProblem>),
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

/// One reason a schema refused a value: where in the value (a JSON pointer,
/// empty for the whole value) and why, in words taken from the schema alone.
/// The value itself is never repeated, so a refused secret stays secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ValueProblem {
    pub pointer: String,
    pub reason: String,
}

/// At most this many problems are reported for one value.
const MAX_PROBLEMS: usize = 8;

fn refused_text(problems: &[ValueProblem]) -> String {
    let parts: Vec<_> = problems
        .iter()
        .map(|problem| match problem.pointer.as_str() {
            "" => format!("the value {}", problem.reason),
            pointer => format!("`{pointer}` {}", problem.reason),
        })
        .collect();
    parts.join("; ")
}

fn listed(values: &[Value]) -> String {
    values
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn article(kind: &str) -> &'static str {
    if matches!(kind.chars().next(), Some('a' | 'e' | 'i' | 'o' | 'u')) {
        "an"
    } else {
        "a"
    }
}

/// Why, from the schema's side of the error only: limits, choices and
/// field names the schema declares; never the value or its keys.
fn reason(kind: &jsonschema::error::ValidationErrorKind, keyword: &str) -> String {
    use jsonschema::error::{TypeKind, ValidationErrorKind as K};
    match kind {
        K::Type {
            kind: TypeKind::Single(kind),
        } => {
            let kind = kind.to_string();
            format!("must be {} {kind}", article(&kind))
        }
        K::Type { .. } => "has the wrong type".into(),
        K::Minimum { limit } => format!("is below the minimum {limit}"),
        K::Maximum { limit } => format!("is above the maximum {limit}"),
        K::ExclusiveMinimum { limit } => format!("must be more than {limit}"),
        K::ExclusiveMaximum { limit } => format!("must be less than {limit}"),
        K::MinLength { limit } => format!("is shorter than {limit} characters"),
        K::MaxLength { limit } => format!("is longer than {limit} characters"),
        K::MinItems { limit } => format!("has fewer than {limit} items"),
        K::MaxItems { limit } => format!("has more than {limit} items"),
        K::MinProperties { limit } => format!("has fewer than {limit} entries"),
        K::MaxProperties { limit } => format!("has more than {limit} entries"),
        K::Enum { options } => match options.as_array() {
            Some(options) => format!("must be one of: {}", listed(options)),
            None => "isn't one of the choices".into(),
        },
        K::Constant { expected_value } => format!("must be {expected_value}"),
        K::Required { property } => match property.as_str() {
            Some(name) => format!("misses the field `{name}`"),
            None => "misses a required field".into(),
        },
        K::AdditionalProperties { .. } => "has a field the setting doesn't have".into(),
        K::Pattern { pattern } => format!("doesn't match the pattern `{pattern}`"),
        K::Format { format } => format!("isn't a valid {format}"),
        K::UniqueItems => "has the same item twice".into(),
        K::AnyOf { .. } | K::OneOfNotValid { .. } => {
            "doesn't match any of the allowed forms".into()
        }
        K::OneOfMultipleValid { .. } => "matches more than one allowed form".into(),
        _ => format!("doesn't satisfy `{keyword}`"),
    }
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
    categories: BTreeMap<String, SettingCategory>,
}

impl SettingsRegistry {
    /// Declare categories all-or-nothing. An id is one owner's.
    pub fn register_categories(
        &mut self,
        categories: impl IntoIterator<Item = SettingCategory>,
    ) -> Result<(), SettingsError> {
        let mut additions = BTreeMap::new();
        for category in categories {
            if category.id.trim().is_empty() || category.title.trim().is_empty() {
                return Err(SettingsError::InvalidDefinition);
            }
            if self.categories.contains_key(&category.id) || additions.contains_key(&category.id) {
                return Err(SettingsError::DuplicateKey);
            }
            additions.insert(category.id.clone(), category);
        }
        self.categories.extend(additions);
        Ok(())
    }

    /// The category `id`: as declared, or [`SettingCategory::undeclared`].
    pub fn category(&self, id: &str) -> SettingCategory {
        self.categories
            .get(id)
            .cloned()
            .unwrap_or_else(|| SettingCategory::undeclared(id))
    }

    /// The declared categories.
    pub fn categories(&self) -> impl Iterator<Item = &SettingCategory> {
        self.categories.values()
    }

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
        self.categories
            .retain(|_, category| category.owner != owner);
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
        || (definition.secret
            && (definition.default.is_some() || contains_default(&definition.schema)))
    {
        return Err(SettingsError::InvalidDefinition);
    }
    bounded(&definition.schema)?;
    // Schemas are immutable; registries for projects and extension generations
    // share compiled validators without sharing values or owner metadata.
    static VALIDATORS: std::sync::OnceLock<
        std::sync::Mutex<BTreeMap<String, Arc<jsonschema::Validator>>>,
    > = std::sync::OnceLock::new();
    let schema_key = definition.schema.to_string();
    let mut cache = VALIDATORS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let validator = match cache.get(&schema_key) {
        Some(validator) => validator.clone(),
        None => {
            let validator = Arc::new(
                jsonschema::options()
                    .offline()
                    .build(&definition.schema)
                    .map_err(|_| SettingsError::InvalidSchema)?,
            );
            // Bound retained schemas even when many extension generations run.
            if cache.len() >= 512 {
                cache.clear();
            }
            cache.insert(schema_key, validator.clone());
            validator
        }
    };
    drop(cache);
    if let Some(default) = &definition.default {
        validate_value(&validator, default)?;
    }
    Ok(RegisteredSetting {
        definition,
        validator,
    })
}

/// Preserve one declaration property as a single registered key segment.
pub fn key_segment(segment: &str) -> String {
    if !segment.is_empty()
        && !segment.starts_with("__hex_")
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        segment.to_owned()
    } else {
        format!(
            "__hex_{}",
            segment
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        )
    }
}

pub fn decoded_segment(segment: &str) -> Option<String> {
    let Some(hex) = segment.strip_prefix("__hex_") else {
        return Some(segment.to_owned());
    };
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|at| {
            hex.get(at..at + 2)
                .and_then(|byte| u8::from_str_radix(byte, 16).ok())
        })
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
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
        return Ok(());
    }
    let mut problems: Vec<_> = validator
        .iter_errors(value)
        .map(|error| ValueProblem {
            pointer: error.instance_path().to_string(),
            reason: reason(error.kind(), error.kind().keyword()),
        })
        .collect();
    problems.sort_by(|a, b| a.pointer.cmp(&b.pointer));
    problems.dedup();
    problems.truncate(MAX_PROBLEMS);
    Err(SettingsError::ValueRefused(problems))
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

    /// Read the three files once, for any number of reads.
    pub fn snapshot(&self) -> Result<SettingsSnapshot, SettingsError> {
        Ok(self.snapshot_of(Documents::read(&self.project_dir, &self.global_dir)?))
    }

    /// Reads over documents the caller already captured.
    pub fn snapshot_of(&self, documents: Documents) -> SettingsSnapshot {
        SettingsSnapshot {
            store: self.clone(),
            documents,
        }
    }

    /// Make an isolated registry for a declaration-specific projection.
    pub fn fork_registry(&self) -> Self {
        let mut fork = self.clone();
        fork.registry = Arc::new(RwLock::new(
            self.registry
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .clone(),
        ));
        fork
    }

    /// Apply ordinary settings to one level with a single locked replacement.
    /// Secrets cannot participate in a YAML transaction.
    pub fn update(
        &self,
        changes: &[(String, Option<Value>)],
        level: Level,
    ) -> Result<(), SettingsError> {
        for (key, value) in changes {
            if self.definition(key)?.secret {
                return Err(SettingsError::PlaintextSecret);
            }
            if let Some(value) = value {
                self.validate(key, value)?;
            }
        }
        write_layer_keys(&self.path(level), changes)
    }

    pub fn set(&self, key: &str, value: Value, level: Level) -> Result<(), SettingsError> {
        let definition = self.definition(key)?;
        self.validate(key, &value)?;
        if definition.secret {
            if self.raw(level)?.contains_key(key) {
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
            if self.raw(level)?.contains_key(key) {
                return Err(SettingsError::PlaintextSecret);
            }
            self.secrets
                .delete(&self.secret_identity(key, level))
                .map_err(|_| SettingsError::SecureStorage)
        } else {
            write_layer_key(&self.path(level), key, None)
        }
    }

    fn raw(&self, level: Level) -> Result<BTreeMap<String, Value>, SettingsError> {
        read_layer(&self.project_dir, &self.global_dir, level)
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

/// Settings reads over one capture of the three files ([`Documents`]).
/// Secrets are read from secure storage at each read; a secret's endpoint
/// check uses these same documents.
#[derive(Clone)]
pub struct SettingsSnapshot {
    store: SettingsStore,
    documents: Documents,
}

impl SettingsSnapshot {
    pub fn documents(&self) -> &Documents {
        &self.documents
    }

    pub fn store(&self) -> &SettingsStore {
        &self.store
    }

    /// The value: the effective one (or the default) without a level, else
    /// only what `level` sets. A secret's value is included.
    pub fn get(&self, key: &str, level: Option<Level>) -> Result<SettingValue, SettingsError> {
        self.read(&self.store.definition(key)?, level, false)
    }

    /// As [`Self::get`], with a secret's value left out: presence and origin.
    pub fn inspect(&self, key: &str, level: Option<Level>) -> Result<SettingValue, SettingsError> {
        self.read(&self.store.definition(key)?, level, true)
    }

    /// One setting's Settings row. A failure stays in the row: a broken
    /// credential cannot hide other settings.
    pub fn row(&self, key: &str) -> Result<SettingRow, SettingsError> {
        Ok(self.row_of(&self.store.definition(key)?))
    }

    /// Every registered setting's row.
    pub fn inventory(&self) -> Vec<SettingRow> {
        let definitions: Vec<_> = self
            .store
            .registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .definitions()
            .cloned()
            .collect();
        definitions
            .iter()
            .map(|definition| self.row_of(definition))
            .collect()
    }

    fn read(
        &self,
        definition: &SettingDefinition,
        level: Option<Level>,
        redact: bool,
    ) -> Result<SettingValue, SettingsError> {
        match level {
            None => self.read_levels(definition, &Level::PRECEDENCE, true, redact),
            Some(level) => self.read_levels(definition, &[level], false, redact),
        }
    }

    fn read_levels(
        &self,
        definition: &SettingDefinition,
        levels: &[Level],
        include_default: bool,
        redact: bool,
    ) -> Result<SettingValue, SettingsError> {
        let key = &definition.key;
        if definition.secret
            && levels
                .iter()
                .any(|level| self.documents.at(*level).contains_key(key))
        {
            return Err(SettingsError::PlaintextSecret);
        }
        for level in levels {
            let value = if definition.secret {
                self.store
                    .secrets
                    .get_with_documents(&self.store.secret_identity(key, *level), &self.documents)
                    .map_err(|_| SettingsError::SecureStorage)?
                    .map(|secret| {
                        serde_json::from_str(&secret).map_err(|_| SettingsError::SecureStorage)
                    })
                    .transpose()?
            } else {
                self.documents.at(*level).get(key).cloned()
            };
            if let Some(value) = value {
                self.store.validate(key, &value)?;
                return Ok(SettingValue {
                    key: key.clone(),
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
            key: key.clone(),
            origin: value.as_ref().map(|_| Origin::Default),
            is_set: value.is_some(),
            value,
            secret: definition.secret,
        })
    }

    fn row_of(&self, definition: &SettingDefinition) -> SettingRow {
        let levels: [_; 3] =
            Level::PRECEDENCE.map(|level| self.read_levels(definition, &[level], false, true));
        let statuses = std::array::from_fn(|index| match &levels[index] {
            Ok(value) if value.is_set => "set",
            Ok(_) => "unset",
            Err(_) => "unavailable",
        });
        // A failure in a shadowed level doesn't obscure a valid effective value.
        let effective = levels
            .iter()
            .find(|value| !matches!(value, Ok(value) if !value.is_set));
        let (value, origin, is_set, unavailable) = match effective {
            Some(Ok(value)) => (value.value.clone(), value.origin, true, false),
            Some(Err(_)) => (None, None, false, true),
            None => (
                definition.default.clone(),
                definition.default.as_ref().map(|_| Origin::Default),
                definition.default.is_some(),
                false,
            ),
        };
        // Plaintext secrets are refused even beneath a valid secure override.
        let plaintext = definition.secret
            && self
                .documents
                .levels()
                .any(|(_, values)| values.contains_key(&definition.key));
        let overrides = if definition.secret {
            [None, None, None]
        } else {
            Level::PRECEDENCE.map(|level| self.documents.at(level).get(&definition.key).cloned())
        };
        let category = self
            .store
            .registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .category(&definition.category);
        SettingRow {
            category,
            setting: SettingInventory {
                definition: definition.clone(),
                value: if plaintext { None } else { value },
                origin: if plaintext { None } else { origin },
                is_set: !plaintext && is_set,
                unavailable: plaintext || unavailable,
            },
            overrides,
            statuses,
        }
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
    write_layer_keys(path, &[(key.to_owned(), value.cloned())])
}

/// Edit all keys before replacing the file, so failures leave the old document.
pub fn write_layer_keys(
    path: &Path,
    changes: &[(String, Option<Value>)],
) -> Result<(), SettingsError> {
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
    let mut edited = original;
    for (key, value) in changes {
        match value {
            Some(value) => {
                expected.insert(key.clone(), value.clone());
            }
            None => {
                expected.remove(key);
            }
        }
        let block = value
            .as_ref()
            .map(|value| {
                let mut map = serde_yaml::Mapping::new();
                map.insert(
                    serde_yaml::Value::String(key.clone()),
                    crate::to_yaml(value),
                );
                serde_yaml::to_string(&map).map_err(|_| SettingsError::InvalidValue)
            })
            .transpose()?;
        edited = crate::edit_top_level_key(&edited, key, block.as_deref());
    }
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
            ui: Default::default(),
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
    fn inventory_reads_each_secure_level_once_and_keeps_columns_consistent() {
        struct ChangingSecrets {
            reads: std::sync::atomic::AtomicUsize,
            path: PathBuf,
        }
        impl SecretStore for ChangingSecrets {
            fn get(&self, _: &str) -> Result<Option<String>, String> {
                self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                std::fs::write(&self.path, "example.limit: 99\n").unwrap();
                Ok(Some("\"secret-marker\"".into()))
            }
            fn set(&self, _: &str, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn delete(&self, _: &str) -> Result<(), String> {
                Ok(())
            }
        }
        let mut secret = definition("example.aSecret", json!({"type":"string"}), None);
        secret.secret = true;
        let (_project, _global, mut store) = store(vec![number_definition(), secret]);
        store
            .set("example.limit", json!(7), Level::Project)
            .unwrap();
        let secrets = Arc::new(ChangingSecrets {
            reads: Default::default(),
            path: store.path(Level::Project),
        });
        store.secrets = secrets.clone();
        let snapshot = store.snapshot().unwrap().inventory();
        assert_eq!(secrets.reads.load(std::sync::atomic::Ordering::SeqCst), 3);
        let number = snapshot
            .iter()
            .find(|row| row.setting.definition.key == "example.limit")
            .unwrap();
        assert_eq!(number.setting.value, Some(json!(7)));
        assert_eq!(number.overrides, [None, Some(json!(7)), None]);
        assert_eq!(number.statuses, ["unset", "set", "unset"]);
        let secret = snapshot
            .iter()
            .find(|row| row.setting.definition.secret)
            .unwrap();
        assert_eq!(secret.statuses, ["set", "set", "set"]);
        assert_eq!(secret.overrides, [None, None, None]);
        assert!(secret.setting.value.is_none());
        assert_eq!(
            store
                .snapshot()
                .unwrap()
                .get("example.limit", None)
                .unwrap()
                .value,
            Some(json!(99))
        );
    }

    #[test]
    fn a_refused_value_says_where_and_why_without_the_value() {
        let (_project, _global, store) = store(vec![
            number_definition(),
            definition(
                "example.names",
                json!({"type":"array","items":{"type":"string","maxLength":3}}),
                None,
            ),
            definition("example.mode", json!({"enum":["fast","safe"]}), None),
        ]);
        let refused = |key: &str, value: Value| match store.validate(key, &value) {
            Err(SettingsError::ValueRefused(problems)) => problems,
            other => panic!("{other:?}"),
        };
        let problems = refused("example.limit", json!(0));
        assert_eq!(problems[0].pointer, "");
        assert_eq!(problems[0].reason, "is below the minimum 1");
        let problems = refused("example.names", json!(["ok", 7, "hunter2"]));
        let pointers: Vec<_> = problems.iter().map(|p| p.pointer.as_str()).collect();
        assert_eq!(pointers, ["/1", "/2"]);
        assert_eq!(problems[0].reason, "must be a string");
        assert_eq!(problems[1].reason, "is longer than 3 characters");
        let text = SettingsError::ValueRefused(problems).to_string();
        assert!(text.contains("`/1` must be a string"), "{text}");
        assert!(!text.contains("hunter2"), "{text}");
        assert_eq!(
            refused("example.mode", json!("slow"))[0].reason,
            "must be one of: \"fast\", \"safe\""
        );
    }

    #[test]
    fn a_category_is_registered_with_its_title_or_gets_one_from_its_id() {
        let mut registry = SettingsRegistry::default();
        registry
            .register_categories([SettingCategory {
                id: "example".into(),
                title: "Examples".into(),
                description: "Settings the tests use.".into(),
                order: 5,
                owner: "example".into(),
            }])
            .unwrap();
        assert_eq!(registry.category("example").title, "Examples");
        // Another owner can't take an id that is registered.
        assert!(matches!(
            registry.register_categories([SettingCategory::undeclared("example")]),
            Err(SettingsError::DuplicateKey)
        ));
        let undeclared = registry.category("languageServers.extra_tools");
        assert_eq!(undeclared.title, "Language servers extra tools");
        assert_eq!(undeclared.order, UNDECLARED_CATEGORY_ORDER);
        registry.remove_owner("example");
        assert_eq!(registry.category("example").title, "Example");
        let (_project, _global, store) = store(vec![number_definition()]);
        store
            .registry
            .write()
            .unwrap()
            .register_categories([SettingCategory {
                id: "example".into(),
                title: "Examples".into(),
                description: String::new(),
                order: 1,
                owner: "example".into(),
            }])
            .unwrap();
        let row = store.snapshot().unwrap().row("example.limit").unwrap();
        assert_eq!(row.category.title, "Examples");
    }

    #[test]
    fn one_snapshot_answers_every_read_from_the_files_it_read() {
        let (_project, _global, store) = store(vec![
            number_definition(),
            definition("example.name", json!({"type":"string"}), None),
        ]);
        store.set("example.limit", json!(3), Level::Global).unwrap();
        store
            .set("example.limit", json!(7), Level::Project)
            .unwrap();
        let snapshot = store.snapshot().unwrap();
        // A write after the snapshot is not part of it.
        store.set("example.limit", json!(9), Level::Local).unwrap();
        let effective = snapshot.get("example.limit", None).unwrap();
        assert_eq!(effective.value, Some(json!(7)));
        assert_eq!(effective.origin, Some(Origin::Project));
        assert_eq!(
            snapshot
                .get("example.limit", Some(Level::Global))
                .unwrap()
                .value,
            Some(json!(3))
        );
        assert!(snapshot
            .get("example.limit", Some(Level::Local))
            .unwrap()
            .value
            .is_none());
        assert_eq!(snapshot.documents().at(Level::Project)["example.limit"], 7);
        let row = snapshot.row("example.limit").unwrap();
        assert_eq!(row.setting.value, Some(json!(7)));
        assert_eq!(row.overrides, [None, Some(json!(7)), Some(json!(3))]);
        assert_eq!(snapshot.inventory().len(), 2);
        assert!(matches!(
            snapshot.get("example.missing", None),
            Err(SettingsError::UnknownSetting)
        ));
        assert_eq!(
            store
                .snapshot()
                .unwrap()
                .get("example.limit", None)
                .unwrap()
                .value,
            Some(json!(9))
        );
    }

    #[test]
    fn ordinary_batches_validate_every_key_before_one_replacement() {
        let (_project, _global, store) = store(vec![
            number_definition(),
            definition("example.name", json!({"type":"string"}), None),
        ]);
        store
            .set("example.limit", json!(5), Level::Project)
            .unwrap();
        let original = std::fs::read(store.path(Level::Project)).unwrap();
        assert!(store
            .update(
                &[
                    ("example.limit".into(), Some(json!(8))),
                    ("example.name".into(), Some(json!(9))),
                ],
                Level::Project
            )
            .is_err());
        assert_eq!(std::fs::read(store.path(Level::Project)).unwrap(), original);
        store
            .update(
                &[
                    ("example.limit".into(), Some(json!(8))),
                    ("example.name".into(), Some(json!("valid"))),
                ],
                Level::Project,
            )
            .unwrap();
        let values = ["example.limit", "example.name"]
            .map(|key| store.snapshot().unwrap().get(key, None).unwrap());
        assert_eq!(values[0].value, Some(json!(8)));
        assert_eq!(values[1].value, Some(json!("valid")));
    }

    #[test]
    fn identical_schemas_share_compilation_across_registry_instances() {
        let mut first = SettingsRegistry::default();
        let mut second = SettingsRegistry::default();
        first.register(number_definition()).unwrap();
        second.register(number_definition()).unwrap();
        assert!(Arc::ptr_eq(
            &first.settings["example.limit"].validator,
            &second.settings["example.limit"].validator
        ));
    }

    #[test]
    fn inventory_retains_ordinary_settings_when_secure_storage_is_unavailable() {
        let mut secret = definition("example.token", json!({"type":"string"}), None);
        secret.secret = true;
        let (project, global, original) = store(vec![number_definition(), secret]);
        let settings = SettingsStore::new(
            project.path().into(),
            global.path().into(),
            original.registry.clone(),
            Arc::new(MemorySecrets {
                fail: true,
                ..Default::default()
            }),
        );
        let inventory: Vec<_> = settings
            .snapshot()
            .unwrap()
            .inventory()
            .into_iter()
            .map(|row| row.setting)
            .collect();
        let public = inventory
            .iter()
            .find(|item| item.definition.key == "example.limit")
            .unwrap();
        assert_eq!(public.value, Some(json!(5)));
        assert!(!public.unavailable);
        let secret = inventory
            .iter()
            .find(|item| item.definition.key == "example.token")
            .unwrap();
        assert!(secret.unavailable);
        assert_eq!(secret.origin, None);
        assert_eq!(secret.value, None);
        assert!(!serde_json::to_string(&inventory)
            .unwrap()
            .contains("token-sensitive-value"));
        assert!(matches!(
            settings.snapshot().unwrap().get("example.token", None),
            Err(SettingsError::SecureStorage)
        ));
    }

    #[test]
    fn settings_resolve_each_level_and_unset_reveals_inheritance() {
        let (_project, _global, store) = store(vec![number_definition()]);
        let read = || {
            store
                .snapshot()
                .unwrap()
                .get("example.limit", None)
                .unwrap()
        };
        assert_eq!(read().value, Some(json!(5)));
        assert_eq!(read().origin, Some(Origin::Default));
        assert!(
            !store
                .snapshot()
                .unwrap()
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
                .snapshot()
                .unwrap()
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
            store
                .snapshot()
                .unwrap()
                .get("example.object", None)
                .unwrap()
                .value,
            Some(json!({"d":4}))
        );
        assert_eq!(
            store
                .snapshot()
                .unwrap()
                .get("example.array", None)
                .unwrap()
                .value,
            Some(json!([3]))
        );
        store.unset("example.object", Level::Local).unwrap();
        assert_eq!(
            store
                .snapshot()
                .unwrap()
                .get("example.object", None)
                .unwrap()
                .value,
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
        let value = store
            .snapshot()
            .unwrap()
            .get("example.nullable", None)
            .unwrap();
        assert_eq!(value.value, Some(Value::Null));
        assert_eq!(value.origin, Some(Origin::Local));
        store.unset("example.nullable", Level::Local).unwrap();
        assert_eq!(
            store
                .snapshot()
                .unwrap()
                .get("example.nullable", None)
                .unwrap()
                .value,
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
            Err(SettingsError::ValueRefused(_))
        ));
        assert!(matches!(
            store.set("example.missing", json!(10), Level::Project),
            Err(SettingsError::UnknownSetting)
        ));
        assert_eq!(std::fs::read(store.path(Level::Project)).unwrap(), before);
        assert!(matches!(
            store.snapshot().unwrap().get("example.missing", None),
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
            Err(SettingsError::ValueRefused(_))
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
        assert!(
            !store
                .snapshot()
                .unwrap()
                .get("example.token", None)
                .unwrap()
                .is_set
        );
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
        let value = store
            .snapshot()
            .unwrap()
            .get("example.token", None)
            .unwrap();
        assert_eq!(value.value, Some(json!("project-secret")));
        assert_eq!(value.origin, Some(Origin::Project));
        assert!(!format!("{value:?}").contains("project-secret"));
        let inventory: Vec<_> = store
            .snapshot()
            .unwrap()
            .inventory()
            .into_iter()
            .map(|row| row.setting)
            .collect();
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
            store
                .snapshot()
                .unwrap()
                .get("example.token", None)
                .unwrap()
                .value,
            Some(json!("token-sensitive-value"))
        );
        store.unset("example.token", Level::Global).unwrap();
        assert!(
            !store
                .snapshot()
                .unwrap()
                .get("example.token", None)
                .unwrap()
                .is_set
        );
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
            second
                .snapshot()
                .unwrap()
                .get("example.token", None)
                .unwrap()
                .value,
            Some(json!("ours"))
        );
        assert!(
            !second
                .snapshot()
                .unwrap()
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
        assert!(
            !isolated
                .snapshot()
                .unwrap()
                .get("example.token", None)
                .unwrap()
                .is_set
        );
        first.unset("example.token", Level::Local).unwrap();
        assert_eq!(
            first
                .snapshot()
                .unwrap()
                .get("example.token", None)
                .unwrap()
                .value,
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
            store
                .snapshot()
                .unwrap()
                .get("example.token", None)
                .unwrap()
                .value,
            Some(json!("fixture-token"))
        );
        store.unset("example.token", Level::Project).unwrap();
        assert!(
            !store
                .snapshot()
                .unwrap()
                .get("example.token", None)
                .unwrap()
                .is_set
        );
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
            store
                .snapshot()
                .unwrap()
                .get("example.token", None)
                .map(|_| ()),
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
            store.snapshot().unwrap().get("example.token", None),
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
            store
                .snapshot()
                .unwrap()
                .get("example.limit", None)
                .unwrap()
                .origin,
            Some(Origin::Project)
        );
        assert_eq!(
            store
                .snapshot()
                .unwrap()
                .documents()
                .at(Level::Project)
                .clone()
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
        let values = store
            .snapshot()
            .unwrap()
            .documents()
            .at(Level::Global)
            .clone();
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
            second
                .snapshot()
                .unwrap()
                .get("example.limit", None)
                .unwrap()
                .value,
            Some(json!(25))
        );
        second
            .set("example.limit", json!(40), Level::Local)
            .unwrap();
        first
            .set("example.limit", json!(30), Level::Global)
            .unwrap();
        assert_eq!(
            second
                .snapshot()
                .unwrap()
                .get("example.limit", None)
                .unwrap()
                .value,
            Some(json!(40))
        );
    }

    #[test]
    fn settings_invalid_documents_are_not_silently_ignored() {
        let (_project, _global, store) = store(vec![number_definition()]);
        let path = store.path(Level::Global);
        std::fs::write(&path, "[not, a, mapping]\n").unwrap();
        assert!(matches!(
            store.snapshot(),
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
        let error = store
            .snapshot()
            .unwrap()
            .get("example.token", None)
            .unwrap_err();
        assert!(matches!(error, SettingsError::ValueRefused(_)));
        assert!(!format!("{error:?}").contains("token-sensitive-value"));
        assert!(!error.to_string().contains("token-sensitive-value"));
    }
}
