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

/// Forget everything `extension` stored here, every scope: it was removed.
pub fn forget_extension_tx(
    conn: &rusqlite::Connection,
    extension: &str,
) -> Result<(), oxplow_domain::DomainError> {
    conn.execute(
        "DELETE FROM extension_config WHERE extension = ?1",
        rusqlite::params![extension],
    )
    .map_err(crate::database::map_sql_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> SqliteExtensionConfigStore {
        SqliteExtensionConfigStore::new(Database::in_memory())
    }

    fn ns(extension: &str, scope: Scope) -> ConfigNamespace {
        ConfigNamespace::new(extension.into(), scope).unwrap()
    }

    fn refused(result: Result<ConfigSnapshot, ConfigStoreError>) {
        match result {
            Err(ConfigStoreError::InvalidData) => {}
            Err(other) => panic!("expected InvalidData, got {other:?}"),
            Ok(_) => panic!("expected a refusal"),
        }
    }

    #[tokio::test]
    async fn an_override_layer_writes_and_reads_back_canonically() {
        let store = store();
        let space = ns("notes", Scope::Project);
        assert!(store.read(space.clone()).await.unwrap().is_none());

        let written = store
            .compare_exchange(
                space.clone(),
                None,
                2,
                json!({"b": 1, "a": {"y": 2, "x": 3}}),
            )
            .await
            .unwrap();
        assert_eq!(
            written.stamp(),
            Stamp {
                schema_version: 2,
                revision: 1
            }
        );

        let read = store.read(space.clone()).await.unwrap().unwrap();
        assert_eq!(read.values(), &json!({"a": {"x": 3, "y": 2}, "b": 1}));
        assert_eq!(read.stamp(), written.stamp());

        // A reset is an empty object at the next revision, not a delete.
        let reset = store
            .compare_exchange(space.clone(), Some(read.stamp()), 2, json!({}))
            .await
            .unwrap();
        assert_eq!(reset.stamp().revision, 2);
        assert_eq!(
            store.read(space).await.unwrap().unwrap().values(),
            &json!({})
        );
    }

    #[tokio::test]
    async fn project_and_personal_layers_are_separate_rows() {
        let store = store();
        store
            .compare_exchange(
                ns("notes", Scope::Project),
                None,
                1,
                json!({"who": "project"}),
            )
            .await
            .unwrap();
        store
            .compare_exchange(
                ns("notes", Scope::Personal),
                None,
                1,
                json!({"who": "personal"}),
            )
            .await
            .unwrap();
        let project = store
            .read(ns("notes", Scope::Project))
            .await
            .unwrap()
            .unwrap();
        let personal = store
            .read(ns("notes", Scope::Personal))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(project.values(), &json!({"who": "project"}));
        assert_eq!(personal.values(), &json!({"who": "personal"}));
        assert!(store
            .read(ns("other", Scope::Project))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn a_stale_stamp_is_refused_with_the_current_one() {
        let store = store();
        let space = ns("notes", Scope::Project);
        let first = store
            .compare_exchange(space.clone(), None, 1, json!({"a": 1}))
            .await
            .unwrap();
        let second = store
            .compare_exchange(space.clone(), Some(first.stamp()), 1, json!({"a": 2}))
            .await
            .unwrap();
        for stale in [Some(first.stamp()), None] {
            match store
                .compare_exchange(space.clone(), stale, 1, json!({"a": 3}))
                .await
            {
                Err(ConfigStoreError::Conflict { actual }) => {
                    assert_eq!(actual, Some(second.stamp()));
                }
                Err(other) => panic!("expected a conflict, got {other:?}"),
                Ok(_) => panic!("expected a conflict"),
            }
        }
        assert_eq!(
            store.read(space).await.unwrap().unwrap().values(),
            &json!({"a": 2})
        );
    }

    #[tokio::test]
    async fn the_schema_version_cannot_decrease_but_inspect_reports_it() {
        let store = store();
        let space = ns("notes", Scope::Project);
        let first = store
            .compare_exchange(space.clone(), None, 3, json!({}))
            .await
            .unwrap();
        refused(
            store
                .compare_exchange(space.clone(), Some(first.stamp()), 2, json!({}))
                .await,
        );
        let inspected = store
            .db
            .read_or({
                let space = space.clone();
                move |conn| inspect_tx(conn, &space).map_err(TxError::Aborted)
            })
            .await
            .unwrap();
        assert_eq!(inspected, Some(first.stamp()));
    }

    #[tokio::test]
    async fn values_must_be_a_bounded_object() {
        let store = store();
        let space = ns("notes", Scope::Project);
        for not_object in [json!(1), json!("s"), json!(null), json!([1])] {
            refused(
                store
                    .compare_exchange(space.clone(), None, 1, not_object)
                    .await,
            );
        }
        refused(
            store
                .compare_exchange(
                    space.clone(),
                    None,
                    1,
                    json!({"blob": "x".repeat(MAX_CONFIG_BYTES)}),
                )
                .await,
        );
        let mut deep = json!(1);
        for _ in 0..=MAX_CONFIG_DEPTH {
            deep = json!({"n": deep});
        }
        refused(store.compare_exchange(space.clone(), None, 1, deep).await);
        refused(
            store
                .compare_exchange(space.clone(), None, 0, json!({}))
                .await,
        );
        refused(
            store
                .compare_exchange(
                    space.clone(),
                    Some(Stamp {
                        schema_version: 1,
                        revision: 0,
                    }),
                    1,
                    json!({}),
                )
                .await,
        );
        assert!(ConfigNamespace::new("Not Valid".into(), Scope::Project).is_err());
        assert!(store.read(space).await.unwrap().is_none());
    }

    #[test]
    fn a_write_needs_a_caller_owned_transaction() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../migrations/V53__extension_config.sql"))
            .unwrap();
        let space = ns("notes", Scope::Project);
        // Autocommit: refused before touching the table.
        assert!(matches!(
            compare_exchange_tx(&conn, &space, None, 1, &json!({})),
            Err(ConfigStoreError::TransactionRequired)
        ));
        conn.execute_batch("BEGIN").unwrap();
        compare_exchange_tx(&conn, &space, None, 1, &json!({})).unwrap();
        conn.execute_batch("COMMIT").unwrap();
        assert!(read_tx(&conn, &space).unwrap().is_some());
    }

    #[tokio::test]
    async fn overrides_survive_reopening_the_same_database_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local.sqlite");
        let space = ns("notes", Scope::Personal);
        let stamp = {
            let store = SqliteExtensionConfigStore::new(Database::open(&path).unwrap());
            store
                .compare_exchange(space.clone(), None, 1, json!({"theme": "dark"}))
                .await
                .unwrap()
                .stamp()
        };
        let store = SqliteExtensionConfigStore::new(Database::open(&path).unwrap());
        let read = store.read(space).await.unwrap().unwrap();
        assert_eq!(read.stamp(), stamp);
        assert_eq!(read.values(), &json!({"theme": "dark"}));
    }
}
