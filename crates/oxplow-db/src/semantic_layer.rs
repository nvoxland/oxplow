//! The semantic layer's read surface: `query_sql` over the published
//! models (the `v_*` views `models.rs` compiles; `v_model` and
//! `v_model_column` are their catalog). The models are the versioned
//! contract; physical tables stay internal. See
//! `.context/semantic-layer.md`.

use std::time::Duration;

use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

use crate::Database;

/// Default row cap for [`SemanticLayer::query_sql`].
pub const DEFAULT_ROW_LIMIT: usize = 500;
/// Hard ceiling a caller can raise the row cap to.
pub const MAX_ROW_LIMIT: usize = 10_000;
/// Wall-clock budget for a single query.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Result of a read-only SQL query. `rows` are positional, aligned with
/// `columns`. `truncated` is true when more rows existed than the cap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SqlQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<SqlCell>>,
    pub truncated: bool,
    /// What the query read — what a caller subscribes to (P4.6).
    pub reads: Reads,
    /// When each model it read last changed since the app started (the
    /// SQL gateway fills it, P4.6); a model unchanged since then is absent.
    pub freshness: Vec<ModelFreshness>,
}

/// When one model last changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelFreshness {
    pub model: String,
    /// RFC 3339.
    pub changed_at: String,
}

/// What a query read, as SQLite's authorizer reported it while preparing
/// it (P4.1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Reads {
    /// Views read, directly or through another view; sorted, distinct.
    pub models: Vec<String>,
    /// Tables the query's own SQL read — not through a view (a temp table
    /// as `temp.<name>`); sorted, distinct. A physical table here is what
    /// the read contract refuses once enforced (P4.3).
    pub tables: Vec<String>,
    /// Metric measures a `metric_grid()` read (P4.5), filled by the SQL
    /// gateway; sorted, distinct.
    pub measures: Vec<String>,
}

/// One read-only query: its SQL, parameters, row cap and time budget.
#[derive(Debug, Clone)]
pub struct SqlQuery {
    pub sql: String,
    pub params: SqlParams,
    /// Row cap: default [`DEFAULT_ROW_LIMIT`], at most [`MAX_ROW_LIMIT`].
    pub limit: Option<usize>,
    pub timeout: Duration,
    /// Read physical tables too (the person's explorer, never an agent or
    /// a lens): the reads are still recorded, none is refused.
    pub raw: bool,
    /// Scope to one stream — what the SQL gateway computes a
    /// `metric_grid()`'s series over (a lens passes its `:stream_id`).
    pub stream: Option<i64>,
    /// Temp tables this query reads, created and filled on its connection
    /// before it runs and dropped after, on every path (the gateway's
    /// `metric_grid()` points, P4.5).
    pub temp: Vec<TempTable>,
    /// Temp views this query reads, created in order on its connection
    /// after its temp tables and dropped before them (a check's overlay:
    /// an extension's models and entities that aren't published, P7.C6).
    pub temp_views: Vec<TempView>,
}

/// A temp view a query reads: its name and `SELECT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TempView {
    pub name: String,
    pub sql: String,
}

/// A temp table a query reads: its name, column names (untyped — SQLite
/// keeps each value's own type) and rows.
#[derive(Debug, Clone)]
pub struct TempTable {
    pub name: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<SqlCell>>,
}

/// A query's parameters.
#[derive(Debug, Clone)]
pub enum SqlParams {
    /// `?1`, `?2`, … in order; exactly as many as the statement has.
    Positional(Vec<SqlCell>),
    /// `:name` → value. Every name the statement uses must be given; names
    /// it doesn't use are ignored, so a host can pass one fixed set.
    Named(Vec<(String, SqlCell)>),
}

impl SqlQuery {
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            params: SqlParams::Positional(Vec::new()),
            limit: None,
            timeout: DEFAULT_TIMEOUT,
            raw: false,
            stream: None,
            temp: Vec::new(),
            temp_views: Vec::new(),
        }
    }

    /// Scope to one stream (see [`SqlQuery::stream`]).
    pub fn stream(mut self, stream: Option<i64>) -> Self {
        self.stream = stream;
        self
    }

    /// Read physical tables too (see [`SqlQuery::raw`]).
    pub fn raw(mut self, raw: bool) -> Self {
        self.raw = raw;
        self
    }

    pub fn positional(mut self, params: Vec<SqlCell>) -> Self {
        self.params = SqlParams::Positional(params);
        self
    }

    pub fn named(mut self, params: Vec<(String, SqlCell)>) -> Self {
        self.params = SqlParams::Named(params);
        self
    }

    pub fn limit(mut self, limit: Option<usize>) -> Self {
        self.limit = limit;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// One SQL value, serialized as a plain JSON scalar (`null`, boolean,
/// number or string) so the TS binding is `null | boolean | number |
/// string` rather than serde_json's tagged-enum shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(untagged)]
pub enum SqlCell {
    Null(()),
    Bool(bool),
    Int(i64),
    Real(f64),
    Text(String),
}

impl SqlCell {
    pub(crate) fn to_sql(&self) -> rusqlite::types::Value {
        use rusqlite::types::Value;
        match self {
            SqlCell::Null(()) => Value::Null,
            SqlCell::Bool(b) => Value::Integer(i64::from(*b)),
            SqlCell::Int(i) => Value::Integer(*i),
            SqlCell::Real(f) => Value::Real(*f),
            SqlCell::Text(t) => Value::Text(t.clone()),
        }
    }
}

impl From<serde_json::Value> for SqlCell {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => SqlCell::Null(()),
            serde_json::Value::Bool(b) => SqlCell::Bool(b),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => SqlCell::Int(i),
                None => SqlCell::Real(n.as_f64().unwrap_or(0.0)),
            },
            serde_json::Value::String(s) => SqlCell::Text(s),
            other => SqlCell::Text(other.to_string()),
        }
    }
}

/// Read access to the semantic layer.
#[derive(Clone)]
pub struct SemanticLayer {
    db: Database,
}

impl SemanticLayer {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Run one read-only `SELECT`/`WITH` statement.
    pub async fn run(&self, query: SqlQuery) -> Result<SqlQueryResult, DomainError> {
        crate::sql_tokens::check_single_read(&query.sql)?;
        self.db
            .call(move |conn| Ok(run_read_only(conn, &query)))
            .await?
    }

    /// [`Self::run`] with positional parameters — the short form.
    pub async fn query_sql(
        &self,
        sql: &str,
        params: Vec<SqlCell>,
        limit: Option<usize>,
    ) -> Result<SqlQueryResult, DomainError> {
        self.run(SqlQuery::new(sql).positional(params).limit(limit))
            .await
    }

    /// Check that `sql` would be accepted — a read-only `SELECT`/`WITH` that
    /// compiles against the current schema — without running it, and say
    /// what it would read. Metric, dimension and extension config use this
    /// to reject a bad SQL fragment up front.
    pub async fn check(&self, sql: &str) -> Result<Reads, DomainError> {
        self.check_with(SqlQuery::new(sql)).await
    }

    /// [`Self::check`] for a whole query: its temp tables exist while it
    /// compiles, and `raw` records rather than refuses.
    pub async fn check_with(&self, query: SqlQuery) -> Result<Reads, DomainError> {
        self.db
            .call(move |conn| Ok(check_query_on(conn, &query)))
            .await?
    }

    /// The name of every view in the database — the schema, read directly
    /// rather than through the query contract.
    /// The latest `limit` events of any of `types`, newest first — what a
    /// review dry-runs an event-triggered collector on (P8.C4).
    pub async fn recent_events(
        &self,
        types: Vec<String>,
        limit: usize,
    ) -> Result<Vec<oxplow_domain::StoredEvent>, DomainError> {
        self.db
            .read(move |conn| {
                let mut out = Vec::new();
                for t in &types {
                    // `LIKE` reads `_` as a wildcard: keep the exact type.
                    out.extend(
                        crate::event_log_store::recent_tx(conn, t, None, None, limit)?
                            .into_iter()
                            .filter(|e| &e.envelope.event_type == t),
                    );
                }
                out.sort_by_key(|e| std::cmp::Reverse(e.seq));
                out.truncate(limit);
                Ok(out)
            })
            .await
    }

    pub async fn view_names(&self) -> Result<std::collections::HashSet<String>, DomainError> {
        self.db.call_mut(|conn| view_names(conn)).await
    }

    /// Check extensions' models compile, without publishing them (works
    /// on a read-only database; see `models::check_extensions`).
    pub async fn check_extension_models(
        &self,
        extensions: Vec<crate::models::ExtensionModels>,
        stubs: Vec<crate::models::EntityStub>,
    ) -> Result<crate::models::CheckedModels, DomainError> {
        self.db.check_extension_models(extensions, stubs).await
    }
}

fn invalid(e: rusqlite::Error) -> DomainError {
    DomainError::Invalid(format!("query_sql: {e}"))
}

/// A query that didn't prepare. An unknown table names the published model
/// it most likely meant (tsk1039: `v_tree_facts` → `v_tree_fact`).
fn prepare_failed(conn: &rusqlite::Connection, e: rusqlite::Error) -> DomainError {
    let msg = e.to_string();
    let missing = msg
        .strip_prefix("no such table: ")
        .and_then(|rest| rest.split_whitespace().next())
        .map(str::to_string);
    let nearest = missing.as_deref().and_then(|name| {
        let views = view_names(conn).ok()?;
        let most = (name.len() / 4).max(2);
        views
            .iter()
            .filter(|v| v.starts_with("v_"))
            .map(|v| (edit_distance(name, v), v))
            .filter(|(d, _)| *d <= most)
            .min()
            .map(|(_, v)| v.clone())
    });
    match nearest {
        Some(v) => DomainError::Invalid(format!("query_sql: {msg}; did you mean `{v}`?")),
        None => invalid(e),
    }
}

/// Levenshtein distance between `a` and `b`, by characters.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

fn read_only_only() -> DomainError {
    DomainError::Invalid("query_sql accepts read-only statements only".into())
}

/// Bind `params` to the prepared statement, by the statement's own
/// parameter list: a positional count must match, and a `:name` the
/// statement uses must be given.
fn bind(stmt: &mut rusqlite::Statement<'_>, params: &SqlParams) -> Result<(), DomainError> {
    let count = stmt.parameter_count();
    match params {
        SqlParams::Positional(vals) => {
            if vals.len() != count {
                return Err(DomainError::Invalid(format!(
                    "query_sql: the statement takes {count} parameter(s), {} given",
                    vals.len()
                )));
            }
            for (i, v) in vals.iter().enumerate() {
                stmt.raw_bind_parameter(i + 1, v.to_sql())
                    .map_err(invalid)?;
            }
        }
        SqlParams::Named(vals) => {
            for i in 1..=count {
                let Some(name) = stmt.parameter_name(i).map(str::to_string) else {
                    return Err(DomainError::Invalid(format!(
                        "query_sql: parameter {i} is positional; this query binds by name"
                    )));
                };
                let bare = name.trim_start_matches([':', '@', '$']);
                let Some((_, v)) = vals.iter().find(|(n, _)| n == bare) else {
                    return Err(DomainError::Invalid(format!(
                        "query_sql: no value for {name}"
                    )));
                };
                stmt.raw_bind_parameter(i, v.to_sql()).map_err(invalid)?;
            }
        }
    }
    Ok(())
}

/// Every view in the database, main and temp, named bare — as SQLite
/// reports it wherever it appears (as an accessor too).
fn view_names(
    conn: &rusqlite::Connection,
) -> Result<std::collections::HashSet<String>, DomainError> {
    Ok(schema_names(conn, "view")?
        .into_iter()
        .map(|n| n.strip_prefix("temp.").map_or(n.clone(), str::to_string))
        .collect())
}

/// Table → the models whose view reads it (`model_input`, sources; a
/// materialized model's own table → that model), the table's own model
/// (`v_<table>`) first, then by name (tsk922). Empty before the registry
/// exists.
fn source_readers(
    conn: &rusqlite::Connection,
) -> Result<std::collections::HashMap<String, Vec<String>>, DomainError> {
    let mut out: std::collections::HashMap<String, Vec<String>> = Default::default();
    let has: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'model_input')",
            [],
            |r| r.get(0),
        )
        .map_err(crate::database::map_sql_err)?;
    if !has {
        return Ok(out);
    }
    let materialized: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info('model') WHERE name = 'materialize')",
            [],
            |r| r.get(0),
        )
        .map_err(crate::database::map_sql_err)?;
    let mut st = conn
        .prepare(if materialized {
            "SELECT input, view FROM model_input WHERE kind = 'source'
             UNION ALL
             SELECT 'm_' || view, view FROM model WHERE materialize IS NOT NULL
             ORDER BY 2"
        } else {
            "SELECT input, view FROM model_input WHERE kind = 'source' ORDER BY view"
        })
        .map_err(crate::database::map_sql_err)?;
    let rows = st
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(crate::database::map_sql_err)?;
    for row in rows {
        let (table, view) = row.map_err(crate::database::map_sql_err)?;
        out.entry(table).or_default().push(view);
    }
    for (table, views) in out.iter_mut() {
        let own = format!("v_{table}");
        views.sort_by_key(|v| *v != own);
    }
    Ok(out)
}

/// Every name of `kind` (`table` or `view`) in main and temp; a temp
/// one as `temp.<name>`.
fn schema_names(
    conn: &rusqlite::Connection,
    kind: &str,
) -> Result<std::collections::HashSet<String>, DomainError> {
    let mut st = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = ?1
             UNION ALL SELECT 'temp.' || name FROM sqlite_temp_master WHERE type = ?1",
        )
        .map_err(crate::database::map_sql_err)?;
    let names = st
        .query_map([kind], |r| r.get::<_, String>(0))
        .map_err(crate::database::map_sql_err)?
        .collect::<rusqlite::Result<std::collections::HashSet<_>>>()
        .map_err(crate::database::map_sql_err)?;
    Ok(names)
}

/// What the authorizer saw while a statement was prepared.
#[derive(Default)]
struct Seen {
    /// `(table, column, reached through a view?)`, as reported.
    reads: Vec<(String, String, Option<String>)>,
    /// What the authorizer refused, in words.
    denied: Vec<Denied>,
}

enum Denied {
    /// A physical table read by the query's own SQL.
    Table(String),
    /// Anything but a read (`ATTACH`, a pragma, …).
    Action(String),
}

/// What the authorizer does with what it sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// The read contract (P4.3): the query's own SQL reads only models
    /// (views) and temp tables; a view reads what it reads.
    Enforce,
    /// Record every read, refuse none (the explorer's raw mode; a model's
    /// compile, which reads tables through `source()`).
    Record,
}

/// One read on a pooled connection: the authorizer installed (recording
/// every read) and, once asked, `PRAGMA query_only`. Dropping it — on
/// every path, an error or a panic included — clears both, so the next
/// user of the connection gets it back as it was.
pub(crate) struct ReadSession<'c> {
    conn: &'c rusqlite::Connection,
    seen: std::sync::Arc<std::sync::Mutex<Seen>>,
    /// Every view in the database, to tell a model read from a table read.
    views: std::collections::HashSet<String>,
    /// Every stored table (and the schema tables), to tell a table read
    /// from a table-valued function's (`json_each`).
    tables: std::collections::HashSet<String>,
    /// Table → the models that read it, to point a refused read at them.
    readers: std::collections::HashMap<String, Vec<String>>,
    /// The statement's own CTEs (lowercased): a read in one of them is
    /// the statement's; in any other view or CTE, a view's.
    own_ctes: std::collections::BTreeSet<String>,
}

impl<'c> ReadSession<'c> {
    /// Watch the reads of `sql` (about to be prepared on `conn`).
    pub(crate) fn open(
        conn: &'c rusqlite::Connection,
        access: Access,
        sql: &str,
    ) -> Result<Self, DomainError> {
        let views = view_names(conn)?;
        // A read's accessor is the view or CTE it happened in. Any accessor
        // but one of the statement's own CTEs is inside a view — a view's
        // own CTEs included — so a model built on CTEs reads, while the
        // statement's own reads stay checked. (A statement CTE sharing a
        // name with a view's refuses more, never less.)
        let own_ctes = crate::sql_tokens::cte_names(sql)?;
        let mut tables = schema_names(conn, "table")?;
        tables.extend(
            [
                "sqlite_master",
                "sqlite_schema",
                "temp.sqlite_master",
                "temp.sqlite_schema",
            ]
            .map(String::from),
        );
        let readers = source_readers(conn)?;
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Seen::default()));
        let sink = seen.clone();
        let tables_c = tables.clone();
        let own_c = own_ctes.clone();
        // Tables read inside a view so far (SQLite reports a view's own
        // reads before a `count(*)`'s empty-column read of its base table).
        let mut under: std::collections::HashSet<String> = std::collections::HashSet::new();
        conn.authorizer(Some(
            move |ctx: rusqlite::hooks::AuthContext<'_>| -> rusqlite::hooks::Authorization {
                use rusqlite::hooks::{AuthAction as A, Authorization};
                let mut seen = sink.lock().unwrap_or_else(|e| e.into_inner());
                match ctx.action {
                    A::Read {
                        table_name,
                        column_name,
                    } => {
                        let table = match ctx.database_name {
                            Some("temp") => format!("temp.{table_name}"),
                            _ => table_name.to_string(),
                        };
                        let in_view = ctx
                            .accessor
                            .is_some_and(|a| !own_c.contains(&a.to_lowercase()));
                        if in_view {
                            under.insert(table.clone());
                        }
                        let own_table_read = !in_view
                            && tables_c.contains(&table)
                            && !table.starts_with("temp.")
                            && !(column_name.is_empty() && under.contains(&table));
                        seen.reads.push((
                            table.clone(),
                            column_name.to_string(),
                            ctx.accessor.map(str::to_string),
                        ));
                        if access == Access::Enforce && own_table_read {
                            seen.denied.push(Denied::Table(table));
                            return Authorization::Deny;
                        }
                        Authorization::Allow
                    }
                    A::Select | A::Function { .. } | A::Recursive => Authorization::Allow,
                    A::Pragma { pragma_name, .. }
                        if pragma_name.eq_ignore_ascii_case("query_only") =>
                    {
                        Authorization::Allow
                    }
                    other if access == Access::Enforce => {
                        seen.denied.push(Denied::Action(format!("{other:?}")));
                        Authorization::Deny
                    }
                    _ => Authorization::Allow,
                }
            },
        ))
        .map_err(crate::database::map_sql_err)?;
        Ok(Self {
            conn,
            seen,
            views,
            tables,
            readers,
            own_ctes,
        })
    }

    /// Whether a read with this accessor happened inside a view: the
    /// accessor is a view, or a CTE the statement didn't define (a view's
    /// own CTE).
    fn nested(&self, accessor: &Option<String>) -> bool {
        accessor
            .as_ref()
            .is_some_and(|a| !self.own_ctes.contains(&a.to_lowercase()))
    }

    /// Why a statement didn't prepare: the authorizer's refusal, or the
    /// error itself — read once the session has ended, so naming the model
    /// it most likely meant can read the schema (tsk1039).
    fn failed(self, conn: &rusqlite::Connection, e: rusqlite::Error) -> DomainError {
        let refused = self.refusal();
        drop(self);
        refused.unwrap_or_else(|| prepare_failed(conn, e))
    }

    /// Why the authorizer refused the statement, if it did: the read
    /// contract in words, pointing a table at the models that read it.
    fn refusal(&self) -> Option<DomainError> {
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let first = seen.denied.first()?;
        Some(DomainError::Invalid(match first {
            Denied::Table(table) => {
                let hint = match self.readers.get(table) {
                    Some(views) if !views.is_empty() => format!("read {}", views.join(" or ")),
                    _ => "read a published model (see `v_model`)".to_string(),
                };
                format!("query_sql: `{table}` is a physical table, not a published model; {hint}")
            }
            Denied::Action(action) => {
                format!("query_sql: only reads of published models are allowed, not {action}")
            }
        }))
    }

    /// Refuse writes on this connection until the session ends.
    fn query_only(&self) -> Result<(), DomainError> {
        self.conn
            .execute_batch("PRAGMA query_only = ON")
            .map_err(crate::database::map_sql_err)
    }

    /// The reads so far, classified. A read of a view, or by one, is a
    /// model read; a read whose accessor is a view happened inside it; anything
    /// else is the query's own SQL (a CTE's name can be the accessor too).
    /// SQLite reports `count(*)` over a view as an empty-column read of
    /// the view's base table at top level — that one belongs to the view.
    fn reads(&self) -> Reads {
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let mut models = std::collections::BTreeSet::new();
        let mut under = std::collections::HashSet::new();
        for (table, _, accessor) in &seen.reads {
            if self.views.contains(table) {
                models.insert(table.clone());
            }
            // A view that read something was read (`count(*)` over a view
            // reports only its base table's reads, with the view as
            // accessor).
            if let Some(view) = accessor.as_ref().filter(|a| self.views.contains(*a)) {
                models.insert(view.clone());
            }
            if self.nested(accessor) {
                under.insert(table.clone());
            }
        }
        let tables = seen
            .reads
            .iter()
            .filter(|(table, column, accessor)| {
                self.tables.contains(table)
                    && !self.nested(accessor)
                    && !(column.is_empty() && under.contains(table))
            })
            .map(|(table, _, _)| table.clone())
            .collect::<std::collections::BTreeSet<_>>();
        Reads {
            models: models.into_iter().collect(),
            tables: tables.into_iter().collect(),
            measures: Vec::new(),
        }
    }

    /// What the statement read itself, not through a view: `(views,
    /// tables)` — a model's derived inputs. A view is direct when it was
    /// read at top level, or read something (`count(*)`) without any read
    /// placing it inside another view.
    pub(crate) fn direct_inputs(
        &self,
    ) -> (
        std::collections::BTreeSet<String>,
        std::collections::BTreeSet<String>,
    ) {
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let is_view = |a: &Option<String>| self.nested(a);
        let mut views = std::collections::BTreeSet::new();
        let mut nested = std::collections::HashSet::new();
        let mut under = std::collections::HashSet::new();
        for (table, _, accessor) in &seen.reads {
            if is_view(accessor) {
                under.insert(table.clone());
                if self.views.contains(table) {
                    nested.insert(table.clone());
                }
            }
        }
        for (table, _, accessor) in &seen.reads {
            if self.views.contains(table) && !is_view(accessor) {
                views.insert(table.clone());
            }
            if let Some(v) = accessor.as_ref().filter(|a| self.views.contains(*a)) {
                if !nested.contains(v) {
                    views.insert(v.clone());
                }
            }
        }
        let tables = seen
            .reads
            .iter()
            .filter(|(table, column, accessor)| {
                self.tables.contains(table)
                    && !is_view(accessor)
                    && !(column.is_empty() && under.contains(table))
            })
            .map(|(table, _, _)| table.clone())
            .collect();
        (views, tables)
    }
}

impl Drop for ReadSession<'_> {
    fn drop(&mut self) {
        let _ = self.conn.authorizer(
            None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>,
        );
        if let Err(e) = self.conn.execute_batch("PRAGMA query_only = OFF") {
            tracing::error!(error = %e, "query_sql: could not restore a pooled connection to writable");
        }
    }
}

fn cell(v: rusqlite::types::ValueRef<'_>) -> SqlCell {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Null => SqlCell::Null(()),
        ValueRef::Integer(i) => SqlCell::Int(i),
        ValueRef::Real(f) => SqlCell::Real(f),
        ValueRef::Text(t) => SqlCell::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => SqlCell::Text(format!("<blob {} bytes>", b.len())),
    }
}

/// A query's temp views on its connection, dropped (last first) with this
/// guard.
struct TempViews<'c> {
    conn: &'c rusqlite::Connection,
    names: Vec<String>,
}

impl<'c> TempViews<'c> {
    fn create(conn: &'c rusqlite::Connection, views: &[TempView]) -> Result<Self, DomainError> {
        let mut guard = Self {
            conn,
            names: Vec::new(),
        };
        for v in views {
            conn.execute_batch(&format!(
                "CREATE TEMP VIEW \"{}\" AS {}",
                v.name.replace('"', "\"\""),
                v.sql
            ))
            .map_err(|e| DomainError::Invalid(format!("query_sql: `{}`: {e}", v.name)))?;
            guard.names.push(v.name.clone());
        }
        Ok(guard)
    }
}

impl Drop for TempViews<'_> {
    fn drop(&mut self) {
        for name in self.names.iter().rev() {
            let sql = format!("DROP VIEW IF EXISTS temp.\"{}\"", name.replace('"', "\"\""));
            if let Err(e) = self.conn.execute_batch(&sql) {
                tracing::error!(error = %e, "query_sql: could not drop a temp view");
            }
        }
    }
}

/// A query's temp tables on its connection, dropped with this guard.
struct TempTables<'c> {
    conn: &'c rusqlite::Connection,
    names: Vec<String>,
}

impl<'c> TempTables<'c> {
    fn create(conn: &'c rusqlite::Connection, tables: &[TempTable]) -> Result<Self, DomainError> {
        let quote = |n: &str| format!("\"{}\"", n.replace('"', "\"\""));
        let mut guard = Self {
            conn,
            names: Vec::new(),
        };
        for t in tables {
            let cols: Vec<String> = t.columns.iter().map(|c| quote(c)).collect();
            conn.execute_batch(&format!(
                "CREATE TEMP TABLE {} ({})",
                quote(&t.name),
                cols.join(", ")
            ))
            .map_err(crate::database::map_sql_err)?;
            guard.names.push(t.name.clone());
            let marks = vec!["?"; t.columns.len()].join(", ");
            let mut insert = conn
                .prepare(&format!(
                    "INSERT INTO temp.{} VALUES ({marks})",
                    quote(&t.name)
                ))
                .map_err(crate::database::map_sql_err)?;
            for row in &t.rows {
                let vals: Vec<rusqlite::types::Value> = row.iter().map(SqlCell::to_sql).collect();
                insert
                    .execute(rusqlite::params_from_iter(vals))
                    .map_err(crate::database::map_sql_err)?;
            }
        }
        Ok(guard)
    }
}

impl Drop for TempTables<'_> {
    fn drop(&mut self) {
        for name in &self.names {
            let sql = format!(
                "DROP TABLE IF EXISTS temp.\"{}\"",
                name.replace('"', "\"\"")
            );
            if let Err(e) = self.conn.execute_batch(&sql) {
                tracing::error!(error = %e, "query_sql: could not drop a temp table");
            }
        }
    }
}

/// Prepare under the recording authorizer, then execute under `PRAGMA
/// query_only` with an interrupt timer. The [`ReadSession`] restores the
/// pooled connection on every path.
fn run_read_only(
    conn: &rusqlite::Connection,
    query: &SqlQuery,
) -> Result<SqlQueryResult, DomainError> {
    let cap = query
        .limit
        .unwrap_or(DEFAULT_ROW_LIMIT)
        .clamp(1, MAX_ROW_LIMIT);
    let timeout = query.timeout;
    // Temp tables first — creating them is a write — then the read
    // session; locals drop in reverse, so the session ends before they go.
    let _temp = TempTables::create(conn, &query.temp)?;
    let _views = TempViews::create(conn, &query.temp_views)?;
    let access = if query.raw {
        Access::Record
    } else {
        Access::Enforce
    };
    let session = ReadSession::open(conn, access, &query.sql)?;
    let mut stmt = match conn.prepare(&query.sql) {
        Ok(stmt) => stmt,
        Err(e) => return Err(session.failed(conn, e)),
    };
    if !stmt.readonly() {
        return Err(read_only_only());
    }
    let reads = session.reads();
    bind(&mut stmt, &query.params)?;
    session.query_only()?;

    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let interrupt = conn.get_interrupt_handle();
    let timer = std::thread::spawn(move || {
        if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = done_rx.recv_timeout(timeout) {
            interrupt.interrupt();
        }
    });

    let result = (|| {
        let columns: Vec<String> = stmt
            .column_names()
            .iter()
            .map(|c| (*c).to_string())
            .collect();
        let mut rows_out = Vec::new();
        let mut truncated = false;
        let mut rows = stmt.raw_query();
        while let Some(row) = rows.next()? {
            if rows_out.len() == cap {
                truncated = true;
                break;
            }
            let mut vals = Vec::with_capacity(columns.len());
            for i in 0..columns.len() {
                vals.push(cell(row.get_ref(i)?));
            }
            rows_out.push(vals);
        }
        Ok::<_, rusqlite::Error>(SqlQueryResult {
            columns,
            rows: rows_out,
            truncated,
            reads,
            freshness: Vec::new(),
        })
    })();

    drop(done_tx);
    let _ = timer.join();
    drop(stmt);
    drop(session);

    result.map_err(|e| match e {
        rusqlite::Error::SqliteFailure(f, _)
            if f.code == rusqlite::ErrorCode::OperationInterrupted =>
        {
            DomainError::Invalid(format!("query_sql: timed out after {timeout:?}"))
        }
        other => invalid(other),
    })
}

/// [`SemanticLayer::run`] on `conn` — for a command's handler that reads
/// inside its own transaction (a command's `sql.read`): one
/// read-only `SELECT`/`WITH` over the published models, under the same
/// authorizer, row cap and timeout. The read session is restored before
/// it returns, so the handler's writes that follow are unaffected.
pub fn read_on(
    conn: &rusqlite::Connection,
    query: &SqlQuery,
) -> Result<SqlQueryResult, DomainError> {
    crate::sql_tokens::check_single_read(&query.sql)?;
    run_read_only(conn, query)
}

/// [`SemanticLayer::check_with`] on `conn` — for a command's handler,
/// which checks an agent's SQL inside its own transaction: a single
/// read-only `SELECT`/`WITH` over the published models (the `query_sql`
/// authorizer), compiled but not run. What it would read.
pub fn check_query_on(conn: &rusqlite::Connection, query: &SqlQuery) -> Result<Reads, DomainError> {
    crate::sql_tokens::check_single_read(&query.sql)?;
    let _temp = TempTables::create(conn, &query.temp)?;
    let _views = TempViews::create(conn, &query.temp_views)?;
    let access = if query.raw {
        Access::Record
    } else {
        Access::Enforce
    };
    let session = ReadSession::open(conn, access, &query.sql)?;
    {
        let stmt = match conn.prepare(&query.sql) {
            Ok(stmt) => stmt,
            Err(e) => return Err(session.failed(conn, e)),
        };
        if !stmt.readonly() {
            return Err(read_only_only());
        }
    }
    Ok(session.reads())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn seeded() -> (Database, SemanticLayer) {
        let db = Database::in_memory();
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'oxplow', 'main', 'refs/heads/main', 'local', '/tmp/x', '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                   VALUES (1, 1, 'Thread', 'active', '2026-01-01', '2026-01-01');
                 INSERT INTO task (id, thread_id, title, status, priority, created_by, created_at, updated_at)
                   VALUES (1, 1, 'Live task', 'in_progress', 'medium', 'agent', '2026-01-01', '2026-01-01');
                 INSERT INTO task (id, thread_id, title, status, priority, created_by, created_at, updated_at, deleted_at)
                   VALUES (2, 1, 'Deleted task', 'ready', 'medium', 'agent', '2026-01-01', '2026-01-01', '2026-01-02');",
            )
        })
        .await
        .unwrap();
        let sl = SemanticLayer::new(db.clone());
        (db, sl)
    }

    /// A command's handler reads its `input` with `read_on` inside its
    /// write transaction, then its children write: the read must leave
    /// the connection writable (`query_only` off, no authorizer left).
    #[tokio::test]
    async fn read_on_leaves_the_connection_writable() {
        let (db, _sl) = seeded().await;
        db.transaction(|tx| {
            let read = read_on(tx, &SqlQuery::new("SELECT title FROM v_task WHERE id = 1"))?;
            assert_eq!(read.rows.len(), 1);
            let query_only: i64 = tx
                .query_row("PRAGMA query_only", [], |r| r.get(0))
                .map_err(crate::database::map_sql_err)?;
            assert_eq!(query_only, 0);
            tx.execute("UPDATE task SET title = 'Written after' WHERE id = 1", [])
                .map_err(crate::database::map_sql_err)?;
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_runs_and_their_cases_read_from_the_run_capture() {
        let (db, sl) = seeded().await;
        let payload = json!({"kind": "test-detail", "payload": {
            "command": "bun run test:collect", "exitCode": 1, "durationMs": 4200,
            "passed": 1, "failed": 1, "skipped": 1, "total": 3,
            "suites": [{"name": "app", "cases": [
                {"classname": "a::tests", "name": "ok", "status": "passed", "timeMs": 12},
                {"classname": "a::tests", "name": "bad", "status": "failed"},
                {"classname": "b", "name": "later", "status": "skipped"}
            ]}]
        }})
        .to_string();
        let counts_only = json!({"kind": "test-detail", "payload": {
            "command": "cargo test", "passed": 5, "failed": 0, "total": 5
        }})
        .to_string();
        db.call(move |c| {
            c.execute_batch(
                "INSERT INTO effort (id, work_item, thread_id, started_at, ended_at) VALUES (7, 'work_item:oxplow:tsk1', 1, '2026-01-01', '2026-01-02');
                 INSERT INTO effort (id, work_item, thread_id, started_at) VALUES (8, 'work_item:oxplow:tsk1', 1, '2026-01-01');
                 INSERT INTO metric_capture (id, stream_id, effort_id, producer, provenance, source, captured_at)
                   VALUES (40, 1, 7, 'coverage', 'observed', 'hook', '2026-01-01T00:00:00Z');",
            )?;
            c.execute(
                "INSERT INTO metric_capture (id, stream_id, thread_id, effort_id, producer, provenance, source, captured_at, detail_json)
                 VALUES (41, 1, 1, 7, 'tests', 'observed', 'hook', '2026-01-02T00:00:00Z', ?1),
                        (42, 1, 1, 8, 'test-run', 'asserted', 'mcp', '2026-01-03T00:00:00Z', ?2)",
                [&payload, &counts_only],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let rows = |sql: &'static str| {
            let sl = sl.clone();
            async move {
                serde_json::to_value(sl.query_sql(sql, vec![], None).await.unwrap().rows).unwrap()
            }
        };
        assert_eq!(
            rows("SELECT id, stream_id, thread_id, effort_id, command, exit_code, passed, failed, skipped, total, duration_ms, provenance FROM v_test_run ORDER BY id").await,
            json!([
                [41, 1, 1, 7, "bun run test:collect", 1, 1, 1, 1, 3, 4200, "observed"],
                [42, 1, 1, 8, "cargo test", null, 5, 0, null, 5, null, "asserted"]
            ])
        );
        assert_eq!(
            rows("SELECT run_id, suite, classname, name, status, time_ms FROM v_test_case ORDER BY name").await,
            json!([
                [41, "app", "a::tests", "bad", "failed", null],
                [41, "app", "b", "later", "skipped", null],
                [41, "app", "a::tests", "ok", "passed", 12]
            ])
        );
    }

    #[tokio::test]
    async fn check_sql_compiles_without_running_and_refuses_writes() {
        let (_db, sl) = seeded().await;
        sl.check("SELECT count(*) FROM v_task e WHERE e.status = 'done'")
            .await
            .unwrap();
        let err = |sql: &'static str| {
            let sl = sl.clone();
            async move { sl.check(sql).await.unwrap_err().to_string() }
        };
        assert!(err("SELECT nope FROM v_task")
            .await
            .contains("no such column"));
        assert!(err("DELETE FROM task").await.contains("SELECT"));
    }

    #[test]
    fn sql_cells_round_trip_as_plain_json_scalars() {
        let cells: Vec<SqlCell> = serde_json::from_value(json!([null, true, 3, 1.5, "x"])).unwrap();
        assert_eq!(
            cells,
            vec![
                SqlCell::Null(()),
                SqlCell::Bool(true),
                SqlCell::Int(3),
                SqlCell::Real(1.5),
                SqlCell::Text("x".into())
            ]
        );
        assert_eq!(
            serde_json::to_value(&cells).unwrap(),
            json!([null, true, 3, 1.5, "x"])
        );
        assert_eq!(
            SqlCell::from(json!({"a": 1})),
            SqlCell::Text("{\"a\":1}".into())
        );
    }

    #[tokio::test]
    async fn binds_named_params_ignoring_ones_the_query_does_not_use() {
        let (_db, sl) = seeded().await;
        let out = sl
            .run(
                SqlQuery::new("SELECT title FROM v_task WHERE status = :status AND id >= :min_id")
                    .named(vec![
                        ("status".into(), SqlCell::Text("in_progress".into())),
                        ("min_id".into(), SqlCell::Int(1)),
                        ("unused".into(), SqlCell::Int(9)),
                    ]),
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([["Live task"]])
        );
        // A name the query uses must be given; a positional count must match.
        let err = sl
            .run(SqlQuery::new("SELECT title FROM v_task WHERE id = :id").named(vec![]))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no value for :id"), "{err}");
        let err = sl
            .query_sql("SELECT ?1, ?2", vec![SqlCell::Int(1)], None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("takes 2 parameter(s), 1 given"),
            "{err}"
        );
    }

    /// P4.1 (tsk486): a query says what it read — the views, through other
    /// views too, and any table its own SQL read (not one a view read).
    #[tokio::test]
    async fn a_query_reports_what_it_read() {
        let (_db, sl) = seeded().await;
        // Raw, so the physical-table reads are recorded rather than refused.
        let reads = |sql: &'static str| {
            let sl = sl.clone();
            async move { sl.run(SqlQuery::new(sql).raw(true)).await.unwrap().reads }
        };
        let r = reads("SELECT count(*) FROM v_task").await;
        assert_eq!((r.models, r.tables), (vec!["v_task".to_string()], vec![]));
        let r =
            reads("WITH t AS (SELECT id FROM v_task) SELECT * FROM t JOIN v_thread th ON 1").await;
        assert_eq!(r.models, vec!["v_task".to_string(), "v_thread".to_string()]);
        assert!(r.tables.is_empty(), "{:?}", r.tables);
        let r = reads("SELECT t.title FROM task t JOIN v_thread th ON th.id = t.thread_id").await;
        assert_eq!(r.tables, vec!["task".to_string()]);
        let r = reads("WITH c AS (SELECT title FROM task) SELECT * FROM c").await;
        assert_eq!(
            r.tables,
            vec!["task".to_string()],
            "a CTE body is the query's own SQL"
        );
        let r = reads("SELECT j.value FROM v_task t, json_each('[1,2]') j").await;
        assert!(
            r.tables.is_empty(),
            "a table-valued function isn't a table: {:?}",
            r.tables
        );
        let r = reads("SELECT name FROM sqlite_master").await;
        assert_eq!(r.tables, vec!["sqlite_master".to_string()]);
        let checked = sl
            .check("SELECT id FROM v_task WHERE id = :id")
            .await
            .unwrap();
        assert_eq!(checked.models, vec!["v_task".to_string()]);
    }

    /// tsk1039: a misspelled model names the one it most likely meant —
    /// an agent's lens query and `plugin check` read the same error.
    #[tokio::test]
    async fn an_unknown_model_suggests_the_nearest_one() {
        let (_db, sl) = seeded().await;
        let err = sl
            .query_sql("SELECT * FROM v_tasks", vec![], None)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("no such table: v_tasks"), "{err}");
        assert!(err.contains("did you mean `v_task`?"), "{err}");
        let err = sl
            .query_sql("SELECT * FROM v_zzzzzzzzz", vec![], None)
            .await
            .unwrap_err()
            .to_string();
        assert!(!err.contains("did you mean"), "nothing near it: {err}");
    }

    /// P4.3 (tsk488): the read contract is enforced — a query's own SQL
    /// reads only models, and a refused table points at the models over
    /// it; the explorer's raw mode reads anything, still recorded.
    #[tokio::test]
    async fn a_physical_table_is_refused_naming_its_models() {
        let (db, sl) = seeded().await;
        let refused = |sql: &'static str| {
            let sl = sl.clone();
            async move {
                sl.query_sql(sql, vec![], None)
                    .await
                    .unwrap_err()
                    .to_string()
            }
        };
        let msg = refused("SELECT * FROM task").await;
        // The table's own model comes first, before the others that read
        // it (an index feed, v_search_task — tsk922).
        assert!(
            msg.contains("`task` is a physical table, not a published model; read v_task"),
            "{msg}"
        );
        assert!(!msg.contains("no such table"), "{msg}");
        assert!(
            refused("WITH c AS (SELECT title FROM task) SELECT * FROM c")
                .await
                .contains("`task` is a physical table")
        );
        assert!(refused("SELECT name FROM sqlite_master")
            .await
            .contains("`sqlite_master` is a physical table"));
        assert!(sl.check("SELECT id FROM task").await.is_err());
        // Models, counts over them, table-valued functions and temp tables
        // are all fine.
        for ok in [
            "SELECT count(*) FROM v_task",
            "SELECT t.id FROM v_task t JOIN v_thread th ON th.id = t.thread_id",
            "SELECT j.value FROM v_task t, json_each('[1]') j",
        ] {
            sl.query_sql(ok, vec![], None)
                .await
                .unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        db.call(|c| {
            c.execute_batch("CREATE TEMP TABLE scratch (x); INSERT INTO scratch VALUES (1)")
        })
        .await
        .unwrap();
        sl.query_sql("SELECT x FROM scratch", vec![], None)
            .await
            .unwrap();
        // A model built on a CTE reads fine: SQLite names the CTE, not the
        // view, as the accessor of the reads inside it (P4.11).
        db.call(|c| {
            c.execute_batch(
                "CREATE VIEW v_cte_probe AS WITH live AS (SELECT title FROM task) SELECT title FROM live",
            )
        })
        .await
        .unwrap();
        let out = sl
            .query_sql("SELECT title FROM v_cte_probe ORDER BY title", vec![], None)
            .await
            .unwrap();
        assert_eq!(out.rows.len(), 2);
        assert_eq!(out.reads.models, vec!["v_cte_probe".to_string()]);
        assert!(out.reads.tables.is_empty(), "{:?}", out.reads.tables);
        // Raw reads the table, and says so.
        let out = sl
            .run(SqlQuery::new("SELECT count(*) FROM task").raw(true))
            .await
            .unwrap();
        assert_eq!(out.reads.tables, vec!["task".to_string()]);
        // A refusal leaves the connection writable.
        db.call(|c| c.execute("UPDATE task SET title = 'x' WHERE id = 1", []))
            .await
            .unwrap();
    }

    /// P4.5a (tsk490): a query's temp tables exist for it alone — created
    /// and filled on its connection, read like a model, gone afterwards
    /// even when the query fails.
    #[tokio::test]
    async fn temp_tables_live_for_one_query() {
        let (db, sl) = seeded().await;
        let grid = TempTable {
            name: "grid_t".into(),
            columns: vec!["bucket".into(), "n".into()],
            rows: vec![
                vec![SqlCell::Text("2026-01-01".into()), SqlCell::Int(3)],
                vec![SqlCell::Text("2026-01-02".into()), SqlCell::Real(1.5)],
            ],
        };
        let mut q = SqlQuery::new("SELECT bucket, n FROM temp.grid_t ORDER BY bucket");
        q.temp.push(grid.clone());
        let out = sl.run(q).await.unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([["2026-01-01", 3], ["2026-01-02", 1.5]])
        );
        assert_eq!(out.reads.tables, vec!["temp.grid_t".to_string()]);
        let mut failing = SqlQuery::new("SELECT nope FROM temp.grid_t");
        failing.temp.push(grid);
        assert!(sl.run(failing).await.is_err());
        let left: i64 = db
            .call(|c| {
                c.query_row(
                    "SELECT count(*) FROM sqlite_temp_master WHERE name = 'grid_t'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(left, 0);
        db.call(|c| c.execute("UPDATE task SET title = 'w' WHERE id = 1", []))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn reads_core_views() {
        let (_db, sl) = seeded().await;
        let out = sl
            .query_sql(
                "SELECT id, title, status, stream_id FROM v_task ORDER BY id",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(out.columns, vec!["id", "title", "status", "stream_id"]);
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([[1, "Live task", "in_progress", 1]])
        );
        assert!(!out.truncated);
    }

    #[tokio::test]
    async fn binds_positional_params() {
        let (_db, sl) = seeded().await;
        let out = sl
            .query_sql(
                "SELECT title FROM v_task WHERE status = ?1",
                vec![json!("in_progress").into()],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([["Live task"]])
        );
    }

    #[tokio::test]
    async fn rejects_anything_but_a_single_read() {
        let (_db, sl) = seeded().await;
        for sql in [
            "DELETE FROM task",
            "INSERT INTO task (title) VALUES ('x')",
            "UPDATE task SET title = 'x'",
            "DROP VIEW v_task",
            "PRAGMA query_only = 0",
            "ATTACH DATABASE '/tmp/evil.sqlite' AS evil",
            "SELECT 1; DELETE FROM task",
            "WITH x AS (SELECT 1) DELETE FROM task",
            "",
        ] {
            let err = sl.query_sql(sql, vec![], None).await.unwrap_err();
            assert!(
                matches!(err, DomainError::Invalid(_)),
                "{sql:?} gave {err:?}"
            );
        }
        // Nothing was deleted.
        let out = sl
            .run(SqlQuery::new("SELECT count(*) FROM task").raw(true))
            .await
            .unwrap();
        assert_eq!(serde_json::to_value(&out.rows).unwrap(), json!([[2]]));
    }

    #[tokio::test]
    async fn caps_rows_and_reports_truncation() {
        let (_db, sl) = seeded().await;
        let sql = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 50) SELECT x FROM c";
        let out = sl.query_sql(sql, vec![], Some(10)).await.unwrap();
        assert_eq!(out.rows.len(), 10);
        assert!(out.truncated);
    }

    #[tokio::test]
    async fn times_out_runaway_queries() {
        let (_db, sl) = seeded().await;
        let sql = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c";
        let err = sl
            .run(SqlQuery::new(sql).timeout(Duration::from_millis(100)))
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("timed out")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn leaves_the_pooled_connection_writable() {
        let (db, sl) = seeded().await;
        sl.query_sql("SELECT 1", vec![], None).await.unwrap();
        let _ = sl.query_sql("DELETE FROM task", vec![], None).await;
        // A failure after the authorizer is installed, and one after
        // `query_only` is on, leave nothing behind either.
        let _ = sl.query_sql("SELECT nope FROM v_task", vec![], None).await;
        let _ = sl.query_sql("SELECT ?1", vec![], None).await;
        let _ = sl
            .run(
                SqlQuery::new("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c")
                    .timeout(Duration::from_millis(50)),
            )
            .await;
        // in_memory() has a single pooled connection, so this proves
        // query_only was reset.
        db.call(|c| c.execute("UPDATE task SET title = 'renamed' WHERE id = 1", []))
            .await
            .unwrap();
    }
}
