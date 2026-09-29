//! The semantic layer's read surface: `query_sql` over the stable `v_*`
//! SQL views and `describe_schema` over their documented columns.
//!
//! The `v_*` views (migration `V73__semantic_layer_views.sql`) are the
//! versioned contract; physical tables stay internal. See
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
}

/// One read-only query: its SQL, parameters, row cap and time budget.
#[derive(Debug, Clone)]
pub struct SqlQuery {
    pub sql: String,
    pub params: SqlParams,
    /// Row cap: default [`DEFAULT_ROW_LIMIT`], at most [`MAX_ROW_LIMIT`].
    pub limit: Option<usize>,
    pub timeout: Duration,
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
        }
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
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

/// One documented column of a semantic-layer entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SchemaColumn {
    pub name: String,
    /// Declared SQLite type, as `PRAGMA table_info` reports it.
    pub sql_type: String,
    pub doc: String,
}

/// One queryable entity (a `v_*` view) and its column docs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SchemaEntity {
    /// SQL name to query, e.g. `v_task`.
    pub name: String,
    pub description: String,
    /// Who provides it: `core`, or an extension name.
    pub owner: String,
    pub columns: Vec<SchemaColumn>,
    /// Documented joins to other entities (how the data connects).
    pub relations: Vec<SchemaRelation>,
    /// False for a declared extension entity whose source hasn't synced
    /// yet (its view doesn't exist, so querying it would fail).
    pub available: bool,
}

/// A documented join from one entity to another view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SchemaRelation {
    /// View it joins to, e.g. `v_task`.
    pub to: String,
    /// SQL join condition.
    pub on: String,
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
        crate::sql_tokens::check_single_read(sql)?;
        let sql = sql.to_string();
        self.db
            .call(move |conn| {
                Ok((|| {
                    let session = ReadSession::open(conn)?;
                    {
                        let stmt = conn.prepare(&sql).map_err(invalid)?;
                        if !stmt.readonly() {
                            return Err(read_only_only());
                        }
                    }
                    Ok(session.reads())
                })())
            })
            .await?
    }

    /// The name of every view in the database — the schema, read directly
    /// rather than through the query contract.
    pub async fn view_names(&self) -> Result<std::collections::HashSet<String>, DomainError> {
        self.db.call_mut(|conn| view_names(conn)).await
    }

    /// The catalog of queryable entities with column docs. Column
    /// types come from the live schema, so they can't drift.
    pub async fn describe_schema(&self) -> Result<Vec<SchemaEntity>, DomainError> {
        self.db
            .call(|conn| {
                let mut out = Vec::with_capacity(CATALOG.len());
                for view in CATALOG {
                    let mut st = conn.prepare(&format!("PRAGMA table_info({})", view.name))?;
                    let types: Vec<(String, String)> = st
                        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
                        .collect::<rusqlite::Result<_>>()?;
                    let columns = view
                        .columns
                        .iter()
                        .map(|(name, doc)| SchemaColumn {
                            name: (*name).to_string(),
                            sql_type: types
                                .iter()
                                .find(|(n, _)| n == name)
                                .map(|(_, t)| t.clone())
                                .unwrap_or_default(),
                            doc: (*doc).to_string(),
                        })
                        .collect();
                    out.push(SchemaEntity {
                        name: view.name.to_string(),
                        description: view.description.to_string(),
                        owner: "core".to_string(),
                        columns,
                        relations: Vec::new(),
                        available: true,
                    });
                }
                Ok(out)
            })
            .await
    }
}

fn invalid(e: rusqlite::Error) -> DomainError {
    DomainError::Invalid(format!("query_sql: {e}"))
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
}

impl<'c> ReadSession<'c> {
    pub(crate) fn open(conn: &'c rusqlite::Connection) -> Result<Self, DomainError> {
        let views = view_names(conn)?;
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
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Seen::default()));
        let sink = seen.clone();
        conn.authorizer(Some(
            move |ctx: rusqlite::hooks::AuthContext<'_>| -> rusqlite::hooks::Authorization {
                if let rusqlite::hooks::AuthAction::Read {
                    table_name,
                    column_name,
                } = ctx.action
                {
                    let table = match ctx.database_name {
                        Some("temp") => format!("temp.{table_name}"),
                        _ => table_name.to_string(),
                    };
                    sink.lock().unwrap_or_else(|e| e.into_inner()).reads.push((
                        table,
                        column_name.to_string(),
                        ctx.accessor.map(str::to_string),
                    ));
                }
                rusqlite::hooks::Authorization::Allow
            },
        ))
        .map_err(crate::database::map_sql_err)?;
        Ok(Self {
            conn,
            seen,
            views,
            tables,
        })
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
                under.insert(table.clone());
            }
        }
        let tables = seen
            .reads
            .iter()
            .filter(|(table, column, accessor)| {
                self.tables.contains(table)
                    && !accessor.as_ref().is_some_and(|a| self.views.contains(a))
                    && !(column.is_empty() && under.contains(table))
            })
            .map(|(table, _, _)| table.clone())
            .collect::<std::collections::BTreeSet<_>>();
        Reads {
            models: models.into_iter().collect(),
            tables: tables.into_iter().collect(),
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
        let is_view = |a: &Option<String>| a.as_ref().is_some_and(|a| self.views.contains(a));
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
    let session = ReadSession::open(conn)?;
    let mut stmt = conn.prepare(&query.sql).map_err(invalid)?;
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

struct CatalogView {
    name: &'static str,
    description: &'static str,
    columns: &'static [(&'static str, &'static str)],
}

/// Column docs for every core `v_*` view. Order must match the view's
/// SELECT list; `schema_docs_match_the_views_exactly` enforces it.
const CATALOG: &[CatalogView] = &[
    CatalogView {
        name: "v_stream",
        description: "Streams: parallel lines of work, each its own git worktree. Exactly one is `primary` (the repo itself).",
        columns: &[
            ("id", "Stream id."),
            ("kind", "`primary` (the repo) or `worktree`."),
            ("title", "Display name."),
            ("branch", "Git branch currently checked out in the worktree."),
            ("worktree_path", "Absolute path of the stream's worktree."),
            ("created_at", "RFC 3339 timestamp."),
            ("updated_at", "RFC 3339 timestamp."),
            ("archived_at", "Set when the stream was archived; NULL while live."),
        ],
    },
    CatalogView {
        name: "v_thread",
        description: "Threads: independent lines of thought inside a stream, each with its own agent session.",
        columns: &[
            ("id", "Thread id."),
            ("stream_id", "Owning stream (v_stream.id)."),
            ("title", "Display name."),
            ("status", "`active` (the stream's writer), `queued` (read-only) or `closed`."),
            ("agent", "Agent harness: `claude`, `codex`, `opencode`, or `acp` (an Agent Client Protocol agent)."),
            ("sort_index", "Order within the stream."),
            ("created_at", "RFC 3339 timestamp."),
            ("updated_at", "RFC 3339 timestamp."),
            ("closed_at", "Set when the thread was closed."),
            ("archived_at", "Set when the thread was archived."),
            ("acp_agent", "For an `acp` thread, which ACP agent it runs (`claude`, `gemini`, a project `acpAgents` name); NULL otherwise."),
        ],
    },
    CatalogView {
        name: "v_task",
        description: "Tasks (work items), excluding deleted ones. `thread_id` NULL means the project-wide backlog.",
        columns: &[
            ("id", "Task id (shown as tsk<id>)."),
            ("thread_id", "Owning thread (v_thread.id); NULL = backlog."),
            ("stream_id", "The thread's stream; NULL for backlog tasks."),
            ("parent_id", "Parent task (an epic); NULL at top level."),
            ("title", "Task title."),
            ("description", "Markdown body."),
            ("status", "`ready`, `in_progress`, `blocked`, `done`, `canceled` or `archived`."),
            ("priority", "`low`, `medium`, `high` or `urgent`."),
            ("author", "Who the task came from: `user` or `agent`."),
            ("sort_index", "Order within its list."),
            ("created_at", "RFC 3339 timestamp."),
            ("updated_at", "RFC 3339 timestamp."),
            ("completed_at", "Set when the task reached `done`."),
        ],
    },
    CatalogView {
        name: "v_effort",
        description: "Efforts: one bracketed span of work on a work item (an oxplow task, or another provider's item), between a start and an end snapshot.",
        columns: &[
            ("id", "Effort id."),
            ("work_item", "The work item worked on, as a canonical ref: `work_item:oxplow:tsk42`, `work_item:linear:ENG-12`."),
            ("task_id", "The oxplow task worked on (v_task.id); NULL for another provider's work item."),
            ("thread_id", "Thread that did the work."),
            ("stream_id", "That thread's stream."),
            ("started_at", "RFC 3339 timestamp."),
            ("ended_at", "RFC 3339 timestamp; NULL while the effort is open."),
            ("start_snapshot_id", "Snapshot at the start (v_snapshot.id)."),
            ("end_snapshot_id", "Snapshot at the end; NULL while open."),
            ("summary", "Closing summary written when the effort closed."),
        ],
    },
    CatalogView {
        name: "v_comment",
        description: "Comment threads anchored to a page (file, wiki page, task…). `intent` says who should act.",
        columns: &[
            ("id", "Comment id."),
            ("stream_id", "Stream the comment was made in."),
            ("thread_id", "Thread it was made in, if any."),
            ("target_kind", "Kind of page it is anchored to: `file`, `wiki`, `task`, …"),
            ("target_id", "Id of that page (path, slug, task id…)."),
            ("quote", "The selected text the comment is anchored to."),
            ("intent", "`note` (for me) or `followup` (for the agent to act on)."),
            ("status", "`open` or `resolved`."),
            ("orphaned", "1 when the quote can no longer be found in the page."),
            ("author", "Who started it."),
            ("created_at", "RFC 3339 timestamp."),
            ("updated_at", "RFC 3339 timestamp."),
            ("last_activity_at", "Time of the newest message."),
            ("resolved_at", "Set when resolved."),
            ("body", "Text of the first message."),
            ("message_count", "Number of messages in the thread."),
        ],
    },
    CatalogView {
        name: "v_wiki_page",
        description: "Wiki pages (project knowledge). The full body is on disk; this has the excerpt.",
        columns: &[
            ("slug", "Page slug (link with [[slug]])."),
            ("title", "Page title."),
            ("body_excerpt", "Start of the body."),
            ("body_size_bytes", "Size of the full body."),
            ("created_at", "RFC 3339 timestamp."),
            ("updated_at", "RFC 3339 timestamp."),
        ],
    },
    CatalogView {
        name: "v_snapshot",
        description: "Snapshots: point-in-time captures of a stream's worktree, used to bracket efforts and diffs.",
        columns: &[
            ("id", "Snapshot id."),
            ("stream_id", "Stream captured."),
            ("created_at", "RFC 3339 timestamp."),
            ("git_commit", "HEAD commit when captured, if known."),
            ("git_branch", "Branch when captured, if known."),
            (
                "tree_hash",
                "Whole-tree identity: two snapshots with the same tree_hash hold the same files. NULL before V96.",
            ),
        ],
    },
    CatalogView {
        name: "v_snapshot_op",
        description: "The snapshot operation log: one row per take (why it ran, what it is anchored to, the snapshot it left the worktree at and its parent). A take that changed nothing points at its parent.",
        columns: &[
            ("seq", "Operation order within the project."),
            ("stream_id", "Stream (worktree) the take ran on."),
            ("snapshot_id", "Snapshot the worktree is at after the take (v_snapshot.id)."),
            ("parent_snapshot_id", "Snapshot the worktree was at just before this take; parent → snapshot is what THIS take recorded (for a turn's changes use v_agent_turn.start_snapshot_id → snapshot_id)."),
            ("trigger", "Why: turn_end, quiet, effort_start, effort_end, startup, manual, git_refs, head_moved, legacy."),
            ("thread_id", "Thread the take belongs to, if any."),
            ("turn_id", "Agent turn the take belongs to, if any (v_agent_turn.id)."),
            ("effort_id", "Effort the take belongs to, if any (v_effort.id)."),
            ("at", "RFC 3339 timestamp."),
            ("elapsed_ms", "How long the take took."),
            ("budget_ms", "The caller's time budget, if it had one."),
            ("over_budget", "1 when elapsed_ms exceeded budget_ms."),
            ("file_count", "File rows recorded (0 when nothing changed)."),
        ],
    },
    CatalogView {
        name: "v_measure",
        description: "Measures: the catalog of fact types (what a v_fact.value means).",
        columns: &[
            ("key", "Namespaced key, e.g. `oxplow.coverage`."),
            ("title", "Display name."),
            ("unit", "Unit of value, if any."),
            ("subject_kind", "What a fact is about: `symbol`, `file`, `test`, `model`, …"),
            ("temporal_semantics", "How values combine over time: `additive`, `semi-additive` or `non-additive`."),
            ("scope", "`built-in`, `global` or `project`."),
            ("description", "What the measure means."),
        ],
    },
    CatalogView {
        name: "v_capture",
        description: "Captures: one scan or run that produced facts, with its when/where/who context.",
        columns: &[
            ("id", "Capture id."),
            ("stream_id", "Stream captured in."),
            ("thread_id", "Thread, if known."),
            ("effort_id", "Producing effort, when unambiguous."),
            ("producer", "What produced it (collector, gauge, hook…)."),
            ("status", "`running`, `done` or `failed`."),
            ("trigger", "What started it."),
            ("provenance", "`observed` (measured) or `asserted` (an agent claimed it)."),
            ("source", "Source identifier."),
            ("snapshot_id", "Snapshot it was taken against, if any."),
            ("branch", "Git branch at capture."),
            ("closest_git_version", "Nearest git commit."),
            ("captured_at", "RFC 3339 timestamp."),
            ("ended_at", "When the capture finished."),
            ("scan_kind", "`delta` or `full`."),
        ],
    },
    CatalogView {
        name: "v_fact",
        description: "Facts: atomic measurements (a function's complexity, a test's outcome, a token count), joined to their measure and capture context.",
        columns: &[
            ("id", "Fact id."),
            ("capture_id", "Producing capture (v_capture.id)."),
            ("measure_key", "Measure this is a value of (v_measure.key)."),
            ("value", "The measured value."),
            ("numerator", "Ratio numerator, for ratio measures."),
            ("denominator", "Ratio denominator, for ratio measures."),
            ("subject_kind", "What the fact is about."),
            ("subject_ref", "Logical id of the subject."),
            ("path", "File path, when the subject has one."),
            ("line", "Line number, when it has one."),
            ("severity", "Finding severity, for lint-like facts."),
            ("rule", "Rule id, for lint-like facts."),
            ("detail", "Free-text detail."),
            ("dims_json", "Extra dimensions as a JSON object."),
            ("stream_id", "From the capture."),
            ("thread_id", "From the capture."),
            ("effort_id", "From the capture."),
            ("captured_at", "From the capture."),
            ("branch", "From the capture."),
        ],
    },
    CatalogView {
        name: "v_effort_file",
        description: "Files each effort touched (as claimed or detected at close), with how they changed.",
        columns: &[
            ("effort_id", "The effort (v_effort.id)."),
            ("work_item", "That effort's work item ref."),
            ("task_id", "That effort's oxplow task (v_task.id); NULL for another provider's work item."),
            ("path", "Repo-relative file path."),
            ("change_kind", "`created`, `updated` or `deleted`."),
            ("closest_git_version", "Nearest git commit when recorded."),
        ],
    },
    CatalogView {
        name: "v_task_note",
        description: "Notes: progress notes attached to a task, or thread-level notes (exactly one of task_id / thread_id is set).",
        columns: &[
            ("id", "Note id."),
            ("task_id", "Task it's attached to, if a task note."),
            ("thread_id", "Thread it's attached to, if a thread note."),
            ("body", "Markdown body."),
            ("author", "Who wrote it (`agent`, `user`, …)."),
            ("created_at", "RFC 3339 timestamp."),
        ],
    },
    CatalogView {
        name: "v_task_link",
        description: "Typed links between tasks (one blocks another, duplicates it, …).",
        columns: &[
            ("id", "Link id."),
            ("thread_id", "Thread the link was made in."),
            ("from_task_id", "Source task (v_task.id)."),
            ("to_task_id", "Target task (v_task.id)."),
            ("link_type", "`blocks`, `relates_to`, `discovered_from`, `duplicates`, `supersedes` or `replies_to`."),
            ("created_at", "RFC 3339 timestamp."),
        ],
    },
    CatalogView {
        name: "v_agent_turn",
        description: "Agent turns: each human prompt and the agent's answer, per thread. start_snapshot_id → snapshot_id is what the turn changed (the snapshot the worktree was at when it opened and when it ended).",
        columns: &[
            ("id", "Turn id."),
            ("thread_id", "Thread the turn ran in."),
            ("prompt", "What the human typed."),
            ("answer", "The agent's final answer, once the turn ended."),
            ("session_id", "The harness session id, when it reported one."),
            ("started_at", "RFC 3339 timestamp."),
            ("ended_at", "RFC 3339 timestamp; NULL while running."),
            ("start_snapshot_id", "Snapshot the worktree was at when the turn opened (v_snapshot.id); NULL if the stream had none yet."),
            ("snapshot_id", "Snapshot the worktree was at when the turn ended (v_snapshot.id); NULL while running."),
        ],
    },
    CatalogView {
        name: "v_token_usage",
        description: "Model token usage recorded from agent sessions, per thread, effort and model.",
        columns: &[
            ("id", "Row id."),
            ("stream_id", "Stream."),
            ("thread_id", "Thread."),
            ("effort_id", "Effort open at the time, if any."),
            ("turn_id", "Turn it happened in (v_agent_turn.id), when known."),
            ("agent_kind", "Harness (`claude`, `codex`, …)."),
            ("model", "Model id, if reported."),
            ("prompt", "The human prompt that opened this turn, if the transcript had one."),
            ("input_tokens", "Input tokens."),
            ("output_tokens", "Output tokens."),
            ("cache_creation_input_tokens", "Tokens written to the prompt cache."),
            ("cache_read_input_tokens", "Tokens read from the prompt cache."),
            ("message_count", "Messages in this record."),
            ("recorded_at", "RFC 3339 timestamp."),
        ],
    },
    CatalogView {
        name: "v_event",
        description: "The event log: every activity and state change, oldest first by `seq`, written in the same transaction as the change it records (.context/data-model.md \"event_log\").",
        columns: &[
            ("seq", "Insert order — the delivery order consumers checkpoint on."),
            ("id", "Public identity (UUIDv7; sorts by time)."),
            ("type", "`namespace.name`, e.g. `work_item.transitioned`."),
            ("v", "Schema version of `type`."),
            ("at", "RFC 3339 timestamp."),
            ("source", "What emitted it: `human`, `agent:thr3`, `task_service`, …"),
            ("stream_id", "Anchor, when known."),
            ("thread_id", "Anchor, when known."),
            ("effort_id", "Anchor, when known."),
            ("turn_id", "Anchor (agent_turn), when known."),
            ("snapshot_id", "Anchor, when known."),
            ("subject", "JSON array of the canonical refs the event is about."),
            ("payload", "JSON, validated against `type@v`'s schema."),
            ("payload_hash", "Content hash when the body lives in the content store."),
            ("payload_expired_at", "When retention replaced the payload with `{}` (the envelope stays); NULL while it's kept."),
            ("cause", "The event id that caused this one."),
            ("dedupe_key", "Emitter-derived key; unique."),
        ],
    },
    CatalogView {
        name: "v_event_content",
        description: "Large or sensitive event bodies (tool input and output, prompts), stored by content hash; an event payload's `{hash, size}` points here. Retention deletes a body after its namespace's window; the event stays. The bytes are read with `read_event_content` by event id, not through SQL.",
        columns: &[
            ("hash", "xxh3-128 hex of the bytes."),
            ("namespace", "The event namespace it belongs to (`agent`, `test`, …); sets its retention."),
            ("size", "Length in bytes."),
            ("created_at", "RFC 3339 timestamp."),
        ],
    },
    CatalogView {
        name: "v_event_dead_letter",
        description: "Events a consumer failed on, parked with the error. `pending` letters need a person's retry or discard.",
        columns: &[
            ("id", "Letter id (for `retry_dead_letter` / `discard_dead_letter`)."),
            ("consumer", "The consumer that failed."),
            ("event_seq", "`v_event.seq`."),
            ("event_id", "`v_event.id`."),
            ("event_type", "`v_event.type`."),
            ("error", "The last failure."),
            ("attempts", "How many times it has failed."),
            ("first_failed_at", "RFC 3339 timestamp."),
            ("last_failed_at", "RFC 3339 timestamp."),
            ("state", "`pending`, `retried` or `discarded`."),
        ],
    },
    CatalogView {
        name: "v_event_checkpoint",
        description: "How far each event consumer has read.",
        columns: &[
            ("consumer", "Consumer name."),
            ("last_seq", "Last `v_event.seq` it handled or skipped."),
            ("updated_at", "RFC 3339 timestamp."),
        ],
    },
    CatalogView {
        name: "v_page_visit",
        description: "Pages the human opened in oxplow, and for how long — what they've been looking at.",
        columns: &[
            ("id", "Visit id."),
            ("thread_id", "Thread the page was opened in."),
            ("page_kind", "Page kind (`task`, `file`, `lens`, …)."),
            ("page_id", "Page id (`task:42`, `file:src/a.rs`, …)."),
            ("label", "Title shown at the time."),
            ("visited_at", "RFC 3339 timestamp."),
            ("duration_ms", "Time on the page, when known."),
        ],
    },
    CatalogView {
        name: "v_decision",
        description: "Decisions: forks the agent resolved while working (what it chose, what it didn't, why), recorded by the agent or inferred by oxplow. What a reviewer most wants to check.",
        columns: &[
            ("id", "Decision id."),
            ("thread_id", "Thread it was made in."),
            ("task_id", "Task being worked on, if known."),
            ("effort_id", "Effort it was made during, if one was open."),
            ("turn_id", "Turn it happened in (v_agent_turn.id), when known."),
            ("question", "What had to be decided."),
            ("choice", "What was chosen."),
            ("alternatives", "Options not taken, as a JSON array of strings."),
            ("confidence", "`low`, `medium` or `high`."),
            ("why", "The reasoning."),
            ("provenance", "`recorded` (the agent recorded it) or `inferred` (oxplow's summarize model proposed it from the effort's activity; unconfirmed)."),
            ("created_at", "RFC 3339 timestamp."),
        ],
    },
    CatalogView {
        name: "v_claim",
        description: "Claims the agent made about its work (\"tests pass\", \"no behavior change\"), and whether anything backs them.",
        columns: &[
            ("id", "Claim id."),
            ("thread_id", "Thread it was made in."),
            ("task_id", "Task, if known."),
            ("effort_id", "Effort, if one was open."),
            ("turn_id", "Turn it happened in (v_agent_turn.id), when known."),
            ("statement", "The claim in words."),
            ("kind", "`tests_pass`, `no_behavior_change`, `handles_case` or `other`."),
            ("evidence_ref", "What backs it (`run:<id>`, a test, a file); NULL if nothing was cited."),
            ("verified", "1 if it cites evidence, or is `tests_pass` and its effort's latest test run (in `v_test_run`) passed with at least one test; else 0."),
            ("created_at", "RFC 3339 timestamp."),
        ],
    },
    CatalogView {
        name: "v_tool_call",
        description: "Every tool call the agent made (from the PostToolUse hook): reads, edits, commands.",
        columns: &[
            ("id", "Row id."),
            ("thread_id", "Thread."),
            ("effort_id", "Effort open at the time, if any."),
            ("turn_id", "Turn it happened in (v_agent_turn.id), when known."),
            ("tool", "Tool name (`Read`, `Edit`, `Bash`, `Grep`, …)."),
            ("path", "File it touched, repo-relative when inside the project."),
            ("detail", "Short context: the Bash command (truncated), a search pattern, the question an `await_user` call asked, …"),
            ("ok", "1 succeeded, 0 failed, NULL unknown (Bash often reports no exit code)."),
            ("at", "RFC 3339 timestamp."),
            ("event_id", "The `agent.tool.finished` event (v_event.id) this row records; NULL for rows written before the event log carried tool calls."),
        ],
    },
    CatalogView {
        name: "v_context_read",
        description: "Which `.context/*.md` project docs the agent read, and when. Compare with the files it changed to spot work done without reading the relevant doc.",
        columns: &[
            ("id", "Row id."),
            ("thread_id", "Thread."),
            ("effort_id", "Effort, if one was open."),
            ("path", "Doc path, e.g. `.context/usability.md`."),
            ("at", "RFC 3339 timestamp."),
        ],
    },
    CatalogView {
        name: "v_struggle",
        description: "Where the agent had trouble in an effort: a file edited 5+ times (`repeated_edits`) or 3+ failed commands (`failed_commands`). Likely places for mistakes.",
        columns: &[
            ("effort_id", "Effort."),
            ("thread_id", "Thread."),
            ("kind", "`repeated_edits` or `failed_commands`."),
            ("subject", "The file path, or `Bash`."),
            ("count", "How many edits / failures."),
        ],
    },
    CatalogView {
        name: "v_metric_spec",
        description: "Metric definitions: what each metric aggregates, which direction is good, and its target / warn / fail thresholds. Whether a metric is enabled lives in project.yaml, not here.",
        columns: &[
            ("key", "Metric key, e.g. `oxplow.coverage.abs_pct`."),
            ("title", "Display name."),
            ("unit", "Unit, e.g. `%`, `ms`; NULL for counts."),
            ("source_measure", "Measure whose facts it aggregates; NULL for a formula metric."),
            ("aggregation", "How facts combine within a capture: count, sum, avg, min, max, last, p95, ratio, …"),
            ("direction", "`higher-better`, `lower-better` or `neutral`."),
            ("target", "Goal value, if any."),
            ("warn_at", "Warning threshold, if any (read with `direction`)."),
            ("fail_at", "Failure threshold, if any (read with `direction`)."),
            ("description", "What it measures."),
            ("category", "Grouping on the Metrics page."),
            ("language", "Language it applies to, if language-specific."),
            ("scope", "`built-in`, `global`, `project` or `extension`."),
            ("display_kind", "`gauge`, `findings`, `test`, `coverage` or `event`."),
            ("extension", "The extension that declares it, when `scope` is `extension`."),
        ],
    },
    CatalogView {
        name: "v_agent_nudge",
        description: "Guidance oxplow sent the coding agent mid-effort (e.g. coverage below target, a test run with no report).",
        columns: &[
            ("id", "Row id."),
            ("thread_id", "Thread."),
            ("effort_id", "Effort it was about, if any."),
            ("turn_id", "Turn it happened in (v_agent_turn.id), when known."),
            ("kind", "What kind of nudge, e.g. `coverage-target`, `report-less-run`."),
            ("message", "The text the agent was given."),
            ("trigger", "What prompted it, if recorded."),
            ("created_at", "RFC 3339 timestamp."),
            ("delivered_at", "When a hook response carried it to the agent; NULL while it waits for the thread's next hook."),
        ],
    },
    CatalogView {
        name: "v_code_quality_scan",
        description: "Code-quality scans (duplication and similar): when each ran, over which tree version, and whether it finished.",
        columns: &[
            ("id", "Scan id."),
            ("tool", "Scanner, e.g. `duplication`."),
            ("scope", "What was scanned."),
            ("status", "`pending`, `running`, `done` or `failed`."),
            ("started_at", "RFC 3339 timestamp."),
            ("ended_at", "RFC 3339 timestamp; NULL while running."),
            ("error", "Failure message, if it failed."),
            ("tree_version_kind", "Which tree: `disk`, `git`, …"),
            ("tree_version_value", "The version (a sha, etc.)."),
            ("file_filter", "Fingerprint of the file filter it ran with, if any."),
        ],
    },
    CatalogView {
        name: "v_code_quality_finding",
        description: "Findings from code-quality scans, e.g. duplicated blocks, each with its scan's tool and time.",
        columns: &[
            ("id", "Finding id."),
            ("scan_id", "The scan (v_code_quality_scan.id)."),
            ("tool", "Scanner that found it."),
            ("path", "Repo-relative file."),
            ("start_line", "First line."),
            ("end_line", "Last line."),
            ("kind", "Finding kind, e.g. `duplicate-block`."),
            ("metric_value", "Size or score (lines, for a duplicate)."),
            ("extra_json", "Kind-specific JSON, e.g. the duplicate's peer path and lines."),
            ("scanned_at", "When its scan started (RFC 3339)."),
        ],
    },
    CatalogView {
        name: "v_dashboard",
        description: "User-created dashboards.",
        columns: &[
            ("id", "Dashboard id."),
            ("title", "Title."),
            ("sort_index", "Order in the dashboards list."),
            ("created_at", "RFC 3339 timestamp."),
            ("updated_at", "RFC 3339 timestamp."),
        ],
    },
    CatalogView {
        name: "v_dashboard_item",
        description: "Tiles on dashboards: a metric, a lens or a text note.",
        columns: &[
            ("id", "Tile id."),
            ("dashboard_id", "Its dashboard (v_dashboard.id)."),
            ("sort_index", "Position on the dashboard."),
            ("kind", "`metric`, `lens` or `text`."),
            ("metric_key", "The metric, for a `metric` tile."),
            ("options_json", "Tile options (size, view, lens id, text), JSON."),
        ],
    },
    CatalogView {
        name: "v_effort_metric_delta",
        description: "How each metric moved during an effort (before → after), computed by oxplow's metric engine and refreshed when the effort closes and as data arrives while it's open.",
        columns: &[
            ("effort_id", "Effort (v_effort.id)."),
            ("key", "Metric key (v_metric_spec.key)."),
            ("title", "Metric name."),
            ("unit", "Unit, if any."),
            ("direction", "`higher-better`, `lower-better` or `neutral`."),
            ("kind", "`gauge`, `coverage`, `test`, `event`, …"),
            ("category", "Metric category."),
            ("language", "Language, if language-specific."),
            ("agg", "How the delta was computed: `files` (over the effort's claimed files), `sum` (in-window total) or `level` (before→after)."),
            ("baseline", "Value as the effort began; NULL for `sum`."),
            ("current", "Value at the effort's end (or now, if open)."),
            ("delta", "current − baseline, or the total for `sum`."),
            ("changed", "1 if the value moved during the effort."),
            ("attributed_files", "For `files`: how many claimed files carry the metric."),
            ("sample_count", "Samples considered."),
            ("target", "Metric target."),
            ("warn_at", "Warning threshold."),
            ("fail_at", "Failure threshold."),
            ("crossing", "`warn` or `fail` when the current value sits past that threshold; else NULL."),
            ("latest_capture_id", "The latest contributing capture (v_capture.id), for drill-in."),
            ("refreshed_at", "When oxplow last recomputed it (RFC 3339)."),
        ],
    },
    CatalogView {
        name: "v_effort_observation",
        description: "Evidence gathered during an effort: test runs, diff coverage and analysis results, rebuilt from the captures the effort claimed.",
        columns: &[
            ("effort_id", "Effort (v_effort.id)."),
            ("seq", "Order within the effort."),
            ("kind", "`test-run`, `diff-coverage`, `static-analysis`, …"),
            ("provenance", "`observed` (oxplow saw it) or `asserted` (the agent reported it)."),
            ("source", "Where it came from, e.g. `post-tool-bash`."),
            ("metric_value", "Headline number, e.g. coverage %."),
            ("payload_json", "Kind-specific detail, JSON (e.g. test cases, covered/uncovered lines)."),
            ("local_snapshot_id", "Snapshot it was captured against."),
            ("created_at", "RFC 3339 timestamp."),
        ],
    },
    CatalogView {
        name: "v_commit",
        description: "Git commits the indexer has read (the last 500 reachable from the primary worktree's HEAD, kept as it moves).",
        columns: &[
            ("sha", "Full commit sha."),
            ("author", "Author name."),
            ("email", "Author email."),
            ("committed_at", "Commit time (RFC 3339)."),
            ("subject", "First line of the message."),
            ("body", "The rest of the message."),
            ("first_parent", "First parent's sha; NULL for a root commit."),
            ("parent_count", "Number of parents (2+ for a merge)."),
        ],
    },
    CatalogView {
        name: "v_commit_file",
        description: "Files each commit changed, against its first parent.",
        columns: &[
            ("sha", "The commit (v_commit.sha)."),
            ("path", "Repo-relative path."),
            ("status", "`added`, `modified`, `deleted` or `renamed`."),
            ("additions", "Lines added."),
            ("deletions", "Lines deleted."),
        ],
    },
    CatalogView {
        name: "v_commit_task",
        description: "Tasks a commit's message mentions (`tsk42`, `[[tsk42]]`).",
        columns: &[
            ("sha", "The commit (v_commit.sha)."),
            ("task_id", "The task (v_task.id)."),
        ],
    },
    CatalogView {
        name: "v_branch",
        description: "Local and remote-tracking branches, refreshed on boot and whenever refs move.",
        columns: &[
            ("name", "Branch name (without the remote)."),
            ("kind", "`local` or `remote`."),
            ("remote", "Remote name for a remote-tracking branch."),
            ("head_sha", "The commit it points at."),
            ("stream_id", "The stream whose worktree has it checked out (v_stream.id), if any."),
            ("updated_at", "When oxplow last read it (RFC 3339)."),
        ],
    },
    CatalogView {
        name: "v_test_run",
        description: "Every test run oxplow saw (hook-observed or reported by an agent), with its counts.",
        columns: &[
            ("id", "Run id; `run:<id>` is how claims and decisions cite it."),
            ("stream_id", "Stream it ran in (v_stream.id)."),
            ("thread_id", "Thread whose agent ran it, if known."),
            ("effort_id", "Effort it belongs to: the one that claimed it, else the one open when it ran."),
            ("command", "The command that was run."),
            ("exit_code", "Its exit code, if known."),
            ("passed", "Tests passed (NULL when not reported)."),
            ("failed", "Tests failed (NULL when not reported)."),
            ("skipped", "Tests skipped (NULL when not reported)."),
            ("total", "Tests run in total (NULL when not reported)."),
            ("duration_ms", "Wall-clock duration, if known."),
            ("provenance", "`observed` (oxplow parsed a report) or `asserted` (an agent reported counts)."),
            ("source", "What recorded it (`hook`, `mcp`, …)."),
            ("branch", "Branch checked out when it ran."),
            ("closest_git_version", "Commit the tested tree was at or nearest to."),
            ("captured_at", "When it was recorded (RFC 3339)."),
        ],
    },
    CatalogView {
        name: "v_test_case",
        description: "Each test case in a run that produced a report (runs with only counts have none).",
        columns: &[
            ("run_id", "The run (v_test_run.id)."),
            ("suite", "Report suite name."),
            ("classname", "Grouping path (module path, file·class, describe path)."),
            ("name", "Test name."),
            ("status", "`passed`, `failed` or `skipped`."),
            ("time_ms", "Its duration, when the report gave one."),
        ],
    },
    CatalogView {
        name: "v_diagnostic",
        description: "Errors and warnings the language servers have published for open or indexed files, right now (cleared when a server restarts).",
        columns: &[
            ("stream_id", "Stream whose worktree it's in (v_stream.id)."),
            ("language", "The language server that reported it."),
            ("path", "Repo-relative file."),
            ("severity", "`error`, `warning`, `information` or `hint`."),
            ("message", "The diagnostic text."),
            ("source", "The tool behind it (`rustc`, `ts`, `eslint`, …), if given."),
            ("code", "Its code (`E0308`, `2304`, …), if given."),
            ("line", "Start line (1-based)."),
            ("col", "Start column (1-based)."),
            ("end_line", "End line (1-based)."),
            ("end_col", "End column (1-based)."),
            ("updated_at", "When the server last published this file (RFC 3339)."),
        ],
    },
    CatalogView {
        name: "v_change",
        description: "Analyzed changes: a commit (vs its parent), an effort (start → end snapshot, or → working tree while open), an agent turn (start → end snapshot: what the turn changed) or a stream's working tree (vs HEAD). Created on demand by `ensure_change`; the v_change_* views hold its analysis.",
        columns: &[
            ("id", "Change id (the change_id in v_change_*)."),
            ("stream_id", "Stream whose repo it's in."),
            ("kind", "`commit`, `effort`, `turn` or `working`."),
            ("target", "The commit sha, the effort row id, the turn row id, or '' for the working tree."),
            ("base_label", "What it's compared against (a sha, `HEAD`, `snapshot N`)."),
            ("head_label", "The newer side."),
            ("status", "`pending`, `running`, `done` or `failed`."),
            ("error", "Why it failed, if it did."),
            ("computed_at", "When the analysis last finished (RFC 3339)."),
        ],
    },
    CatalogView {
        name: "v_change_file",
        description: "Each file a change touched, with its zone, whether it's a test, and a \"look here first\" interest score (size × complexity spikes × parameter growth × long new functions).",
        columns: &[
            ("change_id", "The change (v_change.id)."),
            ("path", "Repo-relative path."),
            ("status", "`added`, `modified` or `deleted`."),
            ("additions", "Lines added."),
            ("deletions", "Lines deleted."),
            ("zone", "Architectural zone from the project's zone rules (`other` when none match)."),
            ("is_test", "1 if the path looks like a test file."),
            ("interest", "Review priority score; higher means look here first."),
            ("interest_reasons", "Why the score is high, `; `-separated."),
        ],
    },
    CatalogView {
        name: "v_change_function",
        description: "Functions a change added, deleted or modified (signature and/or body), with metric deltas and per-function churn. Unchanged functions aren't listed.",
        columns: &[
            ("change_id", "The change (v_change.id)."),
            ("path", "File."),
            ("container", "Enclosing class/impl/module path, `::`-joined; '' at top level."),
            ("name", "Function name."),
            ("status", "`added`, `deleted` or `modified`."),
            ("signature_changed", "1 if its parameter count changed."),
            ("body_changed", "1 if its complexity or length changed."),
            ("start_line", "First line (head side, or base side if deleted)."),
            ("visibility", "`public`, `private` or `unknown`."),
            ("is_test", "1 if it's a test (test file, test name or test container)."),
            ("complexity", "Cyclomatic complexity (head side, or base if deleted)."),
            ("length", "Length in lines."),
            ("params_before", "Parameter count before; NULL if added."),
            ("params_after", "Parameter count after; NULL if deleted."),
            ("complexity_delta", "Complexity change, for modified functions."),
            ("length_delta", "Length change, for modified functions."),
            ("added_lines", "Lines added inside it."),
            ("deleted_lines", "Lines deleted inside it."),
            ("modified_lines", "min(added, deleted): edited both ways."),
            ("churn_share", "Its share of the change's function churn (0–1)."),
        ],
    },
    CatalogView {
        name: "v_change_import",
        description: "Imports a change added or removed, with source and target zones. `cross_zone` flags a new import that crosses an architectural boundary.",
        columns: &[
            ("change_id", "The change (v_change.id)."),
            ("path", "Importing file."),
            ("module", "What's imported, as written."),
            ("direction", "`added` or `removed`."),
            ("start_line", "Line of the import."),
            ("from_zone", "The importing file's zone."),
            ("to_zone", "The target's zone, if known (`external` for packages)."),
            ("cross_zone", "1 for an added import into a different known zone."),
        ],
    },
    CatalogView {
        name: "v_change_co_change",
        description: "Files in a change that history says are surprising: their usual co-changers aren't in this change, or they haven't been touched in a long time.",
        columns: &[
            ("change_id", "The change (v_change.id)."),
            ("path", "The surprising file."),
            ("reason", "`usual-co-changers-absent` or `dormant`."),
            ("expected", "The files that usually change with it, `, `-separated."),
            ("dormant_days", "Days since it was last touched, for `dormant`."),
        ],
    },
    CatalogView {
        name: "v_change_test_file",
        description: "How much each changed file's tests check, before and after the change: test functions, assertions and skip markers. Files that are tests or contain tests. The counts are a heuristic across languages (assert*/expect(/t.Error calls; #[ignore], .skip(, xit(, @Disabled, t.Skip markers), not a parse.",
        columns: &[
            ("change_id", "The change (v_change.id)."),
            ("path", "The file."),
            ("tests_before", "Test functions on the base side."),
            ("tests_after", "Test functions on the head side."),
            ("assertions_before", "Assertion calls on the base side."),
            ("assertions_after", "Assertion calls on the head side."),
            ("skips_before", "Skip markers on the base side."),
            ("skips_after", "Skip markers on the head side."),
        ],
    },
    CatalogView {
        name: "v_change_duplicate",
        description: "Blocks in a change's files that duplicate code elsewhere in the tree. Arrives after the rest of the analysis (a whole-tree scan); none for closed efforts yet.",
        columns: &[
            ("change_id", "The change (v_change.id)."),
            ("path", "The changed file."),
            ("start_line", "First line of the block."),
            ("end_line", "Last line of the block."),
            ("lines", "Block size in lines."),
            ("peer_path", "Where the copy is."),
            ("peer_start_line", "First line of the copy."),
            ("peer_end_line", "Last line of the copy."),
        ],
    },
    CatalogView {
        name: "v_ai_call",
        description: "Every model call oxplow itself made (not the coding agent's): role, provider, model, tokens, latency and whether it succeeded.",
        columns: &[
            ("id", "Row id."),
            ("role", "Role used (`main`, `fast`, `summarize`, `embed`, `decide`, `review`)."),
            ("provider", "Provider id from ai.yaml."),
            ("model", "Model id."),
            ("caller", "What asked, e.g. `mcp:ai_decide`, `inferred-decisions`."),
            ("input_tokens", "Input tokens."),
            ("output_tokens", "Output tokens."),
            ("latency_ms", "Round-trip time."),
            ("ok", "1 succeeded, 0 failed."),
            ("error", "Error message when it failed."),
            ("at", "RFC 3339 timestamp."),
        ],
    },
];

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
                        (42, 1, 1, NULL, 'test-run', 'asserted', 'mcp', '2026-01-03T00:00:00Z', ?2)",
                [&payload, &counts_only],
            )?;
            // Run 42 was claimed by effort 8 at close.
            c.execute_batch(
                "INSERT INTO effort_attribution (effort_id, kind, ref, state, recorded_at)
                   VALUES (8, 'run', 'run:42', 'claimed', '2026-01-03');",
            )
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
        let reads = |sql: &'static str| {
            let sl = sl.clone();
            async move { sl.query_sql(sql, vec![], None).await.unwrap().reads }
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
            .query_sql("SELECT count(*) FROM task", vec![], None)
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

    #[tokio::test]
    async fn schema_docs_match_the_views_exactly() {
        let (db, sl) = seeded().await;
        let entities = sl.describe_schema().await.unwrap();
        let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();
        for expected in [
            "v_stream",
            "v_thread",
            "v_task",
            "v_effort",
            "v_comment",
            "v_wiki_page",
            "v_snapshot",
            "v_snapshot_op",
            "v_measure",
            "v_capture",
            "v_fact",
            "v_effort_file",
            "v_task_note",
            "v_task_link",
            "v_agent_turn",
            "v_token_usage",
            "v_page_visit",
            "v_event",
            "v_event_content",
            "v_event_dead_letter",
            "v_event_checkpoint",
            "v_decision",
            "v_claim",
            "v_tool_call",
            "v_context_read",
            "v_struggle",
            "v_ai_call",
            "v_metric_spec",
            "v_agent_nudge",
            "v_code_quality_scan",
            "v_code_quality_finding",
            "v_dashboard",
            "v_dashboard_item",
            "v_effort_metric_delta",
            "v_effort_observation",
            "v_change",
            "v_change_file",
            "v_change_function",
            "v_change_import",
            "v_change_co_change",
            "v_change_duplicate",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
        for e in &entities {
            assert_eq!(e.owner, "core");
            assert!(!e.description.is_empty(), "{} has no description", e.name);
            let name = e.name.clone();
            let actual: Vec<String> = db
                .call(move |c| {
                    let mut st = c.prepare(&format!("PRAGMA table_info({name})"))?;
                    let cols = st.query_map([], |r| r.get::<_, String>(1))?;
                    cols.collect()
                })
                .await
                .unwrap();
            let documented: Vec<String> = e.columns.iter().map(|c| c.name.clone()).collect();
            assert_eq!(documented, actual, "column docs drifted for {}", e.name);
            for c in &e.columns {
                assert!(!c.doc.is_empty(), "{}.{} has no doc", e.name, c.name);
            }
        }
    }
}
