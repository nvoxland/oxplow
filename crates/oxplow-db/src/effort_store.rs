//! Task effort tracking.
//!
//! An "effort" is one continuous push of agent work on a single task,
//! bounded by snapshots at start and end. This module owns:
//!
//! - `task_effort` (the effort row)
//! - `task_effort_file` (per-effort file changes)
//! - `task_effort_turn` (link to agent_turn rows)

use async_trait::async_trait;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, EffortId, TaskId, TaskImpact, ThreadId, Timestamp};

use crate::database::Database;
use crate::page_ref_projections::{
    effort_impact_edges, effort_ref_types, effort_summary_edges, effort_touched_file_edges,
    KIND_TASK,
};
use crate::page_ref_store::SqlitePageRefStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EffortFileChange {
    Created,
    Updated,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct TaskEffort {
    pub id: EffortId,
    pub task_id: TaskId,
    pub thread_id: ThreadId,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    pub start_snapshot_id: Option<i64>,
    pub end_snapshot_id: Option<i64>,
    /// The effort's summary prose — the canonical text.
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct EffortFile {
    pub effort_id: EffortId,
    pub path: String,
    pub change: EffortFileChange,
    /// The snapshot the file ref was captured at. Always set since
    /// V20; 0 only on pre-V20 rows whose owning effort had no
    /// snapshot pin.
    pub local_snapshot_id: i64,
    /// Closest known git commit at capture time. See V20 column
    /// docs. NULL when no git information is available (no commits
    /// yet, headless repo, etc.).
    pub closest_git_version: Option<String>,
    /// `true` when `local_snapshot_id`'s snapshot is byte-equal to
    /// `closest_git_version` (clean worktree at capture, or
    /// auto-resolved later by `set_snapshot_git_commit`).
    pub git_version_exact: bool,
}

/// Snapshot-pinned version data for a file reference. The
/// store/service layer computes this from a snapshot id at capture
/// time and stamps it onto every per-file ref row.
#[derive(Debug, Clone, Copy)]
pub struct FileRefVersion<'a> {
    pub local_snapshot_id: i64,
    pub closest_git_version: Option<&'a str>,
    pub git_version_exact: bool,
}

/// Owned variant of [`FileRefVersion`] for callers that need to move
/// the triple into a `'static` transaction closure.
#[derive(Debug, Clone)]
pub struct OwnedFileRefVersion {
    pub local_snapshot_id: i64,
    pub closest_git_version: Option<String>,
    pub git_version_exact: bool,
}

impl OwnedFileRefVersion {
    pub fn as_ref(&self) -> FileRefVersion<'_> {
        FileRefVersion {
            local_snapshot_id: self.local_snapshot_id,
            closest_git_version: self.closest_git_version.as_deref(),
            git_version_exact: self.git_version_exact,
        }
    }
}

/// One user-visible attribution action for
/// [`SqliteTaskEffortStore::record_effort_atomic`]: merge files,
/// impacts, and a summary into the task's current effort (opening one
/// if none exists) — committed as a single transaction.
#[derive(Debug, Clone)]
pub struct RecordEffortAtomic {
    pub task: TaskId,
    pub thread: ThreadId,
    /// `(path, change)` pairs; callers pre-filter empty paths.
    pub files: Vec<(String, EffortFileChange)>,
    /// Version triple stamped on every file row. Resolved by the
    /// caller BEFORE the transaction (it reads the snapshot store) —
    /// advisory metadata, so a racing effort change between resolve
    /// and commit only yields a slightly stale pin, never bad rows.
    pub version: OwnedFileRefVersion,
    pub impacts: Vec<TaskImpact>,
    pub summary: Option<String>,
}

/// One (snapshot, effort) pair returned from
/// `list_efforts_at_snapshots`. The renderer derives
/// `completed_here` as `effort.end_snapshot_id == Some(snapshot_id)`;
/// every other row is "in flight at this snapshot."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct EffortAtSnapshot {
    pub snapshot_id: i64,
    pub effort: TaskEffort,
}

fn ts_to_string(ts: Timestamp) -> String {
    serde_json::to_string(&ts)
        .expect("Timestamp serializes to JSON")
        .trim_matches('"')
        .to_string()
}

// ---------------------------------------------------------------------------
// Sync `_tx` cores — connection-parameterized so they compose inside a
// single `Database::transaction` closure (a `rusqlite::Transaction`
// derefs to `Connection`). The async trait methods below are thin
// wrappers over these; multi-write actions like `record_effort_atomic`
// compose several cores in one transaction. See `.context/data-model.md`,
// "Transactions".
// ---------------------------------------------------------------------------

pub(crate) fn start_tx(
    conn: &rusqlite::Connection,
    task: TaskId,
    thread: ThreadId,
    start_snapshot_id: Option<i64>,
    now: Timestamp,
) -> rusqlite::Result<EffortId> {
    conn.execute(
        "INSERT INTO task_effort
           (id, task_id, thread_id, started_at, ended_at,
            start_snapshot_id, end_snapshot_id, summary)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5, NULL, NULL)",
        params![
            None::<i64>,
            task.value(),
            thread.value(),
            ts_to_string(now),
            start_snapshot_id,
        ],
    )?;
    Ok(EffortId::new(conn.last_insert_rowid()))
}

pub(crate) fn finish_tx(
    conn: &rusqlite::Connection,
    id: EffortId,
    end_snapshot_id: Option<i64>,
    summary: Option<&str>,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE task_effort
         SET ended_at = ?2, end_snapshot_id = ?3, summary = ?4
         WHERE id = ?1 AND ended_at IS NULL",
        params![id.value(), now, end_snapshot_id, summary],
    )?;
    Ok(())
}

fn set_summary_tx(
    conn: &rusqlite::Connection,
    id: EffortId,
    summary: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE task_effort SET summary = ?2 WHERE id = ?1",
        params![id.value(), summary],
    )?;
    Ok(())
}

fn set_impacts_json_tx(
    conn: &rusqlite::Connection,
    id: EffortId,
    json: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE task_effort SET impacts_json = ?2 WHERE id = ?1",
        params![id.value(), json],
    )?;
    Ok(())
}

fn record_file_tx(
    conn: &rusqlite::Connection,
    id: EffortId,
    path: &str,
    change: EffortFileChange,
    version: FileRefVersion<'_>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO task_effort_file
           (effort_id, path, change_kind,
            local_snapshot_id, closest_git_version, git_version_exact)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id.value(),
            path,
            change_to_str(change),
            version.local_snapshot_id,
            version.closest_git_version,
            if version.git_version_exact { 1 } else { 0 },
        ],
    )?;
    Ok(())
}

pub(crate) fn find_open_for_task_tx(
    conn: &rusqlite::Connection,
    task: TaskId,
) -> rusqlite::Result<Option<TaskEffort>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM task_effort
         WHERE task_id = ?1 AND ended_at IS NULL
         ORDER BY started_at DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map(params![task.value()], row_to_effort)?;
    rows.next().transpose()
}

fn most_recent_for_task_tx(
    conn: &rusqlite::Connection,
    task: TaskId,
) -> rusqlite::Result<Option<TaskEffort>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM task_effort WHERE task_id = ?1
         ORDER BY started_at DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map(params![task.value()], row_to_effort)?;
    rows.next().transpose()
}

fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DomainError::Invalid(format!("bad timestamp: {e}")))
}

fn change_to_str(c: EffortFileChange) -> &'static str {
    match c {
        EffortFileChange::Created => "created",
        EffortFileChange::Updated => "updated",
        EffortFileChange::Deleted => "deleted",
    }
}

fn str_to_change(s: &str) -> Result<EffortFileChange, DomainError> {
    Ok(match s {
        "created" => EffortFileChange::Created,
        "updated" => EffortFileChange::Updated,
        "deleted" => EffortFileChange::Deleted,
        other => {
            return Err(DomainError::Invalid(format!(
                "unknown effort file change kind: {other}"
            )))
        }
    })
}

fn row_to_effort(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskEffort> {
    let id: i64 = row.get("id")?;
    let task_id: i64 = row.get("task_id")?;
    let thread_id: i64 = row.get("thread_id")?;
    let started_at: String = row.get("started_at")?;
    let ended_at: Option<String> = row.get("ended_at")?;
    let start_snapshot_id: Option<i64> = row.get("start_snapshot_id")?;
    let end_snapshot_id: Option<i64> = row.get("end_snapshot_id")?;
    let summary: Option<String> = row.get("summary")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(TaskEffort {
        id: EffortId::new(id),
        task_id: TaskId::new(task_id),
        thread_id: ThreadId::new(thread_id),
        started_at: string_to_ts(&started_at).map_err(map_err)?,
        ended_at: ended_at
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(map_err)?,
        start_snapshot_id,
        end_snapshot_id,
        summary,
    })
}

#[async_trait]
pub trait TaskEffortStore: Send + Sync {
    async fn start(
        &self,
        task: TaskId,
        thread: &ThreadId,
        start_snapshot_id: Option<i64>,
    ) -> Result<TaskEffort, DomainError>;
    async fn finish(
        &self,
        id: &EffortId,
        end_snapshot_id: Option<i64>,
        summary: Option<String>,
    ) -> Result<(), DomainError>;
    /// Record the LLM-declared cross-page impacts for an effort.
    /// Replaces any prior list. The store then re-projects the
    /// owning task's effort slice so impact edges show up in
    /// `page_ref` immediately.
    async fn set_impacts(&self, id: &EffortId, impacts: &[TaskImpact]) -> Result<(), DomainError>;
    async fn list_for_item(&self, item: TaskId) -> Result<Vec<TaskEffort>, DomainError>;
    /// Fetch a single effort row by id. Returns `None` when the row
    /// doesn't exist (e.g. cleared during snapshot prune).
    async fn get_effort(&self, id: &EffortId) -> Result<Option<TaskEffort>, DomainError>;
    /// Open effort (`ended_at IS NULL`) for `task`, if any. Used by
    /// the lifecycle path that opens an effort on in_progress entry
    /// and finishes it on exit, and by `record_effort` to merge
    /// touched-files into the lifecycle row instead of creating a
    /// duplicate.
    async fn find_open_for_task(&self, task: TaskId) -> Result<Option<TaskEffort>, DomainError>;
    /// Open effort (`ended_at IS NULL`) for `thread`, if any. The
    /// orchestrator keeps at most one item `in_progress` per thread, so
    /// this is the effort that hook-driven collection (test runs,
    /// coverage) attributes against. Newest open effort wins.
    async fn find_open_for_thread(
        &self,
        thread: &ThreadId,
    ) -> Result<Option<TaskEffort>, DomainError>;
    /// Most-recent effort for `task` regardless of state, or `None`
    /// when the task has never had one. Used by `record_effort` to
    /// reattach files to a just-closed lifecycle effort.
    async fn most_recent_for_task(&self, task: TaskId) -> Result<Option<TaskEffort>, DomainError>;
    /// Overwrite the summary on an already-finished effort. Used
    /// when `record_effort` runs after the lifecycle finish has
    /// already closed the row.
    async fn set_summary(&self, id: &EffortId, summary: Option<String>) -> Result<(), DomainError>;
    async fn list_files(&self, id: &EffortId) -> Result<Vec<EffortFile>, DomainError>;
    async fn list_impacts(&self, id: &EffortId) -> Result<Vec<TaskImpact>, DomainError>;
    async fn record_file(
        &self,
        id: &EffortId,
        path: &str,
        change: EffortFileChange,
        version: FileRefVersion<'_>,
    ) -> Result<(), DomainError>;
    /// For each snapshot in `snapshot_ids`, return every effort that
    /// was either active at that snapshot OR ending exactly at it.
    /// "Active at S" = `start_snapshot_id <= S` AND
    /// (`end_snapshot_id IS NULL` OR `end_snapshot_id >= S`).
    /// This is the labeling source for the Local History dashboard:
    /// each row shows in-flight + just-completed efforts.
    async fn list_efforts_at_snapshots(
        &self,
        snapshot_ids: Vec<i64>,
    ) -> Result<Vec<EffortAtSnapshot>, DomainError>;
    /// All distinct file paths whose `file_snapshot` rows fall inside
    /// this effort's snapshot bracket — i.e. the auto-diff for the
    /// effort. Returns empty when either `start_snapshot_id` or
    /// `end_snapshot_id` is NULL. Used by the effort-end
    /// reconciliation to compare against the LLM's claimed
    /// `touched_files`.
    async fn list_changed_paths_for_effort(
        &self,
        id: &EffortId,
    ) -> Result<Vec<String>, DomainError>;
    /// Remove specific `task_effort_file` rows. Companion to
    /// `record_file`. Used by the `amend_effort` MCP tool when the
    /// agent disclaims a path that the auto-diff thought was theirs.
    async fn remove_file(&self, id: &EffortId, path: &str) -> Result<(), DomainError>;
    /// Record that the agent explicitly disclaimed `path` for this
    /// effort. Survives Stop-hook recomputes so the same
    /// `changed_but_not_claimed` discrepancy doesn't re-fire the
    /// directive after a successful `amend_effort`. Idempotent.
    async fn acknowledge_unclaimed_path(
        &self,
        id: &EffortId,
        path: &str,
    ) -> Result<(), DomainError>;
    /// Drop a prior acknowledgement. Called when the agent re-claims
    /// a path via `amend_effort(add_files=…)` after having previously
    /// disclaimed it.
    async fn forget_acknowledged_path(&self, id: &EffortId, path: &str) -> Result<(), DomainError>;
    /// All paths the agent has explicitly acknowledged as
    /// not-mine-but-in-the-diff for this effort.
    async fn list_acknowledged_paths(&self, id: &EffortId) -> Result<Vec<String>, DomainError>;
    /// Paths claimed (via `task_effort_file`) by OTHER efforts whose
    /// snapshot window OVERLAPS this effort's window (not merely ends
    /// inside it): `other.start < self.end AND (other.end IS NULL OR
    /// other.end > self.start)`. Such a path changed during this
    /// effort's bracket but another (possibly later-completed) effort
    /// already owns it, so we shouldn't ask this one to claim it too —
    /// regardless of the order the sibling efforts were completed in.
    async fn paths_claimed_by_intervening_efforts(
        &self,
        id: &EffortId,
    ) -> Result<Vec<String>, DomainError>;
}

#[derive(Clone)]
pub struct SqliteTaskEffortStore {
    db: Database,
    page_refs: SqlitePageRefStore,
}

impl SqliteTaskEffortStore {
    pub fn new(db: Database) -> Self {
        Self {
            page_refs: SqlitePageRefStore::new(db.clone()),
            db,
        }
    }

    /// Re-emit the full effort-owned slice for `task_id` — the
    /// union of touched-file edges, the parsed wikilink/file/dir/
    /// task/finding/commit refs pulled from every effort's
    /// `summary` body, and the declared `TaskImpact` rows.
    /// Replaces under `effort_ref_types()` so the task-body slice
    /// (owned by `task_store`) is unaffected.
    async fn project_effort_slice(&self, task_id: TaskId) -> Result<(), DomainError> {
        let refs = &self.page_refs;
        type SliceRows = (Vec<(String, String)>, Vec<String>, Vec<String>);
        let (paths, summaries, impact_jsons): SliceRows = self
            .db
            .call(move |conn| {
                // Pick the most-recent `change_kind` per path across
                // every effort on this task. "Most recent" = the
                // effort with the latest `started_at`. The window
                // function isolates rn=1 so each path appears once.
                let mut path_stmt = conn.prepare(
                    "SELECT path, change_kind FROM (
                       SELECT f.path, f.change_kind,
                              ROW_NUMBER() OVER (
                                PARTITION BY f.path
                                ORDER BY e.started_at DESC
                              ) AS rn
                       FROM task_effort_file f
                       JOIN task_effort e ON e.id = f.effort_id
                       WHERE e.task_id = ?1
                     )
                     WHERE rn = 1
                     ORDER BY path",
                )?;
                let paths: Vec<(String, String)> = path_stmt
                    .query_map(params![task_id.value()], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut sum_stmt = conn.prepare(
                    "SELECT summary FROM task_effort
                      WHERE task_id = ?1
                        AND summary IS NOT NULL
                        AND summary <> ''
                      ORDER BY started_at",
                )?;
                let summaries: Vec<String> = sum_stmt
                    .query_map(params![task_id.value()], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut imp_stmt = conn.prepare(
                    "SELECT impacts_json FROM task_effort
                      WHERE task_id = ?1
                        AND impacts_json IS NOT NULL
                        AND impacts_json <> ''
                      ORDER BY started_at",
                )?;
                let impact_jsons: Vec<String> = imp_stmt
                    .query_map(params![task_id.value()], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok((paths, summaries, impact_jsons))
            })
            .await?;
        let mut impacts: Vec<TaskImpact> = Vec::new();
        for j in &impact_jsons {
            match serde_json::from_str::<Vec<TaskImpact>>(j) {
                Ok(rows) => impacts.extend(rows),
                Err(e) => {
                    tracing::warn!(?e, "effort impacts_json deserialize failed; skipping");
                }
            }
        }
        let id_str = task_id.to_string();
        let mut edges = effort_touched_file_edges(&id_str, &paths);
        edges.extend(effort_summary_edges(&id_str, &summaries));
        edges.extend(effort_impact_edges(&id_str, &impacts));
        refs.replace_source_for_ref_types(KIND_TASK, &id_str, effort_ref_types(), edges)
            .await
    }

    /// One attribution action — start-if-missing + files + impacts +
    /// finish/summary — committed as a single transaction (composing
    /// the `_tx` cores above), then ONE post-commit page_ref slice
    /// projection. Replaces the old 3+N separate statements where a
    /// crash mid-way left files recorded with no summary/finish.
    /// Returns the effort the action landed on.
    pub async fn record_effort_atomic(
        &self,
        args: RecordEffortAtomic,
    ) -> Result<EffortId, DomainError> {
        use crate::database::map_sql_err;
        let task = args.task;
        let a = std::sync::Arc::new(args);
        let effort_id = self
            .db
            .transaction(move |tx| {
                let existing = most_recent_for_task_tx(tx, a.task).map_err(map_sql_err)?;
                let (effort_id, open) = match &existing {
                    Some(e) => (e.id, e.ended_at.is_none()),
                    None => (
                        start_tx(tx, a.task, a.thread, None, Timestamp::now())
                            .map_err(map_sql_err)?,
                        true,
                    ),
                };
                let version = a.version.as_ref();
                for (path, change) in &a.files {
                    record_file_tx(tx, effort_id, path, *change, version).map_err(map_sql_err)?;
                }
                if !a.impacts.is_empty() {
                    let json = serde_json::to_string(&a.impacts).map_err(|e| {
                        DomainError::Invalid(format!("impacts serialize failed: {e}"))
                    })?;
                    set_impacts_json_tx(tx, effort_id, Some(&json)).map_err(map_sql_err)?;
                }
                if open {
                    // No lifecycle close happened (or this is the
                    // freshly-started fallback) — close with the
                    // summary; end_snapshot_id stays NULL because this
                    // is attribution, not a status transition.
                    finish_tx(
                        tx,
                        effort_id,
                        None,
                        a.summary.as_deref(),
                        &ts_to_string(Timestamp::now()),
                    )
                    .map_err(map_sql_err)?;
                } else if a.summary.is_some() {
                    // Lifecycle finish already closed the row but left
                    // summary NULL — backfill it.
                    set_summary_tx(tx, effort_id, a.summary.as_deref()).map_err(map_sql_err)?;
                }
                Ok(effort_id)
            })
            .await?;
        self.project_effort_slice(task).await?;
        Ok(effort_id)
    }

    /// Every open effort row (`ended_at IS NULL`) across all tasks.
    /// Used by boot recovery to heal lifecycle orphans.
    pub async fn list_all_open(&self) -> Result<Vec<TaskEffort>, DomainError> {
        self.db
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM task_effort WHERE ended_at IS NULL ORDER BY started_at",
                )?;
                let rows = stmt.query_map([], row_to_effort)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Backfill the start-snapshot pin on an effort opened by the
    /// transactional lifecycle transition. The snapshot is requested
    /// AFTER that transaction commits, so a snapshot failure degrades
    /// to "effort without a pin" rather than "no effort row".
    pub async fn set_start_snapshot(
        &self,
        id: &EffortId,
        snapshot_id: i64,
    ) -> Result<(), DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE task_effort SET start_snapshot_id = ?2 WHERE id = ?1",
                    params![id.value(), snapshot_id],
                )?;
                Ok(())
            })
            .await
    }

    /// Backfill the end-snapshot pin. See [`Self::set_start_snapshot`].
    pub async fn set_end_snapshot(
        &self,
        id: &EffortId,
        snapshot_id: i64,
    ) -> Result<(), DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE task_effort SET end_snapshot_id = ?2 WHERE id = ?1",
                    params![id.value(), snapshot_id],
                )?;
                Ok(())
            })
            .await
    }

    async fn task_for_effort(&self, effort_id: &EffortId) -> Result<Option<TaskId>, DomainError> {
        let id = *effort_id;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT task_id FROM task_effort WHERE id = ?1")?;
                let mut rows = stmt.query_map(params![id.value()], |r| r.get::<_, i64>(0))?;
                Ok(rows.next().transpose()?.map(TaskId::new))
            })
            .await
    }
}

#[async_trait]
impl TaskEffortStore for SqliteTaskEffortStore {
    async fn start(
        &self,
        task: TaskId,
        thread: &ThreadId,
        start_snapshot_id: Option<i64>,
    ) -> Result<TaskEffort, DomainError> {
        let thread = *thread;
        let now = Timestamp::now();
        let id = self
            .db
            .call(move |conn| start_tx(conn, task, thread, start_snapshot_id, now))
            .await?;
        Ok(TaskEffort {
            id,
            task_id: task,
            thread_id: thread,
            started_at: now,
            ended_at: None,
            start_snapshot_id,
            end_snapshot_id: None,
            summary: None,
        })
    }

    async fn finish(
        &self,
        id: &EffortId,
        end_snapshot_id: Option<i64>,
        summary: Option<String>,
    ) -> Result<(), DomainError> {
        let id_for_sql = *id;
        let summary_has_body = summary
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let now = ts_to_string(Timestamp::now());
        self.db
            .call(move |conn| {
                finish_tx(conn, id_for_sql, end_snapshot_id, summary.as_deref(), &now)
            })
            .await?;
        if summary_has_body {
            if let Some(tid) = self.task_for_effort(id).await? {
                self.project_effort_slice(tid).await?;
            }
        }
        Ok(())
    }

    async fn find_open_for_task(&self, task: TaskId) -> Result<Option<TaskEffort>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM task_effort
                     WHERE task_id = ?1 AND ended_at IS NULL
                     ORDER BY started_at DESC LIMIT 1",
                )?;
                let mut rows = stmt.query_map(params![task.value()], row_to_effort)?;
                rows.next().transpose()
            })
            .await
    }

    async fn find_open_for_thread(
        &self,
        thread: &ThreadId,
    ) -> Result<Option<TaskEffort>, DomainError> {
        let thread = *thread;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM task_effort
                     WHERE thread_id = ?1 AND ended_at IS NULL
                     ORDER BY started_at DESC LIMIT 1",
                )?;
                let mut rows = stmt.query_map(params![thread.value()], row_to_effort)?;
                rows.next().transpose()
            })
            .await
    }

    async fn most_recent_for_task(&self, task: TaskId) -> Result<Option<TaskEffort>, DomainError> {
        self.db
            .call(move |conn| most_recent_for_task_tx(conn, task))
            .await
    }

    async fn set_summary(&self, id: &EffortId, summary: Option<String>) -> Result<(), DomainError> {
        let id_for_sql = *id;
        self.db
            .call(move |conn| set_summary_tx(conn, id_for_sql, summary.as_deref()))
            .await?;
        {
            if let Some(tid) = self.task_for_effort(id).await? {
                self.project_effort_slice(tid).await?;
            }
        }
        Ok(())
    }

    async fn list_for_item(&self, item: TaskId) -> Result<Vec<TaskEffort>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM task_effort WHERE task_id = ?1
                     ORDER BY started_at DESC",
                )?;
                let rows = stmt.query_map(params![item.value()], row_to_effort)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn get_effort(&self, id: &EffortId) -> Result<Option<TaskEffort>, DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT * FROM task_effort WHERE id = ?1")?;
                let mut rows = stmt.query_map(params![id.value()], row_to_effort)?;
                rows.next().transpose()
            })
            .await
    }

    async fn list_files(&self, id: &EffortId) -> Result<Vec<EffortFile>, DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT effort_id, path, change_kind,
                            local_snapshot_id, closest_git_version, git_version_exact
                     FROM task_effort_file
                     WHERE effort_id = ?1 ORDER BY path ASC",
                )?;
                let rows = stmt.query_map(params![id.value()], |r| {
                    let effort_id: i64 = r.get(0)?;
                    let path: String = r.get(1)?;
                    let kind: String = r.get(2)?;
                    let local_snapshot_id: i64 = r.get(3)?;
                    let closest_git_version: Option<String> = r.get(4)?;
                    let git_version_exact: i64 = r.get(5)?;
                    let map_err = |e: DomainError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    };
                    Ok(EffortFile {
                        effort_id: EffortId::new(effort_id),
                        path,
                        change: str_to_change(&kind).map_err(map_err)?,
                        local_snapshot_id,
                        closest_git_version,
                        git_version_exact: git_version_exact != 0,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn set_impacts(&self, id: &EffortId, impacts: &[TaskImpact]) -> Result<(), DomainError> {
        let id_clone = *id;
        let json = if impacts.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(impacts)
                    .map_err(|e| DomainError::Invalid(format!("impacts serialize failed: {e}")))?,
            )
        };
        self.db
            .call(move |conn| set_impacts_json_tx(conn, id_clone, json.as_deref()))
            .await?;
        {
            if let Some(tid) = self.task_for_effort(id).await? {
                self.project_effort_slice(tid).await?;
            }
        }
        Ok(())
    }

    async fn list_impacts(&self, id: &EffortId) -> Result<Vec<TaskImpact>, DomainError> {
        let id = *id;
        let raw: Option<String> = self
            .db
            .call(move |conn| {
                let mut stmt =
                    conn.prepare("SELECT impacts_json FROM task_effort WHERE id = ?1")?;
                let mut rows =
                    stmt.query_map(params![id.value()], |r| r.get::<_, Option<String>>(0))?;
                Ok(rows.next().transpose()?.flatten())
            })
            .await?;
        match raw {
            Some(json) if !json.is_empty() => serde_json::from_str(&json)
                .map_err(|e| DomainError::Invalid(format!("impacts deserialize failed: {e}"))),
            _ => Ok(Vec::new()),
        }
    }

    async fn record_file(
        &self,
        id: &EffortId,
        path: &str,
        change: EffortFileChange,
        version: FileRefVersion<'_>,
    ) -> Result<(), DomainError> {
        let id_clone = *id;
        let owned = OwnedFileRefVersion {
            local_snapshot_id: version.local_snapshot_id,
            closest_git_version: version.closest_git_version.map(|s| s.to_string()),
            git_version_exact: version.git_version_exact,
        };
        let path_clone = path.to_string();
        self.db
            .call(move |conn| record_file_tx(conn, id_clone, &path_clone, change, owned.as_ref()))
            .await?;
        {
            if let Some(tid) = self.task_for_effort(id).await? {
                self.project_effort_slice(tid).await?;
            }
        }
        Ok(())
    }

    async fn list_changed_paths_for_effort(
        &self,
        id: &EffortId,
    ) -> Result<Vec<String>, DomainError> {
        let id_clone = *id;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT fs.path
                     FROM task_effort e
                     JOIN snapshot s_start ON s_start.id = e.start_snapshot_id
                     JOIN file_snapshot fs ON fs.stream_id = s_start.stream_id
                     WHERE e.id = ?1
                       AND e.start_snapshot_id IS NOT NULL
                       AND e.end_snapshot_id IS NOT NULL
                       AND fs.snapshot_id > e.start_snapshot_id
                       AND fs.snapshot_id <= e.end_snapshot_id
                     ORDER BY fs.path",
                )?;
                let rows =
                    stmt.query_map(params![id_clone.value()], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn remove_file(&self, id: &EffortId, path: &str) -> Result<(), DomainError> {
        let id_clone = *id;
        let path_clone = path.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM task_effort_file WHERE effort_id = ?1 AND path = ?2",
                    params![id_clone.value(), path_clone],
                )?;
                Ok(())
            })
            .await?;
        {
            if let Some(tid) = self.task_for_effort(id).await? {
                self.project_effort_slice(tid).await?;
            }
        }
        Ok(())
    }

    async fn acknowledge_unclaimed_path(
        &self,
        id: &EffortId,
        path: &str,
    ) -> Result<(), DomainError> {
        let id_clone = *id;
        let path_clone = path.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO effort_acknowledged_path (effort_id, path) \
                     VALUES (?1, ?2)",
                    params![id_clone.value(), path_clone],
                )?;
                Ok(())
            })
            .await
    }

    async fn forget_acknowledged_path(&self, id: &EffortId, path: &str) -> Result<(), DomainError> {
        let id_clone = *id;
        let path_clone = path.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM effort_acknowledged_path \
                     WHERE effort_id = ?1 AND path = ?2",
                    params![id_clone.value(), path_clone],
                )?;
                Ok(())
            })
            .await
    }

    async fn list_acknowledged_paths(&self, id: &EffortId) -> Result<Vec<String>, DomainError> {
        let id_clone = *id;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT path FROM effort_acknowledged_path \
                     WHERE effort_id = ?1 ORDER BY path",
                )?;
                let rows =
                    stmt.query_map(params![id_clone.value()], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn paths_claimed_by_intervening_efforts(
        &self,
        id: &EffortId,
    ) -> Result<Vec<String>, DomainError> {
        let id_clone = *id;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    // Any OTHER effort whose snapshot window OVERLAPS
                    // self's window (10,30] — not just one that ends
                    // inside it. Overlap of half-open intervals
                    // (a.start, a.end] and (b.start, b.end] is
                    // `a.start < b.end AND a.end > b.start`. An ongoing
                    // effort (NULL end) overlaps if it started before
                    // self's window closed. This way a sibling effort
                    // that's claimed later (ends after self's window)
                    // still suppresses the nag, regardless of the order
                    // the efforts were completed in.
                    "SELECT DISTINCT tef.path
                     FROM task_effort self
                     JOIN task_effort other
                       ON other.id != self.id
                      AND other.start_snapshot_id < self.end_snapshot_id
                      AND (other.end_snapshot_id IS NULL
                           OR other.end_snapshot_id > self.start_snapshot_id)
                     JOIN task_effort_file tef ON tef.effort_id = other.id
                     WHERE self.id = ?1
                       AND self.start_snapshot_id IS NOT NULL
                       AND self.end_snapshot_id IS NOT NULL
                     ORDER BY tef.path",
                )?;
                let rows =
                    stmt.query_map(params![id_clone.value()], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn list_efforts_at_snapshots(
        &self,
        snapshot_ids: Vec<i64>,
    ) -> Result<Vec<EffortAtSnapshot>, DomainError> {
        if snapshot_ids.is_empty() {
            return Ok(vec![]);
        }
        self.db
            .call(move |conn| {
                // Build a derived "wanted" set of snapshot ids via
                // SELECT…UNION ALL so the join can compare each input
                // snapshot against every effort interval.
                let mut union_parts: Vec<String> = Vec::with_capacity(snapshot_ids.len());
                for i in 1..=snapshot_ids.len() {
                    if i == 1 {
                        union_parts.push(format!("SELECT ?{i} AS snapshot_id"));
                    } else {
                        union_parts.push(format!("SELECT ?{i}"));
                    }
                }
                let sql = format!(
                    "SELECT s.snapshot_id, e.* \
                     FROM ({}) s \
                     JOIN task_effort e \
                       ON e.start_snapshot_id IS NOT NULL \
                      AND e.start_snapshot_id <= s.snapshot_id \
                      AND (e.end_snapshot_id IS NULL OR e.end_snapshot_id >= s.snapshot_id) \
                     ORDER BY s.snapshot_id DESC, e.started_at ASC",
                    union_parts.join(" UNION ALL ")
                );
                let mut stmt = conn.prepare(&sql)?;
                let params_iter: Vec<&dyn rusqlite::ToSql> = snapshot_ids
                    .iter()
                    .map(|id| id as &dyn rusqlite::ToSql)
                    .collect();
                let rows = stmt.query_map(rusqlite::params_from_iter(params_iter), |row| {
                    let snapshot_id: i64 = row.get("snapshot_id")?;
                    let effort = row_to_effort(row)?;
                    Ok(EffortAtSnapshot {
                        snapshot_id,
                        effort,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_store::SqliteStreamStore;
    use crate::task_store::SqliteTaskStore;
    use crate::thread_store::SqliteThreadStore;
    use oxplow_domain::stores::{StreamStore, TaskStore, ThreadStore};
    use oxplow_domain::{
        Stream, StreamId, StreamKind, Task, TaskActorKind, TaskAuthor, TaskPriority, TaskStatus,
        Thread, ThreadStatus,
    };

    async fn fixture() -> (SqliteTaskEffortStore, TaskId, ThreadId) {
        let (store, _db, tid, thread) = fixture_with_db().await;
        (store, tid, thread)
    }

    #[tokio::test]
    async fn intervening_efforts_claims_overlapping_window() {
        // self effort spans snapshots (10, 30]. We report a path when
        // the claiming effort's window *overlaps* self's window,
        // regardless of completion order:
        //   ef-inside  (15, 20]  fully inside        → reported
        //   ef-after   (25, 40]  starts in, ends out → reported (the
        //                        sibling-completed-later case)
        //   ef-before  ( 1,  5]  entirely before     → not reported
        //   ef-later   (35, 50]  entirely after      → not reported
        let db = Database::in_memory();
        let store = SqliteTaskEffortStore::new(db.clone());
        let db2 = db.clone();
        tokio::task::spawn_blocking(move || {
            db2.with_conn(|conn| {
                conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
                for (id, start, end) in [
                    (1, 10, 30), // ef-self
                    (2, 15, 20), // ef-inside
                    (3, 25, 40), // ef-after
                    (4, 1, 5),   // ef-before
                    (5, 35, 50), // ef-later
                ] {
                    conn.execute(
                        "INSERT INTO task_effort
                           (id, task_id, thread_id, started_at, ended_at,
                            start_snapshot_id, end_snapshot_id)
                         VALUES (?1, 1, 1, '2026-01-01T00:00:00Z',
                                 '2026-01-01T00:01:00Z', ?2, ?3)",
                        params![id, start, end],
                    )?;
                }
                for (eid, path) in [
                    (2, "inside.rs"), // ef-inside
                    (3, "after.rs"),  // ef-after
                    (4, "before.rs"), // ef-before
                    (5, "later.rs"),  // ef-later
                ] {
                    conn.execute(
                        "INSERT INTO task_effort_file
                           (effort_id, path, change_kind, local_snapshot_id,
                            closest_git_version, git_version_exact)
                         VALUES (?1, ?2, 'updated', 1, NULL, 0)",
                        params![eid, path],
                    )?;
                }
                Ok(())
            })
        })
        .await
        .unwrap()
        .unwrap();

        let got = store
            .paths_claimed_by_intervening_efforts(&EffortId::new(1))
            .await
            .unwrap();
        // Ordered by path; overlapping efforts only.
        assert_eq!(got, vec!["after.rs".to_string(), "inside.rs".to_string()]);
    }

    async fn fixture_with_db() -> (SqliteTaskEffortStore, Database, TaskId, ThreadId) {
        let db = Database::in_memory();
        let now = Timestamp::from_unix_ms(1);
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/p".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteStreamStore::new(db.clone()).upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::new(1),
            stream_id: s.id,
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteThreadStore::new(db.clone()).upsert(&t).await.unwrap();
        let tid = SqliteTaskStore::new(db.clone())
            .insert(&Task {
                id: TaskId::placeholder(),
                thread_id: Some(t.id),
                parent_id: None,
                title: "x".into(),
                description: String::new(),
                status: TaskStatus::Ready,
                priority: TaskPriority::Medium,
                sort_index: 0,
                created_by: TaskActorKind::User,
                created_at: now,
                updated_at: now,
                completed_at: None,
                deleted_at: None,
                note_count: 0,
                author: Some(TaskAuthor::User),
            })
            .await
            .unwrap();
        (SqliteTaskEffortStore::new(db.clone()), db, tid, t.id)
    }

    #[tokio::test]
    async fn start_then_finish_round_trips() {
        let (store, tid, t) = fixture().await;
        let eff = store.start(tid, &t, None).await.unwrap();
        assert!(eff.ended_at.is_none());
        store
            .finish(&eff.id, None, Some("done".into()))
            .await
            .unwrap();
        let list = store.list_for_item(tid).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].ended_at.is_some());
        assert_eq!(list[0].summary.as_deref(), Some("done"));
    }

    fn atomic_args(
        tid: TaskId,
        thread: ThreadId,
        files: Vec<(String, EffortFileChange)>,
        summary: Option<&str>,
    ) -> RecordEffortAtomic {
        RecordEffortAtomic {
            task: tid,
            thread,
            files,
            version: OwnedFileRefVersion {
                local_snapshot_id: 0,
                closest_git_version: None,
                git_version_exact: false,
            },
            impacts: Vec::new(),
            summary: summary.map(|s| s.to_string()),
        }
    }

    #[tokio::test]
    async fn record_effort_atomic_opens_records_and_closes_in_one_action() {
        let (store, tid, t) = fixture().await;
        let eff = store
            .record_effort_atomic(atomic_args(
                tid,
                t,
                vec![
                    ("src/a.rs".into(), EffortFileChange::Updated),
                    ("src/b.rs".into(), EffortFileChange::Created),
                ],
                Some("shipped"),
            ))
            .await
            .unwrap();
        let row = store.get_effort(&eff).await.unwrap().unwrap();
        assert!(row.ended_at.is_some(), "fresh effort is closed");
        assert_eq!(row.summary.as_deref(), Some("shipped"));
        assert_eq!(store.list_files(&eff).await.unwrap().len(), 2);
        assert!(store.find_open_for_task(tid).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn record_effort_atomic_merges_into_open_lifecycle_effort() {
        let (store, tid, t) = fixture().await;
        let lifecycle = store.start(tid, &t, None).await.unwrap();
        let eff = store
            .record_effort_atomic(atomic_args(
                tid,
                t,
                vec![("src/a.rs".into(), EffortFileChange::Updated)],
                Some("done"),
            ))
            .await
            .unwrap();
        // Merged into the lifecycle row, not a duplicate.
        assert_eq!(eff, lifecycle.id);
        let row = store.get_effort(&eff).await.unwrap().unwrap();
        assert!(row.ended_at.is_some());
        assert_eq!(row.summary.as_deref(), Some("done"));
    }

    #[tokio::test]
    async fn record_effort_atomic_backfills_summary_on_closed_effort() {
        let (store, tid, t) = fixture().await;
        let eff = store.start(tid, &t, None).await.unwrap();
        store.finish(&eff.id, None, None).await.unwrap();
        let landed = store
            .record_effort_atomic(atomic_args(tid, t, Vec::new(), Some("late summary")))
            .await
            .unwrap();
        assert_eq!(landed, eff.id);
        let row = store.get_effort(&eff.id).await.unwrap().unwrap();
        assert_eq!(row.summary.as_deref(), Some("late summary"));
    }

    fn task_row(id: TaskId, thread: ThreadId, status: TaskStatus) -> Task {
        let now = Timestamp::from_unix_ms(2);
        Task {
            id,
            thread_id: Some(thread),
            parent_id: None,
            title: "x".into(),
            description: String::new(),
            status,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
        }
    }

    #[tokio::test]
    async fn transition_opens_and_finishes_effort_with_status_flip() {
        use crate::task_store::EffortTransition;
        let (store, db, tid, t) = fixture_with_db().await;
        let tasks = SqliteTaskStore::new(db.clone());

        let entering = tasks
            .update_with_effort_transition(&task_row(tid, t, TaskStatus::InProgress), t, true)
            .await
            .unwrap();
        let EffortTransition::Opened(eff) = entering else {
            panic!("expected Opened, got {entering:?}");
        };
        let open = store.find_open_for_task(tid).await.unwrap().unwrap();
        assert_eq!(open.id, eff);
        assert!(open.start_snapshot_id.is_none(), "pin backfills later");

        // Entering again (e.g. Busy retry / re-issued transition)
        // adopts the open row instead of tripping the V31 index.
        let again = tasks
            .update_with_effort_transition(&task_row(tid, t, TaskStatus::InProgress), t, true)
            .await
            .unwrap();
        assert_eq!(again, EffortTransition::Opened(eff));

        let leaving = tasks
            .update_with_effort_transition(&task_row(tid, t, TaskStatus::Done), t, false)
            .await
            .unwrap();
        assert_eq!(leaving, EffortTransition::Finished(eff));
        assert!(store.find_open_for_task(tid).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn transition_on_missing_task_rolls_back_effort_open() {
        let (store, db, _tid, t) = fixture_with_db().await;
        let tasks = SqliteTaskStore::new(db.clone());
        let ghost = TaskId::new(9999);
        let err = tasks
            .update_with_effort_transition(&task_row(ghost, t, TaskStatus::InProgress), t, true)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound), "got {err:?}");
        // The whole action rolled back — no effort row for the ghost.
        assert!(store.find_open_for_task(ghost).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn second_open_effort_for_same_task_is_a_constraint() {
        // The V31 partial unique index enforces the lifecycle
        // invariant: at most one open effort per task. A double-open
        // must surface as a typed Constraint, never silently diverge.
        let (store, tid, t) = fixture().await;
        store.start(tid, &t, None).await.unwrap();
        let err = store.start(tid, &t, None).await.unwrap_err();
        assert!(matches!(err, DomainError::Constraint(_)), "got {err:?}");
        // Finishing the open row frees the slot.
        let open = store.find_open_for_task(tid).await.unwrap().unwrap();
        store.finish(&open.id, None, None).await.unwrap();
        store.start(tid, &t, None).await.unwrap();
    }

    #[tokio::test]
    async fn record_then_list_files() {
        let (store, tid, t) = fixture().await;
        let eff = store.start(tid, &t, None).await.unwrap();
        let v = FileRefVersion {
            local_snapshot_id: 0,
            closest_git_version: None,
            git_version_exact: false,
        };
        store
            .record_file(&eff.id, "src/a.rs", EffortFileChange::Created, v)
            .await
            .unwrap();
        store
            .record_file(&eff.id, "src/b.rs", EffortFileChange::Updated, v)
            .await
            .unwrap();
        let files = store.list_files(&eff.id).await.unwrap();
        assert_eq!(files.len(), 2);
    }

    #[tokio::test]
    async fn finish_projects_summary_refs_into_page_ref() {
        use crate::page_ref_store::SqlitePageRefStore;
        let (_, db, tid, t) = fixture_with_db().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let store = SqliteTaskEffortStore::new(db);
        let eff = store.start(tid, &t, None).await.unwrap();
        store
            .finish(
                &eff.id,
                None,
                Some("Filed [[url-schemes]] referencing [[src/foo.rs]] and tsk99".into()),
            )
            .await
            .unwrap();

        let wiki_back = page_refs
            .list_backlinks("wiki", "url-schemes", None)
            .await
            .unwrap();
        assert!(
            wiki_back.iter().any(|e| e.source_kind == "task"
                && e.source_id == tid.to_string()
                && e.ref_type == "summary_wikilink"),
            "wiki backlink missing; got {wiki_back:?}"
        );

        let file_back = page_refs
            .list_backlinks("file", "src/foo.rs", None)
            .await
            .unwrap();
        assert!(
            file_back
                .iter()
                .any(|e| e.ref_type == "summary_file_ref" && e.source_id == tid.to_string()),
            "file backlink missing; got {file_back:?}"
        );

        let task_back = page_refs
            .list_backlinks("task", "tsk99", None)
            .await
            .unwrap();
        assert!(
            task_back
                .iter()
                .any(|e| e.ref_type == "summary_task_mention" && e.source_id == tid.to_string()),
            "task backlink missing; got {task_back:?}"
        );
    }

    #[tokio::test]
    async fn set_impacts_projects_edges_and_round_trips() {
        use crate::page_ref_store::SqlitePageRefStore;
        use oxplow_domain::TaskImpact;
        let (_, db, tid, t) = fixture_with_db().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let store = SqliteTaskEffortStore::new(db);
        let eff = store.start(tid, &t, None).await.unwrap();
        let impacts = vec![
            TaskImpact {
                kind: "wiki".into(),
                id: "url-schemes".into(),
                action: Some("created".into()),
            },
            TaskImpact {
                kind: "git_commit".into(),
                id: "abc1234".into(),
                action: Some("referenced".into()),
            },
        ];
        store.set_impacts(&eff.id, &impacts).await.unwrap();

        // Round-trip read
        let listed = store.list_impacts(&eff.id).await.unwrap();
        assert_eq!(listed, impacts);

        // Edges projected with normalized target kind + action extra
        let wiki = page_refs
            .list_backlinks("wiki", "url-schemes", None)
            .await
            .unwrap();
        let row = wiki
            .iter()
            .find(|e| e.source_id == tid.to_string())
            .expect("wiki impact edge missing");
        assert_eq!(row.ref_type, "impact");
        assert!(row
            .source_extra
            .as_deref()
            .is_some_and(|s| s.contains("created")));

        let commit = page_refs
            .list_backlinks("git-commit", "abc1234", None)
            .await
            .unwrap();
        assert!(commit
            .iter()
            .any(|e| e.source_id == tid.to_string() && e.ref_type == "impact"));

        // Replacing the impact set clears old edges
        store
            .set_impacts(
                &eff.id,
                &[TaskImpact {
                    kind: "wiki".into(),
                    id: "other-page".into(),
                    action: None,
                }],
            )
            .await
            .unwrap();
        let wiki = page_refs
            .list_backlinks("wiki", "url-schemes", None)
            .await
            .unwrap();
        assert!(
            wiki.iter().all(|e| e.source_id != tid.to_string()),
            "old wiki impact edge wasn't replaced: {wiki:?}"
        );

        // Empty list nulls the column and clears all impact edges
        store.set_impacts(&eff.id, &[]).await.unwrap();
        let wiki = page_refs
            .list_backlinks("wiki", "other-page", None)
            .await
            .unwrap();
        assert!(wiki.iter().all(|e| e.source_id != tid.to_string()));
        assert!(store.list_impacts(&eff.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn record_file_keeps_summary_slice_alive() {
        // Regression: when a later effort records a touched file via
        // `record_file`, the projection helper re-runs and must still
        // include summary edges from earlier-finished efforts.
        use crate::page_ref_store::SqlitePageRefStore;
        let (_, db, tid, t) = fixture_with_db().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let store = SqliteTaskEffortStore::new(db);
        let first = store.start(tid, &t, None).await.unwrap();
        store
            .finish(&first.id, None, Some("Filed [[url-schemes]]".into()))
            .await
            .unwrap();

        let second = store.start(tid, &t, None).await.unwrap();
        let v = FileRefVersion {
            local_snapshot_id: 0,
            closest_git_version: None,
            git_version_exact: false,
        };
        store
            .record_file(&second.id, "src/bar.rs", EffortFileChange::Updated, v)
            .await
            .unwrap();

        let wiki_back = page_refs
            .list_backlinks("wiki", "url-schemes", None)
            .await
            .unwrap();
        assert!(
            wiki_back.iter().any(|e| e.source_id == tid.to_string()),
            "summary slice was clobbered by record_file: {wiki_back:?}"
        );
    }

    #[tokio::test]
    async fn list_efforts_at_snapshots_buckets_active_and_completed() {
        let db = Database::in_memory();
        let now = Timestamp::from_unix_ms(1);
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/p".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteStreamStore::new(db.clone()).upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::new(1),
            stream_id: s.id,
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteThreadStore::new(db.clone()).upsert(&t).await.unwrap();
        let tid = SqliteTaskStore::new(db.clone())
            .insert(&Task {
                id: TaskId::placeholder(),
                thread_id: Some(t.id),
                parent_id: None,
                title: "x".into(),
                description: String::new(),
                status: TaskStatus::Ready,
                priority: TaskPriority::Medium,
                sort_index: 0,
                created_by: TaskActorKind::User,
                created_at: now,
                updated_at: now,
                completed_at: None,
                deleted_at: None,
                note_count: 0,
                author: Some(TaskAuthor::User),
            })
            .await
            .unwrap();
        // task_effort.end_snapshot_id references snapshot(id), not
        // file_snapshot(id). Build real snapshot grouping rows so the
        // FK validates.
        let snap_store = crate::SqliteSnapshotStore::new(db.clone());
        let snap1 = snap_store.create_snapshot(s.id).await.unwrap();
        let snap2 = snap_store.create_snapshot(s.id).await.unwrap();
        let snap3 = snap_store.create_snapshot(s.id).await.unwrap();

        let store = SqliteTaskEffortStore::new(db);
        // Effort A: start@snap1, end@snap2 — active at snap1 AND snap2
        // (ends exactly there); not active at snap3.
        let a = store.start(tid, &t.id, Some(snap1)).await.unwrap();
        store.finish(&a.id, Some(snap2), None).await.unwrap();
        // Effort B: start@snap2, still open — active at snap2 and
        // snap3.
        let b = store.start(tid, &t.id, Some(snap2)).await.unwrap();

        let rows = store
            .list_efforts_at_snapshots(vec![snap1, snap2, snap3])
            .await
            .unwrap();
        let bucket = |s: i64| -> Vec<&EffortId> {
            rows.iter()
                .filter(|r| r.snapshot_id == s)
                .map(|r| &r.effort.id)
                .collect()
        };
        assert_eq!(bucket(snap1), vec![&a.id]);
        // snap2 sees both — A ends here, B starts here.
        let at_snap2 = bucket(snap2);
        assert!(at_snap2.contains(&&a.id) && at_snap2.contains(&&b.id));
        assert_eq!(bucket(snap3), vec![&b.id]);
    }
}
