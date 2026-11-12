//! Shared models (`.context/models.md`): a table one extension declares
//! (`models:`) and stores fill through `model.write`. oxplow does every
//! write from the declaration; no extension SQL runs here. Each row is a
//! store's item: its `store`, its URI (the model's resource with `store`
//! and `id`), its declared columns, and per-namespace fields
//! (`model_field_value`). A write diffs against what is stored and touches
//! only what changed, answering one [`ModelChange`] per changed or removed
//! row for the caller to log as the model's change event.

use std::collections::BTreeMap;

use oxplow_domain::DomainError;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::database::{map_sql_err, ts_to_string};

/// A declared column's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelType {
    Text,
    Int,
    Real,
    Bool,
    Time,
    /// One resource URI.
    Uri,
    /// A JSON list of resource URIs.
    Uris,
}

impl ModelType {
    fn sql(self) -> &'static str {
        match self {
            ModelType::Int | ModelType::Bool => "INTEGER",
            ModelType::Real => "REAL",
            _ => "TEXT",
        }
    }

    fn name(self) -> &'static str {
        match self {
            ModelType::Text => "text",
            ModelType::Int => "int",
            ModelType::Real => "real",
            ModelType::Bool => "bool",
            ModelType::Time => "time",
            ModelType::Uri => "uri",
            ModelType::Uris => "uris",
        }
    }
}

/// A markdown body column: its refs become edges; `pins` pins its file
/// and directory refs to the primary stream's latest snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Markdown {
    pub pins: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelColumn {
    pub name: String,
    pub ty: ModelType,
    /// An enum's allowed values.
    pub values: Option<Vec<String>>,
    pub markdown: Option<Markdown>,
    /// A `uri` / `uris` column's edge type: each URI is an edge from the
    /// row of this ref type.
    pub edge: Option<String>,
    pub doc: String,
}

impl ModelColumn {
    pub fn new(name: &str, ty: ModelType) -> Self {
        Self {
            name: name.into(),
            ty,
            values: None,
            markdown: None,
            edge: None,
            doc: String::new(),
        }
    }
}

/// A model as its owner declared it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelDecl {
    /// The owner extension.
    pub extension: String,
    /// The owner's namespace: the model's id is `<namespace>/<name>`.
    pub namespace: String,
    pub name: String,
    /// The resource its rows are (`shelf/book`).
    pub resource: String,
    /// The column holding the store's native id (the URI's `id`).
    pub key: String,
    /// The URI param that names the store (`store`; tasks keep
    /// `provider`).
    pub store_param: String,
    pub columns: Vec<ModelColumn>,
    /// The columns search indexes as a row's title and body: a generated
    /// searchable view `v_<ns>_search_<name>`.
    #[serde(default)]
    pub search: Option<ModelSearch>,
}

/// What a model's generated searchable view indexes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSearch {
    pub title: String,
    pub body: String,
}

impl ModelDecl {
    /// `<namespace>/<name>`.
    pub fn id(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }

    fn ns(&self) -> String {
        oxplow_domain::contribution::extension_namespace(&self.namespace)
    }

    /// `model__<ns>__<name>`.
    pub fn table(&self) -> String {
        format!("model__{}__{}", self.ns(), self.name)
    }

    /// `v_<ns>_<name>`: the live rows.
    pub fn view(&self) -> String {
        format!("v_{}_{}", self.ns(), self.name)
    }

    /// `v_<ns>_search_<name>`: what search indexes, when it declares
    /// `search`.
    pub fn search_view(&self) -> String {
        format!("v_{}_search_{}", self.ns(), self.name)
    }

    /// `v_<ns>_<name>_all`: the removed rows too.
    pub fn view_all(&self) -> String {
        format!("{}_all", self.view())
    }

    /// The URI of `store`'s item `id`.
    pub fn uri(&self, store: &str, id: &str) -> String {
        oxplow_domain::uri::build::of(
            &self.resource,
            &[(self.store_param.as_str(), store), ("id", id)],
        )
    }

    fn column(&self, name: &str) -> Option<&ModelColumn> {
        self.columns.iter().find(|c| c.name == name)
    }
}

/// A field a namespace declares on a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldDecl {
    pub name: String,
    pub title: String,
    /// `enum`, `text` or `number`.
    pub kind: String,
    pub values: Option<Vec<String>>,
    pub read_only: bool,
}

/// One row as a store states it: its columns (the key among them) and,
/// under `fields`, its own fields' values.
pub type ModelRow = serde_json::Map<String, Value>;

/// What a store writes.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelWrite {
    /// Write these rows by key, remove these ids, and re-key these items
    /// (`(from, to)`).
    Upsert {
        rows: Vec<ModelRow>,
        delete: Vec<String>,
        rename: Vec<(String, String)>,
    },
    /// These are every row the store has: the rest of its rows go. One
    /// that would remove more than half of the store's rows, and more than
    /// [`REMOVAL_GUARD`], is refused unless `confirm_removals`.
    Snapshot {
        rows: Vec<ModelRow>,
        confirm_removals: bool,
    },
}

/// A snapshot may remove this many of a store's rows (or half of them)
/// without saying `confirm_removals`.
pub const REMOVAL_GUARD: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOp {
    Upserted,
    Removed,
    Renamed,
}

impl ChangeOp {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeOp::Upserted => "upserted",
            ChangeOp::Removed => "removed",
            ChangeOp::Renamed => "renamed",
        }
    }
}

/// One row a write changed or removed.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelChange {
    pub uri: String,
    pub store: String,
    pub id: String,
    pub op: ChangeOp,
    /// The columns (and `fields`) that changed; every column for a new
    /// (or revived) row, none for a removal.
    pub changed: Vec<String>,
    /// The changed non-markdown columns' values before and after.
    pub before: serde_json::Map<String, Value>,
    pub after: serde_json::Map<String, Value>,
    /// A rename's old id.
    pub renamed_from: Option<String>,
}

impl ModelChange {
    fn new(uri: String, store: &str, id: String, op: ChangeOp) -> Self {
        Self {
            uri,
            store: store.to_string(),
            id,
            op,
            changed: Vec::new(),
            before: serde_json::Map::new(),
            after: serde_json::Map::new(),
            renamed_from: None,
        }
    }
}

/// A row a write skipped: where it was, its id when it had one, why.
#[derive(Debug, Clone, PartialEq)]
pub struct Quarantined {
    pub at: usize,
    pub id: Option<String>,
    pub message: String,
}

/// What a write did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Written {
    pub changes: Vec<ModelChange>,
    pub unchanged: usize,
    pub quarantined: Vec<Quarantined>,
}

/// The columns every row has besides the declared ones.
const IMPLICIT: [&str; 6] = ["uri", "store", "fields", "written_at", "removed_at", "refs"];

fn invalid(message: String) -> DomainError {
    DomainError::Invalid(message)
}

fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(ch) if ch.is_ascii_lowercase())
        && c.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

/// Defense in depth: every name that becomes an identifier is checked
/// here even though the manifest parser checked it.
fn validate(decl: &ModelDecl) -> Result<(), DomainError> {
    if !is_ident(&decl.ns()) || !is_ident(&decl.name) {
        return Err(invalid(format!(
            "model `{}`: bad namespace or name",
            decl.id()
        )));
    }
    for c in &decl.columns {
        if !is_ident(&c.name) || IMPLICIT.contains(&c.name.as_str()) {
            return Err(invalid(format!(
                "model `{}`: column `{}` is not a name it may declare",
                decl.id(),
                c.name
            )));
        }
    }
    if let Some(search) = &decl.search {
        for col in [&search.title, &search.body] {
            if !decl.column(col).is_some_and(|c| c.ty == ModelType::Text) {
                return Err(invalid(format!(
                    "model `{}`: search names `{col}`, which isn't a text column",
                    decl.id()
                )));
            }
        }
    }
    match decl.column(&decl.key) {
        Some(c) if matches!(c.ty, ModelType::Text | ModelType::Int) => Ok(()),
        _ => Err(invalid(format!(
            "model `{}`: key `{}` must be a text or int column",
            decl.id(),
            decl.key
        ))),
    }
}

/// Replace every published field declaration with `decls`: `(model,
/// namespace, extension, field)` (`v_model_field_decl`).
pub fn replace_field_decls_tx(
    conn: &rusqlite::Connection,
    decls: &[(String, String, String, FieldDecl)],
) -> Result<(), DomainError> {
    conn.execute("DELETE FROM model_field_decl", [])
        .map_err(map_sql_err)?;
    for (model, namespace, extension, f) in decls {
        conn.execute(
            "INSERT OR IGNORE INTO model_field_decl
               (model, namespace, name, extension, title, kind, values_json, read_only)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                model,
                namespace,
                f.name,
                extension,
                f.title,
                f.kind,
                f.values
                    .as_ref()
                    .map(|v| serde_json::to_string(v).unwrap_or_default()),
                f.read_only,
            ],
        )
        .map_err(map_sql_err)?;
    }
    Ok(())
}

/// Whether `decl` is a model the store can keep: its names, key and
/// columns (what a manifest's `models:` is checked against).
pub fn check_decl(decl: &ModelDecl) -> Result<(), DomainError> {
    validate(decl)
}

/// The stored shape: (column, SQL type), the implicit ones included.
fn shape(decl: &ModelDecl) -> Vec<(String, &'static str)> {
    let mut out = vec![("uri".to_string(), "TEXT"), ("store".to_string(), "TEXT")];
    out.extend(decl.columns.iter().map(|c| (c.name.clone(), c.ty.sql())));
    out.push(("written_at".to_string(), "TEXT"));
    out.push(("removed_at".to_string(), "TEXT"));
    out
}

/// Create (or bring up to date) `decl`'s table and view, and record the
/// view as its owner's dataset. A new column is added in place; a removed
/// or retyped one rebuilds the table, which its stores refill at their
/// next start (`.context/models.md` "Rebuilds").
pub fn ensure_model_tx(conn: &rusqlite::Connection, decl: &ModelDecl) -> Result<(), DomainError> {
    validate(decl)?;
    let table = decl.table();
    let want = shape(decl);
    let have: Vec<(String, String)> = conn
        .prepare(&format!(
            "SELECT name, type FROM pragma_table_info({})",
            sql_text(&table)
        ))
        .and_then(|mut st| {
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(map_sql_err)?;
    let create = || {
        let cols: Vec<String> = want
            .iter()
            .map(|(n, t)| match n.as_str() {
                "uri" => format!("uri {t} PRIMARY KEY"),
                "store" | "written_at" => format!("{n} {t} NOT NULL"),
                _ => format!("{} {t}", quote(n)),
            })
            .collect();
        format!(
            "CREATE TABLE {} ({});
             CREATE INDEX {} ON {} (store);",
            quote(&table),
            cols.join(", "),
            quote(&format!("{table}_store")),
            quote(&table)
        )
    };
    if have.is_empty() {
        conn.execute_batch(&create()).map_err(map_sql_err)?;
    } else {
        let kept_or_retyped = have.iter().any(|(n, t)| {
            want.iter()
                .find(|(w, _)| w == n)
                .is_none_or(|(_, wt)| !wt.eq_ignore_ascii_case(t))
        });
        if kept_or_retyped {
            conn.execute_batch(&format!("DROP TABLE {};", quote(&table)))
                .map_err(map_sql_err)?;
            conn.execute(
                "DELETE FROM model_field_value WHERE model = ?1",
                [decl.id()],
            )
            .map_err(map_sql_err)?;
            conn.execute_batch(&create()).map_err(map_sql_err)?;
        } else {
            for (n, t) in &want {
                if !have.iter().any(|(h, _)| h == n) {
                    conn.execute_batch(&format!(
                        "ALTER TABLE {} ADD COLUMN {} {t};",
                        quote(&table),
                        quote(n)
                    ))
                    .map_err(map_sql_err)?;
                }
            }
        }
    }
    for all in [false, true] {
        let view = if all { decl.view_all() } else { decl.view() };
        let select = view_sql(decl, all);
        let current: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'view' AND name = ?1",
                [&view],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sql_err)?;
        let create_view = format!("CREATE VIEW {} AS {select}", quote(&view));
        if current.as_deref() != Some(create_view.as_str()) {
            conn.execute_batch(&format!(
                "DROP VIEW IF EXISTS {}; {create_view};",
                quote(&view)
            ))
            .map_err(map_sql_err)?;
        }
        register_tx(conn, decl, &view, &select, all)?;
    }
    // Its fields in long form, each with its declared kind.
    let field_view = format!("{}_field", decl.view());
    let select = format!(
        "SELECT f.uri, f.namespace, f.name, f.value, d.kind FROM model_field_value f \
         JOIN {} t ON t.uri = f.uri AND t.removed_at IS NULL \
         LEFT JOIN model_field_decl d ON d.model = f.model AND d.namespace = f.namespace AND d.name = f.name \
         WHERE f.model = {}",
        quote(&decl.table()),
        sql_text(&decl.id())
    );
    let create_view = format!("CREATE VIEW {} AS {select}", quote(&field_view));
    let current: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'view' AND name = ?1",
            [&field_view],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    if current.as_deref() != Some(create_view.as_str()) {
        conn.execute_batch(&format!(
            "DROP VIEW IF EXISTS {}; {create_view};",
            quote(&field_view)
        ))
        .map_err(map_sql_err)?;
    }
    register_field_view_tx(conn, decl, &field_view, &select)?;
    if let Some(search) = &decl.search {
        let view = decl.search_view();
        let select = format!(
            "SELECT t.uri, t.{} AS title, coalesce(t.{}, '') AS body, t.written_at AS version \
             FROM {} t WHERE t.removed_at IS NULL",
            quote(&search.title),
            quote(&search.body),
            quote(&decl.table())
        );
        let create_view = format!("CREATE VIEW {} AS {select}", quote(&view));
        let current: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'view' AND name = ?1",
                [&view],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sql_err)?;
        if current.as_deref() != Some(create_view.as_str()) {
            conn.execute_batch(&format!(
                "DROP VIEW IF EXISTS {}; {create_view};",
                quote(&view)
            ))
            .map_err(map_sql_err)?;
        }
        register_view_tx(
            conn,
            decl,
            &view,
            &format!("search_{}", decl.name),
            &format!("What search indexes of the shared model `{}`.", decl.id()),
            &select,
            serde_json::json!([
                {"name": "uri", "type": "TEXT", "doc": "The item."},
                {"name": "title", "type": "TEXT", "doc": "Its title."},
                {"name": "body", "type": "TEXT", "doc": "What search reads."},
                {"name": "version", "type": "TEXT", "doc": "When oxplow last wrote it."}
            ]),
        )?;
    }
    Ok(())
}

/// Record a generated view as its owner's dataset (an entity).
fn register_view_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    view: &str,
    name: &str,
    description: &str,
    select: &str,
    contract: Value,
) -> Result<(), DomainError> {
    let current: Option<String> = conn
        .query_row("SELECT sql FROM dataset WHERE view = ?1", [view], |r| {
            r.get(0)
        })
        .optional()
        .map_err(map_sql_err)?;
    if current.as_deref() == Some(select) {
        return Ok(());
    }
    let now = ts_to_string(oxplow_domain::Timestamp::now());
    conn.execute(
        "INSERT INTO dataset (view, name, owner, version, description, sql, compiled_at, kind)
         VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, 'entity')
         ON CONFLICT (view) DO UPDATE SET description = excluded.description,
             sql = excluded.sql, compiled_at = excluded.compiled_at",
        params![view, name, decl.extension, description, select, now],
    )
    .map_err(map_sql_err)?;
    conn.execute(
        "INSERT INTO dataset_contract (view, version, columns_json, recorded_at)
         VALUES (?1, 1, ?2, ?3)
         ON CONFLICT (view, version) DO UPDATE SET columns_json = excluded.columns_json,
             recorded_at = excluded.recorded_at",
        params![view, contract.to_string(), now],
    )
    .map_err(map_sql_err)?;
    conn.execute(
        "INSERT OR IGNORE INTO dataset_input (view, input, kind) VALUES (?1, ?2, 'source')",
        params![view, decl.table()],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// Record the long field view as its owner's dataset.
fn register_field_view_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    view: &str,
    select: &str,
) -> Result<(), DomainError> {
    let contract = serde_json::json!([
        {"name": "uri", "type": "TEXT", "doc": "The item."},
        {"name": "namespace", "type": "TEXT", "doc": "Who wrote the value (a store id, or an extension's namespace)."},
        {"name": "name", "type": "TEXT", "doc": "The field."},
        {"name": "value", "type": "TEXT", "doc": "Its value, as JSON."},
        {"name": "kind", "type": "TEXT", "doc": "Its declared kind: enum, text or number."}
    ])
    .to_string();
    let now = ts_to_string(oxplow_domain::Timestamp::now());
    let current: Option<String> = conn
        .query_row("SELECT sql FROM dataset WHERE view = ?1", [view], |r| {
            r.get(0)
        })
        .optional()
        .map_err(map_sql_err)?;
    if current.as_deref() == Some(select) {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO dataset (view, name, owner, version, description, sql, compiled_at, kind)
         VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, 'entity')
         ON CONFLICT (view) DO UPDATE SET description = excluded.description,
             sql = excluded.sql, compiled_at = excluded.compiled_at",
        params![
            view,
            format!("{}_field", decl.name),
            decl.extension,
            format!(
                "The fields of the shared model `{}`'s items, one value per row.",
                decl.id()
            ),
            select,
            now
        ],
    )
    .map_err(map_sql_err)?;
    conn.execute(
        "INSERT INTO dataset_contract (view, version, columns_json, recorded_at)
         VALUES (?1, 1, ?2, ?3)
         ON CONFLICT (view, version) DO UPDATE SET columns_json = excluded.columns_json,
             recorded_at = excluded.recorded_at",
        params![view, contract, now],
    )
    .map_err(map_sql_err)?;
    for input in [
        decl.table(),
        "model_field_value".to_string(),
        "model_field_decl".to_string(),
    ] {
        conn.execute(
            "INSERT OR IGNORE INTO dataset_input (view, input, kind) VALUES (?1, ?2, 'source')",
            params![view, input],
        )
        .map_err(map_sql_err)?;
    }
    Ok(())
}

/// A SQL string literal.
fn sql_text(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The view: the table's columns, and `fields` as `{namespace: {name:
/// value}}` (NULL with none).
fn view_sql(decl: &ModelDecl, all: bool) -> String {
    let cols: Vec<String> = std::iter::once("t.uri".to_string())
        .chain(std::iter::once("t.store".to_string()))
        .chain(decl.columns.iter().map(|c| format!("t.{}", quote(&c.name))))
        .collect();
    format!(
        "SELECT {}, (SELECT json_group_object(ns.namespace, json(ns.vals)) FROM \
         (SELECT f.namespace, json_group_object(f.name, json(f.value)) AS vals \
         FROM model_field_value f WHERE f.model = {} AND f.uri = t.uri \
         GROUP BY f.namespace ORDER BY f.namespace) ns) AS fields, t.written_at{} \
         FROM {} t{}",
        cols.join(", "),
        sql_text(&decl.id()),
        if all { ", t.removed_at" } else { "" },
        quote(&decl.table()),
        if all {
            ""
        } else {
            " WHERE t.removed_at IS NULL"
        }
    )
}

/// Record the view in the dataset registry (an entity: its owner's, with
/// its columns as contract), so the semantic layer reads it and its
/// readers hear its table change.
fn register_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    view: &str,
    select: &str,
    all: bool,
) -> Result<(), DomainError> {
    let mut columns: Vec<Value> = vec![
        serde_json::json!({"name": "uri", "type": "TEXT", "doc": "The item's URI."}),
        serde_json::json!({"name": "store", "type": "TEXT", "doc": "The store that keeps it."}),
    ];
    columns.extend(decl.columns.iter().map(|c| {
        serde_json::json!({"name": c.name, "type": c.ty.sql(), "doc": if c.doc.is_empty() { c.ty.name().to_string() } else { c.doc.clone() }})
    }));
    columns.push(serde_json::json!({"name": "fields", "type": "TEXT", "doc": "Fields, by the namespace that wrote them."}));
    columns.push(serde_json::json!({"name": "written_at", "type": "TEXT", "doc": "When oxplow last wrote it."}));
    if all {
        columns.push(serde_json::json!({"name": "removed_at", "type": "TEXT", "doc": "When its store removed it (kept 30 days)."}));
    }
    let contract = Value::Array(columns).to_string();
    let description = if all {
        format!(
            "The shared model `{}`: every store's items, removed ones too.",
            decl.id()
        )
    } else {
        format!("The shared model `{}`: every store's items.", decl.id())
    };
    let name = if all {
        format!("{}_all", decl.name)
    } else {
        decl.name.clone()
    };
    let now = ts_to_string(oxplow_domain::Timestamp::now());
    let current: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT m.sql, c.columns_json FROM dataset m
             LEFT JOIN dataset_contract c ON c.view = m.view AND c.version = m.version
             WHERE m.view = ?1",
            [view],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(map_sql_err)?;
    if current == Some((select.to_string(), Some(contract.clone()))) {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO dataset (view, name, owner, version, description, sql, compiled_at, kind)
         VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, 'entity')
         ON CONFLICT (view) DO UPDATE SET description = excluded.description,
             sql = excluded.sql, compiled_at = excluded.compiled_at",
        params![view, name, decl.extension, description, select, now],
    )
    .map_err(map_sql_err)?;
    conn.execute(
        "INSERT INTO dataset_contract (view, version, columns_json, recorded_at)
         VALUES (?1, 1, ?2, ?3)
         ON CONFLICT (view, version) DO UPDATE SET columns_json = excluded.columns_json,
             recorded_at = excluded.recorded_at",
        params![view, contract, now],
    )
    .map_err(map_sql_err)?;
    for input in [decl.table(), "model_field_value".to_string()] {
        conn.execute(
            "INSERT OR IGNORE INTO dataset_input (view, input, kind) VALUES (?1, ?2, 'source')",
            params![view, input],
        )
        .map_err(map_sql_err)?;
    }
    Ok(())
}

/// One checked row: its id, its columns as stored, its fields.
struct Checked {
    id: String,
    cells: Vec<rusqlite::types::Value>,
    fields: BTreeMap<String, String>,
    /// The refs the store states, instead of parsing its markdown.
    refs: Option<Vec<String>>,
}

/// `value` as `column` stores it, or why not.
fn cell(column: &ModelColumn, value: &Value) -> Result<rusqlite::types::Value, String> {
    use rusqlite::types::Value as V;
    if value.is_null() {
        return Ok(V::Null);
    }
    let wrong = || format!("expected {}, got {value}", column.ty.name());
    let out = match column.ty {
        ModelType::Text | ModelType::Time | ModelType::Uri => {
            V::Text(value.as_str().ok_or_else(wrong)?.to_string())
        }
        ModelType::Int => V::Integer(value.as_i64().ok_or_else(wrong)?),
        ModelType::Real => V::Real(value.as_f64().ok_or_else(wrong)?),
        ModelType::Bool => V::Integer(i64::from(value.as_bool().ok_or_else(wrong)?)),
        ModelType::Uris => {
            let list = value.as_array().ok_or_else(wrong)?;
            if !list.iter().all(Value::is_string) {
                return Err(wrong());
            }
            V::Text(value.to_string())
        }
    };
    if let (Some(values), V::Text(text)) = (&column.values, &out) {
        if !values.contains(text) {
            return Err(format!("`{text}` is not one of {}", values.join(", ")));
        }
    }
    Ok(out)
}

/// A field value as stored (canonical JSON), or why not.
fn field_value(decl: &FieldDecl, value: &Value) -> Result<String, String> {
    let ok = match decl.kind.as_str() {
        "number" => value.is_number(),
        "text" => value.is_string(),
        "enum" => value.as_str().is_some_and(|v| {
            decl.values
                .as_ref()
                .is_some_and(|vs| vs.iter().any(|x| x == v))
        }),
        _ => false,
    };
    if ok {
        Ok(value.to_string())
    } else {
        Err(format!("`{value}` doesn't fit a {} field", decl.kind))
    }
}

fn check_fields(
    fields: &[FieldDecl],
    values: &serde_json::Map<String, Value>,
) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for (name, value) in values {
        let Some(decl) = fields.iter().find(|f| &f.name == name) else {
            return Err(format!("field `{name}` isn't declared"));
        };
        if value.is_null() {
            continue;
        }
        out.insert(
            name.clone(),
            field_value(decl, value).map_err(|e| format!("field `{name}`: {e}"))?,
        );
    }
    Ok(out)
}

fn check_row(
    decl: &ModelDecl,
    fields: &[FieldDecl],
    at: usize,
    row: &ModelRow,
) -> Result<Checked, DomainError> {
    let fail = |what: String| {
        let id = row
            .get(&decl.key)
            .map_or_else(|| "?".to_string(), |v| v.to_string());
        invalid(format!("model `{}`: row {at} (id {id}): {what}", decl.id()))
    };
    for name in row.keys() {
        if name != "fields" && name != "refs" && decl.column(name).is_none() {
            return Err(fail(format!("column `{name}` isn't declared")));
        }
    }
    let id = match row.get(&decl.key) {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => return Err(fail(format!("key `{}` is missing", decl.key))),
    };
    let mut cells = Vec::with_capacity(decl.columns.len());
    for column in &decl.columns {
        let value = row.get(&column.name).unwrap_or(&Value::Null);
        cells
            .push(cell(column, value).map_err(|e| fail(format!("column `{}`: {e}", column.name)))?);
    }
    let fields = match row.get("fields") {
        None | Some(Value::Null) => BTreeMap::new(),
        Some(Value::Object(values)) => check_fields(fields, values).map_err(fail)?,
        Some(other) => return Err(fail(format!("`fields` must be an object, got {other}"))),
    };
    let refs = match row.get("refs") {
        None | Some(Value::Null) => None,
        Some(Value::Array(list)) if list.iter().all(Value::is_string) => Some(
            list.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
        ),
        Some(other) => return Err(fail(format!("`refs` must be a list of URIs, got {other}"))),
    };
    Ok(Checked {
        id,
        cells,
        fields,
        refs,
    })
}

/// The stored row of `uri`: its cells in declaration order, and whether
/// it is a tombstone.
fn stored_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    uri: &str,
) -> Result<Option<(Vec<rusqlite::types::Value>, bool)>, DomainError> {
    let cols: Vec<String> = decl.columns.iter().map(|c| quote(&c.name)).collect();
    let n = decl.columns.len();
    conn.prepare_cached(&format!(
        "SELECT {}, removed_at IS NOT NULL FROM {} WHERE uri = ?1",
        cols.join(", "),
        quote(&decl.table())
    ))
    .and_then(|mut st| {
        st.query_row([uri], |r| {
            let cells = (0..n)
                .map(|i| r.get::<_, rusqlite::types::Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((cells, r.get::<_, bool>(n)?))
        })
        .optional()
    })
    .map_err(map_sql_err)
}

/// `namespace`'s stored fields on `uri`.
fn stored_fields_tx(
    conn: &rusqlite::Connection,
    model: &str,
    uri: &str,
    namespace: &str,
) -> Result<BTreeMap<String, String>, DomainError> {
    conn.prepare_cached(
        "SELECT name, value FROM model_field_value WHERE model = ?1 AND uri = ?2 AND namespace = ?3",
    )
    .and_then(|mut st| {
        st.query_map(params![model, uri, namespace], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<BTreeMap<String, String>>>()
    })
    .map_err(map_sql_err)
}

/// Restate `namespace`'s fields on `uri` to `fields`; whether they moved.
fn put_fields_tx(
    conn: &rusqlite::Connection,
    model: &str,
    uri: &str,
    namespace: &str,
    fields: &BTreeMap<String, String>,
) -> Result<bool, DomainError> {
    if &stored_fields_tx(conn, model, uri, namespace)? == fields {
        return Ok(false);
    }
    conn.execute(
        "DELETE FROM model_field_value WHERE model = ?1 AND uri = ?2 AND namespace = ?3",
        params![model, uri, namespace],
    )
    .map_err(map_sql_err)?;
    for (name, value) in fields {
        conn.execute(
            "INSERT INTO model_field_value (model, uri, namespace, name, value) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![model, uri, namespace, name, value],
        )
        .map_err(map_sql_err)?;
    }
    Ok(true)
}

/// Tombstone `uri`: the row is hidden, its fields and edges kept for
/// revival (`.context/models.md` "Removal"). Whether a live row went.
fn remove_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    uri: &str,
    now: &str,
) -> Result<bool, DomainError> {
    let n = conn
        .execute(
            &format!(
                "UPDATE {} SET removed_at = ?2 WHERE uri = ?1 AND removed_at IS NULL",
                quote(&decl.table())
            ),
            params![uri, now],
        )
        .map_err(map_sql_err)?;
    Ok(n > 0)
}

/// Delete the rows removed before `before`, with their fields.
pub fn sweep_tombstones_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    before: &str,
) -> Result<usize, DomainError> {
    validate(decl)?;
    conn.execute(
        &format!(
            "DELETE FROM model_field_value WHERE model = ?1 AND uri IN
               (SELECT uri FROM {} WHERE removed_at IS NOT NULL AND removed_at < ?2)",
            quote(&decl.table())
        ),
        params![decl.id(), before],
    )
    .map_err(map_sql_err)?;
    conn.execute(
        &format!(
            "DELETE FROM page_ref WHERE source_uri IN
               (SELECT uri FROM {} WHERE removed_at IS NOT NULL AND removed_at < ?1)",
            quote(&decl.table())
        ),
        [before],
    )
    .map_err(map_sql_err)?;
    conn.execute(
        &format!(
            "DELETE FROM {} WHERE removed_at IS NOT NULL AND removed_at < ?1",
            quote(&decl.table())
        ),
        [before],
    )
    .map_err(map_sql_err)
}

/// A stored cell as JSON (for a change event's `before` / `after`).
fn json_of(column: &ModelColumn, v: &rusqlite::types::Value) -> Value {
    use rusqlite::types::Value as V;
    match (column.ty, v) {
        (_, V::Null) => Value::Null,
        (ModelType::Bool, V::Integer(n)) => Value::Bool(*n != 0),
        (ModelType::Uris, V::Text(t)) => serde_json::from_str(t).unwrap_or(Value::Null),
        (_, V::Integer(n)) => Value::from(*n),
        (_, V::Real(f)) => Value::from(*f),
        (_, V::Text(t)) => Value::String(t.clone()),
        (_, V::Blob(_)) => Value::Null,
    }
}

fn id_text(v: rusqlite::types::Value) -> String {
    match v {
        rusqlite::types::Value::Integer(n) => n.to_string(),
        rusqlite::types::Value::Text(t) => t,
        other => format!("{other:?}"),
    }
}

/// Whether `decl` pins its markdown refs.
fn pins(decl: &ModelDecl) -> bool {
    decl.columns
        .iter()
        .any(|c| c.markdown.is_some_and(|m| m.pins))
}

/// The ref types `decl`'s columns own on a row's source.
fn owned_ref_types(decl: &ModelDecl) -> Vec<String> {
    let mut types = if decl.columns.iter().any(|c| c.markdown.is_some()) {
        crate::page_ref_projections::model_body_ref_types(pins(decl))
    } else {
        Vec::new()
    };
    types.extend(decl.columns.iter().filter_map(|c| c.edge.clone()));
    types.sort();
    types.dedup();
    types
}

/// Restate the edges row `uri` projects (its markdown columns' mentions,
/// or its stated refs, and its edge columns), by diff: pinned refs keep
/// the pin they have, new ones take the primary stream's latest snapshot,
/// and a file verified under a still-cited directory stays.
fn restate_edges_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    kinds: &oxplow_domain::uri::resources::ResourceRegistry,
    uri: &str,
    row: &Checked,
) -> Result<(), DomainError> {
    use crate::page_ref_projections::{
        model_body_edges, model_stated_edges, RT_DIR_PIN, RT_FILE_PIN,
    };
    use crate::page_ref_store::{PageRefEdge, SourceSlice};
    let types = owned_ref_types(decl);
    if types.is_empty() {
        return Ok(());
    }
    let pinned = pins(decl);
    let mut edges: Vec<PageRefEdge> = match &row.refs {
        Some(refs) => model_stated_edges(uri, refs, pinned),
        None => decl
            .columns
            .iter()
            .zip(&row.cells)
            .filter(|(c, _)| c.markdown.is_some())
            .filter_map(|(_, v)| match v {
                rusqlite::types::Value::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .flat_map(|body| model_body_edges(kinds, uri, body, pinned))
            .collect(),
    };
    for (c, v) in decl.columns.iter().zip(&row.cells) {
        let (Some(ty), rusqlite::types::Value::Text(t)) = (&c.edge, v) else {
            continue;
        };
        let targets: Vec<String> = match c.ty {
            ModelType::Uris => serde_json::from_str(t).unwrap_or_default(),
            _ => vec![t.clone()],
        };
        edges.extend(targets.iter().map(|to| PageRefEdge::new(uri, to, ty)));
    }
    edges.sort_by(|a, b| (&a.target, &a.ref_type).cmp(&(&b.target, &b.ref_type)));
    edges.dedup_by(|a, b| a.target == b.target && a.ref_type == b.ref_type);
    if pinned {
        let existing: Vec<PageRefEdge> = conn
            .prepare_cached(
                "SELECT target_uri, ref_type, source_extra, local_snapshot_id, closest_vcs_rev, vcs_rev_exact
                 FROM page_ref WHERE source_uri = ?1 AND ref_type IN (?2, ?3)",
            )
            .and_then(|mut st| {
                st.query_map(params![uri, RT_FILE_PIN, RT_DIR_PIN], |r| {
                    let mut e = PageRefEdge::new(uri, r.get::<_, String>(0)?, r.get::<_, String>(1)?);
                    e.source_extra = r.get(2)?;
                    e.local_snapshot_id = r.get(3)?;
                    e.closest_vcs_rev = r.get(4)?;
                    e.vcs_rev_exact = r.get::<_, i64>(5)? != 0;
                    Ok(e)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(map_sql_err)?;
        let pin = crate::knowledge_record::pin_tx(conn)?;
        for e in edges.iter_mut() {
            if e.ref_type != RT_FILE_PIN && e.ref_type != RT_DIR_PIN {
                continue;
            }
            match existing
                .iter()
                .find(|x| x.target == e.target && x.ref_type == e.ref_type)
            {
                Some(kept) => {
                    e.local_snapshot_id = kept.local_snapshot_id;
                    e.closest_vcs_rev = kept.closest_vcs_rev.clone();
                    e.vcs_rev_exact = kept.vcs_rev_exact;
                }
                None => {
                    if let Some(id) = pin.snapshot_id {
                        e.local_snapshot_id = Some(id);
                        e.closest_vcs_rev = pin.revision.clone();
                        e.vcs_rev_exact = pin.revision.is_some();
                    }
                }
            }
        }
        let dirs: Vec<String> = edges
            .iter()
            .filter(|e| e.ref_type == RT_DIR_PIN)
            .filter_map(|e| {
                oxplow_domain::uri::build::file_of_uri(&e.target)
                    .ok()
                    .map(|f| f.path)
            })
            .collect();
        for kept in existing {
            let cited = edges
                .iter()
                .any(|e| e.target == kept.target && e.ref_type == kept.ref_type);
            let under = kept.ref_type == RT_FILE_PIN
                && oxplow_domain::uri::build::file_of_uri(&kept.target)
                    .is_ok_and(|f| crate::knowledge_record::path_under_any_dir(&f.path, &dirs));
            if !cited && under {
                edges.push(kept);
            }
        }
    }
    let slice = SourceSlice {
        source: uri.to_string(),
        ref_types: Some(types.clone()),
        edges: edges.clone(),
    };
    if crate::page_ref_store::slice_is_stored_tx(conn, &slice)? {
        return Ok(());
    }
    crate::page_ref_store::replace_source_for_ref_types_tx(conn, uri, &types, edges)
}

/// Write `store`'s rows of `decl` (its table must exist: [`ensure_model_tx`]);
/// `fields` are what the store declared, kept under the store's id. A row
/// that fails its checks is quarantined (skipped, reported, and never
/// removed by a snapshot); the rest is written. Only what changed is
/// written. A snapshot that would remove too much is refused (nothing
/// written) unless it confirms.
pub fn write_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    kinds: &oxplow_domain::uri::resources::ResourceRegistry,
    store: &str,
    fields: &[FieldDecl],
    write: &ModelWrite,
) -> Result<Written, DomainError> {
    validate(decl)?;
    if store.is_empty() {
        return Err(invalid(format!(
            "model `{}`: a write needs its store",
            decl.id()
        )));
    }
    let (rows, delete, rename, snapshot) = match write {
        ModelWrite::Upsert {
            rows,
            delete,
            rename,
        } => (rows, delete.as_slice(), rename.as_slice(), None),
        ModelWrite::Snapshot {
            rows,
            confirm_removals,
        } => (rows, &[][..], &[][..], Some(*confirm_removals)),
    };
    let mut out = Written::default();
    let mut kept = std::collections::BTreeSet::new();
    let mut checked = Vec::new();
    for (at, row) in rows.iter().enumerate() {
        match check_row(decl, fields, at, row) {
            Ok(c) => checked.push(c),
            Err(e) => {
                let id = match row.get(&decl.key) {
                    Some(Value::String(s)) => Some(s.clone()),
                    Some(Value::Number(n)) => Some(n.to_string()),
                    _ => None,
                };
                if let Some(id) = &id {
                    kept.insert(decl.uri(store, id));
                }
                out.quarantined.push(Quarantined {
                    at,
                    id,
                    message: e.to_string(),
                });
            }
        }
    }
    let model = decl.id();
    let now = ts_to_string(oxplow_domain::Timestamp::now());
    let table = quote(&decl.table());

    // A snapshot's removals, decided before anything is written.
    let gone: Vec<(String, String)> = match snapshot {
        Some(confirm) => {
            for row in &checked {
                kept.insert(decl.uri(store, &row.id));
            }
            let mut st = conn
                .prepare(&format!(
                    "SELECT uri, {} FROM {table} WHERE store = ?1 AND removed_at IS NULL",
                    quote(&decl.key)
                ))
                .map_err(map_sql_err)?;
            let live = st
                .query_map([store], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, rusqlite::types::Value>(1)?,
                    ))
                })
                .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                .map_err(map_sql_err)?;
            let total = live.len();
            let gone: Vec<(String, String)> = live
                .into_iter()
                .filter(|(uri, _)| !kept.contains(uri))
                .map(|(uri, id)| (uri, id_text(id)))
                .collect();
            if !confirm && gone.len() > REMOVAL_GUARD && gone.len() * 2 > total {
                return Err(invalid(format!(
                    "model `{model}`: store `{store}`'s snapshot would remove {} of its {total} items; \
                     held — send it with confirm_removals when that is meant",
                    gone.len()
                )));
            }
            gone
        }
        None => delete
            .iter()
            .map(|id| (decl.uri(store, id), id.clone()))
            .collect(),
    };

    for (from, to) in rename {
        let (old, new) = (decl.uri(store, from), decl.uri(store, to));
        let moved = conn
            .execute(
                &format!(
                    "UPDATE {table} SET uri = ?2, {} = ?3, written_at = ?4
                     WHERE uri = ?1 AND NOT EXISTS (SELECT 1 FROM {table} WHERE uri = ?2)",
                    quote(&decl.key)
                ),
                params![old, new, to, now],
            )
            .map_err(map_sql_err)?;
        if moved == 0 {
            continue;
        }
        for sql in [
            "UPDATE model_field_value SET uri = ?2 WHERE model = ?3 AND uri = ?1",
            "UPDATE page_ref SET source_uri = ?2 WHERE source_uri = ?1 AND ?3 = ?3",
            "UPDATE page_ref SET target_uri = ?2 WHERE target_uri = ?1 AND ?3 = ?3",
        ] {
            conn.execute(sql, params![old, new, model])
                .map_err(map_sql_err)?;
        }
        let mut change = ModelChange::new(new, store, to.clone(), ChangeOp::Renamed);
        change.renamed_from = Some(from.clone());
        out.changes.push(change);
    }

    let cols: Vec<String> = decl.columns.iter().map(|c| quote(&c.name)).collect();
    let placeholders: Vec<String> = (0..cols.len()).map(|i| format!("?{}", i + 4)).collect();
    let insert = format!(
        "INSERT INTO {table} (uri, store, written_at, removed_at, {}) VALUES (?1, ?2, ?3, NULL, {})
         ON CONFLICT (uri) DO UPDATE SET written_at = excluded.written_at, removed_at = NULL, {}",
        cols.join(", "),
        placeholders.join(", "),
        cols.iter()
            .map(|c| format!("{c} = excluded.{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for row in checked {
        let uri = decl.uri(store, &row.id);
        let before = stored_tx(conn, decl, &uri)?;
        let mut change = ModelChange::new(uri.clone(), store, row.id.clone(), ChangeOp::Upserted);
        match &before {
            // New, or back from removal: every column.
            None | Some((_, true)) => {
                change.changed = decl.columns.iter().map(|c| c.name.clone()).collect();
                for (c, v) in decl.columns.iter().zip(&row.cells) {
                    if c.markdown.is_none() {
                        change.after.insert(c.name.clone(), json_of(c, v));
                    }
                }
            }
            Some((cells, false)) => {
                for (c, (a, b)) in decl.columns.iter().zip(cells.iter().zip(&row.cells)) {
                    if a != b {
                        change.changed.push(c.name.clone());
                        if c.markdown.is_none() {
                            change.before.insert(c.name.clone(), json_of(c, a));
                            change.after.insert(c.name.clone(), json_of(c, b));
                        }
                    }
                }
            }
        }
        if !change.changed.is_empty() {
            let mut values: Vec<rusqlite::types::Value> = vec![
                uri.clone().into(),
                store.to_string().into(),
                now.clone().into(),
            ];
            values.extend(row.cells.iter().cloned());
            conn.prepare_cached(&insert)
                .and_then(|mut st| st.execute(rusqlite::params_from_iter(values)))
                .map_err(map_sql_err)?;
        }
        if !change.changed.is_empty() || row.refs.is_some() {
            restate_edges_tx(conn, decl, kinds, &uri, &row)?;
        }
        if put_fields_tx(conn, &model, &uri, store, &row.fields)? {
            change.changed.push("fields".into());
        }
        if change.changed.is_empty() {
            out.unchanged += 1;
        } else {
            out.changes.push(change);
        }
    }
    for (uri, id) in gone {
        if remove_tx(conn, decl, &uri, &now)? {
            out.changes
                .push(ModelChange::new(uri, store, id, ChangeOp::Removed));
        }
    }
    Ok(out)
}

/// Write `namespace`'s field values on the item `uri` (another
/// extension's fields on a store's item); a null value removes that field.
/// The item must exist. `None` when nothing moved.
pub fn write_fields_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    namespace: &str,
    fields: &[FieldDecl],
    uri: &str,
    values: &serde_json::Map<String, Value>,
) -> Result<Option<ModelChange>, DomainError> {
    validate(decl)?;
    let item: Option<(String, rusqlite::types::Value)> = conn
        .query_row(
            &format!(
                "SELECT store, {} FROM {} WHERE uri = ?1 AND removed_at IS NULL",
                quote(&decl.key),
                quote(&decl.table())
            ),
            [uri],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(map_sql_err)?;
    let Some((store, id)) = item else {
        return Err(invalid(format!("model `{}`: no item `{uri}`", decl.id())));
    };
    let mut merged = stored_fields_tx(conn, &decl.id(), uri, namespace)?;
    let new = check_fields(fields, values)
        .map_err(|e| invalid(format!("model `{}`: `{uri}`: {e}", decl.id())))?;
    for (name, value) in values {
        if value.is_null() {
            merged.remove(name);
        }
    }
    merged.extend(new);
    if !put_fields_tx(conn, &decl.id(), uri, namespace, &merged)? {
        return Ok(None);
    }
    let mut change = ModelChange::new(uri.to_string(), &store, id_text(id), ChangeOp::Upserted);
    change.changed = vec!["fields".into()];
    Ok(Some(change))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use serde_json::json;

    fn book() -> ModelDecl {
        let mut body = ModelColumn::new("body", ModelType::Text);
        body.markdown = Some(Markdown { pins: false });
        let mut state = ModelColumn::new("state", ModelType::Text);
        state.values = Some(vec!["open".into(), "done".into()]);
        ModelDecl {
            extension: "shelf".into(),
            namespace: "shelf".into(),
            name: "book".into(),
            resource: "shelf/book".into(),
            key: "id".into(),
            store_param: "store".into(),
            search: Some(ModelSearch {
                title: "title".into(),
                body: "body".into(),
            }),
            columns: vec![
                ModelColumn::new("id", ModelType::Text),
                ModelColumn::new("title", ModelType::Text),
                ModelColumn::new("pages", ModelType::Int),
                state,
                body,
            ],
        }
    }

    fn kinds() -> oxplow_domain::uri::resources::ResourceRegistry {
        oxplow_domain::vocabulary::VocabularyHandle::core()
            .current()
            .resources
            .clone()
    }

    fn rating() -> Vec<FieldDecl> {
        vec![FieldDecl {
            name: "rating".into(),
            title: "Rating".into(),
            kind: "number".into(),
            values: None,
            read_only: false,
        }]
    }

    fn row(v: Value) -> ModelRow {
        v.as_object().unwrap().clone()
    }

    async fn write(
        db: &Database,
        store: &'static str,
        fields: Vec<FieldDecl>,
        w: ModelWrite,
    ) -> Result<Written, DomainError> {
        db.transaction(move |tx| {
            ensure_model_tx(tx, &book())?;
            write_tx(tx, &book(), &kinds(), store, &fields, &w)
        })
        .await
    }

    async fn rows(db: &Database) -> Vec<(String, String, String, Option<String>)> {
        db.read(|tx| {
            let mut st = tx
                .prepare("SELECT store, uri, title, fields FROM v_shelf_book ORDER BY uri")
                .map_err(map_sql_err)?;
            let out = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(map_sql_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_sql_err)?;
            Ok(out)
        })
        .await
        .unwrap()
    }

    /// An upsert writes rows by key under their store, with a URI made of
    /// the model's resource, store and id; the same rows again write
    /// nothing; a changed column is named; a delete removes the row.
    #[tokio::test]
    async fn an_upsert_writes_only_what_changed() {
        let db = Database::in_memory();
        let first = write(
            &db,
            "shelf-db",
            vec![],
            ModelWrite::Upsert {
                rows: vec![row(json!({"id": "b1", "title": "Dune", "pages": 412}))],
                delete: vec![],
                rename: vec![],
            },
        )
        .await
        .unwrap();
        let uri = "oxplow:shelf/book?id=b1&store=shelf-db";
        assert_eq!(first.changes.len(), 1);
        assert_eq!(first.changes[0].uri, uri);
        assert_eq!(first.changes[0].op, ChangeOp::Upserted);
        assert_eq!(rows(&db).await[0].0, "shelf-db");

        let again = write(
            &db,
            "shelf-db",
            vec![],
            ModelWrite::Upsert {
                rows: vec![row(json!({"id": "b1", "title": "Dune", "pages": 412}))],
                delete: vec![],
                rename: vec![],
            },
        )
        .await
        .unwrap();
        assert_eq!((again.changes.len(), again.unchanged), (0, 1));

        let moved = write(
            &db,
            "shelf-db",
            vec![],
            ModelWrite::Upsert {
                rows: vec![row(
                    json!({"id": "b1", "title": "Dune Messiah", "pages": 412}),
                )],
                delete: vec![],
                rename: vec![],
            },
        )
        .await
        .unwrap();
        assert_eq!(moved.changes[0].changed, vec!["title".to_string()]);
        assert_eq!(moved.changes[0].before["title"], json!("Dune"));
        assert_eq!(moved.changes[0].after["title"], json!("Dune Messiah"));

        let gone = write(
            &db,
            "shelf-db",
            vec![],
            ModelWrite::Upsert {
                rows: vec![],
                delete: vec!["b1".into()],
                rename: vec![],
            },
        )
        .await
        .unwrap();
        assert_eq!(gone.changes[0].op, ChangeOp::Removed);
        assert!(rows(&db).await.is_empty());
    }

    /// A snapshot is every row of its store: the store's other rows go;
    /// another store's rows stay.
    #[tokio::test]
    async fn a_snapshot_removes_only_its_own_stores_missing_rows() {
        let db = Database::in_memory();
        for (store, ids) in [("a", vec!["1", "2"]), ("b", vec!["1"])] {
            let rows = ids
                .iter()
                .map(|id| row(json!({"id": id, "title": format!("{store}{id}")})))
                .collect();
            write(
                &db,
                store,
                vec![],
                ModelWrite::Snapshot {
                    rows,
                    confirm_removals: false,
                },
            )
            .await
            .unwrap();
        }
        let w = write(
            &db,
            "a",
            vec![],
            ModelWrite::Snapshot {
                rows: vec![row(json!({"id": "1", "title": "a1"}))],
                confirm_removals: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(w.changes.len(), 1);
        assert_eq!(w.changes[0].op, ChangeOp::Removed);
        assert_eq!(w.changes[0].id, "2");
        let titles: Vec<String> = rows(&db).await.into_iter().map(|r| r.2).collect();
        assert_eq!(titles, vec!["a1", "b1"]);
    }

    /// A row is checked against the declaration: an undeclared column, a
    /// value of the wrong type, an enum value not allowed, a missing key or
    /// an undeclared field quarantines it (skipped, named), and the rest of
    /// the write lands. A snapshot never removes a quarantined key.
    #[tokio::test]
    async fn a_bad_row_is_quarantined_and_the_rest_written() {
        let db = Database::in_memory();
        for (n, (bad, says)) in [
            (json!({"id": "b1", "colour": "red"}), "colour"),
            (json!({"id": "b1", "pages": "many"}), "pages"),
            (json!({"id": "b1", "state": "lost"}), "state"),
            (json!({"title": "no id"}), "id"),
            (json!({"id": "b1", "fields": {"mood": 3}}), "mood"),
        ]
        .into_iter()
        .enumerate()
        {
            let ok = format!("ok{n}");
            let w = write(
                &db,
                "s",
                rating(),
                ModelWrite::Upsert {
                    rows: vec![row(json!({"id": ok, "title": "fine"})), row(bad)],
                    delete: vec![],
                    rename: vec![],
                },
            )
            .await
            .unwrap();
            assert_eq!(w.changes.len(), 1, "the good row lands");
            assert_eq!(w.quarantined.len(), 1);
            assert!(
                w.quarantined[0].message.contains(says),
                "{:?}",
                w.quarantined
            );
        }
        // b1 exists; a snapshot whose b1 row is bad keeps it.
        write(
            &db,
            "q",
            vec![],
            ModelWrite::Snapshot {
                rows: vec![row(json!({"id": "b1", "title": "Dune"}))],
                confirm_removals: false,
            },
        )
        .await
        .unwrap();
        let w = write(
            &db,
            "q",
            vec![],
            ModelWrite::Snapshot {
                rows: vec![row(json!({"id": "b1", "pages": "many"}))],
                confirm_removals: false,
            },
        )
        .await
        .unwrap();
        assert!(w.changes.is_empty(), "{:?}", w.changes);
        assert!(rows(&db).await.iter().any(|r| r.1.contains("store=q")));
    }

    /// A removed row is a tombstone: hidden from the view, kept in the
    /// `_all` view with its fields; the same id coming back revives it with
    /// them, as a change of every column. A sweep deletes old tombstones.
    #[tokio::test]
    async fn a_removed_row_is_a_tombstone_until_it_comes_back() {
        let db = Database::in_memory();
        let upsert = |rows: Vec<ModelRow>, delete: Vec<String>| ModelWrite::Upsert {
            rows,
            delete,
            rename: vec![],
        };
        write(
            &db,
            "s",
            rating(),
            upsert(
                vec![row(
                    json!({"id": "b1", "title": "Dune", "fields": {"rating": 4}}),
                )],
                vec![],
            ),
        )
        .await
        .unwrap();
        write(&db, "s", rating(), upsert(vec![], vec!["b1".into()]))
            .await
            .unwrap();
        assert!(rows(&db).await.is_empty());
        let all: i64 = db
            .read(|tx| {
                tx.query_row(
                    "SELECT count(*) FROM v_shelf_book_all WHERE removed_at IS NOT NULL AND fields IS NOT NULL",
                    [],
                    |r| r.get(0),
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(all, 1);
        let back = write(
            &db,
            "s",
            rating(),
            upsert(vec![row(json!({"id": "b1", "title": "Dune"}))], vec![]),
        )
        .await
        .unwrap();
        assert_eq!(back.changes[0].op, ChangeOp::Upserted);
        assert!(back.changes[0].changed.contains(&"title".to_string()));
        let live = rows(&db).await;
        assert_eq!(live.len(), 1, "revived");

        write(&db, "s", rating(), upsert(vec![], vec!["b1".into()]))
            .await
            .unwrap();
        let swept = db
            .transaction(|tx| sweep_tombstones_tx(tx, &book(), "9999-01-01"))
            .await
            .unwrap();
        assert_eq!(swept, 1);
        let left: i64 = db
            .read(|tx| {
                tx.query_row("SELECT count(*) FROM model_field_value", [], |r| r.get(0))
                    .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(left, 0, "a swept row's fields go with it");
    }

    /// A rename re-keys an item: its URI, its fields and its edges move,
    /// and one renamed change names the old id.
    #[tokio::test]
    async fn a_rename_moves_the_item_its_fields_and_edges() {
        let db = Database::in_memory();
        write(
            &db,
            "s",
            rating(),
            ModelWrite::Upsert {
                rows: vec![row(
                    json!({"id": "old", "title": "Dune", "fields": {"rating": 4}}),
                )],
                delete: vec![],
                rename: vec![],
            },
        )
        .await
        .unwrap();
        let (old, new) = (
            "oxplow:shelf/book?id=old&store=s",
            "oxplow:shelf/book?id=new&store=s",
        );
        db.transaction(move |tx| {
            crate::page_ref_store::upsert_edge_tx(
                tx,
                &crate::page_ref_store::PageRefEdge::new(
                    old,
                    "oxplow:oxplow/file?path=a.rs",
                    "file_mention",
                ),
            )
        })
        .await
        .unwrap();
        let w = write(
            &db,
            "s",
            rating(),
            ModelWrite::Upsert {
                rows: vec![],
                delete: vec![],
                rename: vec![("old".into(), "new".into())],
            },
        )
        .await
        .unwrap();
        assert_eq!(w.changes.len(), 1);
        assert_eq!(w.changes[0].op, ChangeOp::Renamed);
        assert_eq!(w.changes[0].renamed_from.as_deref(), Some("old"));
        let live = rows(&db).await;
        assert_eq!(live[0].1, new);
        assert!(live[0].3.as_deref().unwrap().contains("rating"));
        let edges: i64 = db
            .read(move |tx| {
                tx.query_row(
                    "SELECT count(*) FROM page_ref WHERE source_uri = ?1",
                    [new],
                    |r| r.get(0),
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(edges, 1);
    }

    /// A snapshot that would remove more than half of a store's rows, and
    /// more than the guard, is refused (nothing written) unless it confirms.
    #[tokio::test]
    async fn a_snapshot_that_empties_a_store_needs_confirming() {
        let db = Database::in_memory();
        let rows_of = |n: usize| -> Vec<ModelRow> {
            (0..n)
                .map(|i| row(json!({"id": format!("b{i}"), "title": "t"})))
                .collect()
        };
        write(
            &db,
            "s",
            vec![],
            ModelWrite::Snapshot {
                rows: rows_of(12),
                confirm_removals: false,
            },
        )
        .await
        .unwrap();
        let err = write(
            &db,
            "s",
            vec![],
            ModelWrite::Snapshot {
                rows: vec![],
                confirm_removals: false,
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("confirm_removals"), "{err}");
        assert_eq!(rows(&db).await.len(), 12, "nothing applied");
        let w = write(
            &db,
            "s",
            vec![],
            ModelWrite::Snapshot {
                rows: vec![],
                confirm_removals: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(w.changes.len(), 12);
    }

    /// Fields are kept apart by namespace: a store writes its own in its
    /// rows; another extension writes its own on the item; the store's
    /// snapshot restates only the store's fields; the item going takes
    /// every namespace's values with it.
    #[tokio::test]
    async fn fields_are_kept_apart_by_namespace() {
        let db = Database::in_memory();
        let uri = "oxplow:shelf/book?id=b1&store=shelf-db";
        write(
            &db,
            "shelf-db",
            rating(),
            ModelWrite::Snapshot {
                rows: vec![row(
                    json!({"id": "b1", "title": "Dune", "fields": {"rating": 4}}),
                )],
                confirm_removals: false,
            },
        )
        .await
        .unwrap();
        let score = vec![FieldDecl {
            name: "score".into(),
            title: "Score".into(),
            kind: "number".into(),
            values: None,
            read_only: false,
        }];
        let changed = db
            .transaction(move |tx| {
                write_fields_tx(
                    tx,
                    &book(),
                    "shelf-score",
                    &score,
                    uri,
                    json!({"score": 9}).as_object().unwrap(),
                )
            })
            .await
            .unwrap();
        assert_eq!(changed.unwrap().changed, vec!["fields".to_string()]);
        let fields = |r: &[(String, String, String, Option<String>)]| -> Value {
            serde_json::from_str(r[0].3.as_deref().unwrap()).unwrap()
        };
        assert_eq!(
            fields(&rows(&db).await),
            json!({"shelf-db": {"rating": 4}, "shelf-score": {"score": 9}})
        );

        write(
            &db,
            "shelf-db",
            rating(),
            ModelWrite::Snapshot {
                rows: vec![row(
                    json!({"id": "b1", "title": "Dune", "fields": {"rating": 5}}),
                )],
                confirm_removals: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            fields(&rows(&db).await),
            json!({"shelf-db": {"rating": 5}, "shelf-score": {"score": 9}}),
            "a reconcile keeps another namespace's values"
        );

        write(
            &db,
            "shelf-db",
            rating(),
            ModelWrite::Snapshot {
                rows: vec![],
                confirm_removals: false,
            },
        )
        .await
        .unwrap();
        let left: i64 = db
            .read(|tx| {
                tx.query_row("SELECT count(*) FROM model_field_value", [], |r| r.get(0))
                    .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(left, 2, "a removed item keeps its values until it is swept");
    }

    /// Fields on an item that doesn't exist are refused.
    #[tokio::test]
    async fn fields_need_the_item() {
        let db = Database::in_memory();
        let err = db
            .transaction(|tx| {
                ensure_model_tx(tx, &book())?;
                write_fields_tx(
                    tx,
                    &book(),
                    "shelf-score",
                    &[],
                    "oxplow:shelf/book?id=nope&store=x",
                    &serde_json::Map::new(),
                )
            })
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("nope"), "{err}");
    }

    fn pinned_book() -> ModelDecl {
        let mut d = book();
        for c in d.columns.iter_mut() {
            if c.name == "body" {
                c.markdown = Some(Markdown { pins: true });
            }
        }
        let mut about = ModelColumn::new("about", ModelType::Uri);
        about.edge = Some("book.about".into());
        d.columns.push(about);
        d
    }

    async fn write_decl(db: &Database, decl: ModelDecl, rows: Vec<ModelRow>) -> Written {
        db.transaction(move |tx| {
            ensure_model_tx(tx, &decl)?;
            write_tx(
                tx,
                &decl,
                &kinds(),
                "s",
                &[],
                &ModelWrite::Upsert {
                    rows: rows.clone(),
                    delete: vec![],
                    rename: vec![],
                },
            )
        })
        .await
        .unwrap()
    }

    async fn edges(db: &Database, source: &'static str) -> Vec<(String, String, Option<i64>)> {
        db.read(move |tx| {
            let mut st = tx
                .prepare(
                    "SELECT target_uri, ref_type, local_snapshot_id FROM page_ref
                     WHERE source_uri = ?1 ORDER BY target_uri, ref_type",
                )
                .map_err(map_sql_err)?;
            let out = st
                .query_map([source], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(map_sql_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_sql_err)?;
            Ok(out)
        })
        .await
        .unwrap()
    }

    async fn snapshot(db: &Database) -> i64 {
        db.transaction(|tx| {
            tx.execute_batch(
                "INSERT OR IGNORE INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                 VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'main', '/r', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
                 INSERT INTO snapshot (stream_id, created_at) VALUES (1, '2026-01-01T00:00:00Z');",
            )
            .map_err(map_sql_err)?;
            Ok(tx.last_insert_rowid())
        })
        .await
        .unwrap()
    }

    const B1: &str = "oxplow:shelf/book?id=b1&store=s";
    const A_RS: &str = "oxplow:oxplow/file?path=src/a.rs";
    const B_RS: &str = "oxplow:oxplow/file?path=src/b.rs";

    /// A markdown column's mentions are edges from the row; a write that
    /// leaves the body alone rewrites none.
    #[tokio::test]
    async fn a_markdown_body_projects_its_mentions_by_diff() {
        let db = Database::in_memory();
        write_decl(
            &db,
            book(),
            vec![row(
                json!({"id": "b1", "title": "t", "body": "see [[file:src/a.rs]]"}),
            )],
        )
        .await;
        assert_eq!(
            edges(&db, B1).await,
            vec![(A_RS.to_string(), "file_mention".to_string(), None)]
        );
        let mut rx = db.subscribe_changes();
        write_decl(
            &db,
            book(),
            vec![row(
                json!({"id": "b1", "title": "t2", "body": "see [[file:src/a.rs]]"}),
            )],
        )
        .await;
        while let Ok(changed) = rx.try_recv() {
            assert!(
                !changed.contains("page_ref"),
                "an unchanged body rewrote its edges"
            );
        }
    }

    /// A pinned body's file refs are pinned to the primary stream's latest
    /// snapshot when new and keep their pin through a prose edit; a uri
    /// column with an edge type is an edge too.
    #[tokio::test]
    async fn a_pinned_body_pins_new_refs_and_keeps_old_pins() {
        let db = Database::in_memory();
        let first = snapshot(&db).await;
        write_decl(
            &db,
            pinned_book(),
            vec![row(json!({"id": "b1", "title": "t", "body": "see [[file:src/a.rs]]", "about": "oxplow:shelf/book?id=b0&store=s"}))],
        )
        .await;
        let second = snapshot(&db).await;
        write_decl(
            &db,
            pinned_book(),
            vec![row(json!({"id": "b1", "title": "t", "body": "now see [[file:src/a.rs]] and [[file:src/b.rs]]", "about": "oxplow:shelf/book?id=b0&store=s"}))],
        )
        .await;
        assert_eq!(
            edges(&db, B1).await,
            vec![
                (A_RS.to_string(), "file_pin".to_string(), Some(first)),
                (B_RS.to_string(), "file_pin".to_string(), Some(second)),
                (
                    "oxplow:shelf/book?id=b0&store=s".to_string(),
                    "book.about".to_string(),
                    None
                ),
            ]
        );
    }

    /// A row's stated refs are its edges in place of its parsed body.
    #[tokio::test]
    async fn stated_refs_replace_the_parsed_body() {
        let db = Database::in_memory();
        write_decl(
            &db,
            book(),
            vec![row(
                json!({"id": "b1", "title": "t", "body": "see [[file:src/a.rs]]", "refs": [B_RS]}),
            )],
        )
        .await;
        assert_eq!(
            edges(&db, B1).await,
            vec![(B_RS.to_string(), "file_mention".to_string(), None)]
        );
    }

    /// `v_ref_pin` says a pinned file is stale once the primary stream has a
    /// snapshot of it newer than its pin.
    #[tokio::test]
    async fn a_pinned_file_goes_stale_when_it_changes_after_its_pin() {
        let db = Database::in_memory();
        snapshot(&db).await;
        write_decl(
            &db,
            pinned_book(),
            vec![row(
                json!({"id": "b1", "title": "t", "body": "see [[file:src/a.rs]]"}),
            )],
        )
        .await;
        let stale = |db: Database| async move {
            db.read(|tx| {
                tx.query_row(
                    "SELECT stale FROM v_ref_pin WHERE source = ?1 AND path = 'src/a.rs'",
                    [B1],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap()
        };
        assert_eq!(stale(db.clone()).await, 0);
        let later = snapshot(&db).await;
        db.transaction(move |tx| {
            tx.execute(
                "INSERT INTO file_snapshot (stream_id, path, blob_hash, size_bytes, captured_at, storage, snapshot_id)
                 VALUES (1, 'src/a.rs', 'h2', 1, '2026-01-02T00:00:00Z', 'oxplow', ?1)",
                [later],
            )
            .map_err(map_sql_err)
        })
        .await
        .unwrap();
        assert_eq!(stale(db.clone()).await, 1);
    }

    /// The declared fields are published (a store's under its store id,
    /// another extension's under its namespace), and each model's long
    /// field view joins a value with its declared kind.
    #[tokio::test]
    async fn field_declarations_and_the_long_field_view() {
        let db = Database::in_memory();
        write(
            &db,
            "shelf-db",
            rating(),
            ModelWrite::Snapshot {
                rows: vec![row(
                    json!({"id": "b1", "title": "Dune", "fields": {"rating": 4}}),
                )],
                confirm_removals: false,
            },
        )
        .await
        .unwrap();
        db.transaction(|tx| {
            replace_field_decls_tx(
                tx,
                &[(
                    "shelf/book".into(),
                    "shelf-db".into(),
                    "shelf-db".into(),
                    rating()[0].clone(),
                )],
            )
        })
        .await
        .unwrap();
        let (kind, value): (String, String) = db
            .read(|tx| {
                tx.query_row(
                    "SELECT kind, value FROM v_shelf_book_field WHERE name = 'rating'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!((kind.as_str(), value.as_str()), ("number", "4"));
        let decls: i64 = db
            .read(|tx| {
                tx.query_row("SELECT count(*) FROM model_field_decl", [], |r| r.get(0))
                    .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(decls, 1);
    }

    /// A model with `search` has a searchable view of its live rows.
    #[tokio::test]
    async fn a_searchable_model_has_its_search_view() {
        let db = Database::in_memory();
        write(
            &db,
            "s",
            vec![],
            ModelWrite::Upsert {
                rows: vec![row(
                    json!({"id": "b1", "title": "Dune", "body": "A desert."}),
                )],
                delete: vec![],
                rename: vec![],
            },
        )
        .await
        .unwrap();
        let (title, body): (String, String) = db
            .read(|tx| {
                tx.query_row("SELECT title, body FROM v_shelf_search_book", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!((title.as_str(), body.as_str()), ("Dune", "A desert."));
    }
}
