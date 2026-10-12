//! SQLite snapshots for namespaced extension workflow state and caches.

use std::collections::BTreeMap;

use oxplow_domain::extension_state::{
    validate_stamp, validate_values, validate_version, Area, Namespace, Snapshot, Stamp, StateError,
};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{database::map_sql_err, Database, TxError};

#[derive(Clone)]
pub struct SqliteExtensionStateStore {
    db: Database,
}

impl SqliteExtensionStateStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn snapshot(&self, namespace: Namespace) -> Result<Option<Snapshot>, StateError> {
        namespace.validate()?;
        self.db
            .read_or(move |tx| match snapshot_tx(tx, &namespace) {
                Ok(snapshot) => Ok(snapshot),
                Err(error) => Err(state_tx_error(error)),
            })
            .await
            .map_err(map_state_tx_error)
    }

    /// Replace a complete namespace only if its whole-namespace stamp still
    /// matches. `None` creates only an absent namespace; rows are never deleted.
    pub async fn compare_exchange(
        &self,
        namespace: Namespace,
        expected: Option<Stamp>,
        schema_version: u32,
        values: BTreeMap<String, serde_json::Value>,
    ) -> Result<Snapshot, StateError> {
        namespace.validate()?;
        validate_version(schema_version)?;
        validate_values(&values)?;
        if let Some(stamp) = expected {
            validate_stamp(stamp)?;
        }

        self.db
            .transaction_or(move |tx| {
                match compare_exchange_tx(tx, &namespace, expected, schema_version, &values) {
                    Ok(snapshot) => Ok(snapshot),
                    Err(error) => Err(state_tx_error(error)),
                }
            })
            .await
            .map_err(map_state_tx_error)
    }
}

/// Read helper for callers composing a larger read transaction.
pub fn snapshot_tx(
    conn: &Connection,
    namespace: &Namespace,
) -> Result<Option<Snapshot>, StateError> {
    namespace.validate()?;
    let row = conn
        .query_row(
            "SELECT schema_version, revision, values_json
             FROM extension_state
             WHERE extension = ?1 AND scope = ?2 AND area = ?3",
            params![
                namespace.extension,
                scope_name(namespace.scope),
                area_name(namespace.area)
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(map_sql_err)?;
    let Some((schema_version, revision, json)) = row else {
        return Ok(None);
    };
    if schema_version <= 0 || schema_version > i64::from(u32::MAX) || revision <= 0 {
        return Err(StateError::Invalid(
            "persisted extension state has an invalid version or revision".into(),
        ));
    }
    if json.len() > oxplow_domain::extension_state::MAX_SNAPSHOT_BYTES {
        return Err(StateError::Invalid(
            "persisted extension state exceeds the serialized byte limit".into(),
        ));
    }
    let values: BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&json).map_err(|_| {
            StateError::Invalid("persisted extension state is not a JSON object".into())
        })?;
    validate_values(&values)?;
    Ok(Some(Snapshot {
        stamp: Stamp {
            schema_version: schema_version as u32,
            revision: revision as u64,
        },
        values,
    }))
}

/// CAS helper for callers composing a larger write transaction. It never
/// starts, commits, or retries a transaction itself.
pub fn compare_exchange_tx(
    conn: &Connection,
    namespace: &Namespace,
    expected: Option<Stamp>,
    schema_version: u32,
    values: &BTreeMap<String, serde_json::Value>,
) -> Result<Snapshot, StateError> {
    namespace.validate()?;
    validate_version(schema_version)?;
    validate_values(values)?;
    if let Some(stamp) = expected {
        validate_stamp(stamp)?;
    }

    let current = snapshot_tx(conn, namespace)?;
    let actual = current.as_ref().map(|snapshot| snapshot.stamp);
    if actual != expected {
        return Err(StateError::Conflict { actual });
    }
    if actual.is_some_and(|stamp| schema_version < stamp.schema_version) {
        return Err(StateError::Invalid(
            "extension state schema version cannot move backwards".into(),
        ));
    }
    let revision = match actual {
        Some(stamp) if stamp.revision < i64::MAX as u64 => stamp.revision + 1,
        Some(_) => {
            return Err(StateError::Invalid(
                "extension state revision limit reached".into(),
            ));
        }
        None => 1,
    };
    let json = serde_json::to_string(values)
        .map_err(|_| StateError::Invalid("extension state is not valid JSON".into()))?;
    if json.len() > oxplow_domain::extension_state::MAX_SNAPSHOT_BYTES {
        return Err(StateError::Invalid(
            "extension namespace exceeds 1048576 serialized bytes".into(),
        ));
    }
    let changed = match expected {
        Some(stamp) => conn.execute(
            "UPDATE extension_state SET schema_version = ?4, revision = ?5, values_json = ?6
             WHERE extension = ?1 AND scope = ?2 AND area = ?3
                 AND schema_version = ?7 AND revision = ?8",
            params![namespace.extension, scope_name(namespace.scope), area_name(namespace.area),
                i64::from(schema_version), revision as i64, json,
                i64::from(stamp.schema_version), stamp.revision as i64],
        ),
        None => conn.execute(
            "INSERT INTO extension_state (extension, scope, area, schema_version, revision, values_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (extension, scope, area) DO NOTHING",
            params![namespace.extension, scope_name(namespace.scope), area_name(namespace.area),
                i64::from(schema_version), revision as i64, json],
        ),
    }.map_err(map_sql_err)?;
    if changed != 1 {
        return Err(StateError::Conflict {
            actual: snapshot_tx(conn, namespace)?.map(|snapshot| snapshot.stamp),
        });
    }
    Ok(Snapshot {
        stamp: Stamp {
            schema_version,
            revision,
        },
        values: values.clone(),
    })
}

fn scope_name(scope: oxplow_domain::extension_state::Scope) -> &'static str {
    match scope {
        oxplow_domain::extension_state::Scope::Project => "project",
        oxplow_domain::extension_state::Scope::Personal => "personal",
    }
}

fn area_name(area: Area) -> &'static str {
    match area {
        Area::State => "state",
        Area::Cache => "cache",
    }
}

fn state_tx_error(error: StateError) -> TxError<StateError> {
    match error {
        StateError::Storage(storage) => TxError::Storage(storage),
        abort => TxError::Aborted(abort),
    }
}

fn map_state_tx_error(error: TxError<StateError>) -> StateError {
    match error {
        TxError::Storage(storage) => StateError::Storage(storage),
        TxError::Aborted(abort) => abort,
    }
}
