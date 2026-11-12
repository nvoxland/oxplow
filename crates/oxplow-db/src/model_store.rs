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
    /// The rows that belong to one row of this model (a task's links and
    /// comments), written nested in the row as the whole set.
    #[serde(default)]
    pub children: Vec<ChildDecl>,
}

/// A set of rows owned by one row of a model (`.context/models.md`
/// "Children"): stored in `model__<ns>__<model>__<name>`, read through
/// `v_<ns>_<model>_<name>` (`parent`, `store`, the columns).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildDecl {
    pub name: String,
    /// The columns that name a child among its parent's (unique within
    /// the parent).
    pub key: Vec<String>,
    pub columns: Vec<ModelColumn>,
    /// Each child is an edge from its parent's URI.
    #[serde(default)]
    pub edge: Option<ChildEdge>,
}

/// How a child projects an edge from its parent: to the URI in column
/// `to`, typed `prefix` + the value of `type_from` (a text column with
/// `values:`), or the fixed `ref_type`. Exactly one of the two.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildEdge {
    pub to: String,
    #[serde(default)]
    pub type_from: Option<String>,
    #[serde(default)]
    pub ref_type: Option<String>,
    #[serde(default)]
    pub prefix: Option<String>,
}

impl ChildEdge {
    /// Every ref type this edge can project.
    fn types(&self, child: &ChildDecl) -> Vec<String> {
        if let Some(fixed) = &self.ref_type {
            return vec![fixed.clone()];
        }
        let prefix = self.prefix.as_deref().unwrap_or_default();
        self.type_from
            .as_deref()
            .and_then(|c| child.column(c))
            .and_then(|c| c.values.as_ref())
            .map(|vs| vs.iter().map(|v| format!("{prefix}{v}")).collect())
            .unwrap_or_default()
    }
}

impl ChildDecl {
    fn column(&self, name: &str) -> Option<&ModelColumn> {
        self.columns.iter().find(|c| c.name == name)
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    fn key_indexes(&self) -> Vec<usize> {
        self.key.iter().filter_map(|k| self.index(k)).collect()
    }
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

    /// `model__<ns>__<name>__<child>`.
    pub fn child_table(&self, child: &ChildDecl) -> String {
        format!("{}__{}", self.table(), child.name)
    }

    /// `v_<ns>_<name>_<child>`: the children of the live rows.
    pub fn child_view(&self, child: &ChildDecl) -> String {
        format!("{}_{}", self.view(), child.name)
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
    if !decl
        .column(&decl.key)
        .is_some_and(|c| matches!(c.ty, ModelType::Text | ModelType::Int))
    {
        return Err(invalid(format!(
            "model `{}`: key `{}` must be a text or int column",
            decl.id(),
            decl.key
        )));
    }
    for (i, child) in decl.children.iter().enumerate() {
        validate_child(decl, child, &decl.children[..i])?;
    }
    Ok(())
}

/// The columns every child row has besides the declared ones.
const CHILD_IMPLICIT: [&str; 3] = ["parent", "store", "written_at"];

/// The rules for one child (`.context/models.md` "Children"). A child's
/// edge ref types are its model's own, so they cannot be a core type:
/// `prefix` is `<model>_<word>:` (core's types are plain words, and the
/// colon keeps a prefixed type apart from them) and a fixed `ref_type` is
/// `<model>.<word>`, as a column's `edge` is.
fn validate_child(
    decl: &ModelDecl,
    child: &ChildDecl,
    before: &[ChildDecl],
) -> Result<(), DomainError> {
    let bad = |what: String| {
        invalid(format!(
            "model `{}`: child `{}`: {what}",
            decl.id(),
            child.name
        ))
    };
    if !is_ident(&child.name)
        || IMPLICIT.contains(&child.name.as_str())
        || matches!(child.name.as_str(), "all" | "field")
        || decl.column(&child.name).is_some()
        || before.iter().any(|b| b.name == child.name)
    {
        return Err(bad("is not a name a child may take".into()));
    }
    if child.columns.is_empty() {
        return Err(bad("declares no columns".into()));
    }
    for (i, c) in child.columns.iter().enumerate() {
        if !is_ident(&c.name)
            || CHILD_IMPLICIT.contains(&c.name.as_str())
            || child.columns[..i].iter().any(|o| o.name == c.name)
        {
            return Err(bad(format!(
                "column `{}` is not a name it may declare",
                c.name
            )));
        }
        if c.edge.is_some() {
            return Err(bad(format!(
                "column `{}`: a child's edge is declared on the child, not a column",
                c.name
            )));
        }
        if c.markdown.is_some_and(|m| m.pins) {
            return Err(bad(format!(
                "column `{}`: a child's markdown can't pin",
                c.name
            )));
        }
    }
    if child.key.is_empty() {
        return Err(bad("needs a key".into()));
    }
    for (i, k) in child.key.iter().enumerate() {
        let ok = child.column(k).is_some_and(|c| {
            matches!(
                c.ty,
                ModelType::Text | ModelType::Int | ModelType::Uri | ModelType::Time
            )
        });
        if !ok || child.key[..i].contains(k) {
            return Err(bad(format!(
                "key `{k}` must be a text, int, uri or time column of the child, once"
            )));
        }
    }
    let Some(edge) = &child.edge else {
        return Ok(());
    };
    if !child
        .column(&edge.to)
        .is_some_and(|c| c.ty == ModelType::Uri)
    {
        return Err(bad(format!("edge `to: {}` must be a uri column", edge.to)));
    }
    match (&edge.type_from, &edge.ref_type) {
        (Some(from), None) => {
            if !child.column(from).is_some_and(|c| {
                c.ty == ModelType::Text && c.values.as_ref().is_some_and(|v| !v.is_empty())
            }) {
                return Err(bad(format!(
                    "edge `type_from: {from}` must be a text column with `values:`"
                )));
            }
            let model = format!("{}_", decl.name);
            let ok = edge.prefix.as_deref().is_some_and(|p| {
                p.strip_suffix(':')
                    .is_some_and(|w| w.starts_with(&model) && is_ident(w))
            });
            if !ok {
                return Err(bad(format!(
                    "edge `prefix` must be `{model}<word>:` (its model's own ref types)"
                )));
            }
        }
        (None, Some(fixed)) => {
            let own = fixed
                .strip_prefix(&format!("{}.", decl.name))
                .is_some_and(is_ident);
            if !own || edge.prefix.is_some() {
                return Err(bad(format!(
                    "edge `ref_type: {fixed}` must be `{}.<word>` and takes no prefix",
                    decl.name
                )));
            }
            // Each edge type's slice is restated on its own: two owners of
            // one type would delete each other's edges.
            if let Some(c) = decl
                .columns
                .iter()
                .find(|c| c.edge.as_deref() == Some(fixed.as_str()))
            {
                return Err(bad(format!(
                    "edge `ref_type: {fixed}` is also the edge of column `{}`",
                    c.name
                )));
            }
            if let Some(o) = before.iter().find(|b| {
                b.edge
                    .as_ref()
                    .is_some_and(|e| e.ref_type.as_deref() == Some(fixed.as_str()))
            }) {
                return Err(bad(format!(
                    "edge `ref_type: {fixed}` is also the edge of child `{}`",
                    o.name
                )));
            }
        }
        _ => {
            return Err(bad(
                "edge needs exactly one of `type_from` and `ref_type`".into()
            ))
        }
    }
    Ok(())
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

/// Re-pin `source`'s pinned refs to now (`oxplow.refs.verify`): `files`,
/// or every file it pins when `None`. Each must be a file it pins or be
/// under a directory it pins (one under a directory becomes a pin of its
/// own). Nothing reaches the store. The paths re-pinned; refused for a
/// source with no pinned refs (its body isn't projected yet).
pub fn verify_pins_tx(
    conn: &rusqlite::Connection,
    source: &str,
    files: Option<&[String]>,
) -> Result<Vec<String>, DomainError> {
    use crate::page_ref_projections::{RT_DIR_PIN, RT_FILE_PIN};
    use crate::page_ref_store::{upsert_edge_tx, PageRefEdge};
    let pinned: Vec<(String, String)> = conn
        .prepare(
            "SELECT target_uri, ref_type FROM page_ref
             WHERE source_uri = ?1 AND ref_type IN (?2, ?3)",
        )
        .and_then(|mut st| {
            st.query_map(params![source, RT_FILE_PIN, RT_DIR_PIN], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(map_sql_err)?;
    if pinned.is_empty() {
        return Err(invalid(format!(
            "`{source}` pins no files yet (has its body been read?)"
        )));
    }
    let path_of = |uri: &str| {
        oxplow_domain::uri::build::file_of_uri(uri)
            .ok()
            .map(|f| f.path)
    };
    let pinned_files: Vec<String> = pinned
        .iter()
        .filter(|(_, t)| t == RT_FILE_PIN)
        .filter_map(|(u, _)| path_of(u))
        .collect();
    let dirs: Vec<String> = pinned
        .iter()
        .filter(|(_, t)| t == RT_DIR_PIN)
        .filter_map(|(u, _)| dir_path(u))
        .collect();
    let wanted: Vec<String> = files.map_or_else(|| pinned_files.clone(), <[String]>::to_vec);
    let outside: Vec<&str> = wanted
        .iter()
        .filter(|p| {
            !pinned_files.contains(p) && !crate::knowledge_record::path_under_any_dir(p, &dirs)
        })
        .map(String::as_str)
        .collect();
    if !outside.is_empty() {
        return Err(invalid(format!(
            "`{source}` neither cites nor is under a cited directory of: {}",
            outside.join(", ")
        )));
    }
    let pin = crate::knowledge_record::pin_tx(conn)?;
    let Some(snapshot) = pin.snapshot_id else {
        return Err(invalid("there is no snapshot to pin to yet".into()));
    };
    for path in &wanted {
        let edge = PageRefEdge::new(
            source,
            oxplow_domain::uri::build::file_uri(path),
            RT_FILE_PIN,
        )
        .with_version(snapshot, pin.revision.clone(), pin.revision.is_some());
        upsert_edge_tx(conn, &edge)?;
    }
    // Verifying all of it re-pins its directories too: what it says of
    // them holds as of now.
    if files.is_none() {
        for (uri, _) in pinned.iter().filter(|(_, t)| t == RT_DIR_PIN) {
            let edge = PageRefEdge::new(source, uri, RT_DIR_PIN).with_version(
                snapshot,
                pin.revision.clone(),
                pin.revision.is_some(),
            );
            upsert_edge_tx(conn, &edge)?;
        }
    }
    Ok(wanted)
}

/// Restate the edges of every row `decl` keeps — its markdown's mentions
/// (or its stated refs), its edge columns, its children's edges — from
/// what is stored, by diff: the repair after a change to how edges are
/// projected (a migration, a fixed projection). Pins already there are
/// kept. How many rows it went over.
pub fn repair_edges_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    kinds: &oxplow_domain::uri::resources::ResourceRegistry,
) -> Result<usize, DomainError> {
    validate(decl)?;
    let cols: Vec<String> = decl.columns.iter().map(|c| quote(&c.name)).collect();
    let n = decl.columns.len();
    let rows: Vec<(String, Vec<rusqlite::types::Value>, Option<String>)> = conn
        .prepare(&format!(
            "SELECT uri, {}, refs FROM {}",
            cols.join(", "),
            quote(&decl.table())
        ))
        .and_then(|mut st| {
            st.query_map([], |r| {
                let cells = (0..n)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i + 1))
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok((r.get(0)?, cells, r.get(n + 1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(map_sql_err)?;
    let count = rows.len();
    for (uri, cells, refs) in rows {
        let row = Checked {
            id: String::new(),
            cells,
            fields: BTreeMap::new(),
            refs: refs.and_then(|r| serde_json::from_str(&r).ok()),
            children: Vec::new(),
        };
        restate_edges_tx(conn, decl, kinds, &uri, &row)?;
        if !decl.children.is_empty() {
            restate_child_edges_tx(conn, decl, kinds, &uri)?;
        }
    }
    Ok(count)
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
    // The refs a store stated (JSON), kept so a repair restates its edges
    // as the write did; NULL when its markdown was parsed.
    out.push(("refs".to_string(), "TEXT"));
    out
}

/// Create (or bring up to date) `decl`'s table and view, and record the
/// view as its owner's dataset. A new column is added in place; a removed
/// or retyped one rebuilds the table in place, its rows and their fields
/// kept (`.context/models.md` "Removal").
pub fn ensure_model_tx(conn: &rusqlite::Connection, decl: &ModelDecl) -> Result<(), DomainError> {
    ensure_model_for_stores_tx(conn, decl, None)
}

/// [`ensure_model_tx`], its views showing only the rows of `live` stores
/// (the enabled ones'), when given: a disabled store's rows are kept but
/// hidden, as tombstones are.
pub fn ensure_model_for_stores_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    live: Option<&[String]>,
) -> Result<(), DomainError> {
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
    let create = |name: &str| {
        let cols: Vec<String> = want
            .iter()
            .map(|(n, t)| match n.as_str() {
                "uri" => format!("uri {t} PRIMARY KEY"),
                "store" | "written_at" => format!("{n} {t} NOT NULL"),
                _ => format!("{} {t}", quote(n)),
            })
            .collect();
        format!("CREATE TABLE {} ({});", quote(name), cols.join(", "))
    };
    if have.is_empty() {
        conn.execute_batch(&create(&table)).map_err(map_sql_err)?;
    } else {
        let removed_or_retyped = have.iter().any(|(n, t)| {
            want.iter()
                .find(|(w, _)| w == n)
                .is_none_or(|(_, wt)| !wt.eq_ignore_ascii_case(t))
        });
        if removed_or_retyped {
            // Rebuilt in place: the rows (and every extension's field
            // values, which are keyed by URI) are kept, the columns both
            // shapes have copied, no event logged.
            let mut views = vec![
                decl.view(),
                decl.view_all(),
                format!("{}_field", decl.view()),
                decl.search_view(),
            ];
            // The children's views read this table: a rename onto its name
            // would find them broken.
            views.extend(decl.children.iter().map(|c| decl.child_view(c)));
            for view in views {
                conn.execute_batch(&format!("DROP VIEW IF EXISTS {};", quote(&view)))
                    .map_err(map_sql_err)?;
            }
            let rebuilt = format!("{table}__rebuilt");
            let common: Vec<String> = want
                .iter()
                .filter(|(n, _)| have.iter().any(|(h, _)| h == n))
                .map(|(n, _)| quote(n))
                .collect();
            conn.execute_batch(&format!(
                "DROP TABLE IF EXISTS {r};
                 {create}
                 INSERT INTO {r} ({cols}) SELECT {cols} FROM {t};
                 DROP TABLE {t};
                 ALTER TABLE {r} RENAME TO {t};",
                r = quote(&rebuilt),
                create = create(&rebuilt),
                cols = common.join(", "),
                t = quote(&table),
            ))
            .map_err(map_sql_err)?;
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
    conn.execute_batch(&format!(
        "CREATE INDEX IF NOT EXISTS {} ON {} (store);",
        quote(&format!("{table}_store")),
        quote(&table)
    ))
    .map_err(map_sql_err)?;
    for all in [false, true] {
        let view = if all { decl.view_all() } else { decl.view() };
        let select = view_sql(decl, all, live);
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
    for child in &decl.children {
        ensure_child_tx(conn, decl, child, live)?;
    }
    Ok(())
}

/// The child's stored shape: (column, SQL type), `parent` first.
fn child_shape(child: &ChildDecl) -> Vec<(String, &'static str)> {
    let mut out = vec![
        ("parent".to_string(), "TEXT"),
        ("store".to_string(), "TEXT"),
    ];
    out.extend(child.columns.iter().map(|c| (c.name.clone(), c.ty.sql())));
    out.push(("written_at".to_string(), "TEXT"));
    out
}

/// Create the child's table and view, and record the view as its owner's
/// dataset. A changed shape keeps the rows: a new column is added in
/// place, a removed or retyped one is rebuilt with the common columns copied
/// (an incremental store never refills a child table).
fn ensure_child_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    child: &ChildDecl,
    live: Option<&[String]>,
) -> Result<(), DomainError> {
    let table = decl.child_table(child);
    let view = decl.child_view(child);
    let want = child_shape(child);
    // The primary key: `parent`, then the key columns in order.
    let pk_of = |name: &str| -> i64 {
        if name == "parent" {
            1
        } else {
            child
                .key
                .iter()
                .position(|k| k == name)
                .map_or(0, |i| i as i64 + 2)
        }
    };
    let have: Vec<(String, String, i64)> = conn
        .prepare(&format!(
            "SELECT name, type, pk FROM pragma_table_info({})",
            sql_text(&table)
        ))
        .and_then(|mut st| {
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(map_sql_err)?;
    let same = have.len() == want.len()
        && want.iter().all(|(n, t)| {
            have.iter()
                .any(|(h, ht, pk)| h == n && ht.eq_ignore_ascii_case(t) && *pk == pk_of(n))
        });
    if !same {
        let rebuilt_needed = have.is_empty()
            || have.iter().any(|(n, t, pk)| {
                want.iter()
                    .find(|(w, _)| w == n)
                    .is_none_or(|(_, wt)| !wt.eq_ignore_ascii_case(t) || *pk != pk_of(n))
            });
        // The view reads the table: dropped before either change, made
        // again below.
        conn.execute_batch(&format!("DROP VIEW IF EXISTS {};", quote(&view)))
            .map_err(map_sql_err)?;
        if rebuilt_needed {
            // Rows have no URI to keep annotations by, but an incremental
            // store never refills them: rebuilt in place, the columns both
            // shapes have copied.
            let mut cols: Vec<String> = want
                .iter()
                .map(|(n, t)| match n.as_str() {
                    "parent" | "store" | "written_at" => format!("{n} {t} NOT NULL"),
                    _ => format!("{} {t}", quote(n)),
                })
                .collect();
            let mut pk = vec!["parent".to_string()];
            pk.extend(child.key.iter().map(|k| quote(k)));
            cols.push(format!("PRIMARY KEY ({})", pk.join(", ")));
            let rebuilt = format!("{table}__rebuilt");
            let common: Vec<String> = want
                .iter()
                .filter(|(n, _)| have.iter().any(|(h, _, _)| h == n))
                .map(|(n, _)| quote(n))
                .collect();
            let copy = if have.is_empty() {
                String::new()
            } else {
                format!(
                    "INSERT OR IGNORE INTO {r} ({common}) SELECT {common} FROM {t};",
                    r = quote(&rebuilt),
                    common = common.join(", "),
                    t = quote(&table),
                )
            };
            conn.execute_batch(&format!(
                "DROP TABLE IF EXISTS {r};
                 CREATE TABLE {r} ({c});
                 {copy}
                 DROP TABLE IF EXISTS {t};
                 ALTER TABLE {r} RENAME TO {t};",
                r = quote(&rebuilt),
                c = cols.join(", "),
                t = quote(&table),
            ))
            .map_err(map_sql_err)?;
        } else {
            for (n, t) in &want {
                if !have.iter().any(|(h, _, _)| h == n) {
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
    let mut filters = vec!["t.removed_at IS NULL".to_string()];
    filters.extend(store_filter(live));
    let cols: Vec<String> = ["parent", "store"]
        .into_iter()
        .map(str::to_string)
        .chain(child.columns.iter().map(|c| quote(&c.name)))
        .chain(std::iter::once("written_at".to_string()))
        .map(|c| format!("c.{c}"))
        .collect();
    let select = format!(
        "SELECT {} FROM {} c JOIN {} t ON t.uri = c.parent WHERE {}",
        cols.join(", "),
        quote(&table),
        quote(&decl.table()),
        filters.join(" AND ")
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
    let mut columns: Vec<Value> = vec![
        serde_json::json!({"name": "parent", "type": "TEXT", "doc": "The item's URI."}),
        serde_json::json!({"name": "store", "type": "TEXT", "doc": "The store that keeps it."}),
    ];
    columns.extend(child.columns.iter().map(|c| {
        serde_json::json!({"name": c.name, "type": c.ty.sql(), "doc": if c.doc.is_empty() { c.ty.name().to_string() } else { c.doc.clone() }})
    }));
    columns.push(serde_json::json!({"name": "written_at", "type": "TEXT", "doc": "When oxplow last wrote it."}));
    register_view_tx(
        conn,
        decl,
        &view,
        &format!("{}_{}", decl.name, child.name),
        &format!(
            "The `{}` of the shared model `{}`'s items, one row each.",
            child.name,
            decl.id()
        ),
        &select,
        Value::Array(columns),
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO dataset_input (view, input, kind) VALUES (?1, ?2, 'source')",
        params![view, table],
    )
    .map_err(map_sql_err)?;
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

/// The filter that hides a disabled store's rows (`t` the model's table).
fn store_filter(live: Option<&[String]>) -> Option<String> {
    live.map(|stores| {
        let list: Vec<String> = stores.iter().map(|s| sql_text(s)).collect();
        format!(
            "t.store IN ({})",
            if list.is_empty() {
                "NULL".to_string()
            } else {
                list.join(", ")
            }
        )
    })
}

/// The view: the table's columns, and `fields` as `{namespace: {name:
/// value}}` (NULL with none).
fn view_sql(decl: &ModelDecl, all: bool, live: Option<&[String]>) -> String {
    let mut filters: Vec<String> = Vec::new();
    if !all {
        filters.push("t.removed_at IS NULL".into());
    }
    filters.extend(store_filter(live));
    let filter = if filters.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", filters.join(" AND "))
    };
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
        filter
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
    /// The whole set of each child the row names: (child index, rows as
    /// stored, the child's columns in order).
    children: Vec<(usize, Vec<Vec<rusqlite::types::Value>>)>,
}

/// A child's set as the row states it, checked, or why not.
fn check_children(
    child: &ChildDecl,
    value: &Value,
) -> Result<Vec<Vec<rusqlite::types::Value>>, String> {
    let Value::Array(items) = value else {
        return Err(format!("expected a list of objects, got {value}"));
    };
    let keys = child.key_indexes();
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let Value::Object(map) = item else {
            return Err(format!("[{i}]: expected an object, got {item}"));
        };
        if let Some(name) = map.keys().find(|k| child.column(k).is_none()) {
            return Err(format!("[{i}]: column `{name}` isn't declared"));
        }
        let mut cells = Vec::with_capacity(child.columns.len());
        for column in &child.columns {
            let value = map.get(&column.name).unwrap_or(&Value::Null);
            cells.push(cell(column, value).map_err(|e| format!("[{i}] `{}`: {e}", column.name))?);
        }
        if keys
            .iter()
            .any(|k| cells[*k] == rusqlite::types::Value::Null)
        {
            return Err(format!("[{i}]: key `{}` is missing", child.key.join(", ")));
        }
        if !seen.insert(key_of(&cells, &keys)) {
            return Err(format!(
                "[{i}]: key `{}` is there twice",
                child.key.join(", ")
            ));
        }
        out.push(cells);
    }
    Ok(out)
}

/// The key cells of a child row, as one comparable string.
fn key_of(cells: &[rusqlite::types::Value], keys: &[usize]) -> String {
    keys.iter()
        .map(|k| format!("{:?}", cells[*k]))
        .collect::<Vec<_>>()
        .join("\u{1f}")
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
        if name != "fields"
            && name != "refs"
            && decl.column(name).is_none()
            && !decl.children.iter().any(|c| &c.name == name)
        {
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
    let mut children = Vec::new();
    for (i, child) in decl.children.iter().enumerate() {
        match row.get(&child.name) {
            None | Some(Value::Null) => {}
            Some(value) => children.push((
                i,
                check_children(child, value)
                    .map_err(|e| fail(format!("child `{}`: {e}", child.name)))?,
            )),
        }
    }
    Ok(Checked {
        id,
        cells,
        fields,
        refs,
        children,
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
/// Restate `namespace`'s fields on `uri` to `fields`, and say what moved:
/// each changed field as `fields.<namespace>.<name>`, with its value
/// before and after (JSON; null when absent). Empty when nothing moved.
fn put_fields_tx(
    conn: &rusqlite::Connection,
    model: &str,
    uri: &str,
    namespace: &str,
    fields: &BTreeMap<String, String>,
) -> Result<Vec<(String, Value, Value)>, DomainError> {
    let stored = stored_fields_tx(conn, model, uri, namespace)?;
    if &stored == fields {
        return Ok(Vec::new());
    }
    let parse = |v: Option<&String>| {
        v.and_then(|v| serde_json::from_str::<Value>(v).ok())
            .unwrap_or(Value::Null)
    };
    let moved: Vec<(String, Value, Value)> = stored
        .keys()
        .chain(fields.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|name| stored.get(*name) != fields.get(*name))
        .map(|name| {
            (
                format!("fields.{namespace}.{name}"),
                parse(stored.get(name)),
                parse(fields.get(name)),
            )
        })
        .collect();
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
    Ok(moved)
}

/// Name each moved field on `change`, with its value before and after.
fn note_fields(change: &mut ModelChange, moved: Vec<(String, Value, Value)>) {
    if moved.is_empty() {
        return;
    }
    change.changed.push("fields".into());
    for (name, before, after) in moved {
        change.changed.push(name.clone());
        change.before.insert(name.clone(), before);
        change.after.insert(name, after);
    }
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
    for child in &decl.children {
        conn.execute(
            &format!(
                "DELETE FROM {} WHERE parent IN
                   (SELECT uri FROM {} WHERE removed_at IS NOT NULL AND removed_at < ?1)",
                quote(&decl.child_table(child)),
                quote(&decl.table())
            ),
            [before],
        )
        .map_err(map_sql_err)?;
    }
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

/// A directory URI's path.
fn dir_path(uri: &str) -> Option<String> {
    oxplow_domain::uri::build::param_of(uri, oxplow_domain::uri::resources::DIR, "path").ok()
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
            .filter_map(|e| dir_path(&e.target))
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

/// A child's stored rows of `parent`, cells in the child's column order.
fn child_rows_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    child: &ChildDecl,
    parent: &str,
) -> Result<Vec<Vec<rusqlite::types::Value>>, DomainError> {
    let cols: Vec<String> = child.columns.iter().map(|c| quote(&c.name)).collect();
    let n = cols.len();
    conn.prepare_cached(&format!(
        "SELECT {} FROM {} WHERE parent = ?1",
        cols.join(", "),
        quote(&decl.child_table(child))
    ))
    .and_then(|mut st| {
        st.query_map([parent], |r| {
            (0..n)
                .map(|i| r.get::<_, rusqlite::types::Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
    })
    .map_err(map_sql_err)
}

/// Restate `child` of `parent` to exactly `rows`: new keys are inserted,
/// changed rows updated, missing ones deleted. Whether anything moved.
fn restate_children_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    child: &ChildDecl,
    store: &str,
    parent: &str,
    rows: &[Vec<rusqlite::types::Value>],
    now: &str,
) -> Result<bool, DomainError> {
    let keys = child.key_indexes();
    let table = quote(&decl.child_table(child));
    let mut stored: BTreeMap<String, Vec<rusqlite::types::Value>> =
        child_rows_tx(conn, decl, child, parent)?
            .into_iter()
            .map(|cells| (key_of(&cells, &keys), cells))
            .collect();
    let cols: Vec<String> = child.columns.iter().map(|c| quote(&c.name)).collect();
    let key_where: Vec<String> = keys
        .iter()
        .enumerate()
        .map(|(n, k)| format!("{} = ?{}", cols[*k], n + 2))
        .collect();
    let mut moved = false;
    for cells in rows {
        match stored.remove(&key_of(cells, &keys)) {
            Some(old) if &old == cells => {}
            Some(_) => {
                moved = true;
                let sets: Vec<String> = (0..cols.len())
                    .filter(|i| !keys.contains(i))
                    .map(|i| format!("{} = ?{}", cols[i], keys.len() + 3 + i))
                    .collect();
                if sets.is_empty() {
                    continue;
                }
                let mut values: Vec<rusqlite::types::Value> = vec![parent.to_string().into()];
                values.extend(keys.iter().map(|k| cells[*k].clone()));
                values.push(now.to_string().into());
                values.extend(cells.iter().cloned());
                conn.execute(
                    &format!(
                        "UPDATE {table} SET written_at = ?{}, {} WHERE parent = ?1 AND {}",
                        keys.len() + 2,
                        sets.join(", "),
                        key_where.join(" AND ")
                    ),
                    rusqlite::params_from_iter(values),
                )
                .map_err(map_sql_err)?;
            }
            None => {
                moved = true;
                let placeholders: Vec<String> =
                    (0..cols.len()).map(|i| format!("?{}", i + 4)).collect();
                let mut values: Vec<rusqlite::types::Value> = vec![
                    parent.to_string().into(),
                    store.to_string().into(),
                    now.to_string().into(),
                ];
                values.extend(cells.iter().cloned());
                conn.execute(
                    &format!(
                        "INSERT INTO {table} (parent, store, written_at, {}) VALUES (?1, ?2, ?3, {})",
                        cols.join(", "),
                        placeholders.join(", ")
                    ),
                    rusqlite::params_from_iter(values),
                )
                .map_err(map_sql_err)?;
            }
        }
    }
    for cells in stored.values() {
        moved = true;
        let mut values: Vec<rusqlite::types::Value> = vec![parent.to_string().into()];
        values.extend(keys.iter().map(|k| cells[*k].clone()));
        conn.execute(
            &format!(
                "DELETE FROM {table} WHERE parent = ?1 AND {}",
                key_where.join(" AND ")
            ),
            rusqlite::params_from_iter(values),
        )
        .map_err(map_sql_err)?;
    }
    Ok(moved)
}

/// Restate the edges `parent`'s children project, by diff: each child
/// `edge` (one per row), and the `comment_*` mentions of every child
/// markdown column (one slice for the parent, like a task's comments).
fn restate_child_edges_tx(
    conn: &rusqlite::Connection,
    decl: &ModelDecl,
    kinds: &oxplow_domain::uri::resources::ResourceRegistry,
    parent: &str,
) -> Result<(), DomainError> {
    use crate::page_ref_store::{PageRefEdge, SourceSlice};
    let mut types: Vec<String> = Vec::new();
    let mut edges: Vec<PageRefEdge> = Vec::new();
    let mut bodies: Vec<String> = Vec::new();
    let mut comments = false;
    for child in &decl.children {
        let markdown: Vec<usize> = (0..child.columns.len())
            .filter(|i| child.columns[*i].markdown.is_some())
            .collect();
        if child.edge.is_none() && markdown.is_empty() {
            continue;
        }
        let rows = child_rows_tx(conn, decl, child, parent)?;
        if let Some(edge) = &child.edge {
            types.extend(edge.types(child));
            let to = child.index(&edge.to);
            let from = edge.type_from.as_deref().and_then(|c| child.index(c));
            for cells in &rows {
                let Some(rusqlite::types::Value::Text(target)) = to.map(|i| &cells[i]) else {
                    continue;
                };
                let ref_type = match (&edge.ref_type, from.map(|i| &cells[i])) {
                    (Some(fixed), _) => fixed.clone(),
                    (None, Some(rusqlite::types::Value::Text(v))) => {
                        format!("{}{v}", edge.prefix.as_deref().unwrap_or_default())
                    }
                    _ => continue,
                };
                if oxplow_domain::uri::OxplowUri::parse(target).is_ok() {
                    edges.push(PageRefEdge::new(parent, target, ref_type));
                }
            }
        }
        if !markdown.is_empty() {
            comments = true;
            for cells in &rows {
                for i in &markdown {
                    if let rusqlite::types::Value::Text(t) = &cells[*i] {
                        bodies.push(t.clone());
                    }
                }
            }
        }
    }
    if comments {
        types.extend(crate::page_ref_projections::task_comment_ref_types());
        edges.extend(crate::page_ref_projections::task_comment_edges(
            kinds,
            parent,
            bodies.iter().map(String::as_str),
        ));
    }
    types.sort();
    types.dedup();
    if types.is_empty() {
        return Ok(());
    }
    edges.sort_by(|a, b| (&a.target, &a.ref_type).cmp(&(&b.target, &b.ref_type)));
    edges.dedup_by(|a, b| a.target == b.target && a.ref_type == b.ref_type);
    let slice = SourceSlice {
        source: parent.to_string(),
        ref_types: Some(types.clone()),
        edges: edges.clone(),
    };
    if crate::page_ref_store::slice_is_stored_tx(conn, &slice)? {
        return Ok(());
    }
    crate::page_ref_store::replace_source_for_ref_types_tx(conn, parent, &types, edges)
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

    for (at, (from, to)) in rename.iter().enumerate() {
        let (old, new) = (decl.uri(store, from), decl.uri(store, to));
        let state = |uri: &str| -> Result<Option<bool>, DomainError> {
            conn.query_row(
                &format!("SELECT removed_at IS NOT NULL FROM {table} WHERE uri = ?1"),
                [uri],
                |r| r.get::<_, bool>(0),
            )
            .optional()
            .map_err(map_sql_err)
        };
        // Only a live item moves; onto a live one it can't.
        if state(&old)? != Some(false) {
            continue;
        }
        match state(&new)? {
            Some(false) => {
                out.quarantined.push(Quarantined {
                    at,
                    id: Some(from.clone()),
                    message: format!(
                        "model `{model}`: rename `{from}` → `{to}`: `{to}` is already an item"
                    ),
                });
                continue;
            }
            // A tombstone at the new id gives way.
            Some(true) => {
                conn.execute(&format!("DELETE FROM {table} WHERE uri = ?1"), [&new])
                    .map_err(map_sql_err)?;
                conn.execute(
                    "DELETE FROM model_field_value WHERE model = ?1 AND uri = ?2",
                    params![model, new],
                )
                .map_err(map_sql_err)?;
                for child in &decl.children {
                    conn.execute(
                        &format!(
                            "DELETE FROM {} WHERE parent = ?1",
                            quote(&decl.child_table(child))
                        ),
                        [&new],
                    )
                    .map_err(map_sql_err)?;
                }
            }
            None => {}
        }
        for child in &decl.children {
            conn.execute(
                &format!(
                    "UPDATE {} SET parent = ?2 WHERE parent = ?1",
                    quote(&decl.child_table(child))
                ),
                params![old, new],
            )
            .map_err(map_sql_err)?;
        }
        conn.execute(
            &format!(
                "UPDATE {table} SET uri = ?2, {} = ?3, written_at = ?4 WHERE uri = ?1",
                quote(&decl.key)
            ),
            params![old, new, to, now],
        )
        .map_err(map_sql_err)?;
        // A source that cites both the old and the new URI keeps one edge.
        for sql in [
            "UPDATE model_field_value SET uri = ?2 WHERE model = ?3 AND uri = ?1",
            "UPDATE OR REPLACE page_ref SET source_uri = ?2 WHERE source_uri = ?1 AND ?3 = ?3",
            "UPDATE OR REPLACE page_ref SET target_uri = ?2 WHERE target_uri = ?1 AND ?3 = ?3",
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
            conn.execute(
                &format!("UPDATE {table} SET refs = ?2 WHERE uri = ?1"),
                params![
                    uri,
                    row.refs
                        .as_ref()
                        .map(|r| serde_json::to_string(r).unwrap_or_default())
                ],
            )
            .map_err(map_sql_err)?;
            restate_edges_tx(conn, decl, kinds, &uri, &row)?;
        }
        let mut children_moved = false;
        for (i, set) in &row.children {
            let child = &decl.children[*i];
            if restate_children_tx(conn, decl, child, store, &uri, set, &now)? {
                change.changed.push(child.name.clone());
                children_moved = true;
            }
        }
        if children_moved {
            restate_child_edges_tx(conn, decl, kinds, &uri)?;
        }
        let moved = put_fields_tx(conn, &model, &uri, store, &row.fields)?;
        note_fields(&mut change, moved);
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
    let moved = put_fields_tx(conn, &decl.id(), uri, namespace, &merged)?;
    if moved.is_empty() {
        return Ok(None);
    }
    let mut change = ModelChange::new(uri.to_string(), &store, id_text(id), ChangeOp::Upserted);
    note_fields(&mut change, moved);
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
            children: vec![],
        }
    }

    /// `book` with children: `links` (an edge typed by `kind`) and
    /// `notes` (a markdown body).
    fn linked_book() -> ModelDecl {
        let mut d = book();
        let mut kind = ModelColumn::new("kind", ModelType::Text);
        kind.values = Some(vec!["blocks".into(), "relates_to".into()]);
        d.children.push(ChildDecl {
            name: "links".into(),
            key: vec!["to".into(), "kind".into()],
            columns: vec![ModelColumn::new("to", ModelType::Uri), kind],
            edge: Some(ChildEdge {
                to: "to".into(),
                type_from: Some("kind".into()),
                ref_type: None,
                prefix: Some("book_link:".into()),
            }),
        });
        let mut text = ModelColumn::new("text", ModelType::Text);
        text.markdown = Some(Markdown { pins: false });
        d.children.push(ChildDecl {
            name: "notes".into(),
            key: vec!["id".into()],
            columns: vec![ModelColumn::new("id", ModelType::Text), text],
            edge: None,
        });
        d
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
        let changed = changed.unwrap();
        assert_eq!(
            changed.changed,
            vec!["fields".to_string(), "fields.shelf-score.score".to_string()],
            "a fields change names each field that moved"
        );
        assert_eq!(changed.before["fields.shelf-score.score"], Value::Null);
        assert_eq!(changed.after["fields.shelf-score.score"], json!(9));
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

    /// Verify re-pins a source's pinned files to the latest snapshot, and a
    /// file under a pinned directory becomes a pin of its own; a file it
    /// doesn't cite is refused, naming it.
    #[tokio::test]
    async fn verify_re_pins_what_a_source_cites() {
        let db = Database::in_memory();
        let first = snapshot(&db).await;
        write_decl(
            &db,
            pinned_book(),
            vec![row(
                json!({"id": "b1", "title": "t", "body": "see [[file:src/a.rs]] and [[dir:docs]]"}),
            )],
        )
        .await;
        let later = snapshot(&db).await;
        let verified = db
            .transaction(|tx| verify_pins_tx(tx, B1, None))
            .await
            .unwrap();
        assert_eq!(verified, vec!["src/a.rs".to_string()]);
        let under = db
            .transaction(|tx| verify_pins_tx(tx, B1, Some(&["docs/x.md".to_string()])))
            .await
            .unwrap();
        assert_eq!(under, vec!["docs/x.md".to_string()]);
        let pins: Vec<(String, String, Option<i64>)> = edges(&db, B1)
            .await
            .into_iter()
            .filter(|e| e.1 == "file_pin")
            .collect();
        assert_eq!(
            pins,
            vec![
                (
                    "oxplow:oxplow/file?path=docs/x.md".to_string(),
                    "file_pin".to_string(),
                    Some(later)
                ),
                (A_RS.to_string(), "file_pin".to_string(), Some(later)),
            ]
        );
        assert_ne!(first, later);
        let dir: Vec<(String, String, Option<i64>)> = edges(&db, B1)
            .await
            .into_iter()
            .filter(|e| e.1 == "dir_pin")
            .collect();
        assert_eq!(
            dir[0].2,
            Some(later),
            "verifying all re-pins its directories"
        );
        let err = db
            .transaction(|tx| verify_pins_tx(tx, B1, Some(&["src/zzz.rs".to_string()])))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("src/zzz.rs"), "{err}");
    }

    /// A removed or retyped column rebuilds the table in place: the rows
    /// and every extension's field values stay.
    #[tokio::test]
    async fn a_rebuild_keeps_the_rows_and_their_fields() {
        let db = Database::in_memory();
        write(
            &db,
            "s",
            rating(),
            ModelWrite::Snapshot {
                rows: vec![row(
                    json!({"id": "b1", "title": "Dune", "pages": 412, "fields": {"rating": 4}}),
                )],
                confirm_removals: false,
            },
        )
        .await
        .unwrap();
        let mut changed = book();
        changed.columns.retain(|c| c.name != "pages");
        changed
            .columns
            .push(ModelColumn::new("pages", ModelType::Text));
        db.transaction(move |tx| ensure_model_tx(tx, &changed))
            .await
            .unwrap();
        let live = rows(&db).await;
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].2, "Dune");
        assert!(live[0].3.as_deref().unwrap().contains("rating"), "{live:?}");
    }

    /// With the live stores named, a store not among them is hidden from
    /// the views (its rows kept).
    #[tokio::test]
    async fn a_store_that_isnt_live_is_hidden() {
        let db = Database::in_memory();
        for store in ["a", "b"] {
            write(
                &db,
                store,
                vec![],
                ModelWrite::Snapshot {
                    rows: vec![row(json!({"id": "1", "title": store}))],
                    confirm_removals: false,
                },
            )
            .await
            .unwrap();
        }
        db.transaction(|tx| ensure_model_for_stores_tx(tx, &book(), Some(&["a".to_string()])))
            .await
            .unwrap();
        let titles: Vec<String> = rows(&db).await.into_iter().map(|r| r.2).collect();
        assert_eq!(titles, vec!["a"]);
    }

    /// A rename replaces a tombstone at its new id, keeps one edge when a
    /// source cites both URIs, and is reported (not done) onto a live item.
    #[tokio::test]
    async fn a_rename_handles_tombstones_shared_edges_and_live_targets() {
        let db = Database::in_memory();
        let upsert = |rows: Vec<ModelRow>, delete: Vec<String>, rename: Vec<(String, String)>| {
            ModelWrite::Upsert {
                rows,
                delete,
                rename,
            }
        };
        write(
            &db,
            "s",
            vec![],
            upsert(
                vec![
                    row(json!({"id": "a", "title": "A"})),
                    row(json!({"id": "b", "title": "B"})),
                ],
                vec![],
                vec![],
            ),
        )
        .await
        .unwrap();
        write(&db, "s", vec![], upsert(vec![], vec!["b".into()], vec![]))
            .await
            .unwrap();
        let (a, b) = (
            "oxplow:shelf/book?id=a&store=s",
            "oxplow:shelf/book?id=b&store=s",
        );
        db.transaction(move |tx| {
            for target in [a, b] {
                crate::page_ref_store::upsert_edge_tx(
                    tx,
                    &crate::page_ref_store::PageRefEdge::new(
                        "oxplow:oxplow/file?path=x.md",
                        target,
                        "file_mention",
                    ),
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
        let w = write(
            &db,
            "s",
            vec![],
            upsert(vec![], vec![], vec![("a".into(), "b".into())]),
        )
        .await
        .unwrap();
        assert_eq!(w.changes.len(), 1, "{w:?}");
        assert_eq!(
            rows(&db)
                .await
                .iter()
                .map(|r| r.2.clone())
                .collect::<Vec<_>>(),
            vec!["A"]
        );
        assert_eq!(edges(&db, "oxplow:oxplow/file?path=x.md").await.len(), 1);

        write(
            &db,
            "s",
            vec![],
            upsert(vec![row(json!({"id": "c", "title": "C"}))], vec![], vec![]),
        )
        .await
        .unwrap();
        let onto_live = write(
            &db,
            "s",
            vec![],
            upsert(vec![], vec![], vec![("b".into(), "c".into())]),
        )
        .await
        .unwrap();
        assert!(onto_live.changes.is_empty());
        assert!(onto_live.quarantined[0].message.contains("already an item"));
    }

    /// A directory pin matches the files under it by their exact prefix:
    /// `my_dir` isn't stale for a change to `myXdir/a.md` (`_` is no
    /// wildcard), only for one under `my_dir/`.
    #[tokio::test]
    async fn a_directory_pin_matches_only_files_under_it() {
        let db = Database::in_memory();
        snapshot(&db).await;
        write_decl(
            &db,
            pinned_book(),
            vec![row(
                json!({"id": "b1", "title": "t", "body": "see [[dir:my_dir]]"}),
            )],
        )
        .await;
        let stale = |db: Database| async move {
            db.read(|tx| {
                tx.query_row(
                    "SELECT stale FROM v_ref_pin WHERE source = ?1 AND kind = 'dir'",
                    [B1],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap()
        };
        let file_at = |db: Database, path: &'static str| async move {
            let id = snapshot(&db).await;
            db.transaction(move |tx| {
                tx.execute(
                    "INSERT INTO file_snapshot (stream_id, path, blob_hash, size_bytes, captured_at, storage, snapshot_id)
                     VALUES (1, ?1, 'h', 1, '2026-01-02T00:00:00Z', 'oxplow', ?2)",
                    params![path, id],
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
        };
        file_at(db.clone(), "myXdir/a.md").await;
        assert_eq!(stale(db.clone()).await, 0);
        file_at(db.clone(), "my_dir/a.md").await;
        assert_eq!(stale(db.clone()).await, 1);
    }

    // ---- children ----

    async fn put(db: &Database, rows: Vec<ModelRow>) -> Written {
        write_decl(db, linked_book(), rows).await
    }

    /// The children in `view`, one string per row.
    async fn kids(db: &Database, sql: &'static str) -> Vec<String> {
        db.read(move |tx| {
            let mut st = tx.prepare(sql).map_err(map_sql_err)?;
            let out = st
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(map_sql_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_sql_err)?;
            Ok(out)
        })
        .await
        .unwrap()
    }

    const LINKS: &str = "SELECT \"to\" || '|' || kind FROM v_shelf_book_links ORDER BY 1";
    const NOTES: &str = "SELECT id || '|' || text FROM v_shelf_book_notes ORDER BY 1";
    const A_URI: &str = "oxplow:oxplow/file?path=src/a.rs";
    const B_URI: &str = "oxplow:oxplow/file?path=src/b.rs";

    /// A key named like a child holds its whole set: new ones are inserted,
    /// changed ones updated, missing ones deleted; the same set again
    /// changes nothing; the parent's change names the child.
    #[tokio::test]
    async fn a_child_set_is_restated_by_diff() {
        let db = Database::in_memory();
        let first = put(
            &db,
            vec![row(json!({"id": "b1", "title": "t",
                "links": [{"to": A_URI, "kind": "blocks"}, {"to": B_URI, "kind": "relates_to"}],
                "notes": [{"id": "n1", "text": "one"}, {"id": "n2", "text": "two"}]}))],
        )
        .await;
        assert!(first.changes[0].changed.contains(&"links".to_string()));
        assert!(first.changes[0].changed.contains(&"notes".to_string()));
        assert_eq!(kids(&db, LINKS).await.len(), 2);
        assert_eq!(kids(&db, NOTES).await, vec!["n1|one", "n2|two"]);

        let same = put(
            &db,
            vec![row(json!({"id": "b1", "title": "t",
                "links": [{"to": B_URI, "kind": "relates_to"}, {"to": A_URI, "kind": "blocks"}],
                "notes": [{"id": "n1", "text": "one"}, {"id": "n2", "text": "two"}]}))],
        )
        .await;
        assert_eq!((same.changes.len(), same.unchanged), (0, 1));

        let moved = put(
            &db,
            vec![row(json!({"id": "b1", "title": "t",
                "links": [{"to": A_URI, "kind": "blocks"}],
                "notes": [{"id": "n1", "text": "uno"}, {"id": "n3", "text": "three"}]}))],
        )
        .await;
        assert_eq!(
            moved.changes[0].changed,
            vec!["links".to_string(), "notes".to_string()]
        );
        assert_eq!(kids(&db, LINKS).await, vec![format!("{A_URI}|blocks")]);
        assert_eq!(kids(&db, NOTES).await, vec!["n1|uno", "n3|three"]);
        let parent: Vec<String> = kids(&db, "SELECT DISTINCT parent FROM v_shelf_book_notes").await;
        assert_eq!(parent, vec![B1.to_string()]);

        let emptied = put(
            &db,
            vec![row(json!({"id": "b1", "title": "t", "links": []}))],
        )
        .await;
        assert_eq!(emptied.changes[0].changed, vec!["links".to_string()]);
        assert!(kids(&db, LINKS).await.is_empty());
        assert_eq!(kids(&db, NOTES).await.len(), 2, "notes were not restated");
    }

    /// A row without the key keeps its children, in an upsert and in a
    /// snapshot.
    #[tokio::test]
    async fn a_row_without_the_key_keeps_its_children() {
        let db = Database::in_memory();
        put(
            &db,
            vec![row(json!({"id": "b1", "title": "t",
                "notes": [{"id": "n1", "text": "one"}]}))],
        )
        .await;
        let w = put(&db, vec![row(json!({"id": "b1", "title": "renamed"}))]).await;
        assert_eq!(w.changes[0].changed, vec!["title".to_string()]);
        assert_eq!(kids(&db, NOTES).await, vec!["n1|one"]);
        db.transaction(|tx| {
            let d = linked_book();
            ensure_model_tx(tx, &d)?;
            write_tx(
                tx,
                &d,
                &kinds(),
                "s",
                &[],
                &ModelWrite::Snapshot {
                    rows: vec![row(json!({"id": "b1", "title": "again"}))],
                    confirm_removals: false,
                },
            )
        })
        .await
        .unwrap();
        assert_eq!(kids(&db, NOTES).await, vec!["n1|one"]);
    }

    async fn child_edges(db: &Database) -> Vec<(String, String, Option<i64>)> {
        edges(db, B1).await
    }

    /// The child edge is the parent's: one per child, typed by prefix and
    /// the `values:` value; a changed set restates the slice by diff.
    #[tokio::test]
    async fn child_edges_take_their_type_from_a_column() {
        let db = Database::in_memory();
        put(
            &db,
            vec![row(json!({"id": "b1", "title": "t",
                "links": [{"to": A_URI, "kind": "blocks"}, {"to": B_URI, "kind": "relates_to"}]}))],
        )
        .await;
        let got: Vec<(String, String)> = child_edges(&db)
            .await
            .into_iter()
            .map(|(t, ty, _)| (t, ty))
            .collect();
        assert_eq!(
            got,
            vec![
                (A_URI.to_string(), "book_link:blocks".to_string()),
                (B_URI.to_string(), "book_link:relates_to".to_string()),
            ]
        );
        put(
            &db,
            vec![row(json!({"id": "b1", "title": "t",
                "links": [{"to": B_URI, "kind": "blocks"}]}))],
        )
        .await;
        let got: Vec<(String, String)> = child_edges(&db)
            .await
            .into_iter()
            .map(|(t, ty, _)| (t, ty))
            .collect();
        assert_eq!(
            got,
            vec![(B_URI.to_string(), "book_link:blocks".to_string())]
        );
    }

    /// A child's markdown mentions are `comment_*` edges from the parent,
    /// one slice over every child body; the parent's own body edges stay.
    #[tokio::test]
    async fn child_markdown_projects_comment_mentions_on_the_parent() {
        let db = Database::in_memory();
        put(
            &db,
            vec![row(
                json!({"id": "b1", "title": "t", "body": "see [[file:src/b.rs]]",
                "notes": [{"id": "n1", "text": "look at [[file:src/a.rs]]"},
                          {"id": "n2", "text": "and [[file:src/a.rs]] again"}]}),
            )],
        )
        .await;
        let got: Vec<(String, String)> = child_edges(&db)
            .await
            .into_iter()
            .map(|(t, ty, _)| (t, ty))
            .collect();
        assert_eq!(
            got,
            vec![
                (A_URI.to_string(), "comment_file_ref".to_string()),
                (B_URI.to_string(), "file_mention".to_string()),
            ]
        );
        put(
            &db,
            vec![row(
                json!({"id": "b1", "title": "t", "body": "see [[file:src/b.rs]]",
                "notes": [{"id": "n2", "text": "plain"}]}),
            )],
        )
        .await;
        let got: Vec<String> = child_edges(&db).await.into_iter().map(|e| e.1).collect();
        assert_eq!(got, vec!["file_mention".to_string()]);
    }

    /// A tombstoned parent hides its children and brings them back with
    /// it; the sweep deletes them.
    #[tokio::test]
    async fn a_tombstoned_parent_hides_its_children() {
        let db = Database::in_memory();
        put(
            &db,
            vec![row(json!({"id": "b1", "title": "t",
                "notes": [{"id": "n1", "text": "one"}]}))],
        )
        .await;
        db.transaction(|tx| {
            write_tx(
                tx,
                &linked_book(),
                &kinds(),
                "s",
                &[],
                &ModelWrite::Upsert {
                    rows: vec![],
                    delete: vec!["b1".into()],
                    rename: vec![],
                },
            )
        })
        .await
        .unwrap();
        assert!(kids(&db, NOTES).await.is_empty());
        put(&db, vec![row(json!({"id": "b1", "title": "t"}))]).await;
        assert_eq!(kids(&db, NOTES).await, vec!["n1|one"], "back with it");
        db.transaction(|tx| {
            write_tx(
                tx,
                &linked_book(),
                &kinds(),
                "s",
                &[],
                &ModelWrite::Upsert {
                    rows: vec![],
                    delete: vec!["b1".into()],
                    rename: vec![],
                },
            )?;
            sweep_tombstones_tx(tx, &linked_book(), "9999-01-01")
        })
        .await
        .unwrap();
        let left: i64 = db
            .read(|tx| {
                tx.query_row("SELECT count(*) FROM model__shelf__book__notes", [], |r| {
                    r.get(0)
                })
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(left, 0, "swept with its parent");
    }

    /// A rename moves the children with the item.
    #[tokio::test]
    async fn a_rename_moves_the_children() {
        let db = Database::in_memory();
        put(
            &db,
            vec![row(json!({"id": "b1", "title": "t",
                "notes": [{"id": "n1", "text": "one"}]}))],
        )
        .await;
        db.transaction(|tx| {
            write_tx(
                tx,
                &linked_book(),
                &kinds(),
                "s",
                &[],
                &ModelWrite::Upsert {
                    rows: vec![],
                    delete: vec![],
                    rename: vec![("b1".into(), "b2".into())],
                },
            )
        })
        .await
        .unwrap();
        assert_eq!(kids(&db, NOTES).await, vec!["n1|one"]);
        assert_eq!(
            kids(&db, "SELECT parent FROM v_shelf_book_notes").await,
            vec!["oxplow:shelf/book?id=b2&store=s".to_string()]
        );
    }

    /// A bad child row quarantines the parent row, whole: nothing of it
    /// is written, and a snapshot does not remove it.
    #[tokio::test]
    async fn a_bad_child_quarantines_the_parent_row() {
        let db = Database::in_memory();
        put(
            &db,
            vec![row(json!({"id": "b1", "title": "t",
                "notes": [{"id": "n1", "text": "one"}]}))],
        )
        .await;
        for bad in [
            json!([{"id": "n1", "text": "x"}, {"id": "n1", "text": "dup"}]),
            json!([{"text": "no key"}]),
            json!([{"id": "n1", "nope": 1}]),
            json!([{"id": "n1", "text": 5}]),
            json!([{"id": "n1", "parent": "x"}]),
            json!("not a list"),
        ] {
            let w = put(
                &db,
                vec![row(json!({"id": "b1", "title": "changed", "notes": bad}))],
            )
            .await;
            assert_eq!(w.quarantined.len(), 1, "{bad}");
            assert!(w.changes.is_empty(), "{bad}");
        }
        assert_eq!(kids(&db, NOTES).await, vec!["n1|one"]);
        assert_eq!(rows(&db).await[0].2, "t");
        let bad_kind = put(
            &db,
            vec![row(
                json!({"id": "b1", "links": [{"to": A_URI, "kind": "nope"}]}),
            )],
        )
        .await;
        assert_eq!(bad_kind.quarantined.len(), 1);
    }

    /// A child's names, key and edge are checked against the model.
    #[test]
    fn a_bad_child_declaration_is_refused() {
        let edit = |f: &dyn Fn(&mut ChildDecl)| {
            let mut d = linked_book();
            f(&mut d.children[0]);
            check_decl(&d).unwrap_err().to_string()
        };
        assert!(edit(&|c| c.key = vec!["nope".into()]).contains("key"));
        assert!(edit(&|c| c.name = "all".into()).contains("name"));
        assert!(edit(&|c| c.edge.as_mut().unwrap().to = "kind".into()).contains("uri"));
        assert!(edit(&|c| c.edge.as_mut().unwrap().type_from = None).contains("exactly one"));
        assert!(
            edit(&|c| c.edge.as_mut().unwrap().prefix = Some("file_mention:".into()))
                .contains("prefix")
        );
        assert!(
            edit(&|c| c.edge.as_mut().unwrap().prefix = Some("book_link".into()))
                .contains("prefix")
        );
        let mut d = linked_book();
        d.children[1].name = "title".into();
        assert!(check_decl(&d).is_err(), "a column's name");
        assert!(check_decl(&linked_book()).is_ok());
    }

    /// A field is found by its value through an index.
    #[tokio::test]
    async fn field_values_are_indexed_by_value() {
        let db = Database::in_memory();
        let n: i64 = db
            .read(|tx| {
                tx.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'model_field_value_value'",
                    [],
                    |r| r.get(0),
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    /// A child's fixed `ref_type` may not be a parent column's `edge` or
    /// another child's: their slices would delete each other's edges.
    #[test]
    fn a_child_ref_type_may_not_repeat_another_edge() {
        let mut d = linked_book();
        d.columns.push(ModelColumn::new("cites", ModelType::Uris));
        d.columns.last_mut().unwrap().edge = Some("book.cites".into());
        let child = |name: &str| ChildDecl {
            name: name.into(),
            key: vec!["to".into()],
            columns: vec![ModelColumn::new("to", ModelType::Uri)],
            edge: Some(ChildEdge {
                to: "to".into(),
                type_from: None,
                ref_type: Some("book.cites".into()),
                prefix: None,
            }),
        };
        d.children.push(child("quotes"));
        let e = check_decl(&d).unwrap_err().to_string();
        assert!(e.contains("column `cites`"), "{e}");
        d.columns.pop();
        d.children.push(child("quotes2"));
        let e = check_decl(&d).unwrap_err().to_string();
        assert!(e.contains("child `quotes`"), "{e}");
    }

    /// A changed child shape (a new column, a retyped one) keeps its rows,
    /// as the parent does: an incremental store never refills them.
    #[tokio::test]
    async fn a_changed_child_shape_keeps_its_rows() {
        let db = Database::in_memory();
        put(
            &db,
            vec![row(json!({"id": "b1", "title": "t",
                "notes": [{"id": "n1", "text": "one"}]}))],
        )
        .await;
        db.transaction(|tx| ensure_model_tx(tx, &linked_book()))
            .await
            .unwrap();
        assert_eq!(kids(&db, NOTES).await, vec!["n1|one"], "same shape keeps");
        let mut wider = linked_book();
        wider.children[1]
            .columns
            .push(ModelColumn::new("at", ModelType::Time));
        db.transaction(move |tx| ensure_model_tx(tx, &wider))
            .await
            .unwrap();
        assert_eq!(kids(&db, NOTES).await, vec!["n1|one"], "added column keeps");
        let mut retyped = linked_book();
        retyped.children[1].columns[1].ty = ModelType::Int;
        retyped.children[1].columns[1].markdown = None;
        db.transaction(move |tx| ensure_model_tx(tx, &retyped))
            .await
            .unwrap();
        assert_eq!(kids(&db, NOTES).await, vec!["n1|one"], "retype keeps");
        let mut dropped = linked_book();
        dropped.children[1].columns.remove(1);
        db.transaction(move |tx| ensure_model_tx(tx, &dropped))
            .await
            .unwrap();
        assert_eq!(
            kids(&db, "SELECT id FROM v_shelf_book_notes").await,
            vec!["n1"],
            "removed column keeps"
        );
        let published: i64 = db
            .read(|tx| {
                tx.query_row(
                    "SELECT count(*) FROM dataset WHERE view = 'v_shelf_book_notes' AND owner = 'shelf' AND kind = 'entity'",
                    [],
                    |r| r.get(0),
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(published, 1);
    }

    /// The repair restates a row's lost edges from what is stored —
    /// a parsed body's and a stated one's alike — and leaves a current one
    /// alone.
    #[tokio::test]
    async fn the_repair_restates_lost_edges_from_storage() {
        let db = Database::in_memory();
        write_decl(
            &db,
            book(),
            vec![
                row(json!({"id": "b1", "title": "t", "body": "see [[file:src/a.rs]]"})),
                row(json!({"id": "b2", "title": "t", "body": "x", "refs": [B_RS]})),
            ],
        )
        .await;
        db.transaction(|tx| tx.execute("DELETE FROM page_ref", []).map_err(map_sql_err))
            .await
            .unwrap();
        let n = db
            .transaction(|tx| repair_edges_tx(tx, &book(), &kinds()))
            .await
            .unwrap();
        assert_eq!(n, 2);
        assert_eq!(
            edges(&db, B1).await,
            vec![(A_RS.to_string(), "file_mention".to_string(), None)]
        );
        assert_eq!(
            edges(&db, "oxplow:shelf/book?id=b2&store=s").await,
            vec![(B_RS.to_string(), "file_mention".to_string(), None)],
            "a stated row's edges come from its stored refs, not its body"
        );
        let mut rx = db.subscribe_changes();
        db.transaction(|tx| repair_edges_tx(tx, &book(), &kinds()))
            .await
            .unwrap();
        while let Ok(changed) = rx.try_recv() {
            assert!(
                !changed.contains("page_ref"),
                "a current graph was rewritten"
            );
        }
    }
}
