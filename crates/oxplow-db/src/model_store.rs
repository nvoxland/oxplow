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
use serde_json::Value;

use crate::database::{map_sql_err, ts_to_string};

/// A declared column's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Markdown {
    pub pins: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelColumn {
    pub name: String,
    pub ty: ModelType,
    /// An enum's allowed values.
    pub values: Option<Vec<String>>,
    pub markdown: Option<Markdown>,
    pub doc: String,
}

impl ModelColumn {
    pub fn new(name: &str, ty: ModelType) -> Self {
        Self {
            name: name.into(),
            ty,
            values: None,
            markdown: None,
            doc: String::new(),
        }
    }
}

/// A model as its owner declared it.
#[derive(Debug, Clone, PartialEq)]
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
    pub columns: Vec<ModelColumn>,
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

    /// `v_<ns>_<name>`.
    pub fn view(&self) -> String {
        format!("v_{}_{}", self.ns(), self.name)
    }

    /// The URI of `store`'s item `id`.
    pub fn uri(&self, store: &str, id: &str) -> String {
        oxplow_domain::uri::build::of(&self.resource, &[("store", store), ("id", id)])
    }

    fn column(&self, name: &str) -> Option<&ModelColumn> {
        self.columns.iter().find(|c| c.name == name)
    }
}

/// A field a namespace declares on a model.
#[derive(Debug, Clone, PartialEq)]
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
    /// Write these rows by key, and remove these ids.
    Upsert {
        rows: Vec<ModelRow>,
        delete: Vec<String>,
    },
    /// These are every row the store has: the rest of its rows go.
    Snapshot(Vec<ModelRow>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOp {
    Upserted,
    Removed,
}

impl ChangeOp {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeOp::Upserted => "upserted",
            ChangeOp::Removed => "removed",
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
    /// row, none for a removal.
    pub changed: Vec<String>,
}

/// What a write did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Written {
    pub changes: Vec<ModelChange>,
    pub unchanged: usize,
}

/// The columns every row has besides the declared ones.
const IMPLICIT: [&str; 4] = ["uri", "store", "fields", "written_at"];

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
    match decl.column(&decl.key) {
        Some(c) if matches!(c.ty, ModelType::Text | ModelType::Int) => Ok(()),
        _ => Err(invalid(format!(
            "model `{}`: key `{}` must be a text or int column",
            decl.id(),
            decl.key
        ))),
    }
}

/// The stored shape: (column, SQL type), the implicit ones included.
fn shape(decl: &ModelDecl) -> Vec<(String, &'static str)> {
    let mut out = vec![("uri".to_string(), "TEXT"), ("store".to_string(), "TEXT")];
    out.extend(decl.columns.iter().map(|c| (c.name.clone(), c.ty.sql())));
    out.push(("written_at".to_string(), "TEXT"));
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
    let select = view_sql(decl);
    let current: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'view' AND name = ?1",
            [decl.view()],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    let create_view = format!("CREATE VIEW {} AS {select}", quote(&decl.view()));
    if current.as_deref() != Some(create_view.as_str()) {
        conn.execute_batch(&format!(
            "DROP VIEW IF EXISTS {}; {create_view};",
            quote(&decl.view())
        ))
        .map_err(map_sql_err)?;
    }
    register_tx(conn, decl, &select)
}

/// A SQL string literal.
fn sql_text(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The view: the table's columns, and `fields` as `{namespace: {name:
/// value}}` (NULL with none).
fn view_sql(decl: &ModelDecl) -> String {
    let cols: Vec<String> = std::iter::once("t.uri".to_string())
        .chain(std::iter::once("t.store".to_string()))
        .chain(decl.columns.iter().map(|c| format!("t.{}", quote(&c.name))))
        .collect();
    format!(
        "SELECT {}, (SELECT json_group_object(ns.namespace, json(ns.vals)) FROM \
         (SELECT f.namespace, json_group_object(f.name, json(f.value)) AS vals \
         FROM model_field_value f WHERE f.model = {} AND f.uri = t.uri \
         GROUP BY f.namespace ORDER BY f.namespace) ns) AS fields, t.written_at \
         FROM {} t",
        cols.join(", "),
        sql_text(&decl.id()),
        quote(&decl.table())
    )
}

/// Record the view in the dataset registry (an entity: its owner's, with
/// its columns as contract), so the semantic layer reads it and its
/// readers hear its table change.
fn register_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    select: &str,
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
    let contract = Value::Array(columns).to_string();
    let description = format!("The shared model `{}`: every store's items.", decl.id());
    let now = ts_to_string(oxplow_domain::Timestamp::now());
    let current: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT m.sql, c.columns_json FROM dataset m
             LEFT JOIN dataset_contract c ON c.view = m.view AND c.version = m.version
             WHERE m.view = ?1",
            [decl.view()],
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
        params![
            decl.view(),
            decl.name,
            decl.extension,
            description,
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
        params![decl.view(), contract, now],
    )
    .map_err(map_sql_err)?;
    for input in [decl.table(), "model_field_value".to_string()] {
        conn.execute(
            "INSERT OR IGNORE INTO dataset_input (view, input, kind) VALUES (?1, ?2, 'source')",
            params![decl.view(), input],
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
        if name != "fields" && decl.column(name).is_none() {
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
    Ok(Checked { id, cells, fields })
}

/// The stored row of `uri`: its cells in declaration order.
fn stored_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    uri: &str,
) -> Result<Option<Vec<rusqlite::types::Value>>, DomainError> {
    let cols: Vec<String> = decl.columns.iter().map(|c| quote(&c.name)).collect();
    conn.prepare_cached(&format!(
        "SELECT {} FROM {} WHERE uri = ?1",
        cols.join(", "),
        quote(&decl.table())
    ))
    .and_then(|mut st| {
        st.query_row([uri], |r| {
            (0..decl.columns.len())
                .map(|i| r.get::<_, rusqlite::types::Value>(i))
                .collect()
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

fn remove_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    uri: &str,
) -> Result<bool, DomainError> {
    let n = conn
        .execute(
            &format!("DELETE FROM {} WHERE uri = ?1", quote(&decl.table())),
            [uri],
        )
        .map_err(map_sql_err)?;
    conn.execute(
        "DELETE FROM model_field_value WHERE model = ?1 AND uri = ?2",
        params![decl.id(), uri],
    )
    .map_err(map_sql_err)?;
    Ok(n > 0)
}

/// Write `store`'s rows of `decl` (its table must exist: [`ensure_model_tx`]);
/// `fields` are what the store declared, kept under the store's id. Every
/// row is checked first, so a bad one writes nothing. Only what changed is
/// written.
pub fn write_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
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
    let (rows, delete, snapshot) = match write {
        ModelWrite::Upsert { rows, delete } => (rows, delete.as_slice(), false),
        ModelWrite::Snapshot(rows) => (rows, &[][..], true),
    };
    let checked: Vec<Checked> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| check_row(decl, fields, i, r))
        .collect::<Result<_, _>>()?;
    let model = decl.id();
    let now = ts_to_string(oxplow_domain::Timestamp::now());
    let mut out = Written::default();
    let mut kept = std::collections::BTreeSet::new();
    let cols: Vec<String> = decl.columns.iter().map(|c| quote(&c.name)).collect();
    let placeholders: Vec<String> = (0..cols.len()).map(|i| format!("?{}", i + 4)).collect();
    let insert = format!(
        "INSERT INTO {} (uri, store, written_at, {}) VALUES (?1, ?2, ?3, {})
         ON CONFLICT (uri) DO UPDATE SET written_at = excluded.written_at, {}",
        quote(&decl.table()),
        cols.join(", "),
        placeholders.join(", "),
        cols.iter()
            .map(|c| format!("{c} = excluded.{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for row in checked {
        let uri = decl.uri(store, &row.id);
        kept.insert(uri.clone());
        let before = stored_tx(conn, decl, &uri)?;
        let mut changed: Vec<String> = match &before {
            None => decl.columns.iter().map(|c| c.name.clone()).collect(),
            Some(cells) => decl
                .columns
                .iter()
                .zip(cells.iter().zip(&row.cells))
                .filter(|(_, (a, b))| a != b)
                .map(|(c, _)| c.name.clone())
                .collect(),
        };
        if before.is_none() || !changed.is_empty() {
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
        if put_fields_tx(conn, &model, &uri, store, &row.fields)? {
            changed.push("fields".into());
        }
        if changed.is_empty() {
            out.unchanged += 1;
        } else {
            out.changes.push(ModelChange {
                uri,
                store: store.to_string(),
                id: row.id,
                op: ChangeOp::Upserted,
                changed,
            });
        }
    }
    let gone: Vec<(String, String)> = if snapshot {
        let mut st = conn
            .prepare(&format!(
                "SELECT uri, {} FROM {} WHERE store = ?1",
                quote(&decl.key),
                quote(&decl.table())
            ))
            .map_err(map_sql_err)?;
        let all = st
            .query_map([store], |r| {
                let id: rusqlite::types::Value = r.get(1)?;
                Ok((r.get::<_, String>(0)?, id))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(map_sql_err)?;
        all.into_iter()
            .filter(|(uri, _)| !kept.contains(uri))
            .map(|(uri, id)| {
                let id = match id {
                    rusqlite::types::Value::Integer(n) => n.to_string(),
                    rusqlite::types::Value::Text(t) => t,
                    other => format!("{other:?}"),
                };
                (uri, id)
            })
            .collect()
    } else {
        delete
            .iter()
            .map(|id| (decl.uri(store, id), id.clone()))
            .collect()
    };
    for (uri, id) in gone {
        if remove_tx(conn, decl, &uri)? {
            out.changes.push(ModelChange {
                uri,
                store: store.to_string(),
                id,
                op: ChangeOp::Removed,
                changed: Vec::new(),
            });
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
                "SELECT store, {} FROM {} WHERE uri = ?1",
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
    let id = match id {
        rusqlite::types::Value::Integer(n) => n.to_string(),
        rusqlite::types::Value::Text(t) => t,
        other => format!("{other:?}"),
    };
    Ok(Some(ModelChange {
        uri: uri.to_string(),
        store,
        id,
        op: ChangeOp::Upserted,
        changed: vec!["fields".into()],
    }))
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
            columns: vec![
                ModelColumn::new("id", ModelType::Text),
                ModelColumn::new("title", ModelType::Text),
                ModelColumn::new("pages", ModelType::Int),
                state,
                body,
            ],
        }
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
            write_tx(tx, &book(), store, &fields, &w)
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
            },
        )
        .await
        .unwrap();
        assert_eq!(moved.changes[0].changed, vec!["title".to_string()]);

        let gone = write(
            &db,
            "shelf-db",
            vec![],
            ModelWrite::Upsert {
                rows: vec![],
                delete: vec!["b1".into()],
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
            write(&db, store, vec![], ModelWrite::Snapshot(rows))
                .await
                .unwrap();
        }
        let w = write(
            &db,
            "a",
            vec![],
            ModelWrite::Snapshot(vec![row(json!({"id": "1", "title": "a1"}))]),
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
    /// an undeclared field fail the whole write, naming the row and column.
    #[tokio::test]
    async fn a_bad_row_fails_the_write_naming_it() {
        let db = Database::in_memory();
        db.transaction(|tx| ensure_model_tx(tx, &book())).await.unwrap();
        for (bad, says) in [
            (json!({"id": "b1", "colour": "red"}), "colour"),
            (json!({"id": "b1", "pages": "many"}), "pages"),
            (json!({"id": "b1", "state": "lost"}), "state"),
            (json!({"title": "no id"}), "id"),
            (json!({"id": "b1", "fields": {"mood": 3}}), "mood"),
        ] {
            let err = write(
                &db,
                "s",
                rating(),
                ModelWrite::Upsert {
                    rows: vec![row(json!({"id": "ok", "title": "fine"})), row(bad)],
                    delete: vec![],
                },
            )
            .await
            .unwrap_err()
            .to_string();
            assert!(err.contains(says), "{err}");
        }
        assert!(rows(&db).await.is_empty(), "nothing of a bad write lands");
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
            ModelWrite::Snapshot(vec![row(
                json!({"id": "b1", "title": "Dune", "fields": {"rating": 4}}),
            )]),
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
            ModelWrite::Snapshot(vec![row(
                json!({"id": "b1", "title": "Dune", "fields": {"rating": 5}}),
            )]),
        )
        .await
        .unwrap();
        assert_eq!(
            fields(&rows(&db).await),
            json!({"shelf-db": {"rating": 5}, "shelf-score": {"score": 9}}),
            "a reconcile keeps another namespace's values"
        );

        write(&db, "shelf-db", rating(), ModelWrite::Snapshot(vec![]))
            .await
            .unwrap();
        let left: i64 = db
            .read(|tx| {
                tx.query_row("SELECT count(*) FROM model_field_value", [], |r| r.get(0))
                    .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(left, 0, "values go when the item goes");
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
}
