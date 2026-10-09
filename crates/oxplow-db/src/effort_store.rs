//! Task effort tracking.
//!
//! An "effort" is one continuous push of agent work on a single work
//! item (an oxplow task, `work_item:oxplow:tsk42`, or another provider's
//! item, `work_item:issues:ENG-12`),
//! bounded by snapshots at start and end. This module owns:
//!
//! - `effort` (the effort row)
//! - `effort_file` (per-effort file changes)

use async_trait::async_trait;
use oxplow_domain::vocabulary::VocabularyHandle;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{AgentSessionId, DomainError, EffortId, EffortImpact, ThreadId, Timestamp};

use crate::database::map_sql_err;
use crate::database::Database;
use crate::database::{string_to_ts, ts_to_string};
use crate::event_log_store::{anchors_for_thread_tx, EventCtx};
use crate::page_ref_projections::{
    effort_impact_edges, effort_ref_types, effort_summary_edges, effort_touched_file_edges,
    KIND_WORK_ITEM,
};
use crate::page_ref_store::SqlitePageRefStore;
use oxplow_domain::events::schema::{
    EffortClosed, EffortClosedV2, EffortLinked, EffortLinkedV1, EffortOpened, EffortOpenedV2,
    EffortRetitled, EffortRetitledV1,
};
use oxplow_domain::refs::build::{
    effort_ref, snapshot_ref, thread_ref, validate_work_item_ref, work_item_id_of_ref,
};
use oxplow_domain::{Anchors, StreamId};

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
    /// The work item it's linked to, as a canonical `work_item` ref; `None`
    /// while unlinked (oxplow opens efforts itself).
    pub work_item: Option<String>,
    /// Its own title, when one was set; `None` means the default
    /// (`v_effort.title`).
    pub title: Option<String>,
    /// What closed it (`commit`, `switch`, `person`, `agent`, `system`).
    pub closed_by: Option<String>,
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
    pub closest_vcs_rev: Option<String>,
    /// `true` when `local_snapshot_id`'s snapshot is byte-equal to
    /// `closest_vcs_rev` (clean worktree at capture, or
    /// set later when the snapshot is stamped with a revision — a take on
    /// a clean head, or a head move; `stamp_revision_tx`).
    pub vcs_rev_exact: bool,
    pub source: FileSource,
}

/// How an effort came to own a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FileSource {
    /// An edit tool named it.
    Claimed,
    /// It changed during one of the thread's turns and no other thread
    /// claimed it — a shell edit, a formatter, a generator.
    Observed,
}

impl FileSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "claimed",
            Self::Observed => "observed",
        }
    }
}

/// Snapshot-pinned version data for a file reference. The
/// store/service layer computes this from a snapshot id at capture
/// time and stamps it onto every per-file ref row.
#[derive(Debug, Clone, Copy)]
pub struct FileRefVersion<'a> {
    pub local_snapshot_id: i64,
    pub closest_vcs_rev: Option<&'a str>,
    pub vcs_rev_exact: bool,
}

/// Owned variant of [`FileRefVersion`] for callers that need to move
/// the triple into a `'static` transaction closure.
#[derive(Debug, Clone)]
pub struct OwnedFileRefVersion {
    pub local_snapshot_id: i64,
    pub closest_vcs_rev: Option<String>,
    pub vcs_rev_exact: bool,
}

impl OwnedFileRefVersion {
    pub fn as_ref(&self) -> FileRefVersion<'_> {
        FileRefVersion {
            local_snapshot_id: self.local_snapshot_id,
            closest_vcs_rev: self.closest_vcs_rev.as_deref(),
            vcs_rev_exact: self.vcs_rev_exact,
        }
    }
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
// wrappers over these; a multi-write action composes several cores in one
// transaction. See `.context/data-model.md`,
// "Transactions".
// ---------------------------------------------------------------------------

/// The derived tables whose rows carry the effort they belong to, with
/// their thread and time columns: what an effort adopts when it opens late,
/// and releases when it closes as of a past point. The event log keeps the
/// anchors its events were written with; it's history, not a projection.
const EFFORT_STAMPED: [(&str, &str); 7] = [
    ("agent_tool_call", "at"),
    ("agent_token_usage", "recorded_at"),
    ("metric_capture", "captured_at"),
    ("agent_nudge", "created_at"),
    ("claim", "created_at"),
    ("decision", "created_at"),
    ("thread_answer", "created_at"),
];

/// How an effort opens ([`start_tx`]).
#[derive(Debug, Clone, Copy)]
pub struct EffortStart<'a> {
    pub thread: ThreadId,
    /// The work item it's linked to, when it is.
    pub work_item: Option<&'a str>,
    /// When it's opened.
    pub at: Timestamp,
    /// Adopt the thread's un-efforted activity back to here — never past
    /// the end of the thread's previous effort. `None` starts at `at`.
    pub adopt_since: Option<Timestamp>,
    /// Its start snapshot, when the opener already has one.
    pub start_snapshot_id: Option<i64>,
}

impl<'a> EffortStart<'a> {
    /// An unlinked effort on `thread` opened at `at`.
    pub fn at(thread: ThreadId, at: Timestamp) -> Self {
        Self {
            thread,
            work_item: None,
            at,
            adopt_since: None,
            start_snapshot_id: None,
        }
    }
}

/// The effort `thread` was in at `at`: the one whose span covers it.
pub fn effort_at_tx(
    conn: &rusqlite::Connection,
    thread: ThreadId,
    at: Timestamp,
) -> rusqlite::Result<Option<EffortId>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT id FROM effort
          WHERE thread_id = ?1 AND started_at <= ?2 AND (ended_at IS NULL OR ended_at > ?2)
          ORDER BY started_at DESC, id DESC LIMIT 1",
        params![thread.value(), ts_to_string(at)],
        |r| r.get(0).map(EffortId::new),
    )
    .optional()
}

/// Opens an effort on `thread`, linked to `work_item` when given (which
/// the caller has validated with `validate_work_item_ref` or built with
/// `work_item_ref`), and logs `effort.opened` in the same transaction —
/// every open, whichever path made it. A thread holds one open effort, so
/// the one it had open closes first (`switch`). An effort opened late
/// adopts the thread's un-efforted activity since `adopt_since`, clamped
/// to its previous effort's end.
pub fn start_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    start: &EffortStart<'_>,
) -> Result<EffortId, DomainError> {
    let EffortStart {
        thread,
        work_item,
        at: now,
        adopt_since,
        start_snapshot_id,
    } = *start;
    if let Some(open) = open_for_thread_tx(conn, thread).map_err(map_sql_err)? {
        finish_tx(conn, ev, open, &EffortEnd::at(now, ClosedBy::Switch))?;
    }
    // Where it starts: back to `adopt_since`, never before the thread's
    // previous effort ended, never after now.
    let started_at = match adopt_since {
        None => now,
        Some(since) => {
            let previous_end: Option<String> = conn
                .query_row(
                    "SELECT max(ended_at) FROM effort WHERE thread_id = ?1",
                    params![thread.value()],
                    |r| r.get(0),
                )
                .map_err(map_sql_err)?;
            let floor = previous_end
                .map(|e| string_to_ts(&e))
                .transpose()?
                .map_or(since, |end| since.max(end));
            floor.min(now)
        }
    };
    conn.execute(
        "INSERT INTO effort
           (id, work_item, thread_id, started_at, ended_at,
            start_snapshot_id, end_snapshot_id, summary)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5, NULL, NULL)",
        params![
            None::<i64>,
            work_item,
            thread.value(),
            ts_to_string(started_at),
            start_snapshot_id,
        ],
    )
    .map_err(map_sql_err)?;
    let id = EffortId::new(conn.last_insert_rowid());
    let since = ts_to_string(started_at);
    for (table, time) in EFFORT_STAMPED {
        conn.execute(
            &format!(
                "UPDATE {table} SET effort_id = ?1
                  WHERE thread_id = ?2 AND effort_id IS NULL AND {time} >= ?3"
            ),
            params![id.value(), thread.value(), since],
        )
        .map_err(map_sql_err)?;
    }
    let env = ev
        .typed::<EffortOpened>(&EffortOpenedV2 {
            effort: effort_ref(id),
            work_item: work_item.map(str::to_string),
            thread: thread_ref(thread),
            start_snapshot: start_snapshot_id.map(snapshot_ref),
        })
        .with_anchors(Anchors {
            effort_id: Some(id),
            snapshot_id: start_snapshot_id,
            ..anchors_for_thread_tx(conn, thread)?
        })
        .with_subject(std::iter::once(effort_ref(id)).chain(work_item.map(str::to_string)));
    ev.append(conn, &env)?;
    Ok(id)
}

/// Link `id` to `work_item` (or unlink it, with `None`) and log
/// `effort.linked`; returns what it was linked to before. `Invalid` for an
/// unknown effort.
pub fn link_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    id: EffortId,
    work_item: Option<&str>,
) -> Result<Option<String>, DomainError> {
    let (before, thread) = effort_field_tx(conn, id, "work_item")?;
    conn.execute(
        "UPDATE effort SET work_item = ?2 WHERE id = ?1",
        params![id.value(), work_item],
    )
    .map_err(map_sql_err)?;
    let env = ev
        .typed::<EffortLinked>(&EffortLinkedV1 {
            effort: effort_ref(id),
            work_item: work_item.map(str::to_string),
        })
        .with_anchors(Anchors {
            effort_id: Some(id),
            ..anchors_for_thread_tx(conn, thread)?
        })
        .with_subject(std::iter::once(effort_ref(id)).chain(work_item.map(str::to_string)));
    ev.append(conn, &env)?;
    Ok(before)
}

/// Set `id`'s own title (or clear it, with `None`) and log
/// `effort.retitled`; returns the title it had.
pub fn retitle_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    id: EffortId,
    title: Option<&str>,
) -> Result<Option<String>, DomainError> {
    let (before, thread) = effort_field_tx(conn, id, "title")?;
    let title = title.map(str::trim).filter(|t| !t.is_empty());
    conn.execute(
        "UPDATE effort SET title = ?2 WHERE id = ?1",
        params![id.value(), title],
    )
    .map_err(map_sql_err)?;
    let env = ev
        .typed::<EffortRetitled>(&EffortRetitledV1 {
            effort: effort_ref(id),
            title: title.map(str::to_string),
        })
        .with_anchors(Anchors {
            effort_id: Some(id),
            ..anchors_for_thread_tx(conn, thread)?
        })
        .with_subject([effort_ref(id)]);
    ev.append(conn, &env)?;
    Ok(before)
}

/// One of an effort's text columns and its thread.
fn effort_field_tx(
    conn: &rusqlite::Connection,
    id: EffortId,
    column: &str,
) -> Result<(Option<String>, ThreadId), DomainError> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        &format!("SELECT {column}, thread_id FROM effort WHERE id = ?1"),
        params![id.value()],
        |r| Ok((r.get(0)?, ThreadId::new(r.get(1)?))),
    )
    .optional()
    .map_err(map_sql_err)?
    .ok_or_else(|| DomainError::Invalid(format!("no effort `{id}`")))
}

/// How an effort is closed ([`finish_tx`]).
#[derive(Debug, Clone, Copy)]
pub struct EffortEnd<'a> {
    pub end_snapshot_id: Option<i64>,
    pub summary: Option<&'a str>,
    pub at: Timestamp,
    pub closed_by: ClosedBy,
}

impl<'a> EffortEnd<'a> {
    /// A close now-ish at `at`, by `closed_by`, with nothing else.
    pub fn at(at: Timestamp, closed_by: ClosedBy) -> Self {
        Self {
            end_snapshot_id: None,
            summary: None,
            at,
            closed_by,
        }
    }
}

/// How an effort ended (`effort.closed_by`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosedBy {
    /// A commit landed its changes.
    Commit,
    /// The thread moved on to other work, or its item finished.
    Switch,
    Person,
    Agent,
    /// oxplow itself (its thread closed, a migration).
    System,
}

impl ClosedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Switch => "switch",
            Self::Person => "person",
            Self::Agent => "agent",
            Self::System => "system",
        }
    }
}

/// Closes an open effort and logs `effort.closed` in the same
/// transaction. An effort that is already closed (or gone) is left alone
/// and nothing is logged; returns whether this call closed it.
pub fn finish_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    id: EffortId,
    end: &EffortEnd<'_>,
) -> Result<bool, DomainError> {
    let EffortEnd {
        end_snapshot_id,
        summary,
        at: now,
        closed_by,
    } = *end;
    use rusqlite::OptionalExtension;
    let closed: Option<(Option<String>, i64)> = conn
        .query_row(
            "UPDATE effort
             SET ended_at = ?2, end_snapshot_id = ?3, summary = ?4, closed_by = ?5
             WHERE id = ?1 AND ended_at IS NULL
             RETURNING work_item, thread_id",
            params![
                id.value(),
                ts_to_string(now),
                end_snapshot_id,
                summary,
                closed_by.as_str()
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(map_sql_err)?;
    let Some((work_item, thread)) = closed else {
        return Ok(false);
    };
    // Closed as of a past point: what came after belongs to whatever the
    // thread opens next.
    for (table, time) in EFFORT_STAMPED {
        conn.execute(
            &format!("UPDATE {table} SET effort_id = NULL WHERE effort_id = ?1 AND {time} > ?2"),
            params![id.value(), ts_to_string(now)],
        )
        .map_err(map_sql_err)?;
    }
    let env = ev
        .typed::<EffortClosed>(&EffortClosedV2 {
            effort: effort_ref(id),
            work_item: work_item.clone(),
            end_snapshot: end_snapshot_id.map(snapshot_ref),
            closed_by: Some(closed_by.as_str().to_string()),
        })
        .with_anchors(Anchors {
            effort_id: Some(id),
            snapshot_id: end_snapshot_id,
            ..anchors_for_thread_tx(conn, ThreadId::new(thread))?
        })
        .with_subject(std::iter::once(effort_ref(id)).chain(work_item));
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
    session: Option<AgentSessionId>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO effort_file
           (effort_id, path, change_kind,
            local_snapshot_id, closest_vcs_rev, vcs_rev_exact, source,
            agent_session_id, shared)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'claimed', ?7, 0)",
        params![
            id.value(),
            path,
            change_to_str(change),
            version.local_snapshot_id,
            version.closest_vcs_rev,
            if version.vcs_rev_exact { 1 } else { 0 },
            session.map(|s| s.value()),
        ],
    )?;
    Ok(())
}

/// Record the files a turn of `thread` changed (`changes`, in the window
/// `from`..=`to`) as `effort`'s `observed` files, at `version`: each one
/// it hasn't already, unless another thread's effort that overlaps the
/// window claimed it (an edit tool there named it).
pub fn observe_files_tx(
    conn: &rusqlite::Connection,
    effort: EffortId,
    thread: ThreadId,
    from: &str,
    to: &str,
    changes: &[(String, EffortFileChange)],
    version: FileRefVersion<'_>,
) -> rusqlite::Result<usize> {
    let mut observed = 0;
    for (path, change) in changes {
        let claimed_elsewhere: bool = conn.query_row(
            "SELECT EXISTS (
               SELECT 1 FROM effort_file f JOIN effort o ON o.id = f.effort_id
                WHERE f.path = ?1 AND f.source = 'claimed' AND o.thread_id != ?2
                  AND o.started_at <= ?4 AND (o.ended_at IS NULL OR o.ended_at >= ?3))",
            params![path, thread.value(), from, to],
            |r| r.get(0),
        )?;
        if claimed_elsewhere {
            continue;
        }
        observed += conn.execute(
            "INSERT INTO effort_file
               (effort_id, path, change_kind,
                local_snapshot_id, closest_vcs_rev, vcs_rev_exact, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'observed')
             ON CONFLICT (effort_id, path) DO NOTHING",
            params![
                effort.value(),
                path,
                change_to_str(*change),
                version.local_snapshot_id,
                version.closest_vcs_rev,
                if version.vcs_rev_exact { 1 } else { 0 },
            ],
        )?;
    }
    Ok(observed)
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

/// The thread's open effort, if any (at most one is open per thread).
pub fn open_for_thread_tx(
    conn: &rusqlite::Connection,
    thread: ThreadId,
) -> rusqlite::Result<Option<EffortId>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT id FROM effort WHERE thread_id = ?1 AND ended_at IS NULL",
        params![thread.value()],
        |r| r.get(0).map(EffortId::new),
    )
    .optional()
}

fn row_to_effort(row: &rusqlite::Row<'_>) -> rusqlite::Result<Effort> {
    let id: i64 = row.get("id")?;
    let work_item: Option<String> = row.get("work_item")?;
    let title: Option<String> = row.get("title")?;
    let closed_by: Option<String> = row.get("closed_by")?;
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
        title,
        closed_by,
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
    async fn set_impacts(&self, id: &EffortId, impacts: &[EffortImpact])
        -> Result<(), DomainError>;
    async fn list_for_work_item(&self, work_item: &str) -> Result<Vec<Effort>, DomainError>;
    /// The work item an effort is on; `None` when the row is gone.
    async fn work_item_for_effort(&self, id: &EffortId) -> Result<Option<String>, DomainError>;
    /// Fetch a single effort row by id. Returns `None` when the row
    /// doesn't exist (e.g. cleared during snapshot prune).
    async fn get_effort(&self, id: &EffortId) -> Result<Option<Effort>, DomainError>;
    /// Open effort (`ended_at IS NULL`) for `work_item`, if any. Used by
    /// `record_effort` to merge touched-files into the open row instead of
    /// creating a duplicate.
    async fn find_open_for_work_item(&self, work_item: &str)
        -> Result<Option<Effort>, DomainError>;
    /// The thread's open effort, if any (at most one is open per thread):
    /// what activity on the thread is attributed to now.
    async fn find_open_for_thread(&self, thread: &ThreadId) -> Result<Option<Effort>, DomainError>;
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
    /// Record a report on effort `id` — its summary and its impacts, each
    /// when given — in one transaction: both land or neither does.
    async fn record_report(
        &self,
        id: &EffortId,
        summary: Option<String>,
        impacts: Option<Vec<EffortImpact>>,
    ) -> Result<(), DomainError>;
    async fn list_files(&self, id: &EffortId) -> Result<Vec<EffortFile>, DomainError>;
    async fn list_impacts(&self, id: &EffortId) -> Result<Vec<EffortImpact>, DomainError>;
    /// Record a turn's changed files as `effort`'s observed ones (see
    /// [`observe_files_tx`]); how many it added.
    async fn observe_files(
        &self,
        effort: &EffortId,
        thread: ThreadId,
        window: (String, String),
        changes: Vec<(String, EffortFileChange)>,
        version: OwnedFileRefVersion,
    ) -> Result<usize, DomainError>;
    /// Claim `path` for `id` on behalf of `session` (the agent session
    /// whose edit named it; `None` when no session is known).
    async fn record_claimed_file(
        &self,
        id: &EffortId,
        path: &str,
        change: EffortFileChange,
        version: FileRefVersion<'_>,
        session: Option<AgentSessionId>,
    ) -> Result<(), DomainError>;
    /// [`Self::record_claimed_file`] with no session known.
    async fn record_file(
        &self,
        id: &EffortId,
        path: &str,
        change: EffortFileChange,
        version: FileRefVersion<'_>,
    ) -> Result<(), DomainError> {
        self.record_claimed_file(id, path, change, version, None)
            .await
    }
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
}

#[derive(Clone)]
pub struct SqliteEffortStore {
    db: Database,
    page_refs: SqlitePageRefStore,
    /// Validates the `effort.*` envelopes this store logs.
    vocabulary: VocabularyHandle,
}

impl SqliteEffortStore {
    /// A store with its own core schema registry. `Services` shares one
    /// registry across stores via [`Self::with_vocabulary`].
    pub fn new(db: Database) -> Self {
        Self::with_vocabulary(db, VocabularyHandle::core())
    }

    pub fn with_vocabulary(db: Database, vocabulary: VocabularyHandle) -> Self {
        Self {
            page_refs: SqlitePageRefStore::new(db.clone()),
            db,
            vocabulary,
        }
    }

    /// Re-emit the full effort-owned slice for `work_item` — the
    /// union of touched-file edges, the parsed wikilink/file/dir/
    /// task/finding/commit refs pulled from every effort's
    /// `summary` body, and the declared `EffortImpact` rows.
    /// Replaces under `effort_ref_types()` so the task-body slice
    /// (owned by `task_store`) is unaffected.
    pub async fn project_effort_slice(&self, work_item: &str) -> Result<(), DomainError> {
        let vocabulary = self.vocabulary.current();
        let work_item = work_item.to_string();
        let slice = self
            .db
            .call(move |conn| effort_slice_on(conn, &vocabulary.kinds, &work_item))
            .await?;
        self.page_refs
            .replace_source_for_ref_types(
                &slice.source_kind,
                &slice.source_id,
                effort_ref_types(),
                slice.edges,
            )
            .await
    }

    /// The effort-owned slice of every work item with an effort, read in one
    /// go — what the page-ref repair restates in batches rather than a read
    /// and a write per work item.
    pub async fn effort_slices(&self) -> Result<Vec<crate::SourceSlice>, DomainError> {
        let vocabulary = self.vocabulary.current();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT work_item FROM effort WHERE work_item IS NOT NULL ORDER BY work_item",
                )?;
                let work_items: Vec<String> = stmt
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut slices = Vec::with_capacity(work_items.len());
                for work_item in work_items {
                    match effort_slice_on(conn, &vocabulary.kinds, &work_item) {
                        Ok(slice) => slices.push(slice),
                        Err(e) => tracing::warn!(?e, %work_item, "effort slice skipped"),
                    }
                }
                Ok(slices)
            })
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

    /// The open efforts on `stream`'s threads, oldest first.
    /// The end snapshot of `thread`'s effort that closed exactly at `at`:
    /// where an effort that starts there (adopting no further back than
    /// its predecessor's close) begins.
    pub async fn end_snapshot_closed_at(
        &self,
        thread: ThreadId,
        at: Timestamp,
    ) -> Result<Option<i64>, DomainError> {
        use rusqlite::OptionalExtension;
        let at = ts_to_string(at);
        self.db
            .call(move |c| {
                c.query_row(
                    "SELECT end_snapshot_id FROM effort
                      WHERE thread_id = ?1 AND ended_at = ?2 AND end_snapshot_id IS NOT NULL
                      LIMIT 1",
                    params![thread.value(), at],
                    |r| r.get(0),
                )
                .optional()
            })
            .await
    }

    /// `thread`'s most recently started effort.
    pub async fn latest_for_thread(&self, thread: ThreadId) -> Result<Option<Effort>, DomainError> {
        self.db
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT * FROM effort WHERE thread_id = ?1
                      ORDER BY started_at DESC, id DESC LIMIT 1",
                )?;
                let mut rows = stmt.query_map(params![thread.value()], row_to_effort)?;
                rows.next().transpose()
            })
            .await
    }

    /// Close `id` now at `end_snapshot_id`, as `by` closed it.
    pub async fn close(
        &self,
        id: EffortId,
        end_snapshot_id: Option<i64>,
        by: ClosedBy,
    ) -> Result<(), DomainError> {
        let vocabulary = self.vocabulary.clone();
        self.db
            .transaction(move |tx| {
                let vocabulary = vocabulary.current();
                let ev = EventCtx::system(&vocabulary, "effort_store");
                finish_tx(
                    tx,
                    &ev,
                    id,
                    &EffortEnd {
                        end_snapshot_id,
                        ..EffortEnd::at(Timestamp::now(), by)
                    },
                )
            })
            .await
            .map(|_| ())
    }

    pub async fn list_open_for_stream(&self, stream: StreamId) -> Result<Vec<Effort>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT e.* FROM effort e JOIN threads th ON th.id = e.thread_id
                      WHERE th.stream_id = ?1 AND e.ended_at IS NULL
                      ORDER BY e.started_at, e.id",
                )?;
                let rows = stmt.query_map(params![stream.value()], row_to_effort)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Every work item that has an effort — an oxplow task's or another
    /// provider's — for the boot `page_ref` backfill.
    pub async fn list_work_items(&self) -> Result<Vec<String>, DomainError> {
        self.db
            .call(|conn| {
                let mut stmt =
                    conn.prepare("SELECT DISTINCT work_item FROM effort ORDER BY work_item")?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
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

    /// The paths `effort` changed (its `effort_file` rows).
    pub async fn paths(&self, effort: &EffortId) -> Result<Vec<String>, DomainError> {
        let id = effort.value();
        self.db
            .call(move |conn| {
                let mut stmt = conn
                    .prepare("SELECT path FROM effort_file WHERE effort_id = ?1 ORDER BY path")?;
                let rows = stmt.query_map([id], |r| r.get(0))?;
                rows.collect()
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
    /// to "effort without a pin" rather than "no effort row". Only an
    /// unpinned effort is stamped: a redelivered event can't move a pin.
    pub async fn set_start_snapshot(
        &self,
        id: &EffortId,
        snapshot_id: i64,
    ) -> Result<(), DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE effort SET start_snapshot_id = ?2
                     WHERE id = ?1 AND start_snapshot_id IS NULL",
                    params![id.value(), snapshot_id],
                )?;
                Ok(())
            })
            .await
    }

    /// Backfill the end-snapshot pin, when it's still unset. See
    /// [`Self::set_start_snapshot`].
    pub async fn set_end_snapshot(
        &self,
        id: &EffortId,
        snapshot_id: i64,
    ) -> Result<(), DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE effort SET end_snapshot_id = ?2
                     WHERE id = ?1 AND end_snapshot_id IS NULL",
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
        let vocabulary = self.vocabulary.clone();
        let id = self
            .db
            .transaction(move |tx| {
                let vocabulary = vocabulary.current();
                let ev = EventCtx::system(&vocabulary, "effort_store");
                start_tx(
                    tx,
                    &ev,
                    &EffortStart {
                        work_item: Some(&w),
                        start_snapshot_id,
                        ..EffortStart::at(thread, now)
                    },
                )
            })
            .await?;
        Ok(Effort {
            id,
            work_item: Some(work_item),
            title: None,
            closed_by: None,
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
        let vocabulary = self.vocabulary.clone();
        self.db
            .transaction(move |tx| {
                let vocabulary = vocabulary.current();
                let ev = EventCtx::system(&vocabulary, "effort_store");
                finish_tx(
                    tx,
                    &ev,
                    id_for_sql,
                    &EffortEnd {
                        end_snapshot_id,
                        summary: summary.as_deref(),
                        at: now,
                        closed_by: ClosedBy::Agent,
                    },
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
                let mut rows =
                    stmt.query_map(params![id.value()], |r| r.get::<_, Option<String>>(0))?;
                Ok(rows.next().transpose()?.flatten())
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

    async fn most_recent_for_work_item(
        &self,
        work_item: &str,
    ) -> Result<Option<Effort>, DomainError> {
        let work_item = work_item.to_string();
        self.db
            .call(move |conn| most_recent_for_work_item_tx(conn, &work_item))
            .await
    }

    async fn record_report(
        &self,
        id: &EffortId,
        summary: Option<String>,
        impacts: Option<Vec<EffortImpact>>,
    ) -> Result<(), DomainError> {
        let id_for_sql = *id;
        let impacts_json = match impacts {
            None => None,
            Some(v) if v.is_empty() => Some(None),
            Some(v) => Some(Some(serde_json::to_string(&v).map_err(|e| {
                DomainError::Invalid(format!("impacts serialize failed: {e}"))
            })?)),
        };
        self.db
            .transaction(move |tx| {
                if let Some(summary) = &summary {
                    set_summary_tx(tx, id_for_sql, Some(summary)).map_err(map_sql_err)?;
                }
                if let Some(json) = &impacts_json {
                    set_impacts_json_tx(tx, id_for_sql, json.as_deref()).map_err(map_sql_err)?;
                }
                Ok(())
            })
            .await?;
        if let Some(w) = self.work_item_for_effort(id).await? {
            self.project_effort_slice(&w).await?;
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
                            local_snapshot_id, closest_vcs_rev, vcs_rev_exact, source
                     FROM effort_file
                     WHERE effort_id = ?1 ORDER BY path ASC",
                )?;
                let rows = stmt.query_map(params![id.value()], |r| {
                    let effort_id: i64 = r.get(0)?;
                    let path: String = r.get(1)?;
                    let kind: String = r.get(2)?;
                    let local_snapshot_id: i64 = r.get(3)?;
                    let closest_vcs_rev: Option<String> = r.get(4)?;
                    let vcs_rev_exact: i64 = r.get(5)?;
                    let source: String = r.get(6)?;
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
                        closest_vcs_rev,
                        vcs_rev_exact: vcs_rev_exact != 0,
                        source: if source == "observed" {
                            FileSource::Observed
                        } else {
                            FileSource::Claimed
                        },
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn set_impacts(
        &self,
        id: &EffortId,
        impacts: &[EffortImpact],
    ) -> Result<(), DomainError> {
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

    async fn list_impacts(&self, id: &EffortId) -> Result<Vec<EffortImpact>, DomainError> {
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

    async fn observe_files(
        &self,
        effort: &EffortId,
        thread: ThreadId,
        window: (String, String),
        changes: Vec<(String, EffortFileChange)>,
        version: OwnedFileRefVersion,
    ) -> Result<usize, DomainError> {
        let effort = *effort;
        let added = self
            .db
            .transaction(move |tx| {
                observe_files_tx(
                    tx,
                    effort,
                    thread,
                    &window.0,
                    &window.1,
                    &changes,
                    version.as_ref(),
                )
                .map_err(map_sql_err)
            })
            .await?;
        if added > 0 {
            if let Some(w) = self.work_item_for_effort(&effort).await? {
                self.project_effort_slice(&w).await?;
            }
        }
        Ok(added)
    }

    async fn record_claimed_file(
        &self,
        id: &EffortId,
        path: &str,
        change: EffortFileChange,
        version: FileRefVersion<'_>,
        session: Option<AgentSessionId>,
    ) -> Result<(), DomainError> {
        let id_clone = *id;
        let owned = OwnedFileRefVersion {
            local_snapshot_id: version.local_snapshot_id,
            closest_vcs_rev: version.closest_vcs_rev.map(|s| s.to_string()),
            vcs_rev_exact: version.vcs_rev_exact,
        };
        let path_clone = path.to_string();
        self.db
            .call(move |conn| {
                record_file_tx(conn, id_clone, &path_clone, change, owned.as_ref(), session)
            })
            .await?;
        {
            if let Some(w) = self.work_item_for_effort(id).await? {
                self.project_effort_slice(&w).await?;
            }
        }
        Ok(())
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

/// `work_item`'s effort-owned page-ref slice (see
/// [`SqliteEffortStore::project_effort_slice`]), from its efforts' rows.
fn effort_slice_on(
    conn: &rusqlite::Connection,
    kinds: &oxplow_domain::refs::kind::KindRegistry,
    work_item: &str,
) -> rusqlite::Result<crate::SourceSlice> {
    let Some(source) = work_item_id_of_ref(work_item).map(str::to_string) else {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "`{work_item}` is not a work_item ref"
        )));
    };
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
    let mut impacts: Vec<EffortImpact> = Vec::new();
    for j in &impact_jsons {
        match serde_json::from_str::<Vec<EffortImpact>>(j) {
            Ok(rows) => impacts.extend(rows),
            Err(e) => {
                tracing::warn!(?e, "effort impacts_json deserialize failed; skipping");
            }
        }
    }
    let mut edges = effort_touched_file_edges(&source, &paths);
    edges.extend(effort_summary_edges(kinds, &source, &summaries));
    edges.extend(effort_impact_edges(kinds, &source, &impacts));
    Ok(crate::SourceSlice {
        source_kind: KIND_WORK_ITEM.to_string(),
        source_id: source,
        ref_types: Some(effort_ref_types()),
        edges,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_store::SqliteStreamStore;
    use crate::test_tasks::a_task;
    use crate::thread_store::SqliteThreadStore;
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadStatus};

    async fn fixture() -> (SqliteEffortStore, String, ThreadId) {
        let (store, _db, tid, thread) = fixture_with_db().await;
        (store, tid, thread)
    }

    /// A tool call at `at` on `thread`, with no effort yet (as one made
    /// before any effort opened).
    fn tool_call(conn: &rusqlite::Connection, thread: ThreadId, at: &str) -> i64 {
        conn.execute(
            "INSERT INTO agent_tool_call (thread_id, tool, path, at) VALUES (?1, 'Edit', 'a.rs', ?2)",
            params![thread.value(), at],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn effort_of_call(conn: &rusqlite::Connection, id: i64) -> Option<i64> {
        conn.query_row(
            "SELECT effort_id FROM agent_tool_call WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn ts(s: &str) -> Timestamp {
        Timestamp::parse(s).unwrap()
    }

    /// An effort opened late adopts the thread's un-efforted activity back
    /// to `adopt_since` — never past the thread's previous effort's end,
    /// never another thread's.
    #[tokio::test]
    async fn opening_adopts_the_threads_activity_since_its_last_effort() {
        let (_store, db, _tid, thread) = fixture_with_db().await;
        let vocabulary = oxplow_domain::vocabulary::Vocabulary::core();
        db.transaction(move |tx| {
            let ev = EventCtx::system(&vocabulary, "test");
            let before = tool_call(tx, thread, "2026-01-01T00:00:01.000000Z");
            let prev = start_tx(
                tx,
                &ev,
                &EffortStart::at(thread, ts("2026-01-01T00:00:02.000000Z")),
            )?;
            finish_tx(
                tx,
                &ev,
                prev,
                &EffortEnd::at(ts("2026-01-01T00:00:03.000000Z"), ClosedBy::Commit),
            )?;
            let after_prev = tool_call(tx, thread, "2026-01-01T00:00:04.000000Z");
            let later = tool_call(tx, thread, "2026-01-01T00:00:05.000000Z");
            let id = start_tx(
                tx,
                &ev,
                &EffortStart {
                    adopt_since: Some(ts("2026-01-01T00:00:00.000000Z")),
                    ..EffortStart::at(thread, ts("2026-01-01T00:00:06.000000Z"))
                },
            )?;
            // Clamped to the previous effort's end.
            let started: String = tx
                .query_row(
                    "SELECT started_at FROM effort WHERE id = ?1",
                    [id.value()],
                    |r| r.get(0),
                )
                .map_err(map_sql_err)?;
            assert_eq!(started, "2026-01-01T00:00:03.000000Z");
            assert_eq!(effort_of_call(tx, before), None);
            assert_eq!(effort_of_call(tx, after_prev), Some(id.value()));
            assert_eq!(effort_of_call(tx, later), Some(id.value()));
            assert_eq!(
                effort_at_tx(tx, thread, ts("2026-01-01T00:00:04.500000Z")).map_err(map_sql_err)?,
                Some(id)
            );
            assert_eq!(
                effort_at_tx(tx, thread, ts("2026-01-01T00:00:02.500000Z")).map_err(map_sql_err)?,
                Some(prev)
            );
            Ok(())
        })
        .await
        .unwrap();
    }

    /// Closing as of a past point leaves what came after it to the next
    /// effort.
    #[tokio::test]
    async fn closing_as_of_releases_what_came_after() {
        let (_store, db, _tid, thread) = fixture_with_db().await;
        let vocabulary = oxplow_domain::vocabulary::Vocabulary::core();
        db.transaction(move |tx| {
            let ev = EventCtx::system(&vocabulary, "test");
            let id = start_tx(
                tx,
                &ev,
                &EffortStart::at(thread, ts("2026-01-01T00:00:01.000000Z")),
            )?;
            let inside = tool_call(tx, thread, "2026-01-01T00:00:02.000000Z");
            let after = tool_call(tx, thread, "2026-01-01T00:00:04.000000Z");
            tx.execute("UPDATE agent_tool_call SET effort_id = ?1", [id.value()])
                .map_err(map_sql_err)?;
            finish_tx(
                tx,
                &ev,
                id,
                &EffortEnd::at(ts("2026-01-01T00:00:03.000000Z"), ClosedBy::Commit),
            )?;
            assert_eq!(effort_of_call(tx, inside), Some(id.value()));
            assert_eq!(effort_of_call(tx, after), None);
            Ok(())
        })
        .await
        .unwrap();
    }

    /// An effort needs no work item: unlinked, its title is the first
    /// line of the thread's first prompt in it; a thread holds one open.
    #[tokio::test]
    async fn an_unlinked_effort_takes_its_first_prompts_title() {
        let (_store, db, _tid, thread) = fixture_with_db().await;
        let vocabulary = oxplow_domain::vocabulary::Vocabulary::core();
        let (title, second) = db
            .transaction(move |tx| {
                tx.execute(
                    "INSERT INTO agent_turn (thread_id, prompt, session_id, started_at)
                     VALUES (?1, 'Fix the login page\nIt 500s.', 's', '2026-01-01T00:00:00Z')",
                    params![thread.value()],
                )
                .map_err(map_sql_err)?;
                let ev = EventCtx::system(&vocabulary, "test");
                let id = start_tx(tx, &ev, &EffortStart::at(thread, Timestamp::now()))?;
                let title: String = tx
                    .query_row(
                        "SELECT title FROM v_effort WHERE id = ?1",
                        [id.value()],
                        |r| r.get(0),
                    )
                    .map_err(map_sql_err)?;
                start_tx(tx, &ev, &EffortStart::at(thread, Timestamp::now()))?;
                let open: i64 = tx
                    .query_row(
                        "SELECT count(*) FROM effort WHERE thread_id = ?1 AND ended_at IS NULL",
                        [thread.value()],
                        |r| r.get(0),
                    )
                    .map_err(map_sql_err)?;
                Ok((title, open == 1))
            })
            .await
            .unwrap();
        assert_eq!(title, "Fix the login page");
        assert!(second, "opening another switches: one stays open");
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

    async fn fixture_with_db() -> (SqliteEffortStore, Database, String, ThreadId) {
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
            host: oxplow_domain::HostId::LOCAL,
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
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteThreadStore::new(db.clone()).upsert(&t).await.unwrap();
        let tid = a_task(&db, Some(t.id)).await;
        (SqliteEffortStore::new(db.clone()), db, tid, t.id)
    }

    /// P2.5b (tsk428): an effort is on a work item — an oxplow task's or
    /// another provider's — and the store never accepts a non-ref.
    #[tokio::test]
    async fn efforts_are_keyed_by_work_item_ref() {
        let (store, tid, t) = fixture().await;
        let ours = tid.clone();
        let eff = store.start(&ours, &t, None).await.unwrap();
        assert_eq!(eff.work_item.as_deref(), Some(ours.as_str()));

        let foreign = "work_item:issues:ENG-12";
        store.finish(&eff.id, None, None).await.unwrap();
        let other = store.start(foreign, &t, None).await.unwrap();
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
        let eff = store.start(&tid, &t, None).await.unwrap();
        assert!(eff.ended_at.is_none());
        store
            .finish(&eff.id, None, Some("done".into()))
            .await
            .unwrap();
        let list = store.list_for_work_item(&tid).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].ended_at.is_some());
        assert_eq!(list[0].summary.as_deref(), Some("done"));
    }

    /// A claim carries the session whose edit named the file, and
    /// `shared = 0`.
    #[tokio::test]
    async fn a_claimed_file_carries_its_session() {
        let (store, db, _tid, t) = fixture_with_db().await;
        let eff = store
            .start("work_item:oxplow:tsk1", &t, None)
            .await
            .unwrap();
        let v = FileRefVersion {
            local_snapshot_id: 0,
            closest_vcs_rev: None,
            vcs_rev_exact: false,
        };
        store
            .record_claimed_file(
                &eff.id,
                "a.rs",
                EffortFileChange::Updated,
                v,
                Some(AgentSessionId::new(4)),
            )
            .await
            .unwrap();
        let row: (Option<i64>, i64) = db
            .call(|c| {
                c.query_row(
                    "SELECT agent_session_id, shared FROM effort_file WHERE path = 'a.rs'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!(row, (Some(4), 0));
    }

    /// A turn's observed files: a file another thread's overlapping effort
    /// claimed stays that thread's; one this effort already claimed stays
    /// claimed; a second observation adds nothing.
    #[tokio::test]
    async fn observing_files_skips_other_threads_claims_and_keeps_its_own() {
        let (store, db, _tid, t) = fixture_with_db().await;
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                   VALUES (2, 1, 'other', 'queued', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');",
            )
        })
        .await
        .unwrap();
        let mine = store.start("work_item:issues:A-1", &t, None).await.unwrap();
        let theirs = store
            .start("work_item:issues:B-1", &ThreadId::new(2), None)
            .await
            .unwrap();
        let v = FileRefVersion {
            local_snapshot_id: 0,
            closest_vcs_rev: None,
            vcs_rev_exact: false,
        };
        store
            .record_file(&theirs.id, "theirs.rs", EffortFileChange::Updated, v)
            .await
            .unwrap();
        store
            .record_file(&mine.id, "claimed.rs", EffortFileChange::Updated, v)
            .await
            .unwrap();
        let window = (
            "2000-01-01T00:00:00Z".to_string(),
            "2999-01-01T00:00:00Z".to_string(),
        );
        let changes = vec![
            ("theirs.rs".to_string(), EffortFileChange::Updated),
            ("claimed.rs".to_string(), EffortFileChange::Updated),
            ("shell.rs".to_string(), EffortFileChange::Created),
        ];
        let owned = || OwnedFileRefVersion {
            local_snapshot_id: 0,
            closest_vcs_rev: None,
            vcs_rev_exact: false,
        };
        let added = store
            .observe_files(&mine.id, t, window.clone(), changes.clone(), owned())
            .await
            .unwrap();
        assert_eq!(added, 1);
        let files: Vec<(String, FileSource)> = store
            .list_files(&mine.id)
            .await
            .unwrap()
            .into_iter()
            .map(|f| (f.path, f.source))
            .collect();
        assert_eq!(
            files,
            vec![
                ("claimed.rs".to_string(), FileSource::Claimed),
                ("shell.rs".to_string(), FileSource::Observed),
            ]
        );
        assert_eq!(
            store
                .observe_files(&mine.id, t, window, changes, owned())
                .await
                .unwrap(),
            0
        );
    }

    /// Every effort open and close is logged in the write's own
    /// transaction.
    #[tokio::test]
    async fn effort_open_and_close_are_logged_with_the_write() {
        let (store, db, _tid, t) = fixture_with_db().await;
        let foreign = "work_item:issues:ENG-12";
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

        let events = db
            .call_mut(|c| crate::event_log_store::read_after_tx(c, 0, 10))
            .await
            .unwrap();
        let seen: Vec<(String, Vec<String>)> = events
            .iter()
            .map(|e| (e.envelope.event_type.clone(), e.envelope.subject.clone()))
            .collect();
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
    }

    #[tokio::test]
    async fn opening_another_effort_switches_the_thread() {
        // A thread holds one open effort (V5): opening another closes the
        // one it had, as a switch.
        let (store, tid, t) = fixture().await;
        let first = store.start(&tid, &t, None).await.unwrap();
        let second = store
            .start("work_item:issues:ENG-1", &t, None)
            .await
            .unwrap();
        let open = store
            .find_open_for_thread(&t)
            .await
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(
            open.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![second.id]
        );
        let closed = store.get_effort(&first.id).await.unwrap().unwrap();
        assert_eq!(closed.closed_by.as_deref(), Some("switch"));
    }

    #[tokio::test]
    async fn find_open_for_thread_finds_the_one_open_effort() {
        let (store, db, tid, thread) = fixture_with_db().await;
        // Zero open → None.
        assert!(store.find_open_for_thread(&thread).await.unwrap().is_none());
        // Exactly one open → Some.
        store.start(&tid, &thread, None).await.unwrap();
        assert!(store.find_open_for_thread(&thread).await.unwrap().is_some());
        // Opening another moves the thread on: still exactly one open.
        let _ = db;
        store
            .start("work_item:issues:ENG-1", &thread, None)
            .await
            .unwrap();
        assert!(store.find_open_for_thread(&thread).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn record_then_list_files() {
        let (store, tid, t) = fixture().await;
        let eff = store.start(&tid, &t, None).await.unwrap();
        let v = FileRefVersion {
            local_snapshot_id: 0,
            closest_vcs_rev: None,
            vcs_rev_exact: false,
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
        // oxplow's tasks the work list: `tsk99` is one of its items.
        let mut vocabulary = oxplow_domain::vocabulary::Vocabulary::core();
        vocabulary.kinds = vocabulary
            .kinds
            .with_work_item_ids("oxplow", r"tsk\d+")
            .unwrap();
        let store = SqliteEffortStore::with_vocabulary(
            db,
            oxplow_domain::vocabulary::VocabularyHandle::new(vocabulary),
        );
        let eff = store.start(&tid, &t, None).await.unwrap();
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
                && e.source_id == tid.trim_start_matches("work_item:")
                && e.ref_type == "summary_wikilink"),
            "wiki backlink missing; got {wiki_back:?}"
        );

        let file_back = page_refs
            .list_backlinks("file", "src/foo.rs", None)
            .await
            .unwrap();
        assert!(
            file_back.iter().any(|e| e.ref_type == "summary_file_ref"
                && e.source_id == tid.trim_start_matches("work_item:")),
            "file backlink missing; got {file_back:?}"
        );

        let task_back = page_refs
            .list_backlinks("work_item", "oxplow:tsk99", None)
            .await
            .unwrap();
        assert!(
            task_back
                .iter()
                .any(|e| e.ref_type == "summary_work_item_mention"
                    && e.source_id == tid.trim_start_matches("work_item:")),
            "task backlink missing; got {task_back:?}"
        );
    }

    /// A report's summary and impacts land together, in one transaction;
    /// one left out keeps what the effort had.
    #[tokio::test]
    async fn a_report_records_its_summary_and_impacts_together() {
        use oxplow_domain::EffortImpact;
        let (_, db, tid, t) = fixture_with_db().await;
        let store = SqliteEffortStore::new(db);
        let eff = store.start(&tid, &t, None).await.unwrap();
        let impacts = vec![EffortImpact {
            kind: "wiki".into(),
            id: "url-schemes".into(),
            action: Some("created".into()),
        }];
        store
            .record_report(&eff.id, Some("Done.".into()), Some(impacts.clone()))
            .await
            .unwrap();
        let after = store.get_effort(&eff.id).await.unwrap().unwrap();
        assert_eq!(after.summary.as_deref(), Some("Done."));
        assert_eq!(store.list_impacts(&eff.id).await.unwrap(), impacts);
        store
            .record_report(&eff.id, Some("Done, again.".into()), None)
            .await
            .unwrap();
        assert_eq!(store.list_impacts(&eff.id).await.unwrap(), impacts);
    }

    #[tokio::test]
    async fn set_impacts_projects_edges_and_round_trips() {
        use crate::page_ref_store::SqlitePageRefStore;
        use oxplow_domain::EffortImpact;
        let (_, db, tid, t) = fixture_with_db().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let store = SqliteEffortStore::new(db);
        let eff = store.start(&tid, &t, None).await.unwrap();
        let impacts = vec![
            EffortImpact {
                kind: "wiki".into(),
                id: "url-schemes".into(),
                action: Some("created".into()),
            },
            EffortImpact {
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
            .find(|e| e.source_id == tid.trim_start_matches("work_item:"))
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
        assert!(
            commit
                .iter()
                .any(|e| e.source_id == tid.trim_start_matches("work_item:")
                    && e.ref_type == "impact")
        );

        // Replacing the impact set clears old edges
        store
            .set_impacts(
                &eff.id,
                &[EffortImpact {
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
            wiki.iter()
                .all(|e| e.source_id != tid.rsplit(':').next().unwrap()),
            "old wiki impact edge wasn't replaced: {wiki:?}"
        );

        // Empty list nulls the column and clears all impact edges
        store.set_impacts(&eff.id, &[]).await.unwrap();
        let wiki = page_refs
            .list_backlinks("wiki", "other-page", None)
            .await
            .unwrap();
        assert!(wiki
            .iter()
            .all(|e| e.source_id != tid.rsplit(':').next().unwrap()));
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
        let first = store.start(&tid, &t, None).await.unwrap();
        store
            .finish(&first.id, None, Some("Filed [[url-schemes]]".into()))
            .await
            .unwrap();

        let second = store.start(&tid, &t, None).await.unwrap();
        let v = FileRefVersion {
            local_snapshot_id: 0,
            closest_vcs_rev: None,
            vcs_rev_exact: false,
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
                .any(|e| e.source_id == tid.trim_start_matches("work_item:")),
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
            host: oxplow_domain::HostId::LOCAL,
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
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteThreadStore::new(db.clone()).upsert(&t).await.unwrap();
        let tid = a_task(&db, Some(t.id)).await;
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
        let a = store.start(&tid, &t.id, Some(snap1)).await.unwrap();
        store.finish(&a.id, Some(snap2), None).await.unwrap();
        // Effort B: start@snap2, still open — active at snap2 and
        // snap3.
        let b = store.start(&tid, &t.id, Some(snap2)).await.unwrap();

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
            host: oxplow_domain::HostId::LOCAL,
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
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteThreadStore::new(db.clone()).upsert(&t).await.unwrap();
        let tid = a_task(&db, Some(t.id)).await;
        let snap_store = crate::SqliteSnapshotStore::new(db.clone());
        let s1 = snap_store.create_snapshot(s.id).await.unwrap();
        let s2 = snap_store.create_snapshot(s.id).await.unwrap();
        let _s3 = snap_store.create_snapshot(s.id).await.unwrap();
        let s4 = snap_store.create_snapshot(s.id).await.unwrap();
        let s5 = snap_store.create_snapshot(s.id).await.unwrap();
        let store = SqliteEffortStore::new(db);

        // Range under test: (s2, s4].
        // A [s1,s2] — ends exactly at range start → excluded.
        let a = store.start(&tid, &t.id, Some(s1)).await.unwrap();
        store.finish(&a.id, Some(s2), None).await.unwrap();
        // B [s2,s4] — straddles the range end → included.
        let b = store.start(&tid, &t.id, Some(s2)).await.unwrap();
        store.finish(&b.id, Some(s4), None).await.unwrap();
        // C [s4,s5] — starts exactly at range end → excluded.
        let c = store.start(&tid, &t.id, Some(s4)).await.unwrap();
        store.finish(&c.id, Some(s5), None).await.unwrap();
        // D [s1,s5] — fully contains the range → included.
        let d = store.start(&tid, &t.id, Some(s1)).await.unwrap();
        store.finish(&d.id, Some(s5), None).await.unwrap();
        // E [s2,open] — still in progress → included.
        let e = store.start(&tid, &t.id, Some(s2)).await.unwrap();

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
    async fn stream_thread_task(db: &Database, n: i64) -> (StreamId, ThreadId, String) {
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
            host: oxplow_domain::HostId::LOCAL,
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
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteThreadStore::new(db.clone()).upsert(&t).await.unwrap();
        let tid = a_task(db, Some(t.id)).await;
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
        let ea = store.start(&tida, &ta, Some(a1)).await.unwrap();
        store.finish(&ea.id, Some(a2), None).await.unwrap();
        let eb = store.start(&tidb, &tb, Some(b1)).await.unwrap();
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
