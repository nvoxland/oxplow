//! Task effort tracking.
//!
//! An "effort" is one continuous push of agent work on a single work
//! item (an oxplow task, `work_item:oxplow:tsk42`, or another provider's
//! item, `work_item:linear:ENG-12`),
//! bounded by snapshots at start and end. This module owns:
//!
//! - `effort` (the effort row)
//! - `effort_file` (per-effort file changes)

use async_trait::async_trait;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, EffortId, TaskId, TaskImpact, ThreadId, Timestamp};

use crate::database::map_sql_err;
use crate::database::Database;
use crate::database::{string_to_ts, ts_to_string};
use crate::event_log_store::{anchors_for_thread_tx, EventCtx};
use crate::page_ref_projections::{
    effort_impact_edges, effort_ref_types, effort_summary_edges, effort_touched_file_edges,
    KIND_WORK_ITEM,
};
use crate::page_ref_store::SqlitePageRefStore;
use oxplow_domain::events::schema::{EffortClosed, EffortClosedV1, EffortOpened, EffortOpenedV1};
use oxplow_domain::refs::build::{
    effort_ref, snapshot_ref, task_of_work_item_ref, thread_ref, validate_work_item_ref,
    work_item_id_of_ref,
};
use oxplow_domain::{Anchors, EventSchemaRegistry};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EffortFileChange {
    Created,
    Updated,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Effort {
    pub id: EffortId,
    /// The work item worked on, as a canonical `work_item` ref.
    pub work_item: String,
    pub thread_id: ThreadId,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    pub start_snapshot_id: Option<i64>,
    pub end_snapshot_id: Option<i64>,
    /// The effort's summary prose — the canonical text.
    pub summary: Option<String>,
}

impl Effort {
    /// The oxplow task this effort is on; `None` for another provider's
    /// work item.
    pub fn task_id(&self) -> Option<TaskId> {
        task_of_work_item_ref(&self.work_item)
    }
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

/// The snapshot-bracket changed paths for an effort, split by whether the
/// effort CLAIMED each one (via `effort_file`). Mirrors the
/// claimed/unclaimed attribution of the history view
/// (`apps/desktop/src/snapshot-effort-grouping.ts`): `claimed` =
/// changed-during-the-bracket AND claimed by this effort; `unclaimed` =
/// changed but never claimed (parallel/external writes, formatters, capture
/// gaps). Claim-first attribution, Child 3.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct EffortChangedPaths {
    pub claimed: Vec<String>,
    pub unclaimed: Vec<String>,
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
/// [`SqliteEffortStore::record_effort_atomic`]: merge files,
/// impacts, and a summary into the work item's current effort (opening
/// one if none exists) — committed as a single transaction.
#[derive(Debug, Clone)]
pub struct RecordEffortAtomic {
    /// A `work_item` ref (validated before the transaction).
    pub work_item: String,
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
    pub effort: Effort,
}

// ---------------------------------------------------------------------------
// Sync `_tx` cores — connection-parameterized so they compose inside a
// single `Database::transaction` closure (a `rusqlite::Transaction`
// derefs to `Connection`). The async trait methods below are thin
// wrappers over these; multi-write actions like `record_effort_atomic`
// compose several cores in one transaction. See `.context/data-model.md`,
// "Transactions".
// ---------------------------------------------------------------------------

/// Opens an effort on `work_item`, which the caller has validated
/// (`validate_work_item_ref`) or built with `work_item_ref`, and logs
/// `effort.opened@1` in the same transaction — every open, whichever path
/// made it (lifecycle, `record_effort_atomic`, recovery, a command).
pub fn start_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    work_item: &str,
    thread: ThreadId,
    start_snapshot_id: Option<i64>,
    now: Timestamp,
    retroactive: bool,
) -> Result<EffortId, DomainError> {
    conn.execute(
        "INSERT INTO effort
           (id, work_item, thread_id, started_at, ended_at,
            start_snapshot_id, end_snapshot_id, summary)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5, NULL, NULL)",
        params![
            None::<i64>,
            work_item,
            thread.value(),
            ts_to_string(now),
            start_snapshot_id,
        ],
    )
    .map_err(map_sql_err)?;
    let id = EffortId::new(conn.last_insert_rowid());
    let env = ev
        .typed::<EffortOpened>(&EffortOpenedV1 {
            effort: effort_ref(id),
            work_item: work_item.to_string(),
            thread: thread_ref(thread),
            start_snapshot: start_snapshot_id.map(snapshot_ref),
            retroactive,
        })
        .with_anchors(Anchors {
            effort_id: Some(id),
            snapshot_id: start_snapshot_id,
            ..anchors_for_thread_tx(conn, thread)?
        })
        .with_subject([effort_ref(id), work_item.to_string()]);
    ev.append(conn, &env)?;
    Ok(id)
}

/// Closes an open effort and logs `effort.closed@1` in the same
/// transaction. An effort that is already closed (or gone) is left alone
/// and nothing is logged; returns whether this call closed it.
pub fn finish_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    id: EffortId,
    end_snapshot_id: Option<i64>,
    summary: Option<&str>,
    now: Timestamp,
    retroactive: bool,
) -> Result<bool, DomainError> {
    use rusqlite::OptionalExtension;
    let closed: Option<(String, i64)> = conn
        .query_row(
            "UPDATE effort
             SET ended_at = ?2, end_snapshot_id = ?3, summary = ?4
             WHERE id = ?1 AND ended_at IS NULL
             RETURNING work_item, thread_id",
            params![id.value(), ts_to_string(now), end_snapshot_id, summary],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(map_sql_err)?;
    let Some((work_item, thread)) = closed else {
        return Ok(false);
    };
    let env = ev
        .typed::<EffortClosed>(&EffortClosedV1 {
            effort: effort_ref(id),
            work_item: work_item.clone(),
            end_snapshot: end_snapshot_id.map(snapshot_ref),
            retroactive,
        })
        .with_anchors(Anchors {
            effort_id: Some(id),
            snapshot_id: end_snapshot_id,
            ..anchors_for_thread_tx(conn, ThreadId::new(thread))?
        })
        .with_subject([effort_ref(id), work_item]);
    ev.append(conn, &env)?;
    Ok(true)
}

fn set_summary_tx(
    conn: &rusqlite::Connection,
    id: EffortId,
    summary: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE effort SET summary = ?2 WHERE id = ?1",
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
        "UPDATE effort SET impacts_json = ?2 WHERE id = ?1",
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
        "INSERT OR REPLACE INTO effort_file
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
    // Claim-first invariant: a path is CLAIMED or UNATTRIBUTED, never both.
    // Claiming clears any audit residue recorded for it on close.
    conn.execute(
        "DELETE FROM effort_unattributed_file WHERE effort_id = ?1 AND path = ?2",
        params![id.value(), path],
    )?;
    Ok(())
}

pub fn find_open_for_work_item_tx(
    conn: &rusqlite::Connection,
    work_item: &str,
) -> rusqlite::Result<Option<Effort>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM effort
         WHERE work_item = ?1 AND ended_at IS NULL
         ORDER BY started_at DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map(params![work_item], row_to_effort)?;
    rows.next().transpose()
}

fn most_recent_for_work_item_tx(
    conn: &rusqlite::Connection,
    work_item: &str,
) -> rusqlite::Result<Option<Effort>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM effort WHERE work_item = ?1
         ORDER BY started_at DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map(params![work_item], row_to_effort)?;
    rows.next().transpose()
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

fn row_to_effort(row: &rusqlite::Row<'_>) -> rusqlite::Result<Effort> {
    let id: i64 = row.get("id")?;
    let work_item: String = row.get("work_item")?;
    let thread_id: i64 = row.get("thread_id")?;
    let started_at: String = row.get("started_at")?;
    let ended_at: Option<String> = row.get("ended_at")?;
    let start_snapshot_id: Option<i64> = row.get("start_snapshot_id")?;
    let end_snapshot_id: Option<i64> = row.get("end_snapshot_id")?;
    let summary: Option<String> = row.get("summary")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(Effort {
        id: EffortId::new(id),
        work_item,
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
pub trait EffortStore: Send + Sync {
    /// Open an effort on `work_item`; `Invalid` unless it's a `work_item`
    /// ref, `Constraint` when that work item already has an open one.
    async fn start(
        &self,
        work_item: &str,
        thread: &ThreadId,
        start_snapshot_id: Option<i64>,
    ) -> Result<Effort, DomainError>;
    async fn finish(
        &self,
        id: &EffortId,
        end_snapshot_id: Option<i64>,
        summary: Option<String>,
    ) -> Result<(), DomainError>;
    /// Record the LLM-declared cross-page impacts for an effort.
    /// Replaces any prior list. The store then re-projects the
    /// owning work item's effort slice so impact edges show up in
    /// `page_ref` immediately.
    async fn set_impacts(&self, id: &EffortId, impacts: &[TaskImpact]) -> Result<(), DomainError>;
    async fn list_for_work_item(&self, work_item: &str) -> Result<Vec<Effort>, DomainError>;
    /// The work item an effort is on; `None` when the row is gone.
    async fn work_item_for_effort(&self, id: &EffortId) -> Result<Option<String>, DomainError>;
    /// Fetch a single effort row by id. Returns `None` when the row
    /// doesn't exist (e.g. cleared during snapshot prune).
    async fn get_effort(&self, id: &EffortId) -> Result<Option<Effort>, DomainError>;
    /// Open effort (`ended_at IS NULL`) for `work_item`, if any. Used by
    /// the lifecycle path that opens an effort on in_progress entry
    /// and finishes it on exit, and by `record_effort` to merge
    /// touched-files into the lifecycle row instead of creating a
    /// duplicate.
    async fn find_open_for_work_item(&self, work_item: &str)
        -> Result<Option<Effort>, DomainError>;
    /// Open effort (`ended_at IS NULL`) for `thread`, if any. The
    /// orchestrator keeps at most one item `in_progress` per thread, so
    /// this is the effort that hook-driven collection (test runs,
    /// coverage) attributes against. Newest open effort wins.
    async fn find_open_for_thread(&self, thread: &ThreadId) -> Result<Option<Effort>, DomainError>;
    /// Open effort for `thread` ONLY when it's unambiguous — exactly one open
    /// effort. Returns `None` when zero OR two-plus are open (parallel
    /// sub-agents on one thread), so attribution never silently guesses the
    /// wrong one; the ambiguous case defers to claim+reconcile (tsk263). The
    /// concurrency-safe replacement for `find_open_for_thread` on the
    /// attribution path.
    async fn find_single_open_for_thread(
        &self,
        thread: &ThreadId,
    ) -> Result<Option<Effort>, DomainError>;
    /// EVERY open effort for `thread`, newest first. The disambiguation input
    /// for target-overlap attribution (tsk169): when more than one is open
    /// `find_single_open_for_thread` declines by design, and the caller scores
    /// these candidates by what the run's command actually names.
    async fn list_open_for_thread(&self, thread: &ThreadId) -> Result<Vec<Effort>, DomainError>;
    /// Most-recent effort for `work_item` regardless of state, or `None`
    /// when it has never had one. Used by `record_effort` to
    /// reattach files to a just-closed lifecycle effort.
    async fn most_recent_for_work_item(
        &self,
        work_item: &str,
    ) -> Result<Option<Effort>, DomainError>;
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
    /// Every effort whose snapshot window OVERLAPS the half-open range
    /// `(range_start, range_end]` — including efforts that merely
    /// started or ended inside the range, fully contain it, or are
    /// still open (NULL end). Overlap of half-open intervals
    /// `(a.start, a.end]` and `(range_start, range_end]` is
    /// `a.start < range_end AND a.end > range_start`; an open effort
    /// (NULL end) overlaps if it started before `range_end`. Efforts
    /// with a NULL `start_snapshot_id` are skipped (can't be placed on
    /// the snapshot timeline). Results are **scoped to the stream that
    /// `range_end`'s snapshot belongs to** — snapshot ids are global, so
    /// without this an effort from another stream/branch whose
    /// snapshot-id window merely overlapped would leak in. Drives the
    /// diff view's roster of other efforts that overlapped the diffed
    /// range, within the same stream. Ordered by `started_at` ASC.
    async fn list_efforts_overlapping_range(
        &self,
        range_start: i64,
        range_end: i64,
    ) -> Result<Vec<Effort>, DomainError>;
    /// All distinct file paths whose `file_snapshot` rows fall inside
    /// this effort's snapshot bracket — i.e. the auto-diff for the
    /// effort. Returns empty when either `start_snapshot_id` or
    /// `end_snapshot_id` is NULL. Used by the effort-end
    /// reconciliation to compare against the LLM's claimed
    /// `touched_files`.
    async fn list_changed_paths_for_effort(
        &self,
        id: &EffortId,
    ) -> Result<EffortChangedPaths, DomainError>;
    /// Remove specific `effort_file` rows. Companion to
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
    /// Paths claimed (via `effort_file`) by OTHER efforts whose
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
    /// Replace the effort's UNATTRIBUTED audit residue with `paths`
    /// (delete-all-for-effort, then insert) — the claim-first
    /// reconciliation's record of `changed_but_not_claimed` paths an
    /// out-of-band close couldn't attribute. Idempotent. See migration
    /// `V34__effort_unattributed_file.sql`.
    async fn replace_unattributed_files(
        &self,
        id: &EffortId,
        paths: &[String],
    ) -> Result<(), DomainError>;
    /// The effort's recorded unattributed/unreviewed paths.
    async fn list_unattributed_files(&self, id: &EffortId) -> Result<Vec<String>, DomainError>;
}

#[derive(Clone)]
pub struct SqliteEffortStore {
    db: Database,
    page_refs: SqlitePageRefStore,
    /// Validates the `effort.*` envelopes this store logs.
    event_schemas: Arc<EventSchemaRegistry>,
}

impl SqliteEffortStore {
    /// A store with its own core schema registry. `Services` shares one
    /// registry across stores via [`Self::with_event_schemas`].
    pub fn new(db: Database) -> Self {
        Self::with_event_schemas(db, Arc::new(EventSchemaRegistry::core()))
    }

    pub fn with_event_schemas(db: Database, event_schemas: Arc<EventSchemaRegistry>) -> Self {
        Self {
            page_refs: SqlitePageRefStore::new(db.clone()),
            db,
            event_schemas,
        }
    }

    /// Re-emit the full effort-owned slice for `task_id` — the
    /// union of touched-file edges, the parsed wikilink/file/dir/
    /// task/finding/commit refs pulled from every effort's
    /// `summary` body, and the declared `TaskImpact` rows.
    /// Replaces under `effort_ref_types()` so the task-body slice
    /// (owned by `task_store`) is unaffected.
    async fn project_effort_slice(&self, work_item: &str) -> Result<(), DomainError> {
        let Some(source) = work_item_id_of_ref(work_item).map(str::to_string) else {
            return Err(DomainError::Invalid(format!(
                "`{work_item}` is not a work_item ref"
            )));
        };
        let refs = &self.page_refs;
        type SliceRows = (Vec<(String, String)>, Vec<String>, Vec<String>);
        let work_item = work_item.to_string();
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
                       FROM effort_file f
                       JOIN effort e ON e.id = f.effort_id
                       WHERE e.work_item = ?1
                     )
                     WHERE rn = 1
                     ORDER BY path",
                )?;
                let paths: Vec<(String, String)> = path_stmt
                    .query_map(params![work_item], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut sum_stmt = conn.prepare(
                    "SELECT summary FROM effort
                      WHERE work_item = ?1
                        AND summary IS NOT NULL
                        AND summary <> ''
                      ORDER BY started_at",
                )?;
                let summaries: Vec<String> = sum_stmt
                    .query_map(params![work_item], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut imp_stmt = conn.prepare(
                    "SELECT impacts_json FROM effort
                      WHERE work_item = ?1
                        AND impacts_json IS NOT NULL
                        AND impacts_json <> ''
                      ORDER BY started_at",
                )?;
                let impact_jsons: Vec<String> = imp_stmt
                    .query_map(params![work_item], |r| r.get::<_, String>(0))?
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
        let mut edges = effort_touched_file_edges(&source, &paths);
        edges.extend(effort_summary_edges(&source, &summaries));
        edges.extend(effort_impact_edges(&source, &impacts));
        refs.replace_source_for_ref_types(KIND_WORK_ITEM, &source, effort_ref_types(), edges)
            .await
    }

    /// Other efforts on the same thread whose time-window is **strictly nested**
    /// inside this effort's window (`other ⊆ self`, not equal). Used by run
    /// attribution's window-dominance (tsk267): a run that falls inside a nested
    /// sibling's window is the *narrower* (more specific) effort's to own, so the
    /// wider effort drops it from its residue. Only closed efforts can be nested
    /// (an open effort's end is unbounded ⇒ never `<= self.ended_at`). Empty when
    /// self has no bounded window. Timestamps are fixed-width RFC3339, so the
    /// `<=`/`>=` string comparisons order them correctly.
    pub async fn nested_efforts(&self, id: &EffortId) -> Result<Vec<Effort>, DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT other.* FROM effort other
                     JOIN effort self ON self.id = ?1
                     WHERE other.id != self.id
                       AND other.thread_id = self.thread_id
                       AND self.ended_at IS NOT NULL
                       AND other.ended_at IS NOT NULL
                       AND other.started_at >= self.started_at
                       AND other.ended_at <= self.ended_at
                       AND (other.started_at > self.started_at
                            OR other.ended_at < self.ended_at)",
                )?;
                let rows = stmt.query_map(params![id.value()], row_to_effort)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
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
        validate_work_item_ref(&args.work_item)?;
        let work_item = args.work_item.clone();
        let a = std::sync::Arc::new(args);
        let schemas = self.event_schemas.clone();
        let effort_id = self
            .db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "effort_attribution");
                let existing =
                    most_recent_for_work_item_tx(tx, &a.work_item).map_err(map_sql_err)?;
                let (effort_id, open) = match &existing {
                    Some(e) => (e.id, e.ended_at.is_none()),
                    None => (
                        // Synthesized: the item was never opened, so this
                        // effort is recorded after the fact.
                        start_tx(
                            tx,
                            &ev,
                            &a.work_item,
                            a.thread,
                            None,
                            Timestamp::now(),
                            true,
                        )?,
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
                        &ev,
                        effort_id,
                        None,
                        a.summary.as_deref(),
                        Timestamp::now(),
                        existing.is_none(),
                    )?;
                } else if a.summary.is_some() {
                    // Lifecycle finish already closed the row but left
                    // summary NULL — backfill it.
                    set_summary_tx(tx, effort_id, a.summary.as_deref()).map_err(map_sql_err)?;
                }
                Ok(effort_id)
            })
            .await?;
        self.project_effort_slice(&work_item).await?;
        Ok(effort_id)
    }

    /// Every open effort row (`ended_at IS NULL`) across all tasks.
    /// Used by boot recovery to heal lifecycle orphans.
    pub async fn list_all_open(&self) -> Result<Vec<Effort>, DomainError> {
        self.db
            .call(|conn| {
                let mut stmt = conn
                    .prepare("SELECT * FROM effort WHERE ended_at IS NULL ORDER BY started_at")?;
                let rows = stmt.query_map([], row_to_effort)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Every effort whose span overlaps the `[window_start, window_end]` time
    /// range — the efforts-as-overlay read powering the Metrics Explorer's
    /// effort bands (tsk233). An effort overlaps when it started before the
    /// window ends AND is still open or ended after the window starts. Open
    /// efforts (`ended_at IS NULL`) extend to "now", so they always overlap a
    /// window that reaches the present.
    pub async fn list_in_window(
        &self,
        window_start: Timestamp,
        window_end: Timestamp,
    ) -> Result<Vec<Effort>, DomainError> {
        let start = ts_to_string(window_start);
        let end = ts_to_string(window_end);
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM effort
                      WHERE started_at <= ?2
                        AND (ended_at IS NULL OR ended_at >= ?1)
                      ORDER BY started_at ASC",
                )?;
                let rows = stmt.query_map(params![start, end], row_to_effort)?;
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
                    "UPDATE effort SET start_snapshot_id = ?2 WHERE id = ?1",
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
                    "UPDATE effort SET end_snapshot_id = ?2 WHERE id = ?1",
                    params![id.value(), snapshot_id],
                )?;
                Ok(())
            })
            .await
    }
}

#[async_trait]
impl EffortStore for SqliteEffortStore {
    async fn start(
        &self,
        work_item: &str,
        thread: &ThreadId,
        start_snapshot_id: Option<i64>,
    ) -> Result<Effort, DomainError> {
        validate_work_item_ref(work_item)?;
        let thread = *thread;
        let now = Timestamp::now();
        let work_item = work_item.to_string();
        let w = work_item.clone();
        let schemas = self.event_schemas.clone();
        let id = self
            .db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "effort_store");
                start_tx(tx, &ev, &w, thread, start_snapshot_id, now, false)
            })
            .await?;
        Ok(Effort {
            id,
            work_item,
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
        let now = Timestamp::now();
        let schemas = self.event_schemas.clone();
        self.db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "effort_store");
                finish_tx(
                    tx,
                    &ev,
                    id_for_sql,
                    end_snapshot_id,
                    summary.as_deref(),
                    now,
                    false,
                )
            })
            .await?;
        if summary_has_body {
            if let Some(w) = self.work_item_for_effort(id).await? {
                self.project_effort_slice(&w).await?;
            }
        }
        Ok(())
    }

    async fn work_item_for_effort(
        &self,
        effort_id: &EffortId,
    ) -> Result<Option<String>, DomainError> {
        let id = *effort_id;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT work_item FROM effort WHERE id = ?1")?;
                let mut rows = stmt.query_map(params![id.value()], |r| r.get::<_, String>(0))?;
                rows.next().transpose()
            })
            .await
    }

    async fn find_open_for_work_item(
        &self,
        work_item: &str,
    ) -> Result<Option<Effort>, DomainError> {
        let work_item = work_item.to_string();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM effort
                     WHERE work_item = ?1 AND ended_at IS NULL
                     ORDER BY started_at DESC LIMIT 1",
                )?;
                let mut rows = stmt.query_map(params![work_item], row_to_effort)?;
                rows.next().transpose()
            })
            .await
    }

    async fn find_open_for_thread(&self, thread: &ThreadId) -> Result<Option<Effort>, DomainError> {
        let thread = *thread;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM effort
                     WHERE thread_id = ?1 AND ended_at IS NULL
                     ORDER BY started_at DESC LIMIT 1",
                )?;
                let mut rows = stmt.query_map(params![thread.value()], row_to_effort)?;
                rows.next().transpose()
            })
            .await
    }

    async fn find_single_open_for_thread(
        &self,
        thread: &ThreadId,
    ) -> Result<Option<Effort>, DomainError> {
        let thread = *thread;
        self.db
            .call(move |conn| {
                // LIMIT 2 distinguishes "exactly one" from "two-or-more" cheaply.
                let mut stmt = conn.prepare(
                    "SELECT * FROM effort
                     WHERE thread_id = ?1 AND ended_at IS NULL
                     ORDER BY started_at DESC LIMIT 2",
                )?;
                let mut rows = stmt.query_map(params![thread.value()], row_to_effort)?;
                let first = rows.next().transpose()?;
                let second = rows.next().transpose()?;
                // Some only when unambiguous; None for zero or two-plus open.
                Ok(match (first, second) {
                    (Some(e), None) => Some(e),
                    _ => None,
                })
            })
            .await
    }

    async fn list_open_for_thread(&self, thread: &ThreadId) -> Result<Vec<Effort>, DomainError> {
        let thread = *thread;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM effort
                     WHERE thread_id = ?1 AND ended_at IS NULL
                     ORDER BY started_at DESC",
                )?;
                let rows = stmt.query_map(params![thread.value()], row_to_effort)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn most_recent_for_work_item(
        &self,
        work_item: &str,
    ) -> Result<Option<Effort>, DomainError> {
        let work_item = work_item.to_string();
        self.db
            .call(move |conn| most_recent_for_work_item_tx(conn, &work_item))
            .await
    }

    async fn set_summary(&self, id: &EffortId, summary: Option<String>) -> Result<(), DomainError> {
        let id_for_sql = *id;
        self.db
            .call(move |conn| set_summary_tx(conn, id_for_sql, summary.as_deref()))
            .await?;
        {
            if let Some(w) = self.work_item_for_effort(id).await? {
                self.project_effort_slice(&w).await?;
            }
        }
        Ok(())
    }

    async fn list_for_work_item(&self, work_item: &str) -> Result<Vec<Effort>, DomainError> {
        let work_item = work_item.to_string();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM effort WHERE work_item = ?1
                     ORDER BY started_at DESC",
                )?;
                let rows = stmt.query_map(params![work_item], row_to_effort)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn get_effort(&self, id: &EffortId) -> Result<Option<Effort>, DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT * FROM effort WHERE id = ?1")?;
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
                     FROM effort_file
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
            if let Some(w) = self.work_item_for_effort(id).await? {
                self.project_effort_slice(&w).await?;
            }
        }
        Ok(())
    }

    async fn list_impacts(&self, id: &EffortId) -> Result<Vec<TaskImpact>, DomainError> {
        let id = *id;
        let raw: Option<String> = self
            .db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT impacts_json FROM effort WHERE id = ?1")?;
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
            if let Some(w) = self.work_item_for_effort(id).await? {
                self.project_effort_slice(&w).await?;
            }
        }
        Ok(())
    }

    async fn list_changed_paths_for_effort(
        &self,
        id: &EffortId,
    ) -> Result<EffortChangedPaths, DomainError> {
        let id_clone = *id;
        // Raw snapshot-bracket changed paths …
        let changed: Vec<String> = self
            .db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT fs.path
                     FROM effort e
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
            .await?;
        // … partitioned by whether this effort CLAIMED each one.
        let claimed_set: std::collections::HashSet<String> = self
            .list_files(id)
            .await?
            .into_iter()
            .map(|f| f.path)
            .collect();
        let (claimed, unclaimed): (Vec<String>, Vec<String>) =
            changed.into_iter().partition(|p| claimed_set.contains(p));
        Ok(EffortChangedPaths { claimed, unclaimed })
    }

    async fn remove_file(&self, id: &EffortId, path: &str) -> Result<(), DomainError> {
        let id_clone = *id;
        let path_clone = path.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM effort_file WHERE effort_id = ?1 AND path = ?2",
                    params![id_clone.value(), path_clone],
                )?;
                Ok(())
            })
            .await?;
        {
            if let Some(w) = self.work_item_for_effort(id).await? {
                self.project_effort_slice(&w).await?;
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
                     FROM effort self
                     JOIN effort other
                       ON other.id != self.id
                      AND other.start_snapshot_id < self.end_snapshot_id
                      AND (other.end_snapshot_id IS NULL
                           OR other.end_snapshot_id > self.start_snapshot_id)
                     JOIN effort_file tef ON tef.effort_id = other.id
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

    async fn replace_unattributed_files(
        &self,
        id: &EffortId,
        paths: &[String],
    ) -> Result<(), DomainError> {
        let id_clone = *id;
        let paths = paths.to_vec();
        self.db
            .call_mut(move |conn| {
                let sql_err = crate::database::map_sql_err;
                let tx = conn.transaction().map_err(sql_err)?;
                tx.execute(
                    "DELETE FROM effort_unattributed_file WHERE effort_id = ?1",
                    params![id_clone.value()],
                )
                .map_err(sql_err)?;
                let now = ts_to_string(Timestamp::now());
                for path in &paths {
                    tx.execute(
                        "INSERT OR REPLACE INTO effort_unattributed_file
                           (effort_id, path, recorded_at)
                         VALUES (?1, ?2, ?3)",
                        params![id_clone.value(), path, now],
                    )
                    .map_err(sql_err)?;
                }
                tx.commit().map_err(sql_err)?;
                Ok(())
            })
            .await
    }

    async fn list_unattributed_files(&self, id: &EffortId) -> Result<Vec<String>, DomainError> {
        let id_clone = *id;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT path FROM effort_unattributed_file \
                     WHERE effort_id = ?1 ORDER BY path",
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
                     JOIN effort e \
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

    async fn list_efforts_overlapping_range(
        &self,
        range_start: i64,
        range_end: i64,
    ) -> Result<Vec<Effort>, DomainError> {
        self.db
            .call(move |conn| {
                // Half-open overlap (range_start, range_end]:
                //   start < range_end AND (end IS NULL OR end > range_start).
                // NULL start ⇒ unplaced ⇒ excluded (NULL < x is NULL).
                //
                // Scope to the stream the diffed snapshot (`range_end`)
                // belongs to. Snapshot ids are global (autoincrement
                // across every stream), so without this an effort from a
                // *different* stream/branch whose snapshot-id window
                // merely overlapped would surface as a bogus "concurrent
                // effort". A diff lives within one stream's snapshot
                // timeline, so `range_end`'s stream IS the diff's stream;
                // we join the effort's thread to recover its stream. When
                // `range_end` names no `snapshot` row the subquery is NULL
                // and nothing matches (empty roster) — safer than leaking
                // cross-stream efforts.
                let mut stmt = conn.prepare(
                    "SELECT e.* FROM effort e
                     JOIN threads t ON t.id = e.thread_id
                     WHERE e.start_snapshot_id IS NOT NULL
                       AND e.start_snapshot_id < ?2
                       AND (e.end_snapshot_id IS NULL OR e.end_snapshot_id > ?1)
                       AND t.stream_id = (SELECT stream_id FROM snapshot WHERE id = ?2)
                     ORDER BY e.started_at ASC, e.id ASC",
                )?;
                let rows = stmt.query_map(params![range_start, range_end], row_to_effort)?;
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
    use oxplow_domain::refs::build::work_item_ref;
    use oxplow_domain::stores::{StreamStore, TaskStore, ThreadStore};
    use oxplow_domain::{
        Stream, StreamId, StreamKind, Task, TaskActorKind, TaskAuthor, TaskPriority, TaskStatus,
        Thread, ThreadStatus,
    };

    async fn fixture() -> (SqliteEffortStore, TaskId, ThreadId) {
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
        let store = SqliteEffortStore::new(db.clone());
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
                        "INSERT INTO effort
                           (id, work_item, thread_id, started_at, ended_at,
                            start_snapshot_id, end_snapshot_id)
                         VALUES (?1, 'work_item:oxplow:tsk1', 1, '2026-01-01T00:00:00Z',
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
                        "INSERT INTO effort_file
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

    #[tokio::test]
    async fn nested_efforts_returns_only_strictly_contained_siblings() {
        // tsk267 window-dominance: self = effort 1 [10:00, 11:00]. A sibling is
        // "nested" only when its window is strictly inside self's.
        let db = Database::in_memory();
        let store = SqliteEffortStore::new(db.clone());
        let db2 = db.clone();
        tokio::task::spawn_blocking(move || {
            db2.with_conn(|conn| {
                conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
                for (id, start, end) in [
                    (1, "2026-01-01T10:00:00Z", Some("2026-01-01T11:00:00Z")), // self
                    (2, "2026-01-01T10:20:00Z", Some("2026-01-01T10:40:00Z")), // nested ✓
                    (3, "2026-01-01T10:30:00Z", Some("2026-01-01T11:30:00Z")), // ends after — not nested
                    (4, "2026-01-01T09:00:00Z", Some("2026-01-01T09:30:00Z")), // entirely before
                    (5, "2026-01-01T10:00:00Z", Some("2026-01-01T11:00:00Z")), // equal — not STRICTLY nested
                    (6, "2026-01-01T10:15:00Z", None::<&str>), // still open — not nested
                ] {
                    conn.execute(
                        "INSERT INTO effort
                           (id, work_item, thread_id, started_at, ended_at,
                            start_snapshot_id, end_snapshot_id)
                         VALUES (?1, 'work_item:oxplow:tsk1', 1, ?2, ?3, NULL, NULL)",
                        params![id, start, end],
                    )?;
                }
                Ok(())
            })
        })
        .await
        .unwrap()
        .unwrap();

        let nested = store.nested_efforts(&EffortId::new(1)).await.unwrap();
        let mut ids: Vec<i64> = nested.iter().map(|e| e.id.value()).collect();
        ids.sort();
        assert_eq!(ids, vec![2], "only the strictly-contained sibling");
    }

    #[tokio::test]
    async fn list_in_window_returns_only_overlapping_efforts() {
        let db = Database::in_memory();
        let store = SqliteEffortStore::new(db.clone());
        let db2 = db.clone();
        tokio::task::spawn_blocking(move || {
            db2.with_conn(|conn| {
                conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
                // (id, started_at, ended_at) — NULL ended_at = open.
                for (id, start, end) in [
                    (1, "2026-01-01T00:00:00Z", Some("2026-01-02T00:00:00Z")), // before
                    (2, "2026-01-05T00:00:00Z", Some("2026-01-07T00:00:00Z")), // overlaps start
                    (3, "2026-01-06T00:00:00Z", None),                         // open → overlaps
                    (4, "2026-01-20T00:00:00Z", Some("2026-01-21T00:00:00Z")), // after
                ] {
                    conn.execute(
                        "INSERT INTO effort
                           (id, work_item, thread_id, started_at, ended_at)
                         VALUES (?1, 'work_item:oxplow:tsk1', 1, ?2, ?3)",
                        params![id, start, end],
                    )?;
                }
                Ok(())
            })
        })
        .await
        .unwrap()
        .unwrap();

        let ts = |s: &str| -> Timestamp { serde_json::from_str(&format!("\"{s}\"")).unwrap() };
        let got = store
            .list_in_window(ts("2026-01-06T00:00:00Z"), ts("2026-01-10T00:00:00Z"))
            .await
            .unwrap();
        // e2 (01-05) and e3 (open) overlap; e1 (before) and e4 (after) don't.
        assert_eq!(
            got.iter().map(|e| e.id.value()).collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    async fn fixture_with_db() -> (SqliteEffortStore, Database, TaskId, ThreadId) {
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
            acp_agent: None,
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
        (SqliteEffortStore::new(db.clone()), db, tid, t.id)
    }

    /// P2.5b (tsk428): an effort is on a work item — an oxplow task's or
    /// another provider's — and the store never accepts a non-ref.
    #[tokio::test]
    async fn efforts_are_keyed_by_work_item_ref() {
        let (store, tid, t) = fixture().await;
        let ours = work_item_ref(tid);
        let eff = store.start(&ours, &t, None).await.unwrap();
        assert_eq!(eff.work_item, ours);
        assert_eq!(eff.task_id(), Some(tid));

        let foreign = "work_item:linear:ENG-12";
        store.finish(&eff.id, None, None).await.unwrap();
        let other = store.start(foreign, &t, None).await.unwrap();
        assert_eq!(other.task_id(), None);
        assert_eq!(
            store
                .find_open_for_work_item(foreign)
                .await
                .unwrap()
                .map(|e| e.id),
            Some(other.id)
        );
        assert!(store
            .find_open_for_work_item(&ours)
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .most_recent_for_work_item(&ours)
                .await
                .unwrap()
                .map(|e| e.id),
            Some(eff.id)
        );
        assert_eq!(store.list_for_work_item(foreign).await.unwrap().len(), 1);
        assert_eq!(
            store
                .work_item_for_effort(&other.id)
                .await
                .unwrap()
                .as_deref(),
            Some(foreign)
        );

        for bad in ["", "tsk1", "effort:eff1", "work_item:"] {
            assert!(
                matches!(
                    store.start(bad, &t, None).await,
                    Err(DomainError::Invalid(_))
                ),
                "`{bad}` must be refused"
            );
        }
    }

    #[tokio::test]
    async fn start_then_finish_round_trips() {
        let (store, tid, t) = fixture().await;
        let eff = store.start(&work_item_ref(tid), &t, None).await.unwrap();
        assert!(eff.ended_at.is_none());
        store
            .finish(&eff.id, None, Some("done".into()))
            .await
            .unwrap();
        let list = store.list_for_work_item(&work_item_ref(tid)).await.unwrap();
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
            work_item: work_item_ref(tid),
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
        assert!(store
            .find_open_for_work_item(&work_item_ref(tid))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn record_effort_atomic_merges_into_open_lifecycle_effort() {
        let (store, tid, t) = fixture().await;
        let lifecycle = store.start(&work_item_ref(tid), &t, None).await.unwrap();
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
        let eff = store.start(&work_item_ref(tid), &t, None).await.unwrap();
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
            .update_logged(&task_row(tid, t, TaskStatus::InProgress), TaskStatus::Ready)
            .await
            .unwrap();
        let EffortTransition::Opened(eff) = entering else {
            panic!("expected Opened, got {entering:?}");
        };
        let open = store
            .find_open_for_work_item(&work_item_ref(tid))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(open.id, eff);
        assert!(open.start_snapshot_id.is_none(), "pin backfills later");

        // Re-issuing the same status changes nothing: the open effort
        // stays and nothing is logged.
        let again = tasks
            .update_logged(
                &task_row(tid, t, TaskStatus::InProgress),
                TaskStatus::InProgress,
            )
            .await
            .unwrap();
        assert_eq!(again, EffortTransition::Untouched);

        let leaving = tasks
            .update_logged(&task_row(tid, t, TaskStatus::Done), TaskStatus::InProgress)
            .await
            .unwrap();
        assert_eq!(leaving, EffortTransition::Finished(eff));
        assert!(store
            .find_open_for_work_item(&work_item_ref(tid))
            .await
            .unwrap()
            .is_none());

        // The outbox: one `work_item.transitioned@1` per status change,
        // committed with it — the re-issued (same-status) call logged
        // nothing. Subject and anchors name the task and its effort.
        let all = db
            .call_mut(|c| crate::event_log_store::read_after_tx(c, 0, 10))
            .await
            .unwrap();
        // The effort's own open/close ride the same transactions.
        let types: Vec<&str> = all.iter().map(|e| e.envelope.event_type.as_str()).collect();
        assert_eq!(
            types,
            vec![
                "effort.opened",
                "work_item.transitioned",
                "effort.closed",
                "work_item.transitioned"
            ]
        );
        let events: Vec<_> = all
            .iter()
            .filter(|e| e.envelope.event_type == "work_item.transitioned")
            .collect();
        assert_eq!(events.len(), 2, "{events:#?}");
        let opened = &events[0].envelope;
        assert_eq!(opened.event_type, "work_item.transitioned");
        assert_eq!(opened.v, 1);
        assert_eq!(
            opened.subject,
            vec![format!("work_item:oxplow:{tid}"), format!("effort:{eff}")]
        );
        assert_eq!(opened.anchors.thread_id, Some(t));
        assert_eq!(opened.anchors.effort_id, Some(eff));
        assert!(
            opened.anchors.stream_id.is_some(),
            "the thread's stream anchors it too"
        );
        assert_eq!(opened.source, "system:task_service");
        assert_eq!(opened.payload["from"], "ready");
        assert_eq!(opened.payload["to"], "in_progress");
        assert_eq!(
            opened.payload["work_item"],
            format!("work_item:oxplow:{tid}")
        );
        assert_eq!(opened.payload["effort"], format!("effort:{eff}"));
        let finished = &events[1].envelope;
        assert_eq!(finished.payload["from"], "in_progress");
        assert_eq!(finished.payload["to"], "done");
        assert_eq!(finished.anchors.effort_id, Some(eff));
        assert!(events[1].seq > events[0].seq);
    }

    /// P2.6.1 (tsk453): every effort open and close is logged in the
    /// write's own transaction, whichever path made it.
    #[tokio::test]
    async fn effort_open_and_close_are_logged_with_the_write() {
        let (store, db, tid, t) = fixture_with_db().await;
        let foreign = "work_item:linear:ENG-12";
        let eff = store.start(foreign, &t, Some(1)).await.ok();
        // No snapshot 1 exists; the FK refuses it and nothing is logged.
        assert!(eff.is_none());
        let eff = store.start(foreign, &t, None).await.unwrap();
        store
            .finish(&eff.id, None, Some("done".into()))
            .await
            .unwrap();
        // Finishing an already-closed effort changes nothing and logs nothing.
        store.finish(&eff.id, None, None).await.unwrap();
        // A synthesized effort (no lifecycle open) logs both.
        let synthesized = store
            .record_effort_atomic(RecordEffortAtomic {
                work_item: work_item_ref(tid),
                thread: t,
                files: vec![],
                version: OwnedFileRefVersion {
                    local_snapshot_id: 0,
                    closest_git_version: None,
                    git_version_exact: false,
                },
                impacts: vec![],
                summary: Some("s".into()),
            })
            .await
            .unwrap();

        let events = db
            .call_mut(|c| crate::event_log_store::read_after_tx(c, 0, 10))
            .await
            .unwrap();
        let seen: Vec<(String, Vec<String>)> = events
            .iter()
            .map(|e| (e.envelope.event_type.clone(), e.envelope.subject.clone()))
            .collect();
        let ours = work_item_ref(tid);
        assert_eq!(
            seen,
            vec![
                (
                    "effort.opened".into(),
                    vec![format!("effort:{}", eff.id), foreign.into()]
                ),
                (
                    "effort.closed".into(),
                    vec![format!("effort:{}", eff.id), foreign.into()]
                ),
                (
                    "effort.opened".into(),
                    vec![format!("effort:{synthesized}"), ours.clone()]
                ),
                (
                    "effort.closed".into(),
                    vec![format!("effort:{synthesized}"), ours]
                ),
            ]
        );
        let opened = &events[0].envelope;
        assert_eq!(opened.anchors.effort_id, Some(eff.id));
        assert_eq!(opened.anchors.thread_id, Some(t));
        assert!(opened.anchors.stream_id.is_some());
        assert_eq!(opened.payload["work_item"], foreign);
        assert_eq!(opened.payload["thread"], format!("thread:{t}"));
        assert_eq!(
            events[1].envelope.payload["effort"],
            format!("effort:{}", eff.id)
        );
        // A lifecycle effort has a bracket to snapshot; a synthesized one
        // was recorded after the fact.
        assert!(opened.payload.get("retroactive").is_none());
        assert_eq!(events[2].envelope.payload["retroactive"], true);
        assert_eq!(events[3].envelope.payload["retroactive"], true);
    }

    #[tokio::test]
    async fn transition_on_missing_task_rolls_back_effort_open() {
        let (store, db, _tid, t) = fixture_with_db().await;
        let tasks = SqliteTaskStore::new(db.clone());
        let ghost = TaskId::new(9999);
        let err = tasks
            .update_logged(
                &task_row(ghost, t, TaskStatus::InProgress),
                TaskStatus::Ready,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound), "got {err:?}");
        // The whole action rolled back — no effort row for the ghost,
        // and nothing in the event log either.
        assert!(store
            .find_open_for_work_item(&work_item_ref(ghost))
            .await
            .unwrap()
            .is_none());
        let events = db
            .call_mut(|c| crate::event_log_store::read_after_tx(c, 0, 10))
            .await
            .unwrap();
        assert!(events.is_empty(), "{events:#?}");
    }

    #[tokio::test]
    async fn second_open_effort_for_same_task_is_a_constraint() {
        // The V31 partial unique index enforces the lifecycle
        // invariant: at most one open effort per task. A double-open
        // must surface as a typed Constraint, never silently diverge.
        let (store, tid, t) = fixture().await;
        store.start(&work_item_ref(tid), &t, None).await.unwrap();
        let err = store
            .start(&work_item_ref(tid), &t, None)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Constraint(_)), "got {err:?}");
        // Finishing the open row frees the slot.
        let open = store
            .find_open_for_work_item(&work_item_ref(tid))
            .await
            .unwrap()
            .unwrap();
        store.finish(&open.id, None, None).await.unwrap();
        store.start(&work_item_ref(tid), &t, None).await.unwrap();
    }

    #[tokio::test]
    async fn find_single_open_for_thread_only_when_unambiguous() {
        let (store, db, tid, thread) = fixture_with_db().await;
        // Zero open → None.
        assert!(store
            .find_single_open_for_thread(&thread)
            .await
            .unwrap()
            .is_none());
        // Exactly one open → Some.
        store
            .start(&work_item_ref(tid), &thread, None)
            .await
            .unwrap();
        assert!(store
            .find_single_open_for_thread(&thread)
            .await
            .unwrap()
            .is_some());
        // A second task's effort open on the same thread (parallel sub-agents)
        // → ambiguous → None: attribution must not guess.
        let now = Timestamp::from_unix_ms(2);
        let tid2 = SqliteTaskStore::new(db.clone())
            .insert(&Task {
                id: TaskId::placeholder(),
                thread_id: Some(thread),
                parent_id: None,
                title: "x2".into(),
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
        store
            .start(&work_item_ref(tid2), &thread, None)
            .await
            .unwrap();
        assert!(
            store
                .find_single_open_for_thread(&thread)
                .await
                .unwrap()
                .is_none(),
            "two open efforts on one thread → no single attribution"
        );
        // The legacy lookup still returns one (the silent guess we're replacing).
        assert!(store.find_open_for_thread(&thread).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn record_then_list_files() {
        let (store, tid, t) = fixture().await;
        let eff = store.start(&work_item_ref(tid), &t, None).await.unwrap();
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
        let store = SqliteEffortStore::new(db);
        let eff = store.start(&work_item_ref(tid), &t, None).await.unwrap();
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
            wiki_back.iter().any(|e| e.source_kind == "work_item"
                && e.source_id == format!("oxplow:{tid}")
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
                .any(|e| e.ref_type == "summary_file_ref" && e.source_id == format!("oxplow:{tid}")),
            "file backlink missing; got {file_back:?}"
        );

        let task_back = page_refs
            .list_backlinks("work_item", "oxplow:tsk99", None)
            .await
            .unwrap();
        assert!(
            task_back
                .iter()
                .any(|e| e.ref_type == "summary_task_mention"
                    && e.source_id == format!("oxplow:{tid}")),
            "task backlink missing; got {task_back:?}"
        );
    }

    #[tokio::test]
    async fn set_impacts_projects_edges_and_round_trips() {
        use crate::page_ref_store::SqlitePageRefStore;
        use oxplow_domain::TaskImpact;
        let (_, db, tid, t) = fixture_with_db().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let store = SqliteEffortStore::new(db);
        let eff = store.start(&work_item_ref(tid), &t, None).await.unwrap();
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
            .find(|e| e.source_id == format!("oxplow:{tid}"))
            .expect("wiki impact edge missing");
        assert_eq!(row.ref_type, "impact");
        assert!(row
            .source_extra
            .as_deref()
            .is_some_and(|s| s.contains("created")));

        let commit = page_refs
            .list_backlinks("commit", "abc1234", None)
            .await
            .unwrap();
        assert!(commit
            .iter()
            .any(|e| e.source_id == format!("oxplow:{tid}") && e.ref_type == "impact"));

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
        let store = SqliteEffortStore::new(db);
        let first = store.start(&work_item_ref(tid), &t, None).await.unwrap();
        store
            .finish(&first.id, None, Some("Filed [[url-schemes]]".into()))
            .await
            .unwrap();

        let second = store.start(&work_item_ref(tid), &t, None).await.unwrap();
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
            wiki_back
                .iter()
                .any(|e| e.source_id == format!("oxplow:{tid}")),
            "summary slice was clobbered by record_file: {wiki_back:?}"
        );
    }

    #[tokio::test]
    async fn unattributed_files_replace_list_and_cascade() {
        // replace_unattributed_files records the audit residue; list reads
        // it back; deleting the effort cascades it away.
        let (_, db, tid, t) = fixture_with_db().await;
        let store = SqliteEffortStore::new(db);
        let eff = store.start(&work_item_ref(tid), &t, None).await.unwrap();
        store
            .replace_unattributed_files(&eff.id, &["a.rs".into(), "b.rs".into()])
            .await
            .unwrap();
        let mut got = store.list_unattributed_files(&eff.id).await.unwrap();
        got.sort();
        assert_eq!(got, vec!["a.rs".to_string(), "b.rs".to_string()]);
        // Replace is idempotent / overwrites the whole set.
        store
            .replace_unattributed_files(&eff.id, &["c.rs".into()])
            .await
            .unwrap();
        assert_eq!(
            store.list_unattributed_files(&eff.id).await.unwrap(),
            vec!["c.rs".to_string()]
        );
    }

    #[tokio::test]
    async fn record_file_clears_unattributed_mark() {
        // Invariant: a path is CLAIMED or UNATTRIBUTED, never both.
        // Claiming a previously-unattributed path drops its residue row.
        let (_, db, tid, t) = fixture_with_db().await;
        let store = SqliteEffortStore::new(db);
        let eff = store.start(&work_item_ref(tid), &t, None).await.unwrap();
        store
            .replace_unattributed_files(&eff.id, &["shared.rs".into(), "other.rs".into()])
            .await
            .unwrap();
        let v = FileRefVersion {
            local_snapshot_id: 0,
            closest_git_version: None,
            git_version_exact: false,
        };
        store
            .record_file(&eff.id, "shared.rs", EffortFileChange::Updated, v)
            .await
            .unwrap();
        // shared.rs is now claimed → no longer unattributed; other.rs stays.
        assert_eq!(
            store.list_unattributed_files(&eff.id).await.unwrap(),
            vec!["other.rs".to_string()]
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
            acp_agent: None,
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
        // effort.end_snapshot_id references snapshot(id), not
        // file_snapshot(id). Build real snapshot grouping rows so the
        // FK validates.
        let snap_store = crate::SqliteSnapshotStore::new(db.clone());
        let snap1 = snap_store.create_snapshot(s.id).await.unwrap();
        let snap2 = snap_store.create_snapshot(s.id).await.unwrap();
        let snap3 = snap_store.create_snapshot(s.id).await.unwrap();

        let store = SqliteEffortStore::new(db);
        // Effort A: start@snap1, end@snap2 — active at snap1 AND snap2
        // (ends exactly there); not active at snap3.
        let a = store
            .start(&work_item_ref(tid), &t.id, Some(snap1))
            .await
            .unwrap();
        store.finish(&a.id, Some(snap2), None).await.unwrap();
        // Effort B: start@snap2, still open — active at snap2 and
        // snap3.
        let b = store
            .start(&work_item_ref(tid), &t.id, Some(snap2))
            .await
            .unwrap();

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

    #[tokio::test]
    async fn list_efforts_overlapping_range_includes_straddle_contain_and_open() {
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
            acp_agent: None,
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
        let snap_store = crate::SqliteSnapshotStore::new(db.clone());
        let s1 = snap_store.create_snapshot(s.id).await.unwrap();
        let s2 = snap_store.create_snapshot(s.id).await.unwrap();
        let _s3 = snap_store.create_snapshot(s.id).await.unwrap();
        let s4 = snap_store.create_snapshot(s.id).await.unwrap();
        let s5 = snap_store.create_snapshot(s.id).await.unwrap();
        let store = SqliteEffortStore::new(db);

        // Range under test: (s2, s4].
        // A [s1,s2] — ends exactly at range start → excluded.
        let a = store
            .start(&work_item_ref(tid), &t.id, Some(s1))
            .await
            .unwrap();
        store.finish(&a.id, Some(s2), None).await.unwrap();
        // B [s2,s4] — straddles the range end → included.
        let b = store
            .start(&work_item_ref(tid), &t.id, Some(s2))
            .await
            .unwrap();
        store.finish(&b.id, Some(s4), None).await.unwrap();
        // C [s4,s5] — starts exactly at range end → excluded.
        let c = store
            .start(&work_item_ref(tid), &t.id, Some(s4))
            .await
            .unwrap();
        store.finish(&c.id, Some(s5), None).await.unwrap();
        // D [s1,s5] — fully contains the range → included.
        let d = store
            .start(&work_item_ref(tid), &t.id, Some(s1))
            .await
            .unwrap();
        store.finish(&d.id, Some(s5), None).await.unwrap();
        // E [s2,open] — still in progress → included.
        let e = store
            .start(&work_item_ref(tid), &t.id, Some(s2))
            .await
            .unwrap();

        let rows = store.list_efforts_overlapping_range(s2, s4).await.unwrap();
        let ids: std::collections::HashSet<i64> = rows.iter().map(|r| r.id.value()).collect();
        assert!(ids.contains(&b.id.value()), "straddling effort included");
        assert!(ids.contains(&d.id.value()), "containing effort included");
        assert!(ids.contains(&e.id.value()), "open effort included");
        assert!(
            !ids.contains(&a.id.value()),
            "effort ending at range start excluded"
        );
        assert!(
            !ids.contains(&c.id.value()),
            "effort starting at range end excluded"
        );
    }

    /// Build a stream (`n`) + one thread + one task, all keyed off `n`.
    /// Stream 1 is the Primary; any other `n` is a Worktree (the unique
    /// partial index allows only one Primary).
    async fn stream_thread_task(db: &Database, n: i64) -> (StreamId, ThreadId, TaskId) {
        let now = Timestamp::from_unix_ms(1);
        let s = Stream {
            id: StreamId::new(n),
            kind: if n == 1 {
                StreamKind::Primary
            } else {
                StreamKind::Worktree
            },
            title: format!("s{n}"),
            branch: format!("b{n}"),
            branch_ref: format!("refs/heads/b{n}"),
            branch_source: "main".into(),
            worktree_path: format!("/p{n}"),
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
            id: ThreadId::new(n),
            stream_id: s.id,
            title: format!("t{n}"),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            acp_agent: None,
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
                title: format!("task{n}"),
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
        (s.id, t.id, tid)
    }

    /// Concurrent efforts must be scoped to the diffed snapshot's own
    /// stream. Snapshot ids are global, so two streams' efforts can have
    /// numerically-overlapping snapshot windows; the query must NOT leak
    /// the other stream's effort into the diff's "Concurrent Efforts"
    /// roster.
    #[tokio::test]
    async fn overlapping_range_excludes_other_stream_efforts() {
        let db = Database::in_memory();
        let (sa, ta, tida) = stream_thread_task(&db, 1).await;
        let (sb, tb, tidb) = stream_thread_task(&db, 2).await;
        let snap = crate::SqliteSnapshotStore::new(db.clone());

        // Interleave snapshot creation so the global ids straddle across
        // streams: a1 < b1 < a2 < b2. Effort A spans (a1, a2], effort B
        // spans (b1, b2] — B's window numerically overlaps (a1, a2].
        let a1 = snap.create_snapshot(sa).await.unwrap();
        let b1 = snap.create_snapshot(sb).await.unwrap();
        let a2 = snap.create_snapshot(sa).await.unwrap();
        let b2 = snap.create_snapshot(sb).await.unwrap();

        let store = SqliteEffortStore::new(db);
        let ea = store
            .start(&work_item_ref(tida), &ta, Some(a1))
            .await
            .unwrap();
        store.finish(&ea.id, Some(a2), None).await.unwrap();
        let eb = store
            .start(&work_item_ref(tidb), &tb, Some(b1))
            .await
            .unwrap();
        store.finish(&eb.id, Some(b2), None).await.unwrap();

        // Diff range is stream A's (a1, a2]; range_end (a2) is a stream-A
        // snapshot, so only stream-A efforts should come back.
        let rows = store.list_efforts_overlapping_range(a1, a2).await.unwrap();
        let ids: std::collections::HashSet<i64> = rows.iter().map(|r| r.id.value()).collect();
        assert!(
            ids.contains(&ea.id.value()),
            "the diffed stream's own effort is included"
        );
        assert!(
            !ids.contains(&eb.id.value()),
            "an effort from another stream must NOT leak in, even though \
             its snapshot-id window overlaps numerically"
        );
    }
}
