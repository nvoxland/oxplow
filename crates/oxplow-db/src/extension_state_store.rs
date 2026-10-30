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

/// Forget everything `extension` stored here, every scope: it was removed.
pub fn forget_extension_tx(
    conn: &rusqlite::Connection,
    extension: &str,
) -> Result<(), oxplow_domain::DomainError> {
    conn.execute(
        "DELETE FROM extension_state WHERE extension = ?1",
        rusqlite::params![extension],
    )
    .map_err(crate::database::map_sql_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::extension_state::{
        Scope, MAX_KEYS, MAX_KEY_BYTES, MAX_SNAPSHOT_BYTES, MAX_VALUE_DEPTH,
    };
    use serde_json::json;

    fn store() -> SqliteExtensionStateStore {
        SqliteExtensionStateStore::new(Database::in_memory())
    }

    fn ns(extension: &str, scope: Scope, area: Area) -> Namespace {
        Namespace {
            extension: extension.into(),
            scope,
            area,
        }
    }

    fn values(pairs: &[(&str, serde_json::Value)]) -> BTreeMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    fn invalid(result: Result<Snapshot, StateError>) -> String {
        match result {
            Err(StateError::Invalid(message)) => message,
            other => panic!("expected an Invalid refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_namespace_snapshot_writes_and_reads_back() {
        let store = store();
        let space = ns("notes", Scope::Project, Area::State);
        assert!(store.snapshot(space.clone()).await.unwrap().is_none());

        let written = store
            .compare_exchange(
                space.clone(),
                None,
                3,
                values(&[("cursor", json!({"page": 4})), ("seen", json!([1, 2]))]),
            )
            .await
            .unwrap();
        assert_eq!(
            written.stamp,
            Stamp {
                schema_version: 3,
                revision: 1
            }
        );

        let read = store.snapshot(space.clone()).await.unwrap().unwrap();
        assert_eq!(read, written);

        // The update replaces the whole namespace and bumps the revision.
        let next = store
            .compare_exchange(
                space.clone(),
                Some(read.stamp),
                3,
                values(&[("n", json!(1))]),
            )
            .await
            .unwrap();
        assert_eq!(next.stamp.revision, 2);
        let read = store.snapshot(space).await.unwrap().unwrap();
        assert_eq!(read.values, values(&[("n", json!(1))]));
    }

    #[tokio::test]
    async fn scopes_areas_and_extensions_are_separate_namespaces() {
        let store = store();
        for (extension, scope, area, marker) in [
            ("alpha", Scope::Project, Area::State, 1),
            ("alpha", Scope::Personal, Area::State, 2),
            ("alpha", Scope::Project, Area::Cache, 3),
            ("beta", Scope::Project, Area::State, 4),
        ] {
            store
                .compare_exchange(
                    ns(extension, scope, area),
                    None,
                    1,
                    values(&[("m", json!(marker))]),
                )
                .await
                .unwrap();
        }
        let read = |extension, scope, area| {
            let store = store.clone();
            async move {
                store
                    .snapshot(ns(extension, scope, area))
                    .await
                    .unwrap()
                    .unwrap()
                    .values["m"]
                    .clone()
            }
        };
        assert_eq!(read("alpha", Scope::Project, Area::State).await, json!(1));
        assert_eq!(read("alpha", Scope::Personal, Area::State).await, json!(2));
        assert_eq!(read("alpha", Scope::Project, Area::Cache).await, json!(3));
        assert_eq!(read("beta", Scope::Project, Area::State).await, json!(4));
    }

    #[tokio::test]
    async fn a_stale_stamp_is_refused_and_leaves_the_namespace_untouched() {
        let store = store();
        let space = ns("notes", Scope::Project, Area::State);
        let first = store
            .compare_exchange(space.clone(), None, 1, values(&[("a", json!(1))]))
            .await
            .unwrap();
        let second = store
            .compare_exchange(
                space.clone(),
                Some(first.stamp),
                1,
                values(&[("a", json!(2))]),
            )
            .await
            .unwrap();

        // Writing from the first stamp again is stale.
        match store
            .compare_exchange(
                space.clone(),
                Some(first.stamp),
                1,
                values(&[("a", json!(9))]),
            )
            .await
        {
            Err(StateError::Conflict { actual }) => assert_eq!(actual, Some(second.stamp)),
            other => panic!("expected a conflict, got {other:?}"),
        }
        // Creating over an existing namespace conflicts too.
        match store
            .compare_exchange(space.clone(), None, 1, values(&[("a", json!(9))]))
            .await
        {
            Err(StateError::Conflict { actual }) => assert_eq!(actual, Some(second.stamp)),
            other => panic!("expected a conflict, got {other:?}"),
        }
        // Updating an absent namespace conflicts with `actual: None`.
        let absent = ns("other", Scope::Project, Area::State);
        match store
            .compare_exchange(absent, Some(first.stamp), 1, BTreeMap::new())
            .await
        {
            Err(StateError::Conflict { actual }) => assert_eq!(actual, None),
            other => panic!("expected a conflict, got {other:?}"),
        }
        let read = store.snapshot(space).await.unwrap().unwrap();
        assert_eq!(read, second);
    }

    #[tokio::test]
    async fn the_writer_sets_the_schema_version_either_way() {
        // The running version writes at its own schema: a later one moves
        // it up, a rolled-back one down. Only the stamp's revision guards.
        let store = store();
        let space = ns("notes", Scope::Project, Area::State);
        let first = store
            .compare_exchange(space.clone(), None, 2, BTreeMap::new())
            .await
            .unwrap();
        let back = store
            .compare_exchange(space.clone(), Some(first.stamp), 1, BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(back.stamp.schema_version, 1);
        let forward = store
            .compare_exchange(space, Some(back.stamp), 5, BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(forward.stamp.schema_version, 5);
    }

    #[tokio::test]
    async fn keys_bytes_and_nesting_are_bounded() {
        let store = store();
        let space = ns("notes", Scope::Project, Area::State);
        let write = |values| {
            let store = store.clone();
            let space = space.clone();
            async move { store.compare_exchange(space, None, 1, values).await }
        };

        // Key shape: lowercase start/end, lowercase/digit/dot/dash/underscore inside.
        for bad in ["", "Upper", "1starts-digit", "ends-", "has space", "ünï"] {
            invalid(write(values(&[(bad, json!(1))])).await);
        }
        invalid(write(values(&[(&"a".repeat(MAX_KEY_BYTES + 1), json!(1))])).await);
        // The boundary itself is fine.
        write(values(&[(&"a".repeat(MAX_KEY_BYTES), json!(1))]))
            .await
            .unwrap();
        let space = ns("notes", Scope::Personal, Area::State);
        let store2 = store.clone();
        let write_ok = |values| {
            let store = store2.clone();
            let space = space.clone();
            async move { store.compare_exchange(space, None, 1, values).await }
        };

        // Key count.
        let many: BTreeMap<_, _> = (0..=MAX_KEYS)
            .map(|n| (format!("k{n}"), json!(0)))
            .collect();
        invalid(write_ok(many).await);

        // Serialized bytes.
        invalid(write_ok(values(&[("blob", json!("x".repeat(MAX_SNAPSHOT_BYTES)))])).await);

        // Nesting.
        let mut deep = json!(1);
        for _ in 0..=MAX_VALUE_DEPTH {
            deep = json!([deep]);
        }
        invalid(write_ok(values(&[("deep", deep)])).await);

        // Version and stamp bounds.
        invalid(
            store
                .compare_exchange(
                    ns("x", Scope::Project, Area::State),
                    None,
                    0,
                    BTreeMap::new(),
                )
                .await,
        );
        invalid(
            store
                .compare_exchange(
                    ns("x", Scope::Project, Area::State),
                    Some(Stamp {
                        schema_version: 1,
                        revision: 0,
                    }),
                    1,
                    BTreeMap::new(),
                )
                .await,
        );
        // An invalid extension name never reaches the table.
        invalid(
            store
                .compare_exchange(
                    ns("Not Valid", Scope::Project, Area::State),
                    None,
                    1,
                    BTreeMap::new(),
                )
                .await,
        );
        assert!(store
            .snapshot(ns("Not Valid", Scope::Project, Area::State))
            .await
            .is_err());

        // None of the refused writes left a row behind.
        assert!(store
            .snapshot(ns("notes", Scope::Personal, Area::State))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn state_survives_reopening_the_same_database_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local.sqlite");
        let space = ns("notes", Scope::Project, Area::State);
        let stamp = {
            let store = SqliteExtensionStateStore::new(Database::open(&path).unwrap());
            store
                .compare_exchange(space.clone(), None, 2, values(&[("kept", json!("yes"))]))
                .await
                .unwrap()
                .stamp
        };
        let store = SqliteExtensionStateStore::new(Database::open(&path).unwrap());
        let read = store.snapshot(space.clone()).await.unwrap().unwrap();
        assert_eq!(read.stamp, stamp);
        assert_eq!(read.values, values(&[("kept", json!("yes"))]));
        // And it keeps accepting writes from the persisted stamp.
        let next = store
            .compare_exchange(space, Some(stamp), 2, BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(next.stamp.revision, stamp.revision + 1);
    }
}
