//! Private, project-local extension configuration overrides.
//!
//! This store persists opaque JSON values. Schema validation, provenance,
//! secret resolution, and authority checks belong to the host.

use std::io::{self, Write};

use oxplow_domain::{
    contribution::valid_extension_name,
    extension_state::{Scope, Stamp},
    DomainError,
};
use rusqlite::{params, types::ValueRef, Connection};
use serde_json::{Map, Value};

use crate::{database::map_sql_err, Database, TxError};

const MAX_NAMESPACE_BYTES: usize = 128;
const MAX_CONFIG_BYTES: usize = 1_048_576;
const MAX_CONFIG_DEPTH: usize = 64;
const MAX_REVISION: u64 = i64::MAX as u64;

/// A host-derived extension namespace. It cannot be deserialized from an
/// extension request and carries no authority by itself.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigNamespace {
    extension: String,
    scope: Scope,
}

impl ConfigNamespace {
    pub fn new(extension: String, scope: Scope) -> Result<Self, ConfigStoreError> {
        if extension.len() > MAX_NAMESPACE_BYTES || !valid_extension_name(&extension) {
            return Err(ConfigStoreError::InvalidData);
        }
        Ok(Self { extension, scope })
    }

    pub fn extension(&self) -> &str {
        &self.extension
    }

    pub fn scope(&self) -> Scope {
        self.scope
    }
}

/// A persisted whole-namespace value and its compare-and-swap stamp.
/// Values are deliberately not exposed through Debug or a wire schema.
#[derive(Clone, PartialEq)]
pub struct ConfigSnapshot {
    stamp: oxplow_domain::extension_state::Stamp,
    values: Value,
}

impl ConfigSnapshot {
    pub fn stamp(&self) -> oxplow_domain::extension_state::Stamp {
        self.stamp
    }

    pub fn values(&self) -> &Value {
        &self.values
    }
}

/// Errors avoid echoing configuration values or invalid namespace contents.
#[derive(Debug, thiserror::Error)]
pub enum ConfigStoreError {
    #[error("invalid extension configuration data")]
    InvalidData,
    #[error("extension configuration writes require a caller-owned transaction")]
    TransactionRequired,
    #[error("extension configuration conflict; current stamp: {actual:?}")]
    Conflict {
        actual: Option<oxplow_domain::extension_state::Stamp>,
    },
    #[error("extension configuration storage failed")]
    Storage(#[source] DomainError),
}

#[derive(Clone)]
pub struct SqliteExtensionConfigStore {
    db: Database,
}

impl SqliteExtensionConfigStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn read(
        &self,
        namespace: ConfigNamespace,
    ) -> Result<Option<ConfigSnapshot>, ConfigStoreError> {
        self.db
            .read_or(move |conn| match read_tx(conn, &namespace) {
                Ok(snapshot) => Ok(snapshot),
                Err(ConfigStoreError::Storage(storage)) => Err(TxError::Storage(storage)),
                Err(error) => Err(TxError::Aborted(error)),
            })
            .await
            .map_err(map_tx_error)
    }

    pub async fn compare_exchange(
        &self,
        namespace: ConfigNamespace,
        expected: Option<oxplow_domain::extension_state::Stamp>,
        schema_version: u32,
        values: Value,
    ) -> Result<ConfigSnapshot, ConfigStoreError> {
        self.db
            .transaction_or(move |conn| {
                match compare_exchange_tx(conn, &namespace, expected, schema_version, &values) {
                    Ok(snapshot) => Ok(snapshot),
                    Err(ConfigStoreError::Storage(storage)) => Err(TxError::Storage(storage)),
                    Err(error) => Err(TxError::Aborted(error)),
                }
            })
            .await
            .map_err(map_tx_error)
    }
}

/// Read one namespace on a caller-owned connection. Stored text is bounded
/// while borrowed from SQLite, before allocating a String or JSON tree.
pub fn read_tx(
    conn: &Connection,
    namespace: &ConfigNamespace,
) -> Result<Option<ConfigSnapshot>, ConfigStoreError> {
    validate_namespace(namespace)?;
    let mut statement = conn
        .prepare(
            "SELECT extension, scope, schema_version, revision, values_json
             FROM extension_config WHERE extension = ?1 AND scope = ?2",
        )
        .map_err(storage)?;
    let mut rows = statement
        .query(params![namespace.extension, scope_name(namespace.scope)])
        .map_err(storage)?;
    let Some(row) = rows.next().map_err(storage)? else {
        return Ok(None);
    };

    let extension = row.get_ref(0).map_err(storage)?;
    let scope = row.get_ref(1).map_err(storage)?;
    if !text_equals(extension, namespace.extension.as_bytes())
        || !text_equals(scope, scope_name(namespace.scope).as_bytes())
    {
        return Err(ConfigStoreError::InvalidData);
    }
    let stamp = checked_stamp(
        integer_value(row.get_ref(2).map_err(storage)?)?,
        integer_value(row.get_ref(3).map_err(storage)?)?,
    )?;
    let json = match row.get_ref(4).map_err(storage)? {
        ValueRef::Text(bytes) if bytes.len() <= MAX_CONFIG_BYTES => {
            std::str::from_utf8(bytes).map_err(|_| ConfigStoreError::InvalidData)?
        }
        _ => return Err(ConfigStoreError::InvalidData),
    };
    let values: Value = serde_json::from_str(json).map_err(|_| ConfigStoreError::InvalidData)?;
    validate_value(&values)?;
    let canonical = canonicalize(&values);
    let canonical_json =
        serde_json::to_string(&canonical).map_err(|_| ConfigStoreError::InvalidData)?;
    if canonical_json != json {
        return Err(ConfigStoreError::InvalidData);
    }
    Ok(Some(ConfigSnapshot {
        stamp,
        values: canonical,
    }))
}

/// Read only the layer's version and revision. This remains useful when a
/// stored value is invalid under the current declaration and needs migration.
pub fn inspect_tx(
    conn: &Connection,
    namespace: &ConfigNamespace,
) -> Result<Option<Stamp>, ConfigStoreError> {
    validate_namespace(namespace)?;
    let mut statement = conn
        .prepare(
            "SELECT schema_version, revision FROM extension_config
             WHERE extension = ?1 AND scope = ?2",
        )
        .map_err(storage)?;
    let mut rows = statement
        .query(params![namespace.extension, scope_name(namespace.scope)])
        .map_err(storage)?;
    let Some(row) = rows.next().map_err(storage)? else {
        return Ok(None);
    };
    checked_stamp(
        integer_value(row.get_ref(0).map_err(storage)?)?,
        integer_value(row.get_ref(1).map_err(storage)?)?,
    )
    .map(Some)
}

/// Compare and replace a complete namespace inside the caller's transaction.
/// No transaction is started or committed here.
pub fn compare_exchange_tx(
    conn: &Connection,
    namespace: &ConfigNamespace,
    expected: Option<oxplow_domain::extension_state::Stamp>,
    schema_version: u32,
    values: &Value,
) -> Result<ConfigSnapshot, ConfigStoreError> {
    if conn.is_autocommit() {
        return Err(ConfigStoreError::TransactionRequired);
    }
    validate_version(schema_version)?;
    if let Some(stamp) = expected {
        validate_stamp(stamp)?;
    }
    validate_value(values)?;

    let current = read_tx(conn, namespace)?;
    let actual = current.as_ref().map(ConfigSnapshot::stamp);
    if actual != expected {
        return Err(ConfigStoreError::Conflict { actual });
    }
    if actual.is_some_and(|stamp| schema_version < stamp.schema_version) {
        return Err(ConfigStoreError::InvalidData);
    }
    let revision = match actual {
        Some(stamp) if stamp.revision < MAX_REVISION => stamp.revision + 1,
        Some(_) => return Err(ConfigStoreError::InvalidData),
        None => 1,
    };
    let values = canonicalize(values);
    let json = serde_json::to_string(&values).map_err(|_| ConfigStoreError::InvalidData)?;
    if json.len() > MAX_CONFIG_BYTES {
        return Err(ConfigStoreError::InvalidData);
    }

    conn.execute_batch("SAVEPOINT extension_config_publication")
        .map_err(storage)?;
    let write = match expected {
        Some(stamp) => conn.execute(
            "UPDATE extension_config
             SET schema_version = ?3, revision = ?4, values_json = ?5
             WHERE extension = ?1 AND scope = ?2 AND schema_version = ?6 AND revision = ?7",
            params![
                namespace.extension,
                scope_name(namespace.scope),
                i64::from(schema_version),
                revision as i64,
                json,
                i64::from(stamp.schema_version),
                stamp.revision as i64
            ],
        ),
        None => conn.execute(
            "INSERT INTO extension_config (extension, scope, schema_version, revision, values_json)
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (extension, scope) DO NOTHING",
            params![
                namespace.extension,
                scope_name(namespace.scope),
                i64::from(schema_version),
                revision as i64,
                json
            ],
        ),
    };
    let written = match write {
        Ok(written) => written,
        Err(error) => {
            rollback_savepoint(conn)?;
            return Err(storage(error));
        }
    };
    if written != 1 {
        rollback_savepoint(conn)?;
        return Err(ConfigStoreError::Conflict {
            actual: read_tx(conn, namespace)?.map(|snapshot| snapshot.stamp),
        });
    }
    if let Err(error) = conn.execute_batch("RELEASE extension_config_publication") {
        rollback_savepoint(conn)?;
        return Err(storage(error));
    }
    Ok(ConfigSnapshot {
        stamp: oxplow_domain::extension_state::Stamp {
            schema_version,
            revision,
        },
        values,
    })
}

fn rollback_savepoint(conn: &Connection) -> Result<(), ConfigStoreError> {
    conn.execute_batch(
        "ROLLBACK TO extension_config_publication; RELEASE extension_config_publication",
    )
    .map_err(storage)
}

fn validate_version(version: u32) -> Result<(), ConfigStoreError> {
    if version == 0 {
        Err(ConfigStoreError::InvalidData)
    } else {
        Ok(())
    }
}

fn validate_stamp(stamp: oxplow_domain::extension_state::Stamp) -> Result<(), ConfigStoreError> {
    if stamp.schema_version == 0 || stamp.revision == 0 || stamp.revision > MAX_REVISION {
        Err(ConfigStoreError::InvalidData)
    } else {
        Ok(())
    }
}

fn checked_stamp(schema_version: i64, revision: i64) -> Result<Stamp, ConfigStoreError> {
    if !(1..=i64::from(u32::MAX)).contains(&schema_version) || revision <= 0 {
        return Err(ConfigStoreError::InvalidData);
    }
    Ok(Stamp {
        schema_version: schema_version as u32,
        revision: revision as u64,
    })
}

fn validate_namespace(namespace: &ConfigNamespace) -> Result<(), ConfigStoreError> {
    if namespace.extension.len() > MAX_NAMESPACE_BYTES
        || !valid_extension_name(&namespace.extension)
    {
        return Err(ConfigStoreError::InvalidData);
    }
    Ok(())
}

fn validate_value(value: &Value) -> Result<(), ConfigStoreError> {
    if !value.is_object() || depth_exceeded(value, 1) {
        return Err(ConfigStoreError::InvalidData);
    }
    serde_json::to_writer(&mut SizeCounter(0), value).map_err(|_| ConfigStoreError::InvalidData)?;
    Ok(())
}

fn depth_exceeded(value: &Value, depth: usize) -> bool {
    if depth > MAX_CONFIG_DEPTH {
        return true;
    }
    match value {
        Value::Array(items) => items.iter().any(|child| depth_exceeded(child, depth + 1)),
        Value::Object(items) => items.values().any(|child| depth_exceeded(child, depth + 1)),
        _ => false,
    }
}

struct SizeCounter(usize);

impl Write for SizeCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_CONFIG_BYTES)
            .ok_or_else(|| io::Error::other("extension config size limit"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        Value::Object(items) => {
            let mut keys: Vec<_> = items.keys().collect();
            keys.sort_unstable();
            let mut canonical = Map::new();
            for key in keys {
                canonical.insert(
                    key.clone(),
                    canonicalize(items.get(key).expect("key came from this object")),
                );
            }
            Value::Object(canonical)
        }
        scalar => scalar.clone(),
    }
}

fn text_equals(value: ValueRef<'_>, expected: &[u8]) -> bool {
    matches!(value, ValueRef::Text(bytes) if bytes == expected)
}

fn integer_value(value: ValueRef<'_>) -> Result<i64, ConfigStoreError> {
    match value {
        ValueRef::Integer(value) => Ok(value),
        _ => Err(ConfigStoreError::InvalidData),
    }
}

fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::Project => "project",
        Scope::Personal => "personal",
    }
}

fn storage(error: rusqlite::Error) -> ConfigStoreError {
    ConfigStoreError::Storage(map_sql_err(error))
}

fn map_tx_error(error: TxError<ConfigStoreError>) -> ConfigStoreError {
    match error {
        TxError::Storage(storage) => ConfigStoreError::Storage(storage),
        TxError::Aborted(error) => error,
    }
}
