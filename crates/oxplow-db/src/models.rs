//! Models (P4.2, `.context/semantic-layer.md` "Models"). A published view
//! is a model: one `SELECT` in a file, reading only through
//! `ref('<model>')` and `source('<table>')`, compiled to a view at every
//! open. The compiler resolves the references (an unknown one is an error
//! at `file:line`), orders the models so each is created after what it
//! reads (a cycle is an error), creates the views, and records them — the
//! `model`, `model_input` and `model_contract` tables the `v_model*` views
//! read. It holds each model to two promises:
//!
//! - **Lineage:** what SQLite reports the view reading (the recording
//!   authorizer, `semantic_layer::ReadSession`) is exactly what it
//!   declares with `ref()` and `source()`.
//! - **Contract:** the view's columns are the ones declared, and the
//!   columns a version once promised don't change under it — a changed
//!   contract needs a new version.
//!
//! Declared tests (`not_null`, `unique`, `accepted_values`,
//! `relationships`, `sql`) run on demand ([`run_tests`]) and record their
//! result in `model_test`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use oxplow_domain::DomainError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::database::{map_sql_err, ts_to_string};
use crate::sql_tokens::{calls, line_col, string_literal};

/// The core models, embedded from `crates/oxplow-db/models/` by the build
/// script.
mod core_files {
    include!(concat!(env!("OUT_DIR"), "/core_models.rs"));
}

/// The owner of the core models.
pub const CORE: &str = "core";

/// One model as its owner declares it (an entry of `models.yaml`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDecl {
    pub name: String,
    pub version: u32,
    pub description: String,
    /// The contract: the view's columns, in order.
    pub columns: Vec<ColumnDecl>,
    #[serde(default)]
    pub tests: Vec<TestDecl>,
}

/// One promised column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnDecl {
    pub name: String,
    /// The declared type SQLite reports for the view's column
    /// (`PRAGMA table_info`); empty for a computed column.
    #[serde(rename = "type", default)]
    pub sql_type: String,
    pub doc: String,
}

/// One declared test, written as a one-key map: `{ not_null: id }`,
/// `{ unique: id }`, `{ accepted_values: { column, values } }`,
/// `{ relationships: { column, to, field } }` or `{ sql: "SELECT …" }` (a
/// query returning the failing rows).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestDecl {
    #[serde(default)]
    pub not_null: Option<String>,
    #[serde(default)]
    pub unique: Option<String>,
    #[serde(default)]
    pub accepted_values: Option<AcceptedValues>,
    #[serde(default)]
    pub relationships: Option<Relationship>,
    #[serde(default)]
    pub sql: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedValues {
    pub column: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relationship {
    pub column: String,
    /// The model the column points into.
    pub to: String,
    pub field: String,
}

/// A model's declaration and its SQL file.
#[derive(Debug, Clone)]
pub struct ModelSource {
    pub decl: ModelDecl,
    /// Where the SQL came from, for error locations (`models/task.sql`).
    pub file: String,
    pub sql: String,
}

fn invalid(msg: impl Into<String>) -> DomainError {
    DomainError::Invalid(msg.into())
}

/// The core models: `models.yaml` joined with each `<name>.sql`. Every
/// declared model needs its file and every file its declaration.
pub fn core_sources() -> Result<Vec<ModelSource>, DomainError> {
    let files: HashMap<&str, &str> = core_files::FILES.iter().copied().collect();
    let yaml = files
        .get("models.yaml")
        .ok_or_else(|| invalid("models/models.yaml is missing"))?;
    sources_from(
        yaml,
        "models",
        |name| files.get(name).map(|s| s.to_string()),
        || {
            files
                .keys()
                .filter_map(|f| f.strip_suffix(".sql"))
                .map(str::to_string)
                .collect()
        },
    )
}

/// Join a `models.yaml` with its SQL files (`file(name)` reads
/// `<name>.sql`; `sql_files()` lists the stems present).
pub fn sources_from(
    yaml: &str,
    dir: &str,
    file: impl Fn(&str) -> Option<String>,
    sql_files: impl Fn() -> Vec<String>,
) -> Result<Vec<ModelSource>, DomainError> {
    let decls: Vec<ModelDecl> =
        serde_yaml::from_str(yaml).map_err(|e| invalid(format!("{dir}/models.yaml: {e}")))?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(decls.len());
    for decl in decls {
        if !seen.insert(decl.name.clone()) {
            return Err(invalid(format!(
                "{dir}/models.yaml: model `{}` is declared twice",
                decl.name
            )));
        }
        let path = format!("{}.sql", decl.name);
        let sql = file(&path).ok_or_else(|| {
            invalid(format!(
                "{dir}/models.yaml declares `{}` but {dir}/{path} is missing",
                decl.name
            ))
        })?;
        out.push(ModelSource {
            decl,
            file: format!("{dir}/{path}"),
            sql,
        });
    }
    for stem in sql_files() {
        if !seen.contains(&stem) {
            return Err(invalid(format!(
                "{dir}/{stem}.sql has no entry in {dir}/models.yaml"
            )));
        }
    }
    Ok(out)
}

/// The view a core model publishes.
pub fn core_view(name: &str) -> String {
    format!("v_{name}")
}

/// A model resolved against the database: its view, its SQL with the
/// references replaced, and what it declared it reads.
#[derive(Debug, Clone)]
struct Resolved<'a> {
    source: &'a ModelSource,
    view: String,
    sql: String,
    /// Views of the models it `ref()`s.
    refs: BTreeSet<String>,
    /// Tables it `source()`s.
    sources: BTreeSet<String>,
    /// Where it first wrote a `ref()`, for a cycle's error.
    first_ref_at: Option<String>,
}

fn at(src: &ModelSource, offset: usize) -> String {
    let (line, col) = line_col(&src.sql, offset);
    format!("{}:{line}:{col}", src.file)
}

/// The one quoted-name argument of a `ref()` / `source()` call.
fn only_name(
    src: &ModelSource,
    call: &crate::sql_tokens::Call,
    what: &str,
) -> Result<String, DomainError> {
    match call.args.as_slice() {
        [arg] => string_literal(arg).ok_or_else(|| {
            invalid(format!(
                "{}: {what}() takes one quoted name, e.g. {what}('task')",
                at(src, call.start)
            ))
        }),
        _ => Err(invalid(format!(
            "{}: {what}() takes one quoted name",
            at(src, call.start)
        ))),
    }
}

/// `"name"` — a quoted identifier.
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn resolve<'a>(
    conn: &Connection,
    sources: &'a [ModelSource],
    view_of: &dyn Fn(&str) -> String,
) -> Result<Vec<Resolved<'a>>, DomainError> {
    let names: BTreeSet<&str> = sources.iter().map(|s| s.decl.name.as_str()).collect();
    let tables = stored_tables(conn)?;
    let ref_view = |target: &str| names.contains(target).then(|| view_of(target));
    let source_ok = |table: &str| {
        if tables.contains(table) {
            Ok(())
        } else {
            Err("names no table".to_string())
        }
    };
    sources
        .iter()
        .map(|src| resolve_one(src, view_of(&src.decl.name), &ref_view, &source_ok))
        .collect()
}

/// Resolve one model's `ref()`s (through `ref_view`: a name → its view,
/// `None` when it names nothing) and `source()`s (`source_ok` says why a
/// table can't be read, if it can't).
fn resolve_one<'a>(
    src: &'a ModelSource,
    view: String,
    ref_view: &dyn Fn(&str) -> Option<String>,
    source_ok: &dyn Fn(&str) -> Result<(), String>,
) -> Result<Resolved<'a>, DomainError> {
    crate::sql_tokens::check_single_read(&src.sql)
        .map_err(|e| invalid(format!("{}: {e}", src.file)))?;
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut refs = BTreeSet::new();
    let mut first_ref_at = None;
    for call in calls(&src.sql, "ref").map_err(|e| invalid(format!("{}: {e}", src.file)))? {
        let target = only_name(src, &call, "ref")?;
        let Some(to) = ref_view(&target) else {
            return Err(invalid(format!(
                "{}: ref('{target}') names no model",
                at(src, call.start)
            )));
        };
        edits.push((call.start, call.end, quote(&to)));
        refs.insert(to);
        first_ref_at.get_or_insert_with(|| at(src, call.start));
    }
    let mut read_tables = BTreeSet::new();
    for call in calls(&src.sql, "source").map_err(|e| invalid(format!("{}: {e}", src.file)))? {
        let table = only_name(src, &call, "source")?;
        source_ok(&table)
            .map_err(|why| invalid(format!("{}: source('{table}') {why}", at(src, call.start))))?;
        edits.push((call.start, call.end, quote(&table)));
        read_tables.insert(table);
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut sql = src.sql.clone();
    for (start, end, with) in edits {
        sql.replace_range(start..end, &with);
    }
    Ok(Resolved {
        source: src,
        view,
        sql: sql.trim().trim_end_matches(';').trim_end().to_string(),
        refs,
        sources: read_tables,
        first_ref_at,
    })
}

fn stored_tables(conn: &Connection) -> Result<BTreeSet<String>, DomainError> {
    let mut st = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .map_err(map_sql_err)?;
    let names = st
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(map_sql_err)?
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .map_err(map_sql_err)?;
    Ok(names)
}

/// The models in an order where each comes after every model it reads.
fn ordered(models: Vec<Resolved<'_>>) -> Result<Vec<Resolved<'_>>, DomainError> {
    let mut pending = models;
    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        let (ready, waiting): (Vec<_>, Vec<_>) = pending
            .into_iter()
            .partition(|m| m.refs.iter().all(|r| done.contains(r)));
        if ready.is_empty() {
            let cycle: Vec<String> = waiting
                .iter()
                .map(|m| {
                    format!(
                        "{} ({})",
                        m.source.decl.name,
                        m.first_ref_at.clone().unwrap_or_default()
                    )
                })
                .collect();
            return Err(invalid(format!(
                "models read each other in a cycle: {}",
                cycle.join(", ")
            )));
        }
        done.extend(ready.iter().map(|m| m.view.clone()));
        out.extend(ready);
        pending = waiting;
    }
    Ok(out)
}

/// The view's columns as SQLite reports them: `(name, declared type)`.
pub fn view_columns(conn: &Connection, view: &str) -> Result<Vec<(String, String)>, DomainError> {
    let mut st = conn
        .prepare(&format!("PRAGMA table_info({})", quote(view)))
        .map_err(map_sql_err)?;
    let cols = st
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
        .map_err(map_sql_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sql_err)?;
    Ok(cols)
}

/// Drop every compiled model's view (before the migrations run). An
/// extension's entity views stay: they are made when its sources sync,
/// not at open. A database from before the registry has none recorded.
pub fn drop_all(conn: &Connection) -> Result<(), DomainError> {
    let has_registry: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'model')",
            [],
            |r| r.get(0),
        )
        .map_err(map_sql_err)?;
    if !has_registry {
        return Ok(());
    }
    // Before V109 every registered model was compiled.
    let has_kind: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info('model') WHERE name = 'kind')",
            [],
            |r| r.get(0),
        )
        .map_err(map_sql_err)?;
    let views: Vec<String> = {
        let mut st = conn
            .prepare(if has_kind {
                "SELECT view FROM model WHERE kind = 'sql'"
            } else {
                "SELECT view FROM model"
            })
            .map_err(map_sql_err)?;
        let rows = st
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(map_sql_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sql_err)?;
        rows
    };
    for view in views {
        conn.execute_batch(&format!("DROP VIEW IF EXISTS {}", quote(&view)))
            .map_err(map_sql_err)?;
    }
    Ok(())
}

/// Compile the core models on `conn` (after migrations, at every open).
pub fn compile_core(conn: &mut Connection) -> Result<(), DomainError> {
    let sources = core_sources()?;
    compile(conn, CORE, &sources, &core_view)
}

/// Compile `owner`'s models in one transaction: drop the views it had,
/// create the new ones in order, check each one's lineage and contract,
/// and record them. Nothing changes if any model fails.
pub fn compile(
    conn: &mut Connection,
    owner: &str,
    sources: &[ModelSource],
    view_of: &dyn Fn(&str) -> String,
) -> Result<(), DomainError> {
    let tx = conn.transaction().map_err(map_sql_err)?;
    let models = ordered(resolve(&tx, sources, view_of)?)?;
    let previous: Vec<String> = {
        let mut st = tx
            .prepare("SELECT view FROM model WHERE owner = ?1 AND kind = 'sql'")
            .map_err(map_sql_err)?;
        let rows = st
            .query_map([owner], |r| r.get::<_, String>(0))
            .map_err(map_sql_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sql_err)?;
        rows
    };
    for view in previous.iter().chain(models.iter().map(|m| &m.view)) {
        tx.execute_batch(&format!("DROP VIEW IF EXISTS {}", quote(view)))
            .map_err(map_sql_err)?;
    }
    tx.execute(
        "DELETE FROM model WHERE owner = ?1 AND kind = 'sql'",
        [owner],
    )
    .map_err(map_sql_err)?;
    let now = ts_to_string(oxplow_domain::Timestamp::now());
    for m in &models {
        publish(&tx, m, owner, &now)?;
    }
    tx.commit().map_err(map_sql_err)
}

/// Create one resolved model's view, check its lineage and contract, and
/// record it.
fn publish(conn: &Connection, m: &Resolved<'_>, owner: &str, now: &str) -> Result<(), DomainError> {
    let decl = &m.source.decl;
    conn.execute_batch(&format!("CREATE VIEW {} AS {}", quote(&m.view), m.sql))
        .map_err(|e| invalid(format!("{}: {e}", m.source.file)))?;
    check_lineage(conn, m)?;
    check_contract(conn, m, now)?;
    conn.execute(
        "INSERT INTO model (view, name, owner, version, description, sql, compiled_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            m.view,
            decl.name,
            owner,
            decl.version,
            decl.description,
            m.sql,
            now
        ],
    )
    .map_err(map_sql_err)?;
    for (input, kind) in m
        .refs
        .iter()
        .map(|r| (r, "ref"))
        .chain(m.sources.iter().map(|s| (s, "source")))
    {
        conn.execute(
            "INSERT INTO model_input (view, input, kind) VALUES (?1, ?2, ?3)",
            params![m.view, input, kind],
        )
        .map_err(map_sql_err)?;
    }
    Ok(())
}

/// What SQLite reports the view reading is what it declared.
fn check_lineage(conn: &Connection, m: &Resolved<'_>) -> Result<(), DomainError> {
    let session =
        crate::semantic_layer::ReadSession::open(conn, crate::semantic_layer::Access::Record)?;
    conn.prepare(&m.sql)
        .map_err(|e| invalid(format!("{}: {e}", m.source.file)))?;
    let (views, tables) = session.direct_inputs();
    drop(session);
    if views == m.refs && tables == m.sources {
        return Ok(());
    }
    let undeclared: Vec<String> = views
        .difference(&m.refs)
        .chain(tables.difference(&m.sources))
        .cloned()
        .collect();
    let unread: Vec<String> = m
        .refs
        .difference(&views)
        .chain(m.sources.difference(&tables))
        .cloned()
        .collect();
    let mut problems = Vec::new();
    if !undeclared.is_empty() {
        problems.push(format!(
            "reads {} without ref()/source()",
            undeclared.join(", ")
        ));
    }
    if !unread.is_empty() {
        problems.push(format!("declares {} but never reads it", unread.join(", ")));
    }
    Err(invalid(format!(
        "{}: {}",
        m.source.file,
        problems.join("; ")
    )))
}

/// The view's columns are the declared ones, and a contract recorded for
/// this version is unchanged.
fn check_contract(conn: &Connection, m: &Resolved<'_>, now: &str) -> Result<(), DomainError> {
    let decl = &m.source.decl;
    let actual = view_columns(conn, &m.view)?;
    let declared: Vec<(String, String)> = decl
        .columns
        .iter()
        .map(|c| (c.name.clone(), c.sql_type.clone()))
        .collect();
    if actual != declared {
        return Err(invalid(format!(
            "{}: the view's columns {} differ from its declared contract {}",
            m.source.file,
            show(&actual),
            show(&declared)
        )));
    }
    let json = serde_json::to_string(&decl.columns).map_err(|e| invalid(e.to_string()))?;
    let stored: Option<String> = conn
        .query_row(
            "SELECT columns_json FROM model_contract WHERE view = ?1 AND version = ?2",
            params![m.view, decl.version],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    match stored {
        None => {
            conn.execute(
                "INSERT INTO model_contract (view, version, columns_json, recorded_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![m.view, decl.version, json, now],
            )
            .map_err(map_sql_err)?;
            Ok(())
        }
        Some(before) if before != json => {
            let before: Vec<ColumnDecl> = serde_json::from_str(&before).unwrap_or_default();
            Err(invalid(format!(
                "{}: {} v{}'s contract changed ({}); bump its version",
                m.source.file,
                decl.name,
                decl.version,
                contract_change(&before, &decl.columns)
            )))
        }
        Some(_) => Ok(()),
    }
}

fn show(cols: &[(String, String)]) -> String {
    let parts: Vec<String> = cols
        .iter()
        .map(|(n, t)| {
            if t.is_empty() {
                n.clone()
            } else {
                format!("{n} {t}")
            }
        })
        .collect();
    format!("[{}]", parts.join(", "))
}

/// The first difference between two contracts, in words.
fn contract_change(before: &[ColumnDecl], after: &[ColumnDecl]) -> String {
    for (i, a) in after.iter().enumerate() {
        match before.get(i) {
            None => return format!("column `{}` added", a.name),
            Some(b) if b.name != a.name => {
                return format!("column `{}` became `{}`", b.name, a.name)
            }
            Some(b) if b.sql_type != a.sql_type => {
                return format!(
                    "column `{}` changed type from `{}` to `{}`",
                    a.name, b.sql_type, a.sql_type
                )
            }
            Some(b) if b.doc != a.doc => return format!("column `{}`'s doc changed", a.name),
            _ => {}
        }
    }
    match before.get(after.len()) {
        Some(b) => format!("column `{}` removed", b.name),
        None => "no column changed".into(),
    }
}

/// One declared test's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResult {
    pub view: String,
    pub test: String,
    /// `passed`, `failed` (rows broke it) or `error` (it didn't run).
    pub state: &'static str,
    pub detail: Option<String>,
}

/// Run the declared tests of `sources` and record each result in
/// `model_test`.
pub fn run_tests(
    conn: &Connection,
    sources: &[ModelSource],
    view_of: &dyn Fn(&str) -> String,
) -> Result<Vec<TestResult>, DomainError> {
    let now = ts_to_string(oxplow_domain::Timestamp::now());
    let mut out = Vec::new();
    for src in sources {
        let view = view_of(&src.decl.name);
        conn.execute("DELETE FROM model_test WHERE view = ?1", [&view])
            .map_err(map_sql_err)?;
        for t in &src.decl.tests {
            let (name, failing) =
                test_sql(t, &view, view_of).map_err(|e| invalid(format!("{}: {e}", src.file)))?;
            let result = conn.query_row(&format!("SELECT count(*) FROM ({failing})"), [], |r| {
                r.get::<_, i64>(0)
            });
            let (state, detail) = match result {
                Ok(0) => ("passed", None),
                Ok(n) => ("failed", Some(format!("{n} row(s) break it"))),
                Err(e) => ("error", Some(e.to_string())),
            };
            conn.execute(
                "INSERT INTO model_test (view, test, state, detail, ran_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![view, name, state, detail, now],
            )
            .map_err(map_sql_err)?;
            out.push(TestResult {
                view: view.clone(),
                test: name,
                state,
                detail,
            });
        }
    }
    Ok(out)
}

/// A test's name and the query returning the rows that break it.
fn test_sql(
    t: &TestDecl,
    view: &str,
    view_of: &dyn Fn(&str) -> String,
) -> Result<(String, String), DomainError> {
    let v = quote(view);
    let lit = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let kinds = [
        t.not_null.is_some(),
        t.unique.is_some(),
        t.accepted_values.is_some(),
        t.relationships.is_some(),
        t.sql.is_some(),
    ];
    if kinds.iter().filter(|b| **b).count() != 1 {
        return Err(invalid(
            "a test is one of not_null, unique, accepted_values, relationships or sql",
        ));
    }
    Ok(if let Some(c) = &t.not_null {
        (
            format!("not_null({c})"),
            format!("SELECT 1 FROM {v} WHERE {} IS NULL", quote(c)),
        )
    } else if let Some(c) = &t.unique {
        (
            format!("unique({c})"),
            format!(
                "SELECT {c} FROM {v} WHERE {c} IS NOT NULL GROUP BY {c} HAVING count(*) > 1",
                c = quote(c)
            ),
        )
    } else if let Some(a) = &t.accepted_values {
        let values: Vec<String> = a.values.iter().map(|x| lit(x)).collect();
        (
            format!("accepted_values({})", a.column),
            format!(
                "SELECT 1 FROM {v} WHERE {c} IS NOT NULL AND {c} NOT IN ({})",
                values.join(", "),
                c = quote(&a.column)
            ),
        )
    } else if let Some(r) = &t.relationships {
        (
            format!("relationships({} -> {}.{})", r.column, r.to, r.field),
            format!(
                "SELECT 1 FROM {v} a WHERE a.{c} IS NOT NULL
                   AND NOT EXISTS (SELECT 1 FROM {to} b WHERE b.{f} = a.{c})",
                c = quote(&r.column),
                to = quote(&view_of(&r.to)),
                f = quote(&r.field)
            ),
        )
    } else {
        let q = t.sql.clone().unwrap_or_default();
        (format!("sql({})", q.trim()), q)
    })
}

/// One extension's models, as its manifest declares them.
#[derive(Debug, Clone)]
pub struct ExtensionModels {
    pub extension: String,
    pub sources: Vec<ModelSource>,
}

/// The view an extension's model (or entity) publishes:
/// `v_<extension>_<name>`, dashes as underscores.
pub fn extension_view(extension: &str, name: &str) -> String {
    format!("v_{}_{}", extension.replace('-', "_"), name)
}

/// Compile every enabled extension's models in one pass (P4.9), after the
/// core models: returns each extension's errors, empty when all of its
/// models compiled. A model that fails — and every model reading it —
/// is left out and reported; the others are published.
pub fn compile_extensions(
    conn: &mut Connection,
    extensions: &[ExtensionModels],
) -> Result<BTreeMap<String, Vec<String>>, DomainError> {
    let tx = conn.transaction().map_err(map_sql_err)?;
    let mut errors: BTreeMap<String, Vec<String>> = extensions
        .iter()
        .map(|e| (e.extension.clone(), Vec::new()))
        .collect();
    // The last pass's views go first: the set is recompiled whole.
    let previous: Vec<String> = {
        let mut st = tx
            .prepare("SELECT view FROM model WHERE kind = 'sql' AND owner <> ?1")
            .map_err(map_sql_err)?;
        let rows = st
            .query_map([CORE], |r| r.get::<_, String>(0))
            .map_err(map_sql_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sql_err)?;
        rows
    };
    for view in &previous {
        tx.execute_batch(&format!("DROP VIEW IF EXISTS {}", quote(view)))
            .map_err(map_sql_err)?;
    }
    tx.execute(
        "DELETE FROM model WHERE kind = 'sql' AND owner <> ?1",
        [CORE],
    )
    .map_err(map_sql_err)?;
    // What a ref() can name: core models, extensions' entities, and the
    // models declared in this pass.
    let registered: Vec<(String, String, String)> = {
        let mut st = tx
            .prepare("SELECT owner, name, view FROM model")
            .map_err(map_sql_err)?;
        let rows = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(map_sql_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sql_err)?;
        rows
    };
    let mut known: HashMap<(String, String), String> = registered
        .iter()
        .map(|(o, n, v)| ((o.clone(), n.clone()), v.clone()))
        .collect();
    let existing: BTreeSet<String> = registered.into_iter().map(|(_, _, v)| v).collect();
    let mut declared: BTreeMap<String, String> = BTreeMap::new();
    for e in extensions {
        for src in &e.sources {
            let view = extension_view(&e.extension, &src.decl.name);
            let owner_of = |v: &str| {
                known
                    .iter()
                    .find(|(_, view)| view.as_str() == v)
                    .map(|((o, _), _)| o.clone())
            };
            if let Some(owner) = owner_of(&view) {
                push(
                    &mut errors,
                    &e.extension,
                    format!(
                        "{}: `{view}` already belongs to `{owner}`; rename the model",
                        src.file
                    ),
                );
                continue;
            }
            if let Some(other) = declared.insert(view.clone(), e.extension.clone()) {
                push(
                    &mut errors,
                    &e.extension,
                    format!(
                        "{}: `{view}` is also `{other}`'s model; rename one",
                        src.file
                    ),
                );
            }
        }
        for src in &e.sources {
            known
                .entry((e.extension.clone(), src.decl.name.clone()))
                .or_insert_with(|| extension_view(&e.extension, &src.decl.name));
        }
    }
    let tables = stored_tables(&tx)?;
    let mut resolved: Vec<(String, Resolved<'_>)> = Vec::new();
    for e in extensions {
        let own_prefix = format!("ext__{}__", e.extension.replace('-', "_"));
        let ref_view = |target: &str| match target.split_once('/') {
            Some((ext, name)) => known.get(&(ext.to_string(), name.to_string())).cloned(),
            None => known
                .get(&(e.extension.clone(), target.to_string()))
                .or_else(|| known.get(&(CORE.to_string(), target.to_string())))
                .cloned(),
        };
        let source_ok = |table: &str| {
            if !table.starts_with(&own_prefix) {
                Err(format!(
                    "is not this extension's: a plugin model reads only its own extension's tables \
                     (`{own_prefix}*`); read oxplow's data through ref()"
                ))
            } else if !tables.contains(table) {
                Err("names no table (has its source synced?)".to_string())
            } else {
                Ok(())
            }
        };
        for src in &e.sources {
            let view = extension_view(&e.extension, &src.decl.name);
            if declared.get(&view) != Some(&e.extension) || existing.contains(&view) {
                continue; // reported above
            }
            match resolve_one(src, view, &ref_view, &source_ok) {
                Ok(m) => resolved.push((e.extension.clone(), m)),
                Err(err) => push(&mut errors, &e.extension, err.to_string()),
            }
        }
    }
    // Publish in dependency order; a model reading one that didn't compile
    // doesn't either.
    let now = ts_to_string(oxplow_domain::Timestamp::now());
    let in_pass: BTreeSet<String> = resolved.iter().map(|(_, m)| m.view.clone()).collect();
    let mut failed: BTreeSet<String> = declared
        .keys()
        .filter(|v| !in_pass.contains(*v))
        .cloned()
        .collect();
    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut pending = resolved;
    while !pending.is_empty() {
        let mut progressed = false;
        let mut waiting = Vec::new();
        for (ext, m) in pending {
            if let Some(bad) = m.refs.iter().find(|r| failed.contains(*r)) {
                push(
                    &mut errors,
                    &ext,
                    format!("{}: reads `{bad}`, which didn't compile", m.source.file),
                );
                failed.insert(m.view.clone());
                progressed = true;
            } else if m
                .refs
                .iter()
                .all(|r| !in_pass.contains(r) || done.contains(r))
            {
                tx.execute_batch("SAVEPOINT extension_model")
                    .map_err(map_sql_err)?;
                match publish(&tx, &m, &ext, &now) {
                    Ok(()) => {
                        tx.execute_batch("RELEASE extension_model")
                            .map_err(map_sql_err)?;
                        done.insert(m.view.clone());
                    }
                    Err(err) => {
                        tx.execute_batch("ROLLBACK TO extension_model; RELEASE extension_model")
                            .map_err(map_sql_err)?;
                        push(&mut errors, &ext, err.to_string());
                        failed.insert(m.view.clone());
                    }
                }
                progressed = true;
            } else {
                waiting.push((ext, m));
            }
        }
        if !progressed {
            for (ext, m) in &waiting {
                push(
                    &mut errors,
                    ext,
                    format!(
                        "{}: models read each other in a cycle ({})",
                        m.source.file,
                        m.first_ref_at.clone().unwrap_or_default()
                    ),
                );
            }
            break;
        }
        pending = waiting;
    }
    tx.commit().map_err(map_sql_err)?;
    Ok(errors)
}

fn push(errors: &mut BTreeMap<String, Vec<String>>, extension: &str, message: String) {
    errors
        .entry(extension.to_string())
        .or_default()
        .push(message);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        crate::database::migrate_and_compile(&mut conn).unwrap();
        conn
    }

    fn source(name: &str, sql: &str, columns: &[&str]) -> ModelSource {
        ModelSource {
            decl: ModelDecl {
                name: name.into(),
                version: 1,
                description: format!("{name}."),
                columns: columns
                    .iter()
                    .map(|c| {
                        let (n, t) = c.split_once(' ').unwrap_or((c, ""));
                        ColumnDecl {
                            name: n.into(),
                            sql_type: t.into(),
                            doc: format!("{n}."),
                        }
                    })
                    .collect(),
                tests: vec![],
            },
            file: format!("models/{name}.sql"),
            sql: sql.into(),
        }
    }

    fn view(name: &str) -> String {
        format!("v_t_{name}")
    }

    /// P4.2 (tsk487): a core model compiles to its view with its documented
    /// columns, and the registry records it and its input.
    #[test]
    fn the_core_models_compile_and_are_recorded() {
        let conn = fresh();
        let names: Vec<String> = view_columns(&conn, "v_stream")
            .unwrap()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            names,
            [
                "id",
                "kind",
                "title",
                "branch",
                "worktree_path",
                "created_at",
                "updated_at",
                "archived_at"
            ]
        );
        let (owner, version): (String, i64) = conn
            .query_row(
                "SELECT owner, version FROM model WHERE view = 'v_stream'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((owner.as_str(), version), ("core", 1));
        let input: (String, String) = conn
            .query_row(
                "SELECT input, kind FROM model_input WHERE view = 'v_stream'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(input, ("streams".to_string(), "source".to_string()));
    }

    #[test]
    fn refs_are_resolved_in_order_and_recorded() {
        let mut conn = fresh();
        let models = [
            source("b", "SELECT id FROM ref('a') WHERE id > 0", &["id INTEGER"]),
            source("a", "SELECT id FROM source('streams')", &["id INTEGER"]),
        ];
        compile(&mut conn, "t", &models, &view).unwrap();
        let inputs: Vec<(String, String)> = conn
            .prepare("SELECT input, kind FROM model_input WHERE view = 'v_t_b'")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(inputs, vec![("v_t_a".to_string(), "ref".to_string())]);
        // A count over a model reads it too (SQLite reports only the base
        // table for that).
        compile(
            &mut conn,
            "t",
            &[
                source("a", "SELECT id FROM source('streams')", &["id INTEGER"]),
                source("c", "SELECT count(*) AS n FROM ref('a')", &["n"]),
            ],
            &view,
        )
        .unwrap();
    }

    #[test]
    fn an_unknown_ref_or_source_and_a_cycle_fail_at_their_line() {
        let mut conn = fresh();
        let err = compile(
            &mut conn,
            "t",
            &[source("a", "SELECT id\nFROM ref('nope')", &["id"])],
            &view,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("models/a.sql:2:6: ref('nope') names no model"),
            "{err}"
        );
        let err = compile(
            &mut conn,
            "t",
            &[source("a", "SELECT id FROM source('no_table')", &["id"])],
            &view,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("models/a.sql:1:16: source('no_table') names no table"),
            "{err}"
        );
        let err = compile(
            &mut conn,
            "t",
            &[
                source("a", "SELECT id FROM ref('b')", &["id"]),
                source("b", "SELECT id FROM\n  ref('a')", &["id"]),
            ],
            &view,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cycle"), "{msg}");
        assert!(msg.contains("models/b.sql:2:3"), "{msg}");
    }

    #[test]
    fn a_read_the_model_didnt_declare_fails() {
        let mut conn = fresh();
        let err = compile(
            &mut conn,
            "t",
            &[source(
                "a",
                "SELECT s.id FROM source('streams') s JOIN threads t ON t.stream_id = s.id",
                &["id INTEGER"],
            )],
            &view,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("models/a.sql: reads threads without ref()/source()"),
            "{err}"
        );
    }

    #[test]
    fn a_contract_changed_at_the_same_version_fails_naming_the_column() {
        let mut conn = fresh();
        let one = |sql: &str, cols: &[&str]| source("a", sql, cols);
        compile(
            &mut conn,
            "t",
            &[one("SELECT id FROM source('streams')", &["id INTEGER"])],
            &view,
        )
        .unwrap();
        // The declared columns must match the view…
        let err = compile(
            &mut conn,
            "t",
            &[one(
                "SELECT id, title FROM source('streams')",
                &["id INTEGER"],
            )],
            &view,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("differ from its declared contract"),
            "{err}"
        );
        // …and a new column at the same version is a changed contract.
        let wider = one(
            "SELECT id, title FROM source('streams')",
            &["id INTEGER", "title TEXT"],
        );
        let err = compile(&mut conn, "t", std::slice::from_ref(&wider), &view).unwrap_err();
        assert!(
            err.to_string()
                .contains("a v1's contract changed (column `title` added); bump its version"),
            "{err}"
        );
        // The failed compile changed nothing: the old view is still there.
        assert_eq!(view_columns(&conn, "v_t_a").unwrap().len(), 1);
        let mut bumped = wider;
        bumped.decl.version = 2;
        compile(&mut conn, "t", &[bumped], &view).unwrap();
    }

    #[test]
    fn declared_tests_record_what_passes_and_what_breaks() {
        let mut conn = fresh();
        conn.execute_batch(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'a', 'main', 'r', 'r', '/a', 't', 't'),
                      (2, 'worktree', 'a', 'f', 'r', 'r', '/b', 't', 't');",
        )
        .unwrap();
        let mut m = source(
            "a",
            "SELECT id, title, kind FROM source('streams')",
            &["id INTEGER", "title TEXT", "kind TEXT"],
        );
        m.decl.tests = serde_yaml::from_str(
            "- { not_null: id }\n- { unique: title }\n- { accepted_values: { column: kind, values: [primary] } }\n- { sql: \"SELECT 1 WHERE 0\" }",
        )
        .unwrap();
        compile(&mut conn, "t", std::slice::from_ref(&m), &view).unwrap();
        let results = run_tests(&conn, &[m], &view).unwrap();
        let states: Vec<(&str, &str)> =
            results.iter().map(|r| (r.test.as_str(), r.state)).collect();
        assert_eq!(
            states,
            vec![
                ("not_null(id)", "passed"),
                ("unique(title)", "failed"),
                ("accepted_values(kind)", "failed"),
                ("sql(SELECT 1 WHERE 0)", "passed"),
            ]
        );
        let recorded: i64 = conn
            .query_row(
                "SELECT count(*) FROM model_test WHERE view = 'v_t_a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(recorded, 4);
    }

    #[test]
    fn every_core_model_passes_its_tests_on_an_empty_database() {
        let conn = fresh();
        let sources = core_sources().unwrap();
        for r in run_tests(&conn, &sources, &core_view).unwrap() {
            assert_eq!(r.state, "passed", "{} {}: {:?}", r.view, r.test, r.detail);
        }
    }

    fn ext(extension: &str, models: Vec<ModelSource>) -> ExtensionModels {
        ExtensionModels {
            extension: extension.into(),
            sources: models
                .into_iter()
                .map(|mut m| {
                    m.file = format!("oxplow/extensions/{extension}/{}", m.file);
                    m
                })
                .collect(),
        }
    }

    fn published(conn: &Connection) -> Vec<(String, String, String)> {
        let mut st = conn
            .prepare(
                "SELECT m.view, m.owner, group_concat(i.input) FROM model m
                 LEFT JOIN model_input i USING (view)
                 WHERE m.owner <> 'core' GROUP BY m.view ORDER BY m.view",
            )
            .unwrap();
        st.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    }

    /// P4.9 (tsk494): an extension's model reads core models through
    /// `ref()`, its own by bare name and another extension's as
    /// `ref('<ext>/<name>')`; each publishes `v_<ext>_<name>` in the
    /// registry. A recompile replaces the set.
    #[test]
    fn extension_models_publish_views_over_core_and_each_other() {
        let mut conn = fresh();
        let errors = compile_extensions(
            &mut conn,
            &[
                ext(
                    "late-work",
                    vec![
                        source(
                            "late",
                            "SELECT id, title FROM ref('task') WHERE status = 'blocked'",
                            &["id INTEGER", "title TEXT"],
                        ),
                        source(
                            "late_count",
                            "SELECT count(*) AS n FROM ref('late')",
                            &["n"],
                        ),
                    ],
                ),
                ext(
                    "digest",
                    vec![source(
                        "late_titles",
                        "SELECT title FROM ref('late-work/late')",
                        &["title TEXT"],
                    )],
                ),
            ],
        )
        .unwrap();
        assert!(errors.values().all(Vec::is_empty), "{errors:?}");
        assert_eq!(
            published(&conn),
            vec![
                (
                    "v_digest_late_titles".into(),
                    "digest".into(),
                    "v_late_work_late".into()
                ),
                (
                    "v_late_work_late".into(),
                    "late-work".into(),
                    "v_task".into()
                ),
                (
                    "v_late_work_late_count".into(),
                    "late-work".into(),
                    "v_late_work_late".into()
                ),
            ]
        );
        conn.query_row("SELECT count(*) FROM v_digest_late_titles", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
        // Recompiled without `digest`: its view goes.
        compile_extensions(
            &mut conn,
            &[ext(
                "late-work",
                vec![source(
                    "late",
                    "SELECT id, title FROM ref('task') WHERE status = 'blocked'",
                    &["id INTEGER", "title TEXT"],
                )],
            )],
        )
        .unwrap();
        assert_eq!(
            published(&conn),
            vec![(
                "v_late_work_late".into(),
                "late-work".into(),
                "v_task".into()
            )]
        );
        assert!(conn.prepare("SELECT * FROM v_digest_late_titles").is_err());
    }

    /// A broken model fails alone — with every model reading it — and its
    /// extension's errors say where; a plugin reads only its own tables.
    #[test]
    fn a_broken_extension_model_fails_alone() {
        let mut conn = fresh();
        let errors = compile_extensions(
            &mut conn,
            &[
                ext(
                    "raw",
                    vec![source(
                        "tasks",
                        "SELECT id FROM\n  source('task')",
                        &["id INTEGER"],
                    )],
                ),
                ext(
                    "shaky",
                    vec![
                        source("bad", "SELECT nope FROM ref('task')", &["nope"]),
                        source("on_bad", "SELECT * FROM ref('bad')", &["nope"]),
                        source("fine", "SELECT id FROM ref('task')", &["id INTEGER"]),
                        source("lost", "SELECT * FROM ref('missing')", &["x"]),
                    ],
                ),
                ext(
                    "loop",
                    vec![
                        source("a", "SELECT * FROM ref('b')", &["x"]),
                        source("b", "SELECT * FROM ref('a')", &["x"]),
                    ],
                ),
            ],
        )
        .unwrap();
        let raw = errors["raw"].join("\n");
        assert!(
            raw.contains("oxplow/extensions/raw/models/tasks.sql:2:3"),
            "{raw}"
        );
        assert!(raw.contains("only its own extension's tables"), "{raw}");
        let shaky = errors["shaky"].join("\n");
        assert!(shaky.contains("models/bad.sql"), "{shaky}");
        assert!(
            shaky.contains("reads `v_shaky_bad`, which didn't compile"),
            "{shaky}"
        );
        assert!(shaky.contains("ref('missing') names no model"), "{shaky}");
        assert!(errors["loop"].join("\n").contains("cycle"), "{errors:?}");
        assert_eq!(
            published(&conn),
            vec![("v_shaky_fine".into(), "shaky".into(), "v_task".into())]
        );
    }
}
