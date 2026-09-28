//! Storage for extension-declared sources: each entity's rows in a
//! prefixed table (`ext__<extension>__<entity>`) exposed through its
//! semantic-layer view (`v_<extension>_<entity>`), plus per-source run
//! state. The data is a re-syncable cache: every successful run replaces
//! it, and a changed column set just rebuilds the table.
//!
//! Oxplow does all the writing from the declared schema; no
//! extension-provided SQL runs here. See `.context/semantic-layer.md`.

use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

use crate::database::map_sql_err;
use crate::semantic_layer::SqlCell;
use crate::Database;

/// SQLite storage class for a declared column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredType {
    Text,
    Integer,
    Real,
}

impl StoredType {
    fn sql(self) -> &'static str {
        match self {
            StoredType::Text => "TEXT",
            StoredType::Integer => "INTEGER",
            StoredType::Real => "REAL",
        }
    }
}

/// The table/view shape for one entity.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityTable {
    pub extension: String,
    pub entity: String,
    /// `v_<extension>_<entity>`.
    pub view: String,
    pub key: String,
    pub columns: Vec<(String, StoredType)>,
}

/// How one entity's rows land in a run.
#[derive(Debug, Clone, PartialEq)]
pub enum EntityWrite {
    /// These rows are the entity now.
    Replace(Vec<Vec<SqlCell>>),
    /// Add or update these rows by key, and remove the `deleted` keys.
    Upsert {
        rows: Vec<Vec<SqlCell>>,
        deleted: Vec<SqlCell>,
    },
}

/// Last run of one source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SourceState {
    pub extension: String,
    pub source_id: String,
    /// `ok` or `error`.
    pub status: String,
    pub last_run_at: String,
    pub error: Option<String>,
    /// Row counts per entity from the last successful run.
    pub row_counts: std::collections::BTreeMap<String, i64>,
}

#[derive(Clone)]
pub struct SqliteExtSourceStore {
    db: Database,
}

impl SqliteExtSourceStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Replace every entity's rows for one source run (the all-`replace`
    /// form of [`Self::write_rows`]).
    pub async fn replace_rows(
        &self,
        entities: Vec<(EntityTable, Vec<Vec<SqlCell>>)>,
    ) -> Result<(), DomainError> {
        self.write_rows(
            entities
                .into_iter()
                .map(|(t, rows)| (t, EntityWrite::Replace(rows)))
                .collect(),
        )
        .await
        .map(|_| ())
    }

    /// Write one source run's entities, atomically: on any error nothing
    /// changes. Returns each entity's row count afterwards, in order.
    pub async fn write_rows(
        &self,
        entities: Vec<(EntityTable, EntityWrite)>,
    ) -> Result<Vec<i64>, DomainError> {
        for (t, _) in &entities {
            validate_table(t)?;
        }
        self.db
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(map_sql_err)?;
                let mut counts = Vec::with_capacity(entities.len());
                for (t, write) in &entities {
                    write_entity(&tx, t, write)?;
                    counts.push(
                        tx.query_row(
                            &format!("SELECT count(*) FROM {}", quote(&table_name(t))),
                            [],
                            |r| r.get(0),
                        )
                        .map_err(map_sql_err)?,
                    );
                }
                tx.commit().map_err(map_sql_err)?;
                Ok(counts)
            })
            .await
    }

    /// Record a source run's outcome.
    pub async fn record_run(&self, state: SourceState) -> Result<(), DomainError> {
        let counts = serde_json::to_string(&state.row_counts)
            .map_err(|e| DomainError::Storage(format!("row counts: {e}")))?;
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO ext_source_state (extension, source_id, status, last_run_at, error, row_counts_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (extension, source_id) DO UPDATE SET
                       status = excluded.status, last_run_at = excluded.last_run_at,
                       error = excluded.error,
                       -- keep the last good counts when a run fails
                       row_counts_json = CASE WHEN excluded.status = 'ok'
                         THEN excluded.row_counts_json ELSE ext_source_state.row_counts_json END",
                    rusqlite::params![
                        state.extension,
                        state.source_id,
                        state.status,
                        state.last_run_at,
                        state.error,
                        counts
                    ],
                )
                .map(|_| ())
            })
            .await
    }

    pub async fn list_states(&self) -> Result<Vec<SourceState>, DomainError> {
        self.db
            .call(|conn| {
                let mut st = conn.prepare(
                    "SELECT extension, source_id, status, last_run_at, error, row_counts_json
                     FROM ext_source_state ORDER BY extension, source_id",
                )?;
                let rows = st.query_map([], |r| {
                    let counts: String = r.get(5)?;
                    Ok(SourceState {
                        extension: r.get(0)?,
                        source_id: r.get(1)?,
                        status: r.get(2)?,
                        last_run_at: r.get(3)?,
                        error: r.get(4)?,
                        row_counts: serde_json::from_str(&counts).unwrap_or_default(),
                    })
                })?;
                rows.collect()
            })
            .await
    }

    /// Drop every table, view and state row an extension owns.
    pub async fn drop_extension(&self, extension: &str) -> Result<(), DomainError> {
        if !is_ext_name(extension) {
            return Err(DomainError::Invalid(format!(
                "bad extension name `{extension}`"
            )));
        }
        let prefix = table_prefix(extension);
        let ext = extension.to_string();
        self.db
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(map_sql_err)?;
                let owned: Vec<(String, String)> = {
                    let mut st = tx
                        // Exact substring, not LIKE: `_` is a LIKE wildcard, so
                        // `ext__gh__` would match `ext__gh_extra__` (tsk368).
                        .prepare("SELECT type, name FROM sqlite_master WHERE (type = 'view' AND instr(sql, ?1) > 0) OR (type = 'table' AND substr(name, 1, length(?1)) = ?1)")
                        .map_err(map_sql_err)?;
                    let rows = st
                        .query_map([&prefix], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                        .map_err(map_sql_err)?;
                    rows.collect::<rusqlite::Result<_>>().map_err(map_sql_err)?
                };
                for (_, name) in owned.iter().filter(|(k, _)| k == "view") {
                    tx.execute_batch(&format!("DROP VIEW IF EXISTS {}", quote(name)))
                        .map_err(map_sql_err)?;
                }
                for (_, name) in owned.iter().filter(|(k, _)| k == "table") {
                    tx.execute_batch(&format!("DROP TABLE IF EXISTS {}", quote(name)))
                        .map_err(map_sql_err)?;
                }
                tx.execute("DELETE FROM ext_source_state WHERE extension = ?1", [&ext])
                    .map_err(map_sql_err)?;
                tx.commit().map_err(map_sql_err)
            })
            .await
    }
}

/// `ext__<extension>__` with dashes as underscores.
fn table_prefix(extension: &str) -> String {
    format!("ext__{}__", extension.replace('-', "_"))
}

fn table_name(t: &EntityTable) -> String {
    format!("{}{}", table_prefix(&t.extension), t.entity)
}

fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(ch) if ch.is_ascii_lowercase())
        && c.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn is_ext_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Defense in depth: everything that becomes an identifier is checked
/// here even though the declaration parser already validated it.
fn validate_table(t: &EntityTable) -> Result<(), DomainError> {
    let bad = |m: String| Err(DomainError::Invalid(m));
    if !is_ext_name(&t.extension) || !is_ident(&t.entity) {
        return bad(format!(
            "bad extension/entity name `{}`/`{}`",
            t.extension, t.entity
        ));
    }
    if t.view != format!("v_{}_{}", t.extension.replace('-', "_"), t.entity) {
        return bad(format!("view `{}` doesn't match its entity", t.view));
    }
    if t.columns.is_empty() || t.columns.iter().any(|(c, _)| !is_ident(c)) {
        return bad(format!("entity `{}` has an invalid column name", t.entity));
    }
    if !t.columns.iter().any(|(c, _)| c == &t.key) {
        return bad(format!(
            "entity `{}`: key `{}` isn't a column",
            t.entity, t.key
        ));
    }
    Ok(())
}

/// (Re)create the table + view when the declared shape changed, then
/// replace the rows.
fn write_entity(
    tx: &rusqlite::Transaction<'_>,
    t: &EntityTable,
    write: &EntityWrite,
) -> Result<(), DomainError> {
    let (rows, deleted) = match write {
        EntityWrite::Replace(rows) => (rows, None),
        EntityWrite::Upsert { rows, deleted } => (rows, Some(deleted)),
    };
    let table = table_name(t);
    // A view name that exists but doesn't read our table belongs to core
    // or another extension: refuse rather than replace it.
    let existing_view: Option<String> = tx
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'view' AND name = ?1",
            [&t.view],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(map_sql_err)?;
    if let Some(sql) = &existing_view {
        if !sql.contains(&table) {
            return Err(DomainError::Invalid(format!(
                "`{}` is already defined by oxplow or another extension; rename the entity",
                t.view
            )));
        }
    }
    let current: Vec<(String, String, i64)> = {
        let mut st = tx
            .prepare(&format!("PRAGMA table_info({})", quote(&table)))
            .map_err(map_sql_err)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(5)?,
                ))
            })
            .map_err(map_sql_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(map_sql_err)?
    };
    let wanted: Vec<(String, String, i64)> = t
        .columns
        .iter()
        .map(|(c, ty)| (c.clone(), ty.sql().to_string(), i64::from(c == &t.key)))
        .collect();
    if current != wanted || existing_view.is_none() {
        let cols = t
            .columns
            .iter()
            .map(|(c, ty)| format!("{} {}", quote(c), ty.sql()))
            .collect::<Vec<_>>()
            .join(", ");
        let names = t
            .columns
            .iter()
            .map(|(c, _)| quote(c))
            .collect::<Vec<_>>()
            .join(", ");
        tx.execute_batch(&format!(
            "DROP VIEW IF EXISTS {view};
             DROP TABLE IF EXISTS {table};
             CREATE TABLE {table} ({cols}, PRIMARY KEY ({key}));
             CREATE VIEW {view} AS SELECT {names} FROM {table};",
            view = quote(&t.view),
            table = quote(&table),
            key = quote(&t.key),
        ))
        .map_err(map_sql_err)?;
    }
    match deleted {
        // Replace: the run's rows are the whole entity.
        None => tx
            .execute_batch(&format!("DELETE FROM {}", quote(&table)))
            .map_err(map_sql_err)?,
        Some(keys) => {
            let mut del = tx
                .prepare(&format!(
                    "DELETE FROM {} WHERE {} = ?1",
                    quote(&table),
                    quote(&t.key)
                ))
                .map_err(map_sql_err)?;
            for k in keys {
                del.execute([k.to_sql()]).map_err(map_sql_err)?;
            }
        }
    }
    let placeholders = (1..=t.columns.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut ins = tx
        .prepare(&format!(
            "INSERT OR REPLACE INTO {} VALUES ({placeholders})",
            quote(&table)
        ))
        .map_err(map_sql_err)?;
    for (i, row) in rows.iter().enumerate() {
        if row.len() != t.columns.len() {
            return Err(DomainError::Invalid(format!(
                "entity `{}` row {i} has {} values, expected {}",
                t.entity,
                row.len(),
                t.columns.len()
            )));
        }
        let vals: Vec<rusqlite::types::Value> = row.iter().map(SqlCell::to_sql).collect();
        ins.execute(rusqlite::params_from_iter(vals.iter()))
            .map_err(map_sql_err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SemanticLayer;
    use serde_json::json;

    fn pr_table(cols: &[(&str, StoredType)]) -> EntityTable {
        EntityTable {
            extension: "my-gh".into(),
            entity: "pr".into(),
            view: "v_my_gh_pr".into(),
            key: "number".into(),
            columns: cols.iter().map(|(n, t)| (n.to_string(), *t)).collect(),
        }
    }

    fn rows(v: serde_json::Value) -> Vec<Vec<SqlCell>> {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn upsert_adds_updates_and_tombstones_by_key() {
        let db = Database::in_memory();
        let store = SqliteExtSourceStore::new(db.clone());
        let sl = SemanticLayer::new(db);
        let t = pr_table(&[("number", StoredType::Integer), ("title", StoredType::Text)]);
        let counts = store
            .write_rows(vec![(
                t.clone(),
                EntityWrite::Replace(rows(json!([[1, "one"], [2, "two"]]))),
            )])
            .await
            .unwrap();
        assert_eq!(counts, vec![2]);
        let counts = store
            .write_rows(vec![(
                t,
                EntityWrite::Upsert {
                    rows: rows(json!([[2, "TWO"], [3, "three"]])),
                    deleted: vec![SqlCell::Int(1)],
                },
            )])
            .await
            .unwrap();
        assert_eq!(counts, vec![2]);
        let out = sl
            .query_sql(
                "SELECT number, title FROM v_my_gh_pr ORDER BY number",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([[2, "TWO"], [3, "three"]])
        );
    }

    #[tokio::test]
    async fn rows_are_queryable_through_the_view_and_replaced_on_each_run() {
        let db = Database::in_memory();
        let store = SqliteExtSourceStore::new(db.clone());
        let sl = SemanticLayer::new(db);
        let t = pr_table(&[("number", StoredType::Integer), ("title", StoredType::Text)]);

        store
            .replace_rows(vec![(t.clone(), rows(json!([[1, "one"], [2, "two"]])))])
            .await
            .unwrap();
        let out = sl
            .query_sql(
                "SELECT number, title FROM v_my_gh_pr ORDER BY number",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([[1, "one"], [2, "two"]])
        );

        store
            .replace_rows(vec![(t, rows(json!([[3, "three"]])))])
            .await
            .unwrap();
        let out = sl
            .query_sql("SELECT number FROM v_my_gh_pr", vec![], None)
            .await
            .unwrap();
        assert_eq!(serde_json::to_value(&out.rows).unwrap(), json!([[3]]));
    }

    #[tokio::test]
    async fn a_changed_column_set_rebuilds_the_table() {
        let db = Database::in_memory();
        let store = SqliteExtSourceStore::new(db.clone());
        let sl = SemanticLayer::new(db);
        store
            .replace_rows(vec![(
                pr_table(&[("number", StoredType::Integer)]),
                rows(json!([[1]])),
            )])
            .await
            .unwrap();
        let t2 = pr_table(&[
            ("number", StoredType::Integer),
            ("merged", StoredType::Integer),
        ]);
        store
            .replace_rows(vec![(t2, rows(json!([[1, 1]])))])
            .await
            .unwrap();
        let out = sl
            .query_sql("SELECT number, merged FROM v_my_gh_pr", vec![], None)
            .await
            .unwrap();
        assert_eq!(out.columns, vec!["number", "merged"]);
    }

    #[tokio::test]
    async fn a_failed_write_changes_nothing() {
        let db = Database::in_memory();
        let store = SqliteExtSourceStore::new(db.clone());
        let sl = SemanticLayer::new(db);
        let t = pr_table(&[("number", StoredType::Integer), ("title", StoredType::Text)]);
        store
            .replace_rows(vec![(t.clone(), rows(json!([[1, "one"]])))])
            .await
            .unwrap();
        // A row with the wrong arity fails the whole run.
        assert!(store
            .replace_rows(vec![(t, rows(json!([[2]])))])
            .await
            .is_err());
        let out = sl
            .query_sql("SELECT title FROM v_my_gh_pr", vec![], None)
            .await
            .unwrap();
        assert_eq!(serde_json::to_value(&out.rows).unwrap(), json!([["one"]]));
    }

    #[tokio::test]
    async fn rejects_unsafe_identifiers() {
        let store = SqliteExtSourceStore::new(Database::in_memory());
        let mut t = pr_table(&[("number", StoredType::Integer)]);
        t.columns[0].0 = "x); DROP TABLE task; --".into();
        assert!(matches!(
            store.replace_rows(vec![(t, vec![])]).await,
            Err(DomainError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn records_state_and_drops_an_extension() {
        let db = Database::in_memory();
        let store = SqliteExtSourceStore::new(db.clone());
        let sl = SemanticLayer::new(db);
        store
            .replace_rows(vec![(
                pr_table(&[("number", StoredType::Integer)]),
                rows(json!([[1]])),
            )])
            .await
            .unwrap();
        let mut counts = std::collections::BTreeMap::new();
        counts.insert("pr".to_string(), 1);
        let state = SourceState {
            extension: "my-gh".into(),
            source_id: "github".into(),
            status: "ok".into(),
            last_run_at: "2026-09-27T00:00:00Z".into(),
            error: None,
            row_counts: counts,
        };
        store.record_run(state.clone()).await.unwrap();
        assert_eq!(store.list_states().await.unwrap(), vec![state]);

        store.drop_extension("my-gh").await.unwrap();
        assert!(store.list_states().await.unwrap().is_empty());
        assert!(sl
            .query_sql("SELECT * FROM v_my_gh_pr", vec![], None)
            .await
            .is_err());
    }

    /// `_` is a LIKE wildcard: dropping `gh` must leave `gh-extra`'s view
    /// (`ext__gh_extra__…`) alone (tsk368).
    #[tokio::test]
    async fn dropping_an_extension_leaves_similar_names_alone() {
        let db = Database::in_memory();
        let store = SqliteExtSourceStore::new(db.clone());
        let sl = SemanticLayer::new(db);
        let table = |extension: &str, view: &str| EntityTable {
            extension: extension.into(),
            view: view.into(),
            ..pr_table(&[("number", StoredType::Integer)])
        };
        store
            .replace_rows(vec![
                (table("gh", "v_gh_pr"), rows(json!([[1]]))),
                (table("gh-extra", "v_gh_extra_pr"), rows(json!([[2]]))),
            ])
            .await
            .unwrap();
        store.drop_extension("gh").await.unwrap();
        assert!(sl
            .query_sql("SELECT * FROM v_gh_pr", vec![], None)
            .await
            .is_err());
        let out = sl
            .query_sql("SELECT number FROM v_gh_extra_pr", vec![], None)
            .await
            .unwrap();
        assert_eq!(serde_json::to_value(out.rows).unwrap(), json!([[2]]));
    }

    #[tokio::test]
    async fn never_replaces_a_core_view() {
        let db = Database::in_memory();
        let store = SqliteExtSourceStore::new(db.clone());
        // extension `task` + entity `note` would be `v_task_note`, a core view.
        let t = EntityTable {
            extension: "task".into(),
            entity: "note".into(),
            view: "v_task_note".into(),
            key: "id".into(),
            columns: vec![("id".into(), StoredType::Integer)],
        };
        let err = store.replace_rows(vec![(t, vec![])]).await.unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("already defined")),
            "{err:?}"
        );
        // The core view still works.
        SemanticLayer::new(db)
            .query_sql("SELECT body FROM v_task_note", vec![], None)
            .await
            .unwrap();
    }
}
