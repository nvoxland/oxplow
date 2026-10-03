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

use rusqlite::{params, OptionalExtension};

use crate::database::{map_sql_err, ts_to_string};
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
    pub fn sql(self) -> &'static str {
        match self {
            StoredType::Text => "TEXT",
            StoredType::Integer => "INTEGER",
            StoredType::Real => "REAL",
        }
    }
}

/// The table/view shape for one entity, and what documents it.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityTable {
    pub extension: String,
    pub entity: String,
    /// `v_<extension>_<entity>`.
    pub view: String,
    pub key: String,
    /// What the entity is, for the catalog (`v_model.description`).
    pub description: String,
    pub columns: Vec<EntityColumn>,
}

/// One column of an entity: stored as `stored`, documented by `doc` (its
/// contract in `v_model_column`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityColumn {
    pub name: String,
    pub stored: StoredType,
    pub doc: String,
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

/// Last run of one collector (`collector_run`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CollectorRun {
    /// The declaring extension, `project` or `built-in`.
    pub owner: String,
    pub id: String,
    /// `ok`, `error`, `needs_approval` or `skipped` (the loop guard).
    pub status: String,
    pub last_run_at: String,
    pub error: Option<String>,
    /// Row counts per entity from the last successful run.
    pub row_counts: std::collections::BTreeMap<String, i64>,
    /// The checkpoint the last successful run returned (opaque).
    #[specta(type = Option<oxplow_domain::Json>)]
    pub cursor: Option<serde_json::Value>,
    /// The seq of the last trigger event it ran for.
    pub last_event_id: Option<i64>,
}

#[derive(Clone)]
pub struct SqliteCollectorStore {
    db: Database,
}

impl SqliteCollectorStore {
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

    /// Write one run's entities, atomically: on any error nothing
    /// changes. Returns each entity's row count afterwards, in order.
    pub async fn write_rows(
        &self,
        entities: Vec<(EntityTable, EntityWrite)>,
    ) -> Result<Vec<i64>, DomainError> {
        self.db
            .transaction(move |tx| write_rows_tx(tx, &entities))
            .await
    }

    /// Record a collector run's outcome. A failed run keeps the last good
    /// counts and checkpoint; `last_event_id` only moves forward.
    pub async fn record_run(&self, run: CollectorRun) -> Result<(), DomainError> {
        self.db.call(move |conn| record_run_in(conn, &run)).await
    }

    /// One collector's last run, if it has run.
    pub async fn run_of(&self, owner: &str, id: &str) -> Result<Option<CollectorRun>, DomainError> {
        let (owner, id) = (owner.to_string(), id.to_string());
        self.db
            .call(move |conn| {
                let mut st = conn.prepare(&format!("{RUN_SELECT} WHERE owner = ?1 AND id = ?2"))?;
                let mut rows = st.query_map([&owner, &id], run_row)?;
                rows.next().transpose()
            })
            .await
    }

    pub async fn list_runs(&self) -> Result<Vec<CollectorRun>, DomainError> {
        self.db
            .call(|conn| {
                let mut st = conn.prepare(&format!("{RUN_SELECT} ORDER BY owner, id"))?;
                let rows = st.query_map([], run_row)?;
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
                let names = |sql: &str, param: &str| -> Result<Vec<String>, DomainError> {
                    let mut st = tx.prepare(sql).map_err(map_sql_err)?;
                    let rows = st
                        .query_map([param], |r| r.get::<_, String>(0))
                        .map_err(map_sql_err)?;
                    rows.collect::<rusqlite::Result<_>>().map_err(map_sql_err)
                };
                // Its views, from the registry: entities and SQL models alike.
                for view in names("SELECT view FROM model WHERE owner = ?1", &ext)? {
                    tx.execute_batch(&format!("DROP VIEW IF EXISTS {}", quote(&view)))
                        .map_err(map_sql_err)?;
                }
                tx.execute("DELETE FROM model WHERE owner = ?1", [&ext])
                    .map_err(map_sql_err)?;
                // Its tables. An exact prefix, not LIKE: `_` is a LIKE
                // wildcard, so `ext__gh__` would match `ext__gh_extra__` (tsk368).
                for table in names(
                    "SELECT name FROM sqlite_master WHERE type = 'table' AND substr(name, 1, length(?1)) = ?1",
                    &prefix,
                )? {
                    tx.execute_batch(&format!("DROP TABLE IF EXISTS {}", quote(&table)))
                        .map_err(map_sql_err)?;
                }
                tx.execute("DELETE FROM collector_run WHERE owner = ?1", [&ext])
                    .map_err(map_sql_err)?;
                tx.commit().map_err(map_sql_err)
            })
            .await
    }
}

/// Write entities on a caller's transaction (a collector run commits its
/// rows, its run state and its `collector.synced` event together).
/// Returns each entity's row count afterwards, in order.
pub fn write_rows_tx(
    tx: &rusqlite::Transaction<'_>,
    entities: &[(EntityTable, EntityWrite)],
) -> Result<Vec<i64>, DomainError> {
    for (t, _) in entities {
        validate_table(t)?;
    }
    let mut counts = Vec::with_capacity(entities.len());
    for (t, write) in entities {
        write_entity(tx, t, write)?;
        counts.push(
            tx.query_row(
                &format!("SELECT count(*) FROM {}", quote(&table_name(t))),
                [],
                |r| r.get(0),
            )
            .map_err(map_sql_err)?,
        );
    }
    Ok(counts)
}

const RUN_SELECT: &str =
    "SELECT owner, id, status, last_run_at, error, row_counts_json, cursor_json, last_event_id
     FROM collector_run";

fn run_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<CollectorRun> {
    let counts: String = r.get(5)?;
    let cursor: Option<String> = r.get(6)?;
    Ok(CollectorRun {
        owner: r.get(0)?,
        id: r.get(1)?,
        status: r.get(2)?,
        last_run_at: r.get(3)?,
        error: r.get(4)?,
        row_counts: serde_json::from_str(&counts).unwrap_or_default(),
        cursor: cursor.and_then(|c| serde_json::from_str(&c).ok()),
        last_event_id: r.get(7)?,
    })
}

/// Upsert one collector's run on `conn` — a caller's transaction when
/// the run's event is logged with it.
pub fn record_run_in(conn: &rusqlite::Connection, run: &CollectorRun) -> rusqlite::Result<()> {
    let counts = serde_json::to_string(&run.row_counts).unwrap_or_else(|_| "{}".into());
    let cursor = run.cursor.as_ref().map(|c| c.to_string());
    conn.execute(
        "INSERT INTO collector_run (owner, id, status, last_run_at, error, row_counts_json, cursor_json, last_event_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (owner, id) DO UPDATE SET
           status = excluded.status, last_run_at = excluded.last_run_at,
           error = excluded.error,
           -- a failed run keeps the last good counts and checkpoint
           row_counts_json = CASE WHEN excluded.status = 'ok'
             THEN excluded.row_counts_json ELSE collector_run.row_counts_json END,
           cursor_json = CASE WHEN excluded.status = 'ok'
             THEN coalesce(excluded.cursor_json, collector_run.cursor_json) ELSE collector_run.cursor_json END,
           last_event_id = max(coalesce(excluded.last_event_id, 0), coalesce(collector_run.last_event_id, 0))",
        rusqlite::params![
            run.owner,
            run.id,
            run.status,
            run.last_run_at,
            run.error,
            counts,
            cursor,
            run.last_event_id
        ],
    )
    .map(|_| ())
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
    if t.columns.is_empty() || t.columns.iter().any(|c| !is_ident(&c.name)) {
        return bad(format!("entity `{}` has an invalid column name", t.entity));
    }
    if !t.columns.iter().any(|c| c.name == t.key) {
        return bad(format!(
            "entity `{}`: key `{}` isn't a column",
            t.entity, t.key
        ));
    }
    Ok(())
}

/// Record an entity view as its extension's model: owner, how it's made
/// (`entity`), its description, the table it reads, and its columns with
/// their docs as its contract. It follows the extension's declaration, so
/// a changed one replaces it; an unchanged one writes nothing.
fn register_entity(
    tx: &rusqlite::Transaction<'_>,
    t: &EntityTable,
    table: &str,
    select: &str,
) -> Result<(), DomainError> {
    let contract = serde_json::to_string(
        &t.columns
            .iter()
            .map(|c| serde_json::json!({ "name": c.name, "type": c.stored.sql(), "doc": c.doc }))
            .collect::<Vec<_>>(),
    )
    .map_err(|e| DomainError::Invalid(e.to_string()))?;
    let current: Option<(String, String, Option<String>)> = tx
        .query_row(
            "SELECT m.description, m.sql, c.columns_json FROM model m
             LEFT JOIN model_contract c ON c.view = m.view AND c.version = m.version
             WHERE m.view = ?1",
            [&t.view],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(map_sql_err)?;
    if current.as_ref()
        == Some(&(
            t.description.clone(),
            select.to_string(),
            Some(contract.clone()),
        ))
    {
        return Ok(());
    }
    let now = ts_to_string(oxplow_domain::Timestamp::now());
    tx.execute(
        "INSERT INTO model (view, name, owner, version, description, sql, compiled_at, kind)
         VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, 'entity')
         ON CONFLICT (view) DO UPDATE SET description = excluded.description,
             sql = excluded.sql, compiled_at = excluded.compiled_at",
        params![t.view, t.entity, t.extension, t.description, select, now],
    )
    .map_err(map_sql_err)?;
    tx.execute(
        "INSERT INTO model_contract (view, version, columns_json, recorded_at)
         VALUES (?1, 1, ?2, ?3)
         ON CONFLICT (view, version) DO UPDATE SET columns_json = excluded.columns_json,
             recorded_at = excluded.recorded_at",
        params![t.view, contract, now],
    )
    .map_err(map_sql_err)?;
    tx.execute(
        "INSERT OR IGNORE INTO model_input (view, input, kind) VALUES (?1, ?2, 'source')",
        params![t.view, table],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// (Re)create the view when it's new or the declared shape changed (the
/// table too, for a new shape), then write the rows.
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
    // The registry says who owns a view (P4.9): only this extension's own
    // entity may be replaced; any other view by that name is refused.
    let owner: Option<(String, String)> = tx
        .query_row(
            "SELECT owner, kind FROM model WHERE view = ?1",
            [&t.view],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(map_sql_err)?;
    let view_exists: bool = tx
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = ?1)",
            [&t.view],
            |r| r.get(0),
        )
        .map_err(map_sql_err)?;
    match &owner {
        Some((o, kind)) if o == &t.extension && kind == "entity" => {}
        Some((o, _)) => {
            return Err(DomainError::Invalid(format!(
                "`{}` belongs to `{o}`; rename the entity",
                t.view
            )))
        }
        None if view_exists => {
            return Err(DomainError::Invalid(format!(
                "`{}` is already defined; rename the entity",
                t.view
            )))
        }
        None => {}
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
        .map(|c| {
            (
                c.name.clone(),
                c.stored.sql().to_string(),
                i64::from(c.name == t.key),
            )
        })
        .collect();
    let names = t
        .columns
        .iter()
        .map(|c| quote(&c.name))
        .collect::<Vec<_>>()
        .join(", ");
    let select = format!("SELECT {names} FROM {}", quote(&table));
    let registered = owner.is_some() && view_exists;
    if current != wanted || !registered {
        let cols = t
            .columns
            .iter()
            .map(|c| format!("{} {}", quote(&c.name), c.stored.sql()))
            .collect::<Vec<_>>()
            .join(", ");
        tx.execute_batch(&format!("DROP VIEW IF EXISTS {}", quote(&t.view)))
            .map_err(map_sql_err)?;
        if current != wanted {
            // The shape changed: the stored rows go with it.
            tx.execute_batch(&format!(
                "DROP TABLE IF EXISTS {table};
                 CREATE TABLE {table} ({cols}, PRIMARY KEY ({key}));",
                table = quote(&table),
                key = quote(&t.key),
            ))
            .map_err(map_sql_err)?;
        }
        tx.execute_batch(&format!("CREATE VIEW {} AS {select}", quote(&t.view)))
            .map_err(map_sql_err)?;
    }
    register_entity(tx, t, &table, &select)?;
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
            description: "A pull request.".into(),
            columns: cols
                .iter()
                .map(|(n, t)| EntityColumn {
                    name: n.to_string(),
                    stored: *t,
                    doc: format!("{n} doc"),
                })
                .collect(),
        }
    }

    fn rows(v: serde_json::Value) -> Vec<Vec<SqlCell>> {
        serde_json::from_value(v).unwrap()
    }

    async fn registered(db: &Database) -> Vec<(String, String, String, String)> {
        db.read(|tx| {
            let mut st = tx
                .prepare(
                    "SELECT m.view, m.owner, m.kind, coalesce(group_concat(i.input), '')
                     FROM model m LEFT JOIN model_input i ON i.view = m.view
                     WHERE m.owner <> 'core' GROUP BY m.view ORDER BY m.view",
                )
                .map_err(map_sql_err)?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(map_sql_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_sql_err)?;
            Ok(rows)
        })
        .await
        .unwrap()
    }

    /// P4.9 (tsk494): an entity view is a model its extension owns — in the
    /// registry with its table as input, so lineage and subscriptions see
    /// it; kept across the open that recompiles the SQL models; gone with
    /// its extension. Another extension, or a core view, can't take the name.
    #[tokio::test]
    async fn an_entity_view_is_a_model_its_extension_owns() {
        let db = Database::in_memory();
        let store = SqliteCollectorStore::new(db.clone());
        let t = pr_table(&[("number", StoredType::Integer), ("title", StoredType::Text)]);
        store
            .replace_rows(vec![(t.clone(), rows(json!([[1, "one"]])))])
            .await
            .unwrap();
        assert_eq!(
            registered(&db).await,
            vec![(
                "v_my_gh_pr".to_string(),
                "my-gh".to_string(),
                "entity".to_string(),
                "ext__my_gh__pr".to_string()
            )]
        );
        // A second extension whose view name would collide.
        let other = EntityTable {
            extension: "my".into(),
            entity: "gh_pr".into(),
            view: "v_my_gh_pr".into(),
            key: "number".into(),
            description: String::new(),
            columns: vec![EntityColumn {
                name: "number".into(),
                stored: StoredType::Integer,
                doc: String::new(),
            }],
        };
        let err = store
            .replace_rows(vec![(other, rows(json!([[1]])))])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("belongs to `my-gh`"), "{err}");
        // A core model's view.
        let core = EntityTable {
            extension: "effort".into(),
            entity: "file".into(),
            view: "v_effort_file".into(),
            key: "path".into(),
            description: String::new(),
            columns: vec![EntityColumn {
                name: "path".into(),
                stored: StoredType::Text,
                doc: String::new(),
            }],
        };
        let err = store
            .replace_rows(vec![(core, rows(json!([["a"]])))])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("belongs to `core`"), "{err}");
        // Its contract is the declared columns, docs included: the catalog
        // (`v_model_column`) documents it like any model.
        let columns: Vec<(String, String, String)> = db
            .read(|tx| {
                let mut st = tx
                    .prepare(
                        "SELECT c.name, c.sql_type, c.doc FROM v_model_column c
                         WHERE c.view = 'v_my_gh_pr' ORDER BY c.position",
                    )
                    .map_err(map_sql_err)?;
                let rows = st
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .map_err(map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(map_sql_err)?;
                Ok(rows)
            })
            .await
            .unwrap();
        assert_eq!(
            columns,
            vec![
                (
                    "number".to_string(),
                    "INTEGER".to_string(),
                    "number doc".to_string()
                ),
                (
                    "title".to_string(),
                    "TEXT".to_string(),
                    "title doc".to_string()
                ),
            ]
        );
        // The open that recompiles models keeps it.
        db.call_mut(|conn| {
            crate::models::drop_all(conn)?;
            crate::models::compile_core(conn)
        })
        .await
        .unwrap();
        let sl = SemanticLayer::new(db.clone());
        assert!(sl
            .query_sql("SELECT * FROM v_my_gh_pr", vec![], None)
            .await
            .is_ok());
        store.drop_extension("my-gh").await.unwrap();
        assert!(registered(&db).await.is_empty());
    }

    /// V109: an entity view made before the registry is registered as its
    /// extension's model — found through the extension's source state —
    /// so the next write sees it as its own.
    #[test]
    fn v109_registers_existing_entity_views() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::database::migrate_to_for_tests(&mut conn, 108);
        conn.execute_batch(
            "INSERT INTO ext_source_state (extension, source_id, status, last_run_at)
               VALUES ('my-gh', 'prs', 'ok', '2026-01-01T00:00:00Z');
             CREATE TABLE ext__my_gh__pr (number INTEGER, PRIMARY KEY (number));
             CREATE VIEW v_my_gh_pr AS SELECT number FROM ext__my_gh__pr;",
        )
        .unwrap();
        crate::database::migrate_and_compile(&mut conn).unwrap();
        let row: (String, String, String) = conn
            .query_row(
                "SELECT m.owner, m.kind, i.input FROM model m JOIN model_input i USING (view)
                 WHERE m.view = 'v_my_gh_pr'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            row,
            ("my-gh".into(), "entity".into(), "ext__my_gh__pr".into())
        );
    }

    #[tokio::test]
    async fn upsert_adds_updates_and_tombstones_by_key() {
        let db = Database::in_memory();
        let store = SqliteCollectorStore::new(db.clone());
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
        let store = SqliteCollectorStore::new(db.clone());
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
        let store = SqliteCollectorStore::new(db.clone());
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
        let store = SqliteCollectorStore::new(db.clone());
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
        let store = SqliteCollectorStore::new(Database::in_memory());
        let mut t = pr_table(&[("number", StoredType::Integer)]);
        t.columns[0].name = "x); DROP TABLE task; --".into();
        assert!(matches!(
            store.replace_rows(vec![(t, vec![])]).await,
            Err(DomainError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn records_state_and_drops_an_extension() {
        let db = Database::in_memory();
        let store = SqliteCollectorStore::new(db.clone());
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
        let state = CollectorRun {
            owner: "my-gh".into(),
            id: "github".into(),
            status: "ok".into(),
            last_run_at: "2026-09-27T00:00:00Z".into(),
            error: None,
            row_counts: counts,
            cursor: None,
            last_event_id: None,
        };
        store.record_run(state.clone()).await.unwrap();
        assert_eq!(store.list_runs().await.unwrap(), vec![state]);

        store.drop_extension("my-gh").await.unwrap();
        assert!(store.list_runs().await.unwrap().is_empty());
        assert!(sl
            .query_sql("SELECT * FROM v_my_gh_pr", vec![], None)
            .await
            .is_err());
    }

    /// P7.B3: a run keeps its checkpoint — the cursor and the last event
    /// it ran for. A failed run keeps the last good cursor and counts, and
    /// the last event never moves back.
    #[tokio::test]
    async fn run_state_keeps_a_cursor_and_last_event() {
        let store = SqliteCollectorStore::new(Database::in_memory());
        let run = |status: &str, cursor: Option<serde_json::Value>, event: i64| CollectorRun {
            owner: "project".into(),
            id: "repo.scan".into(),
            status: status.into(),
            last_run_at: "2026-10-01T00:00:00Z".into(),
            error: (status == "error").then(|| "boom".into()),
            row_counts: [("pr".to_string(), 3)].into(),
            cursor,
            last_event_id: Some(event),
        };
        store
            .record_run(run("ok", Some(json!({"since": "a"})), 7))
            .await
            .unwrap();
        store
            .record_run(run("error", Some(json!({"since": "b"})), 5))
            .await
            .unwrap();
        let got = store.run_of("project", "repo.scan").await.unwrap().unwrap();
        assert_eq!(got.status, "error");
        assert_eq!(got.cursor, Some(json!({"since": "a"})));
        assert_eq!(got.last_event_id, Some(7));
        assert_eq!(got.error.as_deref(), Some("boom"));
        assert!(store.run_of("project", "other").await.unwrap().is_none());
    }

    /// `_` is a LIKE wildcard: dropping `gh` must leave `gh-extra`'s view
    /// (`ext__gh_extra__…`) alone (tsk368).
    #[tokio::test]
    async fn dropping_an_extension_leaves_similar_names_alone() {
        let db = Database::in_memory();
        let store = SqliteCollectorStore::new(db.clone());
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
        let store = SqliteCollectorStore::new(db.clone());
        // extension `task` + entity `note` would be `v_task_note`, a core view.
        let t = EntityTable {
            extension: "task".into(),
            entity: "note".into(),
            view: "v_task_note".into(),
            key: "id".into(),
            description: String::new(),
            columns: vec![EntityColumn {
                name: "id".into(),
                stored: StoredType::Integer,
                doc: String::new(),
            }],
        };
        let err = store.replace_rows(vec![(t, vec![])]).await.unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("belongs to `core`")),
            "{err:?}"
        );
        // The core view still works.
        SemanticLayer::new(db)
            .query_sql("SELECT body FROM v_task_note", vec![], None)
            .await
            .unwrap();
    }
}
