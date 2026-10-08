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
//!
//! A model declared `materialize: on_change` (P7.B1/B2) is checked like
//! any other — its SELECT against lineage and contract — and then
//! published as a view over its own table, [`materialized_table`] (the
//! contract's columns and types), which the asset runner refills when an
//! input changes. Its table persists across opens; a changed contract
//! recreates it empty, and the first recompute fills it.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use oxplow_domain::DomainError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::database::{map_sql_err, ts_to_string};
use crate::semantic_layer::TempView;
use crate::sql_tokens::{calls, line_col, string_literal};

/// The core models, embedded from `crates/oxplow-db/models/` by the build
/// script.
mod core_files {
    include!(concat!(env!("OUT_DIR"), "/core_models.rs"));
}

/// The owner of the core models.
pub const CORE: &str = "core";

/// One model as its owner declares it (an entry of `models.yaml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct ModelDecl {
    pub name: String,
    pub version: u32,
    pub description: String,
    /// The contract: the view's columns, in order.
    pub columns: Vec<ColumnDecl>,
    /// The columns whose values name one row (P8.B1): declared columns,
    /// part of the contract. A key implies its test (never null, never
    /// repeated), and a materialized model's table takes it as its
    /// primary key.
    #[serde(default)]
    pub key: Vec<String>,
    #[serde(default)]
    pub tests: Vec<TestDecl>,
    /// Earlier versions kept published beside this one after a breaking
    /// change, each as `<view>_v<version>` until its date.
    #[serde(default)]
    pub deprecated: Vec<Deprecated>,
    /// How it is computed: absent, on read (a view); `on_change`, stored
    /// and recomputed when one of its inputs changes; `{ every: 1h }`,
    /// stored and recomputed on that clock.
    #[serde(default)]
    pub materialize: Option<Materialize>,
}

/// A model's freshness policy beyond the default (computed on read):
/// `on_change`, or `{ every: 1h }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(untagged)]
pub enum Materialize {
    /// A named policy: `on_change`.
    Named(MaterializePolicy),
    /// Stored and recomputed, whole, on a clock — `{ every: 1h }` (the
    /// collectors' grammar: minutes `15m` or hours `2h`) — whatever its
    /// inputs do: for SQL that reads the time (`'now'`), whose answer
    /// moves without a write.
    Every { every: String },
    /// Stored, and kept by appending the rows past its watermark —
    /// `{ incremental: <column> }`, a declared INTEGER that only grows —
    /// refilled whole when an input saw a rewrite (P8.B4).
    Incremental { incremental: String },
}

/// The named materialization policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum MaterializePolicy {
    /// Stored in its table and recomputed, whole, when an input changes.
    OnChange,
}

impl Materialize {
    /// `materialize: on_change`.
    pub const ON_CHANGE: Materialize = Materialize::Named(MaterializePolicy::OnChange);

    /// What `model.materialize` records: `on_change`, or `every 1h`.
    pub fn recorded(&self) -> String {
        match self {
            Materialize::Named(MaterializePolicy::OnChange) => "on_change".into(),
            Materialize::Every { every } => format!("every {}", every.trim()),
            Materialize::Incremental { incremental } => {
                format!("incremental {}", incremental.trim())
            }
        }
    }

    /// The watermark column of an incremental model.
    pub fn incremental(&self) -> Option<&str> {
        match self {
            Materialize::Incremental { incremental } => Some(incremental.trim()),
            _ => None,
        }
    }

    /// The clock of an `every:` policy: `15m`, `2h`.
    pub fn every(&self) -> Option<std::time::Duration> {
        match self {
            Materialize::Every { every } => oxplow_domain::time::parse_every(every),
            _ => None,
        }
    }
}

/// The policy `model.materialize` recorded (`on_change`, `every 1h`).
pub fn recorded_materialize(text: &str) -> Option<Materialize> {
    if let Some(every) = text.strip_prefix("every ") {
        return Some(Materialize::Every {
            every: every.to_string(),
        });
    }
    if let Some(column) = text.strip_prefix("incremental ") {
        return Some(Materialize::Incremental {
            incremental: column.to_string(),
        });
    }
    (text == "on_change").then_some(Materialize::ON_CHANGE)
}

/// The table a materialized model's view reads: `m_<view>`.
pub fn materialized_table(view: &str) -> String {
    format!("m_{view}")
}

/// Whether `table` is a materialized model's table — never a `source()`:
/// a model reads another (or itself) through `ref()`.
fn is_materialized_table(table: &str) -> bool {
    table.starts_with("m_v_")
}

/// An earlier version kept after a breaking change: its SQL (in `file`,
/// beside the model's) still keeps the contract that version published,
/// until `until` (`YYYY-MM-DD`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct Deprecated {
    pub version: u32,
    pub file: String,
    pub until: String,
}

/// What a kept earlier version is a version of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Twin {
    /// The model's name.
    pub of: String,
    pub until: String,
}

/// One promised column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct AcceptedValues {
    pub column: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct Relationship {
    pub column: String,
    /// The model the column points into.
    pub to: String,
    pub field: String,
}

/// A model's declaration and its SQL file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ModelSource {
    pub decl: ModelDecl,
    /// Where the SQL came from, for error locations (`models/work_item.sql`).
    pub file: String,
    pub sql: String,
    /// For a kept earlier version (named `<name>_v<version>`): what it's a
    /// version of. Its columns are filled from the contract that version
    /// published when it compiles.
    pub twin: Option<Twin>,
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

/// How the declared contracts differ from the pinned ones — the golden
/// `model_contracts.json`, `{ "<name>": { "<version>": [columns] } }`,
/// every version each core model has published. A model's columns at a
/// pinned version must equal the pin (a changed doc included: a
/// database that recorded the version refuses to open otherwise); a
/// version the golden lacks is new and needs blessing. One line per
/// model that drifts.
pub fn contract_drift(golden: &serde_json::Value, decls: &[ModelDecl]) -> Vec<String> {
    let mut out = Vec::new();
    for d in decls {
        let now = contract_json(d);
        match golden
            .get(&d.name)
            .and_then(|m| m.get(d.version.to_string()))
        {
            Some(pinned) if *pinned == now => {}
            Some(pinned) => {
                let was: Contract = serde_json::from_value(pinned.clone()).unwrap_or_default();
                out.push(format!(
                    "{} v{}'s contract changed ({}); bump its version",
                    d.name,
                    d.version,
                    was.change_to(&d.columns, &d.key)
                ));
            }
            None => out.push(format!(
                "{} v{} isn't pinned yet; bless the golden with OXPLOW_BLESS=1",
                d.name, d.version
            )),
        }
    }
    out
}

/// `golden` with every declared model's current contract pinned at its
/// version — earlier versions kept, so the file is the publication
/// history (what `OXPLOW_BLESS=1` writes).
pub fn pin_contracts(golden: &serde_json::Value, decls: &[ModelDecl]) -> serde_json::Value {
    let mut golden = golden.as_object().cloned().unwrap_or_default();
    for d in decls {
        let versions = golden
            .entry(d.name.clone())
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
        if let Some(map) = versions.as_object_mut() {
            map.insert(d.version.to_string(), contract_json(d));
        }
    }
    serde_json::Value::Object(golden)
}

/// What a model version promises: its columns and its key (the golden's
/// and `model_contract`'s shape).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contract {
    pub columns: Vec<ColumnDecl>,
    #[serde(default)]
    pub key: Vec<String>,
}

impl Contract {
    /// The first difference from this contract to `columns` / `key`, in
    /// words.
    pub fn change_to(&self, columns: &[ColumnDecl], key: &[String]) -> String {
        if self.columns != columns {
            contract_change(&self.columns, columns)
        } else {
            format!("its key became [{}]", key.join(", "))
        }
    }
}

fn contract_json(d: &ModelDecl) -> serde_json::Value {
    serde_json::json!({ "columns": d.columns, "key": d.key })
}

/// The 1-based line of the declaration `name: <model>` in `text`.
fn decl_line(text: &str, model: &str) -> Option<usize> {
    text.lines().enumerate().find_map(|(i, line)| {
        let rest = line.trim_start().trim_start_matches("- ").trim_start();
        let value = rest.strip_prefix("name:")?.trim();
        (value.trim_matches(|c| c == '"' || c == '\'') == model).then_some(i + 1)
    })
}

/// An `every:` clock is one the grammar reads; an incremental model's
/// watermark is its key — one INTEGER column, unique per row (a row id, a
/// seq). Appending takes the rows past the table's highest watermark, so a
/// value two rows shared would let the second slip under it unseen; a
/// unique one can't (tsk777).
fn check_materialize(decl: &ModelDecl, at: &str) -> Result<(), DomainError> {
    match &decl.materialize {
        Some(m @ Materialize::Every { every }) if m.every().is_none() => Err(invalid(format!(
            "{at}: `{}` materializes `every: {every}`, which isn't a duration; use minutes (`15m`) or hours (`2h`)",
            decl.name
        ))),
        Some(Materialize::Incremental { incremental }) => {
            if decl.key.is_empty() {
                return Err(invalid(format!(
                    "{at}: `{}` is incremental but declares no key — the key is what keeps an appended row from repeating; declare `key:`",
                    decl.name
                )));
            }
            let integer = decl.columns.iter().any(|c| {
                c.name == incremental.trim() && c.sql_type.eq_ignore_ascii_case("INTEGER")
            });
            if !integer {
                return Err(invalid(format!(
                    "{at}: `{}`'s watermark `{}` must be an INTEGER column it declares",
                    decl.name,
                    incremental.trim()
                )));
            }
            if decl.key.as_slice() != [incremental.trim()] {
                return Err(invalid(format!(
                    "{at}: `{}`'s watermark `{}` must be its key (`key: [{}]`) — a value unique per row, like a row id or seq; rows sharing a watermark would be skipped",
                    decl.name,
                    incremental.trim(),
                    incremental.trim()
                )));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// An incremental model's SQL keeps a row once it has appeared and never
/// rewrites one — a filter and projection over inner joins of append-only
/// inputs: no grouping, de-duplication, window, cap, set operation, outer
/// join, subquery (`EXISTS`, `IN (SELECT …)`, a scalar one — a row already
/// appended would change when a later row lands on another input, tsk778),
/// randomness or reading of the clock (`file:line` of the first, pointing
/// at `on_change`).
fn check_incremental_sql(decl: &ModelDecl, sql: &str, file: &str) -> Result<(), DomainError> {
    if decl
        .materialize
        .as_ref()
        .and_then(Materialize::incremental)
        .is_none()
    {
        return Ok(());
    }
    const AGGREGATES: [&str; 12] = [
        "count",
        "sum",
        "avg",
        "total",
        "min",
        "max",
        "group_concat",
        "string_agg",
        "json_group_array",
        "json_group_object",
        "random",
        "randomblob",
    ];
    const CLOCK: [&str; 6] = [
        "date",
        "time",
        "datetime",
        "julianday",
        "strftime",
        "unixepoch",
    ];
    let tokens = crate::sql_tokens::tokenize(sql)?;
    let sig: Vec<_> = crate::sql_tokens::significant(&tokens).collect();
    let mut selects = 0;
    for (i, t) in sig.iter().enumerate() {
        let next = sig.get(i + 1);
        let called = next.is_some_and(|n| n.is_punct('('));
        if t.is_word("select") {
            selects += 1;
        }
        let outer = ["left", "right", "full"].iter().any(|w| t.is_word(w))
            && sig[i + 1..].iter().take(2).any(|n| n.is_word("join"));
        let what = if t.is_word("group") && next.is_some_and(|n| n.is_word("by")) {
            Some("GROUP BY".to_string())
        } else if outer {
            Some(format!("{} JOIN", t.text.to_ascii_uppercase()))
        } else if t.is_word("exists") || (t.is_word("select") && selects > 1) {
            Some("a subquery".to_string())
        } else if ["distinct", "over", "limit", "union", "intersect", "except"]
            .iter()
            .any(|w| t.is_word(w))
        {
            Some(t.text.to_ascii_uppercase())
        } else if called && AGGREGATES.iter().chain(CLOCK.iter()).any(|w| t.is_word(w)) {
            Some(format!("{}(", t.text.to_ascii_lowercase()))
        } else if ["current_date", "current_time", "current_timestamp"]
            .iter()
            .any(|w| t.is_word(w))
            || (t.kind == crate::sql_tokens::TokenKind::Str && t.text.eq_ignore_ascii_case("'now'"))
        {
            Some(format!("the clock (`{}`)", t.text))
        } else {
            None
        };
        if let Some(what) = what {
            let (line, _) = line_col(sql, t.start);
            return Err(invalid(format!(
                "{file}:{line}: `{}` is incremental, but its SQL uses {what} — appending rows past a watermark can't keep that right; use `materialize: on_change`",
                decl.name
            )));
        }
    }
    Ok(())
}

/// A key names declared columns, each once.
fn check_key(decl: &ModelDecl, at: &str) -> Result<(), DomainError> {
    let mut seen = BTreeSet::new();
    for k in &decl.key {
        if !decl.columns.iter().any(|c| &c.name == k) {
            return Err(invalid(format!(
                "{at}: `{}`'s key names `{k}`, which isn't one of its columns",
                decl.name
            )));
        }
        if !seen.insert(k) {
            return Err(invalid(format!(
                "{at}: `{}`'s key names `{k}` twice",
                decl.name
            )));
        }
    }
    Ok(())
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
    let declared_in = format!("{dir}/models.yaml");
    for d in &decls {
        let at = match decl_line(yaml, &d.name) {
            Some(line) => format!("{declared_in}:{line}"),
            None => declared_in.clone(),
        };
        check_key(d, &at)?;
        check_materialize(d, &at)?;
    }
    join_sources(decls, dir, &declared_in, file, sql_files)
}

/// Join declarations (from `declared_in`) with their `<name>.sql` files in
/// `dir`: every declared model needs its file and every file its
/// declaration.
pub fn join_sources(
    decls: Vec<ModelDecl>,
    dir: &str,
    declared_in: &str,
    file: impl Fn(&str) -> Option<String>,
    sql_files: impl Fn() -> Vec<String>,
) -> Result<Vec<ModelSource>, DomainError> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(decls.len());
    for decl in decls {
        check_key(&decl, declared_in)?;
        check_materialize(&decl, declared_in)?;
        if !seen.insert(decl.name.clone()) {
            return Err(invalid(format!(
                "{declared_in}: model `{}` is declared twice",
                decl.name
            )));
        }
        let path = format!("{}.sql", decl.name);
        let sql = file(&path).ok_or_else(|| {
            invalid(format!(
                "{declared_in} declares `{}` but {dir}/{path} is missing",
                decl.name
            ))
        })?;
        for d in &decl.deprecated {
            if d.version >= decl.version {
                return Err(invalid(format!(
                    "{declared_in}: `{}` keeps version {} under deprecated, but it is at version {}",
                    decl.name, d.version, decl.version
                )));
            }
            let twin_sql = file(&d.file).ok_or_else(|| {
                invalid(format!(
                    "{declared_in} keeps `{}` v{} in {dir}/{}, which is missing",
                    decl.name, d.version, d.file
                ))
            })?;
            seen.insert(d.file.trim_end_matches(".sql").to_string());
            out.push(ModelSource {
                decl: ModelDecl {
                    name: format!("{}_v{}", decl.name, d.version),
                    version: d.version,
                    description: format!(
                        "{} (version {} of `{}`, kept until {}.)",
                        decl.description.trim_end_matches('.'),
                        d.version,
                        decl.name,
                        d.until
                    ),
                    columns: Vec::new(),
                    key: Vec::new(),
                    tests: Vec::new(),
                    materialize: None,
                    deprecated: Vec::new(),
                },
                file: format!("{dir}/{}", d.file),
                sql: twin_sql,
                twin: Some(Twin {
                    of: decl.name.clone(),
                    until: d.until.clone(),
                }),
            });
        }
        check_incremental_sql(&decl, &sql, &format!("{dir}/{path}"))?;
        out.push(ModelSource {
            decl,
            file: format!("{dir}/{path}"),
            sql,
            twin: None,
        });
    }
    for stem in sql_files() {
        if !seen.contains(&stem) {
            return Err(invalid(format!(
                "{dir}/{stem}.sql has no entry in {declared_in}"
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
                "{}: {what}() takes one quoted name, e.g. {what}('work_item')",
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
        if is_materialized_table(&table) {
            return Err(invalid(format!(
                "{}: source('{table}') is a materialized model's table; read the model through \
                 ref() (a model may not read itself)",
                at(src, call.start)
            )));
        }
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
    // Extensions' models are recompiled after the open
    // (`compile_extensions`); until then the registry lists none of them.
    if has_kind {
        conn.execute(
            "DELETE FROM model WHERE kind = 'sql' AND owner <> ?1",
            [CORE],
        )
        .map_err(map_sql_err)?;
    }
    Ok(())
}

/// Whether a source compiles: a kept earlier version is kept only until
/// its date, and only if its version published.
enum Kept {
    Yes(Box<ModelSource>),
    /// Why not.
    No(String),
}

/// A source ready to compile: a kept earlier version gets the columns its
/// version published (`model_contract`).
fn fill_twin(
    conn: &Connection,
    src: &ModelSource,
    view_of: &dyn Fn(&str) -> String,
    today: &str,
) -> Result<Kept, DomainError> {
    let Some(twin) = &src.twin else {
        return Ok(Kept::Yes(Box::new(src.clone())));
    };
    if twin.until.as_str() < today {
        return Ok(Kept::No(format!(
            "{}: `{}` v{} was kept until {}; remove it from `deprecated`",
            src.file, twin.of, src.decl.version, twin.until
        )));
    }
    let stored = stored_contract(conn, &view_of(&twin.of), src.decl.version)?;
    let contract = match stored {
        Some(c) => c,
        None => {
            return Ok(Kept::No(format!(
                "{}: no published contract for `{}` v{} to keep",
                src.file, twin.of, src.decl.version
            )))
        }
    };
    let mut filled = src.clone();
    filled.decl.columns = contract.columns;
    filled.decl.key = contract.key;
    Ok(Kept::Yes(Box::new(filled)))
}

pub(crate) fn today() -> String {
    oxplow_domain::Timestamp::now()
        .to_text()
        .chars()
        .take(10)
        .collect()
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
    // A core twin past its date (or never published) is simply not kept.
    let today = today();
    let mut kept = Vec::with_capacity(sources.len());
    for s in sources {
        if let Kept::Yes(s) = fill_twin(&tx, s, view_of, &today)? {
            kept.push(*s);
        }
    }
    let sources = kept;
    let models = ordered(resolve(&tx, &sources, view_of)?)?;
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
        publish(&tx, m, owner, &now, Pass::Publish)?;
    }
    tx.commit().map_err(map_sql_err)
}

/// Create one resolved model's view, check its lineage and contract, and
/// record it.
fn publish(
    conn: &Connection,
    m: &Resolved<'_>,
    owner: &str,
    now: &str,
    mode: Pass,
) -> Result<(), DomainError> {
    let decl = &m.source.decl;
    let create = match mode {
        Pass::Publish => "CREATE VIEW",
        Pass::Check => "CREATE TEMP VIEW",
    };
    conn.execute_batch(&format!("{create} {} AS {}", quote(&m.view), m.sql))
        .map_err(|e| invalid(format!("{}: {e}", m.source.file)))?;
    check_lineage(conn, m)?;
    check_contract(conn, m, now, mode == Pass::Publish)?;
    if mode == Pass::Check {
        return Ok(());
    }
    if decl.materialize.is_some() {
        // The SELECT checked out: publish the view over its table instead.
        conn.execute_batch(&format!("DROP VIEW {}", quote(&m.view)))
            .map_err(map_sql_err)?;
        materialize_table(conn, &m.view, &decl.columns, &decl.key)?;
        let columns: Vec<String> = decl.columns.iter().map(|c| quote(&c.name)).collect();
        conn.execute_batch(&format!(
            "CREATE VIEW {} AS SELECT {} FROM {}",
            quote(&m.view),
            columns.join(", "),
            quote(&materialized_table(&m.view))
        ))
        .map_err(map_sql_err)?;
    }
    conn.execute(
        "INSERT INTO model (view, name, owner, version, description, sql, compiled_at, materialize)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            m.view,
            decl.name,
            owner,
            decl.version,
            decl.description,
            m.sql,
            now,
            decl.materialize.as_ref().map(Materialize::recorded)
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

/// The table a materialized model's view reads, with the contract's
/// columns and types: kept when it already has them (its rows are the
/// last recompute's), else recreated empty for the first recompute to
/// fill. Not STRICT: a computed (untyped) contract column stays untyped,
/// as the view reports it.
fn materialize_table(
    conn: &Connection,
    view: &str,
    columns: &[ColumnDecl],
    key: &[String],
) -> Result<(), DomainError> {
    let table = materialized_table(view);
    // Each column with its place in the primary key (0 when not in it).
    let wanted: Vec<(String, String, i64)> = columns
        .iter()
        .map(|c| {
            let pk = key
                .iter()
                .position(|k| k == &c.name)
                .map_or(0, |i| i as i64 + 1);
            (c.name.clone(), c.sql_type.clone(), pk)
        })
        .collect();
    let have: Vec<(String, String, i64)> = conn
        .prepare("SELECT name, type, pk FROM pragma_table_info(?1) ORDER BY cid")
        .and_then(|mut st| {
            st.query_map([&table], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
        })
        .map_err(map_sql_err)?;
    if have == wanted {
        return Ok(());
    }
    let mut defs: Vec<String> = columns
        .iter()
        .map(|c| {
            format!("{} {}", quote(&c.name), c.sql_type)
                .trim()
                .to_string()
        })
        .collect();
    if !key.is_empty() {
        let cols: Vec<String> = key.iter().map(|k| quote(k)).collect();
        defs.push(format!("PRIMARY KEY ({})", cols.join(", ")));
    }
    conn.execute_batch(&format!(
        "DROP TABLE IF EXISTS {t}; CREATE TABLE {t} ({});",
        defs.join(", "),
        t = quote(&table)
    ))
    .map_err(map_sql_err)?;
    // Its last recompute is gone with its rows: the next is its first
    // build, whatever its clock says.
    conn.execute("DELETE FROM asset_state WHERE asset = ?1", [view])
        .map_err(map_sql_err)?;
    Ok(())
}

/// Drop the tables of materialized models that are no longer published
/// (run once every model is registered: after the extensions' pass).
fn drop_orphaned_tables(conn: &Connection) -> Result<(), DomainError> {
    let orphans: Vec<String> = {
        let mut st = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'm\\_v\\_%' ESCAPE '\\'
                   AND substr(name, 3) NOT IN (SELECT view FROM model WHERE materialize IS NOT NULL)",
            )
            .map_err(map_sql_err)?;
        let rows = st
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(map_sql_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sql_err)?;
        rows
    };
    for table in orphans {
        conn.execute_batch(&format!("DROP TABLE {}", quote(&table)))
            .map_err(map_sql_err)?;
    }
    Ok(())
}

/// Everything SQLite reports the view reading is declared — a `ref()` or
/// `source()` — so lineage is complete. The other way round needs no
/// check: each declared input is one of those calls in the SQL itself,
/// substituted into it, so it's read by construction (and SQLite can't
/// always say so: a model made only of CTEs reports its reads under the
/// CTEs' names, never its view's).
fn check_lineage(conn: &Connection, m: &Resolved<'_>) -> Result<(), DomainError> {
    let session = crate::semantic_layer::ReadSession::open(
        conn,
        crate::semantic_layer::Access::Record,
        &m.sql,
    )?;
    conn.prepare(&m.sql)
        .map_err(|e| invalid(format!("{}: {e}", m.source.file)))?;
    let (views, tables) = session.direct_inputs();
    drop(session);
    let undeclared: Vec<String> = views
        .difference(&m.refs)
        .chain(tables.difference(&m.sources))
        .cloned()
        .collect();
    if undeclared.is_empty() {
        return Ok(());
    }
    Err(invalid(format!(
        "{}: reads {} without ref()/source()",
        m.source.file,
        undeclared.join(", ")
    )))
}

/// The view's columns are the declared ones, and a contract recorded for
/// this version is unchanged; with `record`, a first contract is recorded.
fn check_contract(
    conn: &Connection,
    m: &Resolved<'_>,
    now: &str,
    record: bool,
) -> Result<(), DomainError> {
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
    match stored_contract(conn, &m.view, decl.version)? {
        None if !record => Ok(()),
        None => {
            let columns =
                serde_json::to_string(&decl.columns).map_err(|e| invalid(e.to_string()))?;
            let key = serde_json::to_string(&decl.key).map_err(|e| invalid(e.to_string()))?;
            conn.execute(
                "INSERT INTO model_contract (view, version, columns_json, key_json, recorded_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![m.view, decl.version, columns, key, now],
            )
            .map_err(map_sql_err)?;
            Ok(())
        }
        Some(before) if before.columns != decl.columns || before.key != decl.key => {
            Err(invalid(format!(
                "{}: {} v{}'s contract changed ({}); bump its version",
                m.source.file,
                decl.name,
                decl.version,
                before.change_to(&decl.columns, &decl.key)
            )))
        }
        Some(_) => Ok(()),
    }
}

/// The contract `(view, version)` recorded, if it published.
fn stored_contract(
    conn: &Connection,
    view: &str,
    version: u32,
) -> Result<Option<Contract>, DomainError> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT columns_json, key_json FROM model_contract WHERE view = ?1 AND version = ?2",
            params![view, version],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(map_sql_err)?;
    row.map(|(columns, key)| {
        Ok(Contract {
            columns: serde_json::from_str(&columns).map_err(|e| invalid(e.to_string()))?,
            key: serde_json::from_str(&key).map_err(|e| invalid(e.to_string()))?,
        })
    })
    .transpose()
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
pub fn contract_change(before: &[ColumnDecl], after: &[ColumnDecl]) -> String {
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

/// Rows to write into tables, in order: `(table, [row])`, each row a JSON
/// object of column → value.
pub type FixtureRows = Vec<(String, Vec<serde_json::Map<String, serde_json::Value>>)>;

/// Whether an incremental model kept by appending holds what a full refill
/// would (P8.B5): `before` written, the model built whole, `after` written,
/// the rows past its watermark appended — then compared with the SELECT's
/// rows now. `None` when they match; else what differs, or why the
/// append failed. Run it in a rehearsal:
/// it writes the rows and a temp table.
pub fn incremental_matches_full(
    tx: &Connection,
    view: &str,
    decl: &ModelDecl,
    before: &FixtureRows,
    after: &FixtureRows,
) -> Result<Option<String>, DomainError> {
    let Some(watermark) = decl.materialize.as_ref().and_then(Materialize::incremental) else {
        return Ok(None);
    };
    // As compiled; a model that didn't compile has no row (and `check`
    // reported why).
    let sql: Option<String> = tx
        .query_row("SELECT sql FROM model WHERE view = ?1", [view], |r| {
            r.get(0)
        })
        .optional()
        .map_err(map_sql_err)?;
    let Some(sql) = sql else {
        return Ok(None);
    };
    let sql = sql.as_str();
    write_fixture_rows(tx, before)?;
    let mut defs: Vec<String> = decl
        .columns
        .iter()
        .map(|c| {
            format!("{} {}", quote(&c.name), c.sql_type)
                .trim()
                .to_string()
        })
        .collect();
    let key: Vec<String> = decl.key.iter().map(|k| quote(k)).collect();
    defs.push(format!("PRIMARY KEY ({})", key.join(", ")));
    tx.execute_batch(&format!(
        "DROP TABLE IF EXISTS temp.incremental_held;
         CREATE TEMP TABLE incremental_held ({});
         INSERT INTO temp.incremental_held SELECT * FROM ({sql});",
        defs.join(", ")
    ))
    .map_err(map_sql_err)?;
    write_fixture_rows(tx, after)?;
    let mark: Option<i64> = tx
        .query_row(
            &format!(
                "SELECT max({}) FROM temp.incremental_held",
                quote(watermark)
            ),
            [],
            |r| r.get(0),
        )
        .map_err(map_sql_err)?;
    let appended = tx.execute(
        &format!(
            "INSERT INTO temp.incremental_held SELECT * FROM ({sql})
             WHERE ?1 IS NULL OR {} > ?1",
            quote(watermark)
        ),
        [mark],
    );
    if let Err(e) = appended {
        // With the watermark the key (tsk777), only SQL that emits one key
        // twice hits it — and at run time that's a failure every time.
        return Ok(Some(format!(
            "fails to append the rows past `{watermark}`: {e} — its SQL emits one key more than once"
        )));
    }
    let count = |q: String| -> Result<i64, DomainError> {
        tx.query_row(&format!("SELECT count(*) FROM ({q})"), [], |r| r.get(0))
            .map_err(map_sql_err)
    };
    let missed = count(format!(
        "SELECT * FROM ({sql}) EXCEPT SELECT * FROM temp.incremental_held"
    ))?;
    let extra = count(format!(
        "SELECT * FROM temp.incremental_held EXCEPT SELECT * FROM ({sql})"
    ))?;
    Ok((missed > 0 || extra > 0).then(|| {
        format!(
            "after appending the rows past `{watermark}` it misses {missed} row(s) a full refill holds and keeps {extra} it doesn't"
        )
    }))
}

/// Write `rows` into their tables.
fn write_fixture_rows(tx: &Connection, rows: &FixtureRows) -> Result<(), DomainError> {
    for (table, rows) in rows {
        for row in rows {
            let cols: Vec<String> = row.keys().map(|k| quote(k)).collect();
            let marks: Vec<String> = (1..=row.len()).map(|i| format!("?{i}")).collect();
            let values: Vec<rusqlite::types::Value> = row.values().map(sql_value).collect();
            tx.execute(
                &format!(
                    "INSERT INTO {} ({}) VALUES ({})",
                    quote(table),
                    cols.join(", "),
                    marks.join(", ")
                ),
                rusqlite::params_from_iter(values),
            )
            .map_err(|e| invalid(format!("a fixture row for `{table}`: {e}")))?;
        }
    }
    Ok(())
}

fn sql_value(v: &serde_json::Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as V;
    match v {
        serde_json::Value::Null => V::Null,
        serde_json::Value::Bool(b) => V::Integer(i64::from(*b)),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => V::Integer(i),
            None => V::Real(n.as_f64().unwrap_or_default()),
        },
        serde_json::Value::String(s) => V::Text(s.clone()),
        other => V::Text(other.to_string()),
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
        let key = (!src.decl.key.is_empty()).then(|| key_test(&src.decl.key, &view));
        let declared = src.decl.tests.iter().map(|t| {
            test_sql(t, &view, view_of).map_err(|e| invalid(format!("{}: {e}", src.file)))
        });
        for test in key.into_iter().map(Ok).chain(declared) {
            let (name, failing) = test?;
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

/// The test a key implies: its columns are never null, and no two rows
/// share them.
fn key_test(key: &[String], view: &str) -> (String, String) {
    let cols: Vec<String> = key.iter().map(|c| quote(c)).collect();
    let nulls: Vec<String> = cols.iter().map(|c| format!("{c} IS NULL")).collect();
    (
        format!("key({})", key.join(", ")),
        format!(
            "SELECT 1 FROM {v} WHERE {} UNION ALL SELECT 1 FROM {v} GROUP BY {} HAVING count(*) > 1",
            nulls.join(" OR "),
            cols.join(", "),
            v = quote(view)
        ),
    )
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
/// is left out and reported; the others are published. In `tx`, which
/// takes the write lock up front (`Database::transaction`): a pass that
/// read first and wrote after another connection's commit failed
/// (tsk977).
pub fn compile_extensions(
    tx: &rusqlite::Transaction<'_>,
    extensions: &[ExtensionModels],
) -> Result<BTreeMap<String, Vec<String>>, DomainError> {
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
    let errors = pass(tx, extensions, &[], Pass::Publish)?.errors;
    // Every model is registered now (core's compiled at the open): a
    // materialized table no published model reads goes.
    drop_orphaned_tables(tx)?;
    Ok(errors)
}

/// A declared entity a check stands in for before its collector ever
/// ran (P7.C6): an empty view with its declared columns, which a model's
/// `ref()` and a lens can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityStub {
    /// The extension that declares it.
    pub owner: String,
    pub name: String,
    /// `v_<owner>_<entity>`.
    pub view: String,
    pub columns: Vec<(String, crate::collector_store::StoredType)>,
}

/// What a check pass found: each extension's errors, and the temp views
/// it compiled — the entity stand-ins and the models, in the order they
/// were created — for the check's queries to read (`SqlQuery::temp_views`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckedModels {
    pub errors: BTreeMap<String, Vec<String>>,
    pub views: Vec<TempView>,
}

/// Check the extensions' models without publishing anything — what
/// `oxplow plugin check` runs, on a read-only database: the same
/// resolution, lineage and contract checks as [`compile_extensions`] (a
/// changed contract at a published version fails), against temp views.
/// An entity in `stubs` with no view yet gets an empty one first. Run it
/// inside a transaction that's rolled back (`Database::read`).
pub fn check_extensions(
    conn: &Connection,
    extensions: &[ExtensionModels],
    stubs: &[EntityStub],
) -> Result<CheckedModels, DomainError> {
    pass(conn, extensions, stubs, Pass::Check)
}

/// What a pass over the extensions' models does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Create the views, record them and their contracts, run their tests.
    Publish,
    /// Create temp views to prove they compile; record nothing.
    Check,
}

/// One pass over the extensions' models (see [`compile_extensions`]).
fn pass(
    tx: &Connection,
    extensions: &[ExtensionModels],
    stubs: &[EntityStub],
    mode: Pass,
) -> Result<CheckedModels, DomainError> {
    let mut errors: BTreeMap<String, Vec<String>> = extensions
        .iter()
        .map(|e| (e.extension.clone(), Vec::new()))
        .collect();
    // Kept earlier versions take the columns they published, or aren't kept.
    let today = today();
    let mut ready = Vec::with_capacity(extensions.len());
    for e in extensions {
        let view_of = |name: &str| extension_view(&e.extension, name);
        let mut sources = Vec::with_capacity(e.sources.len());
        for s in &e.sources {
            match fill_twin(tx, s, &view_of, &today)? {
                Kept::Yes(s) => sources.push(*s),
                Kept::No(why) => push(&mut errors, &e.extension, why),
            }
        }
        ready.push(ExtensionModels {
            extension: e.extension.clone(),
            sources,
        });
    }
    let extensions = &ready;
    // What a ref() can name: core models, extensions' entities, and the
    // models declared in this pass (never the last pass's, which this one
    // replaces).
    let registered: Vec<(String, String, String)> = {
        let mut st = tx
            .prepare("SELECT owner, name, view FROM model WHERE kind = 'entity' OR owner = ?1")
            .map_err(map_sql_err)?;
        let rows = st
            .query_map([CORE], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
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
    // What a check created, for its queries to recreate.
    let mut views: Vec<TempView> = Vec::new();
    for stub in stubs.iter().filter(|_| mode == Pass::Check) {
        let key = (stub.owner.clone(), stub.name.clone());
        if known.contains_key(&key) {
            continue; // it has synced: its view is real
        }
        // Here, a typed empty table under the view, as a synced entity has
        // (so a model's lineage and contract check as they will); for the
        // check's queries, which only read its columns, an empty SELECT.
        let table = format!(
            "ext__{}__{}",
            stub.owner.replace('-', "_"),
            stub.name.replace('-', "_")
        );
        let typed: Vec<String> = stub
            .columns
            .iter()
            .map(|(name, t)| format!("{} {}", quote(name), t.sql()))
            .collect();
        let names: Vec<String> = stub.columns.iter().map(|(name, _)| quote(name)).collect();
        tx.execute_batch(&format!(
            "CREATE TEMP TABLE {table} ({typed});
             CREATE TEMP VIEW {view} AS SELECT {names} FROM temp.{table};",
            table = quote(&table),
            typed = typed.join(", "),
            view = quote(&stub.view),
            names = names.join(", "),
        ))
        .map_err(map_sql_err)?;
        let empty: Vec<String> = stub
            .columns
            .iter()
            .map(|(name, t)| format!("CAST(NULL AS {}) AS {}", t.sql(), quote(name)))
            .collect();
        views.push(TempView {
            name: stub.view.clone(),
            sql: format!("SELECT {} WHERE 0", empty.join(", ")),
        });
        known.insert(key, stub.view.clone());
    }
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
    let tables = stored_tables(tx)?;
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
    let mut published: Vec<(String, &ModelSource)> = Vec::new();
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
                match publish(tx, &m, &ext, &now, mode) {
                    Ok(()) => {
                        tx.execute_batch("RELEASE extension_model")
                            .map_err(map_sql_err)?;
                        if mode == Pass::Check {
                            views.push(TempView {
                                name: m.view.clone(),
                                sql: m.sql.clone(),
                            });
                        }
                        done.insert(m.view.clone());
                        published.push((ext.clone(), m.source));
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
    // Declared tests, on what published: a failure is the extension's
    // health, and the view stays. (A check proves the SQL; running the
    // tests is `plugin test`'s.)
    for e in extensions.iter().filter(|_| mode == Pass::Publish) {
        let mine: Vec<ModelSource> = published
            .iter()
            .filter(|(ext, _)| ext == &e.extension)
            .map(|(_, src)| (*src).clone())
            .collect();
        if mine.is_empty() {
            continue;
        }
        let view_of = |name: &str| {
            known
                .get(&(e.extension.clone(), name.to_string()))
                .or_else(|| known.get(&(CORE.to_string(), name.to_string())))
                .cloned()
                .unwrap_or_else(|| extension_view(&e.extension, name))
        };
        let file_of: HashMap<String, String> = mine
            .iter()
            .map(|src| {
                (
                    extension_view(&e.extension, &src.decl.name),
                    src.file.clone(),
                )
            })
            .collect();
        for r in run_tests(tx, &mine, &view_of)? {
            let file = file_of.get(&r.view).cloned().unwrap_or_default();
            let detail = r.detail.unwrap_or_default();
            let problem = match r.state {
                "failed" => format!("{file}: test {} failed: {detail}", r.test),
                "error" => format!("{file}: test {} didn't run: {detail}", r.test),
                _ => continue,
            };
            push(&mut errors, &e.extension, problem);
        }
    }
    Ok(CheckedModels { errors, views })
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
                key: vec![],
                tests: vec![],
                deprecated: vec![],
                materialize: None,
            },
            file: format!("models/{name}.sql"),
            sql: sql.into(),
            twin: None,
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

    /// A reworded column doc at the same version is a changed contract
    /// (tsk731: P7 did that to `ai_result`, `claim` and `decision`, and
    /// only a database that had recorded the version noticed).
    #[test]
    fn a_contract_changed_at_a_pinned_version_drifts() {
        let decl: ModelDecl = serde_yaml::from_str(
            "name: a\nversion: 1\ndescription: d\ncolumns:\n  - { name: id, type: INTEGER, doc: \"Row id.\" }\n",
        )
        .unwrap();
        let golden = pin_contracts(&serde_json::json!({}), std::slice::from_ref(&decl));
        assert_eq!(
            contract_drift(&golden, std::slice::from_ref(&decl)),
            Vec::<String>::new()
        );
        let mut reworded = decl.clone();
        reworded.columns[0].doc = "The row's id.".into();
        assert_eq!(
            contract_drift(&golden, std::slice::from_ref(&reworded)),
            vec![
                "a v1's contract changed (column `id`'s doc changed); bump its version".to_string()
            ]
        );
        let mut bumped = reworded;
        bumped.version = 2;
        assert_eq!(
            contract_drift(&golden, std::slice::from_ref(&bumped)),
            vec!["a v2 isn't pinned yet; bless the golden with OXPLOW_BLESS=1".to_string()]
        );
        let pinned = pin_contracts(&golden, std::slice::from_ref(&bumped));
        assert_eq!(
            contract_drift(&pinned, std::slice::from_ref(&bumped)),
            Vec::<String>::new()
        );
        assert!(
            pinned["a"]["1"]["columns"].is_array(),
            "the earlier version stays pinned"
        );
    }

    /// Every core model's contract is pinned at its version in
    /// `fixtures/model_contracts.json`, so a change without a bump is red
    /// here and not only against a database that recorded the version.
    /// `OXPLOW_BLESS=1` pins a new version (earlier ones stay).
    /// P8.B1: the key is part of the contract — declaring one at a pinned
    /// version drifts, and a recorded version refuses a changed key.
    #[test]
    fn a_key_is_part_of_the_contract() {
        let decl: ModelDecl = serde_yaml::from_str(
            "name: a\nversion: 1\ndescription: d\ncolumns:\n  - { name: id, type: INTEGER, doc: \"Row id.\" }\n",
        )
        .unwrap();
        assert!(decl.key.is_empty());
        let golden = pin_contracts(&serde_json::json!({}), std::slice::from_ref(&decl));
        assert_eq!(golden["a"]["1"]["key"], serde_json::json!([]));
        let mut keyed = decl.clone();
        keyed.key = vec!["id".into()];
        assert_eq!(
            contract_drift(&golden, std::slice::from_ref(&keyed)),
            vec!["a v1's contract changed (its key became [id]); bump its version".to_string()]
        );

        let mut conn = fresh();
        let m = source("k", "SELECT id FROM source('streams')", &["id INTEGER"]);
        compile(&mut conn, "t", std::slice::from_ref(&m), &view).unwrap();
        let mut rekeyed = m.clone();
        rekeyed.decl.key = vec!["id".into()];
        let err = compile(&mut conn, "t", &[rekeyed], &view).unwrap_err();
        assert!(
            err.to_string()
                .contains("(its key became [id]); bump its version"),
            "{err}"
        );
    }

    /// P8.B2: `materialize: { every: 2h }` is recorded as its clock; a
    /// clock the grammar can't read is an error at its line.
    #[test]
    fn an_every_clock_is_checked_and_recorded() {
        let yaml = |every: &str| {
            format!("- name: a\n  version: 1\n  description: d\n  materialize: {{ every: {every} }}\n  columns:\n    - {{ name: id, type: INTEGER, doc: \"Row id.\" }}\n")
        };
        let ok = sources_from(
            &yaml("2h"),
            "models",
            |_| Some("SELECT id FROM source('streams')".into()),
            || vec!["a".into()],
        )
        .unwrap();
        assert_eq!(
            ok[0].decl.materialize.as_ref().and_then(Materialize::every),
            Some(std::time::Duration::from_secs(7200))
        );
        let mut conn = fresh();
        compile(&mut conn, "t", &ok, &view).unwrap();
        let recorded: String = conn
            .query_row(
                "SELECT materialize FROM model WHERE view = 'v_t_a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(recorded, "every 2h");

        let err = sources_from(
            &yaml("soon"),
            "models",
            |_| Some("SELECT 1 AS id".into()),
            || vec!["a".into()],
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("models/models.yaml:1: `a` materializes `every: soon`"),
            "{err}"
        );
    }

    /// P8.B4: an incremental model needs a key and an INTEGER watermark
    /// column, and SQL that appending past the watermark keeps right — each
    /// refusal at its line, pointing at `on_change`.
    #[test]
    fn an_incremental_model_is_refused_what_appending_cant_keep_right() {
        let decl = |extra: &str| {
            format!(
                "- name: a\n  version: 1\n  description: d\n  materialize: {{ incremental: id }}\n{extra}  columns:\n    - {{ name: id, type: INTEGER, doc: \"Row id.\" }}\n    - {{ name: v, type: TEXT, doc: \"A value.\" }}\n"
            )
        };
        let load = |yaml: String, sql: &str| {
            let sql = sql.to_string();
            sources_from(
                &yaml,
                "models",
                move |_| Some(sql.clone()),
                || vec!["a".into()],
            )
        };
        let err = |yaml: String, sql: &str| load(yaml, sql).unwrap_err().to_string();
        let plain = "SELECT id, v FROM source('streams')";
        assert!(load(decl("  key: [id]\n"), plain).is_ok());
        assert!(
            err(decl(""), plain)
                .contains("models/models.yaml:1: `a` is incremental but declares no key"),
            "{}",
            err(decl(""), plain)
        );
        let on_v = decl("  key: [v]\n").replace("incremental: id", "incremental: v");
        assert!(
            err(on_v, plain).contains("`a`'s watermark `v` must be an INTEGER column"),
            "watermark type"
        );
        // The watermark is the key: a value two rows share would let the
        // second slip under `>` unseen (tsk777).
        let yaml = decl("  key: [id]\n")
            .replace("incremental: id", "incremental: n")
            .replace(
                "    - { name: v, type: TEXT, doc: \"A value.\" }\n",
                "    - { name: v, type: TEXT, doc: \"A value.\" }\n    - { name: n, type: INTEGER, doc: \"A number.\" }\n",
            );
        let e = err(yaml, "SELECT id, v, 1 AS n FROM source('streams')");
        assert!(
            e.contains("models/models.yaml:1: `a`'s watermark `n` must be its key"),
            "{e}"
        );
        for (sql, what) in [
            (
                "SELECT id, v FROM source('streams') GROUP BY id",
                "GROUP BY",
            ),
            ("SELECT DISTINCT id, v FROM source('streams')", "DISTINCT"),
            (
                "SELECT id, row_number() OVER (ORDER BY id) AS v FROM source('streams')",
                "OVER",
            ),
            ("SELECT id, v FROM source('streams') LIMIT 5", "LIMIT"),
            (
                "SELECT id, v FROM source('streams') UNION SELECT id, v FROM source('threads')",
                "UNION",
            ),
            ("SELECT id, max(v) AS v FROM source('streams')", "max("),
            (
                "SELECT id, v FROM source('streams') WHERE v > datetime('now', '-1 day')",
                "datetime(",
            ),
            // A row already appended can change when a later row lands on
            // another input (tsk778).
            (
                "SELECT s.id, t.v FROM source('streams') s\nLEFT JOIN source('threads') t ON t.id = s.id",
                "LEFT JOIN",
            ),
            (
                "SELECT id, v FROM source('streams') s\nWHERE NOT EXISTS (SELECT 1 FROM source('threads') t WHERE t.id = s.id)",
                "a subquery",
            ),
            (
                "SELECT id, v FROM source('streams') WHERE id IN (SELECT id FROM source('threads'))",
                "a subquery",
            ),
            (
                "SELECT id, json_group_array(v) AS v FROM source('streams')",
                "json_group_array(",
            ),
            ("SELECT id, random() AS v FROM source('streams')", "random("),
        ] {
            let e = err(decl("  key: [id]\n"), sql);
            let line = if sql.contains('\n') { 2 } else { 1 };
            assert!(
                e.contains(&format!("models/a.sql:{line}:"))
                    && e.contains(what)
                    && e.contains("on_change"),
                "{sql}: {e}"
            );
        }
    }

    /// A key names declared columns; another is an error at its line.
    #[test]
    fn a_key_on_an_undeclared_column_is_an_error_at_its_line() {
        let yaml = "- name: a\n  version: 1\n  description: d\n  key: [nope]\n  columns:\n    - { name: id, type: INTEGER, doc: \"Row id.\" }\n";
        let err = sources_from(
            yaml,
            "models",
            |_| Some("SELECT 1 AS id".into()),
            || vec!["a".into()],
        )
        .unwrap_err();
        assert!(
            err.to_string().contains(
                "models/models.yaml:1: `a`'s key names `nope`, which isn't one of its columns"
            ),
            "{err}"
        );
    }

    /// A key implies a test: its columns are never null and never repeat.
    #[test]
    fn a_duplicate_key_fails_the_key_test() {
        let mut conn = fresh();
        conn.execute_batch(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'same', 'main', 'r', 'r', '/a', 't', 't'),
                      (2, 'worktree', 'same', 'f', 'r', 'r', '/b', 't', 't');",
        )
        .unwrap();
        let mut by_id = source(
            "by_id",
            "SELECT id, title FROM source('streams')",
            &["id INTEGER", "title TEXT"],
        );
        by_id.decl.key = vec!["id".into()];
        let mut by_title = source(
            "by_title",
            "SELECT id, title FROM source('streams')",
            &["id INTEGER", "title TEXT"],
        );
        by_title.decl.key = vec!["title".into()];
        compile(&mut conn, "t", &[by_id.clone(), by_title.clone()], &view).unwrap();
        let results = run_tests(&conn, &[by_id, by_title], &view).unwrap();
        let states: Vec<(&str, &str, &str)> = results
            .iter()
            .map(|r| (r.view.as_str(), r.test.as_str(), r.state))
            .collect();
        assert_eq!(
            states,
            vec![
                ("v_t_by_id", "key(id)", "passed"),
                ("v_t_by_title", "key(title)", "failed"),
            ]
        );
    }

    /// A keyed materialized model's table carries the key as its primary
    /// key.
    #[test]
    fn a_keyed_materialized_table_has_its_primary_key() {
        let mut conn = fresh();
        let mut m = source(
            "mk",
            "SELECT id, title FROM source('streams')",
            &["id INTEGER", "title TEXT"],
        );
        m.decl.key = vec!["id".into()];
        m.decl.materialize = Some(Materialize::ON_CHANGE);
        compile(&mut conn, "t", std::slice::from_ref(&m), &view).unwrap();
        let pk: Vec<(String, i64)> = conn
            .prepare("SELECT name, pk FROM pragma_table_info('m_v_t_mk') ORDER BY cid")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(pk, vec![("id".to_string(), 1), ("title".to_string(), 0)]);
    }

    #[test]
    fn model_docs_carry_no_plan_labels() {
        // tsk1044: a model's description is read by people (Catalog,
        // Explore Data) and agents; a plan code ("(P5.E1)") means nothing
        // to either.
        let yaml = include_str!("../models/models.yaml");
        let bytes = yaml.as_bytes();
        let found: Vec<String> = (0..bytes.len().saturating_sub(1))
            .filter(|&i| {
                bytes[i] == b'P'
                    && bytes[i + 1].is_ascii_digit()
                    && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
            })
            .map(|i| yaml[i..].chars().take(8).collect())
            .collect();
        assert_eq!(found, Vec::<String>::new());
    }

    #[test]
    fn every_core_model_contract_is_pinned_at_its_version() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/model_contracts.json");
        let decls: Vec<ModelDecl> = core_sources()
            .unwrap()
            .into_iter()
            .map(|s| s.decl)
            .collect();
        let golden: serde_json::Value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        if std::env::var_os("OXPLOW_BLESS").is_some() {
            let pinned = pin_contracts(&golden, &decls);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_string_pretty(&pinned).unwrap() + "\n").unwrap();
            return;
        }
        let drift = contract_drift(&golden, &decls);
        assert!(drift.is_empty(), "{}", drift.join("\n"));
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

    /// One extensions' pass on `conn`, committed.
    fn publish(
        conn: &mut Connection,
        extensions: &[ExtensionModels],
    ) -> Result<BTreeMap<String, Vec<String>>, DomainError> {
        let tx = conn.transaction().map_err(map_sql_err)?;
        let errors = compile_extensions(&tx, extensions)?;
        tx.commit().map_err(map_sql_err)?;
        Ok(errors)
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

    fn on_change(mut m: ModelSource) -> ModelSource {
        m.decl.materialize = Some(Materialize::ON_CHANGE);
        m
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [name],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// P7.B2: an on-change model publishes as a view over its own table
    /// (the contract's columns), keeps the lineage its SELECT declared,
    /// keeps its rows across a recompile, and starts empty again when its
    /// contract changes.
    #[test]
    fn an_on_change_model_is_a_view_over_its_table_and_keeps_its_lineage() {
        let mut conn = fresh();
        let busy = on_change(source(
            "busy",
            "SELECT ref, title FROM source('work_item') WHERE state = 'blocked'",
            &["ref TEXT", "title TEXT"],
        ));
        compile(&mut conn, "t", std::slice::from_ref(&busy), &view).unwrap();
        let view_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'view' AND name = 'v_t_busy'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(view_sql.contains("m_v_t_busy"), "{view_sql}");
        assert_eq!(
            view_columns(&conn, "v_t_busy").unwrap(),
            vec![
                ("ref".to_string(), "TEXT".to_string()),
                ("title".to_string(), "TEXT".to_string())
            ]
        );
        let (materialize, input): (Option<String>, String) = conn
            .query_row(
                "SELECT m.materialize, i.input FROM model m JOIN model_input i USING (view)
                 WHERE m.view = 'v_t_busy'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (materialize.as_deref(), input.as_str()),
            (Some("on_change"), "work_item")
        );

        conn.execute(
            "INSERT INTO m_v_t_busy VALUES ('work_item:oxplow:a', 'kept')",
            [],
        )
        .unwrap();
        compile(&mut conn, "t", std::slice::from_ref(&busy), &view).unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM v_t_busy", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "a recompile keeps the last recompute's rows");
        // A query reads the model, never its table: the refusal names it.
        let refused = crate::semantic_layer::check_query_on(
            &conn,
            &crate::SqlQuery::new("SELECT * FROM m_v_t_busy"),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("read v_t_busy"), "{refused}");

        let mut changed = on_change(source(
            "busy",
            "SELECT ref FROM source('work_item') WHERE state = 'blocked'",
            &["ref TEXT"],
        ));
        changed.decl.version = 2;
        compile(&mut conn, "t", &[changed], &view).unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM v_t_busy", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "a changed contract starts the table empty");
    }

    /// P7.B2: a model never reads a materialized model's table — its own
    /// or another's — through `source()`.
    #[test]
    fn a_model_may_not_source_a_materialized_table() {
        let mut conn = fresh();
        compile(
            &mut conn,
            "t",
            &[on_change(source(
                "busy",
                "SELECT ref FROM source('work_item')",
                &["ref TEXT"],
            ))],
            &view,
        )
        .unwrap();
        let err = compile(
            &mut conn,
            "t",
            &[on_change(source(
                "busy",
                "SELECT ref FROM source('m_v_t_busy')",
                &["ref TEXT"],
            ))],
            &view,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("may not read itself"), "{err}");
    }

    /// tsk977: the extensions' pass publishes even when another connection
    /// is mid-write as it starts and commits while it waits — a pass that
    /// read first and wrote second failed then (`SQLITE_BUSY_SNAPSHOT`,
    /// which no wait fixes), and a fresh project's boot pass never
    /// published its extensions' models.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_extensions_pass_waits_out_another_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local.sqlite");
        let db = crate::Database::open(&path).unwrap();
        let other = Connection::open(&path).unwrap();
        other
            .busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        other
            .execute_batch(
                "BEGIN IMMEDIATE;
                 INSERT INTO event_content (hash, namespace, bytes, size, created_at)
                   VALUES ('h', 'agent', x'00', 1, 't');",
            )
            .unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            other.execute_batch("COMMIT").unwrap();
        });
        let errors = db
            .compile_extension_models(vec![ext(
                "acme",
                vec![source(
                    "busy",
                    "SELECT ref FROM ref('work_item')",
                    &["ref TEXT"],
                )],
            )])
            .await
            .unwrap();
        release.join().unwrap();
        assert_eq!(errors["acme"], Vec::<String>::new());
    }

    /// P7.B2: checking an extension's on-change model (what `plugin check`
    /// runs, read-only) creates no table; publishing does, and a table no
    /// published model reads goes with the next extensions' pass.
    #[test]
    fn a_check_creates_no_table_and_an_orphaned_one_goes() {
        let mut conn = fresh();
        let models = [ext(
            "acme",
            vec![on_change(source(
                "busy",
                "SELECT ref FROM ref('work_item')",
                &["ref TEXT"],
            ))],
        )];
        let tx = conn.transaction().unwrap();
        let errors = check_extensions(&tx, &models, &[]).unwrap().errors;
        assert_eq!(errors["acme"], Vec::<String>::new());
        assert!(!table_exists(&tx, "m_v_acme_busy"));
        drop(tx);
        let errors = publish(&mut conn, &models).unwrap();
        assert_eq!(errors["acme"], Vec::<String>::new());
        assert!(table_exists(&conn, "m_v_acme_busy"));
        publish(&mut conn, &[]).unwrap();
        assert!(!table_exists(&conn, "m_v_acme_busy"));
    }

    /// P4.9 (tsk494): an extension's model reads core models through
    /// `ref()`, its own by bare name and another extension's as
    /// `ref('<ext>/<name>')`; each publishes `v_<ext>_<name>` in the
    /// registry. A recompile replaces the set.
    #[test]
    fn extension_models_publish_views_over_core_and_each_other() {
        let mut conn = fresh();
        let errors = publish(
            &mut conn,
            &[
                ext(
                    "late-work",
                    vec![
                        source(
                            "late",
                            "SELECT ref, title FROM ref('work_item') WHERE state = 'blocked'",
                            &["ref TEXT", "title TEXT"],
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
                    "v_work_item".into()
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
        publish(
            &mut conn,
            &[ext(
                "late-work",
                vec![source(
                    "late",
                    "SELECT ref, title FROM ref('work_item') WHERE state = 'blocked'",
                    &["ref TEXT", "title TEXT"],
                )],
            )],
        )
        .unwrap();
        assert_eq!(
            published(&conn),
            vec![(
                "v_late_work_late".into(),
                "late-work".into(),
                "v_work_item".into()
            )]
        );
        assert!(conn.prepare("SELECT * FROM v_digest_late_titles").is_err());
    }

    /// A broken model fails alone — with every model reading it — and its
    /// extension's errors say where; a plugin reads only its own tables.
    #[test]
    fn a_broken_extension_model_fails_alone() {
        let mut conn = fresh();
        let errors = publish(
            &mut conn,
            &[
                ext(
                    "raw",
                    vec![source(
                        "items",
                        "SELECT ref FROM\n  source('work_item')",
                        &["ref TEXT"],
                    )],
                ),
                ext(
                    "shaky",
                    vec![
                        source("bad", "SELECT nope FROM ref('work_item')", &["nope"]),
                        source("on_bad", "SELECT * FROM ref('bad')", &["nope"]),
                        source("fine", "SELECT ref FROM ref('work_item')", &["ref TEXT"]),
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
            raw.contains("oxplow/extensions/raw/models/items.sql:2:3"),
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
            vec![("v_shaky_fine".into(), "shaky".into(), "v_work_item".into())]
        );
    }

    /// P4.9 (tsk494): an extension model's declared tests run after it
    /// publishes; a failure is in `model_test` and the extension's errors,
    /// and the view stays published.
    #[test]
    fn an_extension_models_failing_test_is_reported_not_fatal() {
        let mut conn = fresh();
        conn.execute_batch(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 't', 'main', 'refs/heads/main', 'main', '/r', '2026-01-01', '2026-01-01');",
        )
        .unwrap();
        let mut m = source(
            "streams",
            "SELECT id, NULL AS owner FROM ref('stream')",
            &["id INTEGER", "owner"],
        );
        m.decl.tests = vec![
            serde_yaml::from_str("{ not_null: owner }").unwrap(),
            serde_yaml::from_str("{ relationships: { column: id, to: stream, field: id } }")
                .unwrap(),
        ];
        let errors = publish(&mut conn, &[ext("checks", vec![m])]).unwrap();
        let joined = errors["checks"].join("\n");
        assert!(
            joined.contains("not_null(owner) failed: 1 row(s) break it"),
            "{joined}"
        );
        assert!(!joined.contains("relationships"), "{joined}");
        let states: Vec<(String, String)> = {
            let mut st = conn
                .prepare("SELECT test, state FROM model_test WHERE view = 'v_checks_streams' ORDER BY test")
                .unwrap();
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(
            states,
            vec![
                ("not_null(owner)".into(), "failed".into()),
                ("relationships(id -> stream.id)".into(), "passed".into()),
            ]
        );
        conn.query_row("SELECT count(*) FROM v_checks_streams", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    }

    /// P4.9 (tsk494): a breaking change ships a new version with the old
    /// one kept under `deprecated` — its own view, `…_v1`, held to the
    /// contract v1 published, until its date. A twin without a published
    /// contract, or past its date, isn't kept, and the extension hears why.
    #[test]
    fn a_deprecated_version_is_kept_beside_the_new_one_until_its_date() {
        let mut conn = fresh();
        let v1 = source("late", "SELECT ref FROM ref('work_item')", &["ref TEXT"]);
        publish(&mut conn, &[ext("late-work", vec![v1])]).unwrap();

        let twin = |until: &str, sql: &str| {
            let mut v2 = source(
                "late",
                "SELECT ref, title FROM ref('work_item')",
                &["ref TEXT", "title TEXT"],
            );
            v2.decl.version = 2;
            v2.decl.deprecated = vec![Deprecated {
                version: 1,
                file: "late.v1.sql".into(),
                until: until.into(),
            }];
            let files: HashMap<String, String> = [
                ("late.sql".to_string(), v2.sql.clone()),
                ("late.v1.sql".to_string(), sql.to_string()),
            ]
            .into();
            ext(
                "late-work",
                join_sources(
                    vec![v2.decl],
                    "models",
                    "extension.yaml",
                    |f| files.get(f).cloned(),
                    || vec!["late".into(), "late.v1".into()],
                )
                .unwrap(),
            )
        };
        let errors = publish(
            &mut conn,
            &[twin("2999-01-01", "SELECT ref FROM ref('work_item')")],
        )
        .unwrap();
        assert!(errors["late-work"].is_empty(), "{errors:?}");
        assert_eq!(
            view_columns(&conn, "v_late_work_late_v1").unwrap(),
            vec![("ref".to_string(), "TEXT".to_string())]
        );
        assert_eq!(view_columns(&conn, "v_late_work_late").unwrap().len(), 2);

        // Its SQL must still keep v1's promise.
        let errors = publish(
            &mut conn,
            &[twin("2999-01-01", "SELECT title FROM ref('work_item')")],
        )
        .unwrap();
        assert!(
            errors["late-work"].join("\n").contains("late.v1.sql"),
            "{errors:?}"
        );
        // Past its date: gone, and said so.
        let errors = publish(
            &mut conn,
            &[twin("2020-01-01", "SELECT ref FROM ref('work_item')")],
        )
        .unwrap();
        assert!(
            errors["late-work"]
                .join("\n")
                .contains("kept until 2020-01-01"),
            "{errors:?}"
        );
        assert!(conn.prepare("SELECT * FROM v_late_work_late_v1").is_err());
        // A version that never published has no promise to keep.
        let mut never = source("fresh", "SELECT ref FROM ref('work_item')", &["ref TEXT"]);
        never.decl.version = 3;
        never.decl.deprecated = vec![Deprecated {
            version: 2,
            file: "fresh.sql".into(),
            until: "2999-01-01".into(),
        }];
        let never = ModelSource {
            twin: None,
            ..never
        };
        let files: HashMap<String, String> = [("fresh.sql".to_string(), never.sql.clone())].into();
        let sources = join_sources(
            vec![never.decl],
            "models",
            "extension.yaml",
            |f| files.get(f).cloned(),
            || vec!["fresh".into()],
        )
        .unwrap();
        let errors = publish(&mut conn, &[ext("late-work", sources)]).unwrap();
        assert!(
            errors["late-work"]
                .join("\n")
                .contains("no published contract for `fresh` v2"),
            "{errors:?}"
        );
    }

    /// P4.9 (tsk494): the check `oxplow plugin check` runs works on a
    /// read-only database and writes nothing: a good model passes, and a
    /// changed contract at a published version fails naming the column.
    #[tokio::test]
    async fn a_check_on_a_read_only_database_catches_a_breaking_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        let late = |sql: &str, cols: &[&str]| ext("late-work", vec![source("late", sql, cols)]);
        {
            let db = crate::Database::open(&path).unwrap();
            db.compile_extension_models(vec![late(
                "SELECT ref FROM ref('work_item')",
                &["ref TEXT"],
            )])
            .await
            .unwrap();
        }
        let ro = crate::Database::open_read_only(&path).unwrap();
        let errors = ro
            .check_extension_models(
                vec![late(
                    "SELECT ref FROM ref('work_item') WHERE state = 'done'",
                    &["ref TEXT"],
                )],
                vec![],
            )
            .await
            .unwrap()
            .errors;
        assert!(errors["late-work"].is_empty(), "{errors:?}");
        let errors = ro
            .check_extension_models(
                vec![late(
                    "SELECT ref, title FROM ref('work_item')",
                    &["ref TEXT", "title TEXT"],
                )],
                vec![],
            )
            .await
            .unwrap()
            .errors;
        let joined = errors["late-work"].join("\n");
        assert!(joined.contains("column `title` added"), "{joined}");
        assert!(joined.contains("bump its version"), "{joined}");
        // Nothing was written: the published model is the one compiled.
        let sql: String = ro
            .read(|tx| {
                tx.query_row(
                    "SELECT sql FROM model WHERE view = 'v_late_work_late'",
                    [],
                    |r| r.get(0),
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert!(!sql.contains("done"), "{sql}");
    }
}
