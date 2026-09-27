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
    fn to_sql(&self) -> rusqlite::types::Value {
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

    /// Run one read-only `SELECT`/`WITH` statement with positional
    /// `params` (`?1`, `?2`, …), capped at `limit` rows (default
    /// [`DEFAULT_ROW_LIMIT`], max [`MAX_ROW_LIMIT`]).
    pub async fn query_sql(
        &self,
        sql: &str,
        params: Vec<SqlCell>,
        limit: Option<usize>,
    ) -> Result<SqlQueryResult, DomainError> {
        self.query_sql_with_timeout(sql, params, limit, DEFAULT_TIMEOUT)
            .await
    }

    pub async fn query_sql_with_timeout(
        &self,
        sql: &str,
        params: Vec<SqlCell>,
        limit: Option<usize>,
        timeout: Duration,
    ) -> Result<SqlQueryResult, DomainError> {
        let binding = Binding::Positional(params.iter().map(SqlCell::to_sql).collect());
        self.run(sql, binding, limit, timeout).await
    }

    /// Like [`Self::query_sql`] but binds `:name` parameters. Names the
    /// statement doesn't reference are ignored, so a caller can pass a
    /// fixed parameter set to queries that use only some of it.
    pub async fn query_sql_named(
        &self,
        sql: &str,
        params: Vec<(String, SqlCell)>,
        limit: Option<usize>,
    ) -> Result<SqlQueryResult, DomainError> {
        let binding = Binding::Named(
            params
                .into_iter()
                .map(|(name, v)| (name, v.to_sql()))
                .collect(),
        );
        self.run(sql, binding, limit, DEFAULT_TIMEOUT).await
    }

    async fn run(
        &self,
        sql: &str,
        binding: Binding,
        limit: Option<usize>,
        timeout: Duration,
    ) -> Result<SqlQueryResult, DomainError> {
        check_leading_keyword(sql)?;
        let sql = sql.to_string();
        let cap = limit.unwrap_or(DEFAULT_ROW_LIMIT).clamp(1, MAX_ROW_LIMIT);
        self.db
            .call(move |conn| Ok(run_read_only(conn, &sql, &binding, cap, timeout)))
            .await?
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
                    });
                }
                Ok(out)
            })
            .await
    }
}

/// Reject anything that doesn't start with `SELECT` or `WITH`, after
/// leading whitespace and comments. Cheap first gate; the prepared
/// statement's `readonly()` check is the real one.
fn check_leading_keyword(sql: &str) -> Result<(), DomainError> {
    let mut rest = sql;
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix("--") {
            rest = r.split_once('\n').map(|(_, t)| t).unwrap_or("");
        } else if let Some(r) = rest.strip_prefix("/*") {
            rest = r.split_once("*/").map(|(_, t)| t).unwrap_or("");
        } else {
            break;
        }
    }
    let word: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    if word == "SELECT" || word == "WITH" {
        Ok(())
    } else {
        Err(DomainError::Invalid(
            "query_sql accepts a single SELECT or WITH statement".into(),
        ))
    }
}

/// How a query's parameters are supplied.
enum Binding {
    /// `?1`, `?2`, … in order.
    Positional(Vec<rusqlite::types::Value>),
    /// `:name` → value; names the statement doesn't use are skipped.
    Named(Vec<(String, rusqlite::types::Value)>),
}

impl Binding {
    fn apply(&self, stmt: &mut rusqlite::Statement<'_>) -> rusqlite::Result<()> {
        match self {
            Binding::Positional(vals) => {
                for (i, v) in vals.iter().enumerate() {
                    stmt.raw_bind_parameter(i + 1, v)?;
                }
            }
            Binding::Named(vals) => {
                for (name, v) in vals {
                    if let Some(idx) = stmt.parameter_index(&format!(":{name}"))? {
                        stmt.raw_bind_parameter(idx, v)?;
                    }
                }
            }
        }
        Ok(())
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

/// Execute under `PRAGMA query_only` with an interrupt timer, always
/// restoring the pooled connection to writable afterwards.
fn run_read_only(
    conn: &rusqlite::Connection,
    sql: &str,
    binding: &Binding,
    cap: usize,
    timeout: Duration,
) -> Result<SqlQueryResult, DomainError> {
    let invalid = |e: rusqlite::Error| DomainError::Invalid(format!("query_sql: {e}"));
    let mut stmt = conn.prepare(sql).map_err(invalid)?;
    if !stmt.readonly() {
        return Err(DomainError::Invalid(
            "query_sql accepts read-only statements only".into(),
        ));
    }
    conn.execute_batch("PRAGMA query_only = ON")
        .map_err(crate::database::map_sql_err)?;

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
        binding.apply(&mut stmt)?;
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
        })
    })();

    drop(done_tx);
    let _ = timer.join();
    let reset = conn.execute_batch("PRAGMA query_only = OFF");

    let out = result.map_err(|e| match e {
        rusqlite::Error::SqliteFailure(f, _)
            if f.code == rusqlite::ErrorCode::OperationInterrupted =>
        {
            DomainError::Invalid(format!("query_sql: timed out after {timeout:?}"))
        }
        other => invalid(other),
    });
    reset.map_err(crate::database::map_sql_err)?;
    out
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
            ("agent", "Agent harness: `claude`, `codex` or `opencode`."),
            ("sort_index", "Order within the stream."),
            ("created_at", "RFC 3339 timestamp."),
            ("updated_at", "RFC 3339 timestamp."),
            ("closed_at", "Set when the thread was closed."),
            ("archived_at", "Set when the thread was archived."),
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
        description: "Efforts: one in_progress → done/blocked span of work on a task, bracketed by snapshots.",
        columns: &[
            ("id", "Effort id."),
            ("task_id", "The task worked on (v_task.id)."),
            ("thread_id", "Thread that did the work."),
            ("stream_id", "That thread's stream."),
            ("started_at", "RFC 3339 timestamp."),
            ("ended_at", "RFC 3339 timestamp; NULL while the effort is open."),
            ("start_snapshot_id", "Snapshot at the start (v_snapshot.id)."),
            ("end_snapshot_id", "Snapshot at the end; NULL while open."),
            ("summary", "Closing summary written when the task was completed."),
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
            ("task_id", "That effort's task (v_task.id)."),
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
        name: "v_task_event",
        description: "Task history: every create / status change / edit, with who did it.",
        columns: &[
            ("id", "Event id."),
            ("thread_id", "Thread it happened in."),
            ("task_id", "Task it's about, if any."),
            ("event_type", "What happened (e.g. `created`, `status_changed`)."),
            ("actor_kind", "`user`, `agent` or `system`."),
            ("actor_id", "Who, within that kind."),
            ("payload_json", "Event details as JSON."),
            ("created_at", "RFC 3339 timestamp."),
        ],
    },
    CatalogView {
        name: "v_agent_turn",
        description: "Agent turns: each human prompt and the agent's answer, per thread (and task when known).",
        columns: &[
            ("id", "Turn id."),
            ("thread_id", "Thread the turn ran in."),
            ("task_id", "Task in progress at the time, if known."),
            ("prompt", "What the human typed."),
            ("answer", "The agent's final answer, once the turn ended."),
            ("started_at", "RFC 3339 timestamp."),
            ("ended_at", "RFC 3339 timestamp; NULL while running."),
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
            ("agent_kind", "Harness (`claude`, `codex`, …)."),
            ("model", "Model id, if reported."),
            ("input_tokens", "Input tokens."),
            ("output_tokens", "Output tokens."),
            ("cache_creation_input_tokens", "Tokens written to the prompt cache."),
            ("cache_read_input_tokens", "Tokens read from the prompt cache."),
            ("message_count", "Messages in this record."),
            ("recorded_at", "RFC 3339 timestamp."),
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
            .query_sql_named(
                "SELECT title FROM v_task WHERE status = :status AND id >= :min_id",
                vec![
                    ("status".into(), SqlCell::Text("in_progress".into())),
                    ("min_id".into(), SqlCell::Int(1)),
                    ("unused".into(), SqlCell::Int(9)),
                ],
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
            .query_sql_with_timeout(sql, vec![], None, Duration::from_millis(100))
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
            "v_measure",
            "v_capture",
            "v_fact",
            "v_effort_file",
            "v_task_note",
            "v_task_link",
            "v_task_event",
            "v_agent_turn",
            "v_token_usage",
            "v_page_visit",
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
