//! Namespaced, project-local extension state and cache snapshots.
//!
//! This is a persistence contract, not an authority boundary. The host must
//! derive the extension name from its execution context before calling a store.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{contribution::valid_extension_name, DomainError, Json};

pub const MAX_KEYS: usize = 256;
pub const MAX_KEY_BYTES: usize = 128;
pub const MAX_SNAPSHOT_BYTES: usize = 1_048_576;
/// Stay below serde_json's decoding recursion limit, including the map.
pub const MAX_VALUE_DEPTH: usize = 64;

/// One extension's storage namespace in the current project's database.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct Namespace {
    pub extension: String,
    pub scope: Scope,
    pub area: Area,
}

impl Namespace {
    pub fn validate(&self) -> Result<(), StateError> {
        if self.extension.len() > MAX_KEY_BYTES || !valid_extension_name(&self.extension) {
            return Err(StateError::Invalid("invalid extension namespace".into()));
        }
        Ok(())
    }
}

/// Both scopes live in the local project database. Project serves this
/// extension's project runs; Personal serves this host's personal workspace.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Type,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Project,
    Personal,
}

/// State is durable workflow data; Cache is disposable, rebuildable data.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Type,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    State,
    Cache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub schema_version: u32,
    pub revision: u64,
}

/// The whole namespace is the compare-and-swap unit, preventing per-key ABA.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub stamp: Stamp,
    #[specta(type = BTreeMap<String, Json>)]
    pub values: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum StateError {
    #[error("extension state conflict; current stamp: {actual:?}")]
    Conflict { actual: Option<Stamp> },
    #[error("invalid extension state: {0}")]
    Invalid(String),
    #[error(transparent)]
    Storage(#[from] DomainError),
}

impl StateError {
    pub fn from_domain(error: DomainError) -> Self {
        match error {
            DomainError::Invalid(message) => Self::Invalid(message),
            other => Self::Storage(other),
        }
    }
}

/// Validate caller-supplied values and the byte budget used by the store.
pub fn validate_values(values: &BTreeMap<String, serde_json::Value>) -> Result<(), StateError> {
    if values.len() > MAX_KEYS {
        return Err(StateError::Invalid(format!(
            "extension namespace exceeds {MAX_KEYS} keys"
        )));
    }
    for key in values.keys() {
        let bytes = key.as_bytes();
        let last = bytes.last().copied();
        let valid = !bytes.is_empty()
            && bytes.len() <= MAX_KEY_BYTES
            && bytes[0].is_ascii_lowercase()
            && last.is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && bytes.iter().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(*byte, b'.' | b'-' | b'_')
            });
        if !valid {
            return Err(StateError::Invalid(format!(
                "extension state keys must be 1..={MAX_KEY_BYTES} ASCII characters, start with a lowercase letter, contain lowercase letters/digits/dot/dash/underscore, and end with a lowercase letter or digit"
            )));
        }
    }
    let mut lower_bound = 2usize; // top-level braces
    for (index, (key, value)) in values.iter().enumerate() {
        add_lower_bound(&mut lower_bound, usize::from(u8::from(index > 0)))?;
        add_lower_bound(&mut lower_bound, key.len().saturating_add(3))?; // quoted key + colon
        lower_bound_value(value, 2, &mut lower_bound)?;
    }
    let mut writer = BoundedCountWriter::default();
    if serde_json::to_writer(&mut writer, values).is_err() {
        return Err(if writer.exceeded {
            too_large()
        } else {
            StateError::Invalid("extension state is not valid JSON".into())
        });
    }
    Ok(())
}

fn add_lower_bound(total: &mut usize, bytes: usize) -> Result<(), StateError> {
    *total = total.saturating_add(bytes);
    if *total > MAX_SNAPSHOT_BYTES {
        return Err(too_large());
    }
    Ok(())
}

fn lower_bound_value(
    value: &serde_json::Value,
    depth: usize,
    total: &mut usize,
) -> Result<(), StateError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(StateError::Invalid(format!(
            "extension state exceeds {MAX_VALUE_DEPTH} levels of JSON nesting"
        )));
    }
    match value {
        serde_json::Value::Null => add_lower_bound(total, 4),
        serde_json::Value::Bool(true) => add_lower_bound(total, 4),
        serde_json::Value::Bool(false) => add_lower_bound(total, 5),
        serde_json::Value::Number(_) => add_lower_bound(total, 1),
        serde_json::Value::String(value) => add_lower_bound(total, value.len().saturating_add(2)),
        serde_json::Value::Array(values) => {
            add_lower_bound(total, 2)?; // brackets
            for (index, child) in values.iter().enumerate() {
                add_lower_bound(total, usize::from(u8::from(index > 0)))?;
                lower_bound_value(child, depth + 1, total)?;
            }
            Ok(())
        }
        serde_json::Value::Object(values) => {
            add_lower_bound(total, 2)?; // braces
            for (index, (key, child)) in values.iter().enumerate() {
                add_lower_bound(total, usize::from(index > 0))?;
                add_lower_bound(total, key.len().saturating_add(3))?; // quoted key + colon
                lower_bound_value(child, depth + 1, total)?;
            }
            Ok(())
        }
    }
}

fn too_large() -> StateError {
    StateError::Invalid(format!(
        "extension namespace exceeds {MAX_SNAPSHOT_BYTES} serialized bytes"
    ))
}

#[derive(Default)]
struct BoundedCountWriter {
    bytes: usize,
    exceeded: bool,
}

impl std::io::Write for BoundedCountWriter {
    fn write(&mut self, chunk: &[u8]) -> std::io::Result<usize> {
        let Some(bytes) = self.bytes.checked_add(chunk.len()) else {
            self.exceeded = true;
            return Err(std::io::Error::other("size limit"));
        };
        if bytes > MAX_SNAPSHOT_BYTES {
            self.exceeded = true;
            return Err(std::io::Error::other("size limit"));
        }
        self.bytes = bytes;
        Ok(chunk.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn validate_version(schema_version: u32) -> Result<(), StateError> {
    if schema_version == 0 {
        return Err(StateError::Invalid(
            "extension state schema version must be positive".into(),
        ));
    }
    Ok(())
}

pub fn validate_stamp(stamp: Stamp) -> Result<(), StateError> {
    validate_version(stamp.schema_version)?;
    if stamp.revision == 0 || stamp.revision > i64::MAX as u64 {
        return Err(StateError::Invalid(
            "extension state revision must be in 1..=i64::MAX".into(),
        ));
    }
    Ok(())
}
