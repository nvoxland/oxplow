use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::{params, OptionalExtension};

use oxplow_domain::events::schema::{
    WorkItemCreated, WorkItemCreatedV1, WorkItemDeleted, WorkItemDeletedV1, WorkItemEdited,
    WorkItemEditedV1, WorkItemTransitioned, WorkItemTransitionedV1,
};
use oxplow_domain::refs::build::{effort_ref, work_item_ref};
use oxplow_domain::stores::TaskStore;
use oxplow_domain::{
    Anchors, DomainError, EffortId, EventSchemaRegistry, Task, TaskActorKind, TaskAuthor, TaskId,
    TaskPriority, TaskStatus, ThreadId, Timestamp,
};

use crate::database::Database;
use crate::database::{string_to_ts, ts_to_string};
use crate::event_log_store::{anchors_for_thread_tx, EventCtx};
use crate::page_ref_projections::{task_body_ref_types, task_edges, work_item_id, KIND_WORK_ITEM};
use crate::page_ref_store::SqlitePageRefStore;

#[derive(Clone)]
pub struct SqliteTaskStore {
    db: Database,
    page_refs: SqlitePageRefStore,
    /// Validates the `work_item.transitioned` envelopes this store logs.
    event_schemas: Arc<EventSchemaRegistry>,
}

/// What a status change did to the task's effort (see [`update_logged_tx`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EffortTransition {
    /// Entered in_progress — this effort row was opened (or an
    /// already-open one adopted) with no snapshot pin yet.
    Opened(EffortId),
    /// Left in_progress — this open effort row was finished with no
    /// end-snapshot pin yet.
    Finished(EffortId),
    /// Left in_progress but no open effort existed to finish.
    NoOpenEffort,
    /// The change didn't cross the in_progress boundary on a thread.
    Untouched,
}

/// A status change made by [`set_status_tx`]: the row before and after,
/// and what happened to its effort.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusChange {
    pub before: Task,
    pub after: Task,
    pub effort: EffortTransition,
}

impl SqliteTaskStore {
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

    /// Every live thread-attached task currently `in_progress`. Used
    /// by boot recovery to heal the "in_progress without an open
    /// effort" orphan.
    pub async fn list_in_progress(&self) -> Result<Vec<Task>, DomainError> {
        self.db
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM task
                     WHERE status = 'in_progress'
                       AND deleted_at IS NULL
                       AND thread_id IS NOT NULL
                     ORDER BY id",
                )?;
                let rows = stmt.query_map([], row_to_task)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Insert a task and apply what its initial status implies, in one
    /// transaction (see [`insert_logged_tx`]). Returns the id and the
    /// effort it opened, if any.
    pub async fn insert_logged(
        &self,
        item: &Task,
    ) -> Result<(TaskId, Option<EffortId>), DomainError> {
        let owned = Arc::new(item.clone());
        let schemas = self.event_schemas.clone();
        let (id, effort) = self
            .db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "task_service");
                insert_logged_tx(tx, &ev, &owned)
            })
            .await?;
        self.project_body_refs(item, id).await?;
        Ok((id, effort))
    }

    /// Re-project a task's body mentions into `page_ref` (the task-body
    /// slice; the effort slice is the effort store's).
    async fn project_body_refs(&self, item: &Task, id: TaskId) -> Result<(), DomainError> {
        let mut placed = item.clone();
        placed.id = id;
        self.page_refs
            .replace_source_for_ref_types(
                KIND_WORK_ITEM,
                &work_item_id(id),
                task_body_ref_types(),
                task_edges(&placed),
            )
            .await
    }

    /// Move a task to another thread (or the backlog, `None`) at the end
    /// of its list, taking its claim with it in the same transaction: an
    /// open effort on the old thread closes, and an `in_progress` task
    /// landing on a thread opens one there (another stream's included —
    /// an effort's snapshots belong to one stream). Returns the moved row.
    pub async fn move_task(&self, id: TaskId, dest: Option<ThreadId>) -> Result<Task, DomainError> {
        let schemas = self.event_schemas.clone();
        let moved = self
            .db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "task_service");
                let now = Timestamp::now();
                let mut item = get_task_tx(tx, id)?.ok_or(DomainError::NotFound)?;
                if item.thread_id == dest {
                    return Ok(item);
                }
                item.thread_id = dest;
                item.sort_index = next_sort_index_tx(tx, dest)?;
                item.updated_at = now;
                if update_task_tx(tx, &item).map_err(crate::database::map_sql_err)? == 0 {
                    return Err(DomainError::NotFound);
                }
                log_edited_tx(tx, &ev, &item, vec!["thread".to_string()])?;
                let work_item = work_item_ref(id);
                if let Some(open) = crate::effort_store::find_open_for_work_item_tx(tx, &work_item)
                    .map_err(crate::database::map_sql_err)?
                {
                    crate::effort_store::finish_tx(tx, &ev, open.id, None, None, now, false)?;
                }
                if let (Some(thread), TaskStatus::InProgress) = (dest, item.status) {
                    crate::effort_store::start_tx(tx, &ev, &work_item, thread, None, now, false)?;
                }
                Ok(item)
            })
            .await?;
        Ok(moved)
    }

    /// Move task `id` to `to` in its own transaction ([`set_status_tx`]).
    pub async fn set_status(
        &self,
        id: TaskId,
        to: TaskStatus,
    ) -> Result<StatusChange, DomainError> {
        let schemas = self.event_schemas.clone();
        self.db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "task_service");
                set_status_tx(tx, &ev, id, to, Timestamp::now())
            })
            .await
    }

    /// Write an edited row's fields and, when `status` is given, move it
    /// there — one transaction (see [`update_with_status_tx`]). Returns the
    /// row as committed.
    pub async fn update_with_status(
        &self,
        item: &Task,
        status: Option<TaskStatus>,
    ) -> Result<Task, DomainError> {
        let owned = Arc::new(item.clone());
        let schemas = self.event_schemas.clone();
        let after = self
            .db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "task_service");
                update_with_status_tx(tx, &ev, &owned, status, Timestamp::now())
                    .map(|(after, _)| after)
            })
            .await?;
        self.project_body_refs(&after, after.id).await?;
        Ok(after)
    }
}

/// What a status change implies, inside the caller's transaction:
/// open (or adopt) the effort when a thread-attached task enters
/// `in_progress`, close it when one leaves, and log
/// `work_item.transitioned@1` when the status changed — every change, not
/// only in_progress crossings, and for thread-less tasks too (no stream
/// anchor then). Snapshot pins are not part of this: the effort-lifecycle
/// consumer takes them after commit. No dedupe key: a transactional
/// producer's retry has already rolled back.
fn apply_status_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    item: &Task,
    from: TaskStatus,
) -> Result<EffortTransition, DomainError> {
    write_status_tx(conn, item)?;
    let transition = effort_for_status_tx(conn, ev, item, from)?;
    let work_item = work_item_ref(item.id);
    if from != item.status {
        let effort = match transition {
            EffortTransition::Opened(e) | EffortTransition::Finished(e) => Some(e),
            EffortTransition::NoOpenEffort | EffortTransition::Untouched => None,
        };
        let mut subject = vec![work_item.clone()];
        subject.extend(effort.map(effort_ref));
        let anchors = match item.thread_id {
            Some(thread) => anchors_for_thread_tx(conn, thread)?,
            None => Anchors::default(),
        };
        let env = ev
            .typed::<WorkItemTransitioned>(&WorkItemTransitionedV1 {
                work_item,
                from,
                to: item.status,
                effort: effort.map(effort_ref),
            })
            .with_anchors(Anchors {
                effort_id: effort,
                ..anchors
            })
            .with_subject(subject);
        ev.append(conn, &env)?;
    }
    Ok(transition)
}

/// The effort a status change implies: open (or adopt) one when a
/// thread-attached task enters `in_progress` from `from`, close the open
/// one when it leaves. Logs the effort's own events, not the transition.
fn effort_for_status_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    item: &Task,
    from: TaskStatus,
) -> Result<EffortTransition, DomainError> {
    use crate::database::map_sql_err;
    let work_item = work_item_ref(item.id);
    let crossed_in = from != TaskStatus::InProgress && item.status == TaskStatus::InProgress;
    let crossed_out = from == TaskStatus::InProgress && item.status != TaskStatus::InProgress;
    Ok(match item.thread_id {
        Some(thread) if crossed_in => {
            match crate::effort_store::find_open_for_work_item_tx(conn, &work_item)
                .map_err(map_sql_err)?
            {
                // Adopted, not an error: the unique open index makes a true
                // double-open impossible.
                Some(open) => EffortTransition::Opened(open.id),
                None => EffortTransition::Opened(crate::effort_store::start_tx(
                    conn,
                    ev,
                    &work_item,
                    thread,
                    None,
                    Timestamp::now(),
                    false,
                )?),
            }
        }
        Some(_) if crossed_out => {
            match crate::effort_store::find_open_for_work_item_tx(conn, &work_item)
                .map_err(map_sql_err)?
            {
                Some(open) => {
                    crate::effort_store::finish_tx(
                        conn,
                        ev,
                        open.id,
                        None,
                        None,
                        Timestamp::now(),
                        false,
                    )?;
                    EffortTransition::Finished(open.id)
                }
                None => EffortTransition::NoOpenEffort,
            }
        }
        _ => EffortTransition::Untouched,
    })
}

/// The only writer of a task's `status` / `completed_at` (with
/// `updated_at`): everything else writes fields ([`update_task_tx`]), so a
/// copy read before a concurrent status change can never revert it.
fn write_status_tx(conn: &rusqlite::Connection, item: &Task) -> Result<(), DomainError> {
    let rows = conn
        .execute(
            "UPDATE task SET status = ?2, completed_at = ?3, updated_at = ?4
             WHERE id = ?1 AND deleted_at IS NULL",
            params![
                item.id.value(),
                status_to_str(item.status),
                item.completed_at.map(ts_to_string),
                ts_to_string(item.updated_at),
            ],
        )
        .map_err(crate::database::map_sql_err)?;
    if rows == 0 {
        return Err(DomainError::NotFound);
    }
    Ok(())
}

/// Which of a task's own fields differ between `before` and `after`, by
/// the names `work_item.edited` uses.
fn edited_fields(before: &Task, after: &Task) -> Vec<String> {
    let mut fields = Vec::new();
    if before.title != after.title {
        fields.push("title".to_string());
    }
    if before.description != after.description {
        fields.push("description".to_string());
    }
    if before.priority != after.priority {
        fields.push("priority".to_string());
    }
    if before.parent_id != after.parent_id {
        fields.push("parent".to_string());
    }
    if before.thread_id != after.thread_id {
        fields.push("thread".to_string());
    }
    fields
}

/// Log `work_item.edited@1` for `fields` of `item`, anchored to the thread
/// it is on now (none on the backlog).
fn log_edited_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    item: &Task,
    fields: Vec<String>,
) -> Result<(), DomainError> {
    let work_item = work_item_ref(item.id);
    let anchors = match item.thread_id {
        Some(thread) => anchors_for_thread_tx(conn, thread)?,
        None => Anchors::default(),
    };
    let env = ev
        .typed::<WorkItemEdited>(&WorkItemEditedV1 {
            work_item: work_item.clone(),
            fields,
        })
        .with_anchors(anchors)
        .with_subject([work_item]);
    ev.append(conn, &env)?;
    Ok(())
}

/// Write `item`'s fields (never its status) and, when `status` is given,
/// move the task there ([`set_status_tx`], which reads the committed
/// status inside this transaction). An edit of the task's own fields logs
/// `work_item.edited@1`. `NotFound` for a missing or deleted row. Returns
/// the row as committed.
pub fn update_with_status_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    item: &Task,
    status: Option<TaskStatus>,
    now: Timestamp,
) -> Result<(Task, EffortTransition), DomainError> {
    let before = get_task_tx(conn, item.id)?.ok_or(DomainError::NotFound)?;
    if update_task_tx(conn, item).map_err(crate::database::map_sql_err)? == 0 {
        return Err(DomainError::NotFound);
    }
    let fields = edited_fields(&before, item);
    if !fields.is_empty() {
        log_edited_tx(conn, ev, item, fields)?;
    }
    let effort = match status {
        Some(to) => set_status_tx(conn, ev, item.id, to, now)?.effort,
        None => EffortTransition::Untouched,
    };
    let after = get_task_tx(conn, item.id)?.ok_or(DomainError::NotFound)?;
    Ok((after, effort))
}

/// Insert `item` and log `work_item.created@1` with its initial status;
/// filed straight into `in_progress` on a thread, it opens the effort in
/// the same transaction. Returns the id and any effort opened.
pub fn insert_logged_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    item: &Task,
) -> Result<(TaskId, Option<EffortId>), DomainError> {
    let id = insert_task_tx(conn, item).map_err(crate::database::map_sql_err)?;
    let placed = Task { id, ..item.clone() };
    let effort = match effort_for_status_tx(conn, ev, &placed, TaskStatus::Ready)? {
        EffortTransition::Opened(e) => Some(e),
        _ => None,
    };
    let work_item = work_item_ref(id);
    let mut subject = vec![work_item.clone()];
    subject.extend(effort.map(effort_ref));
    let anchors = match placed.thread_id {
        Some(thread) => anchors_for_thread_tx(conn, thread)?,
        None => Anchors::default(),
    };
    let env = ev
        .typed::<WorkItemCreated>(&WorkItemCreatedV1 {
            work_item,
            status: placed.status,
            effort: effort.map(effort_ref),
        })
        .with_anchors(Anchors {
            effort_id: effort,
            ..anchors
        })
        .with_subject(subject);
    ev.append(conn, &env)?;
    Ok((id, effort))
}

/// The next `sort_index` at the end of `thread`'s list (or the backlog's),
/// read in the caller's transaction.
pub fn next_sort_index_tx(
    conn: &rusqlite::Connection,
    thread: Option<ThreadId>,
) -> Result<i64, DomainError> {
    conn.query_row(
        "SELECT COALESCE(MAX(sort_index), -1) + 1 FROM task
         WHERE thread_id IS ?1 AND deleted_at IS NULL",
        params![thread.map(|t| t.value())],
        |r| r.get(0),
    )
    .map_err(crate::database::map_sql_err)
}

/// Move task `id` to `to` at `now`, reading the row in the same
/// transaction — the core of `work_item.transition`.
pub fn set_status_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    id: TaskId,
    to: TaskStatus,
    now: Timestamp,
) -> Result<StatusChange, DomainError> {
    let before = get_task_tx(conn, id)?.ok_or(DomainError::NotFound)?;
    let mut after = before.clone();
    after.set_status(to, now);
    let effort = apply_status_tx(conn, ev, &after, before.status)?;
    Ok(StatusChange {
        before,
        after,
        effort,
    })
}

/// Sync core for the task-row INSERT; returns the new id.
pub(crate) fn insert_task_tx(conn: &rusqlite::Connection, item: &Task) -> rusqlite::Result<TaskId> {
    conn.execute(
        "INSERT INTO task (
            thread_id, parent_id, title, description,
            status, priority, sort_index, created_by, created_at, updated_at,
            completed_at, deleted_at, author
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            item.thread_id.as_ref().map(|t| t.value()),
            item.parent_id.map(|p| p.value()),
            item.title,
            item.description,
            status_to_str(item.status),
            priority_to_str(item.priority),
            item.sort_index,
            actor_to_str(item.created_by),
            ts_to_string(item.created_at),
            ts_to_string(item.updated_at),
            item.completed_at.map(ts_to_string),
            item.deleted_at.map(ts_to_string),
            item.author.map(author_to_str),
        ],
    )?;
    Ok(TaskId::new(conn.last_insert_rowid()))
}

/// One live task by id — `None` for a missing or soft-deleted row — for
/// composition inside a transaction (the status cores, the task commands
/// and the event pump's consumers read through it).
pub fn get_task_tx(conn: &rusqlite::Connection, id: TaskId) -> Result<Option<Task>, DomainError> {
    use rusqlite::OptionalExtension;
    let sql = format!("{} WHERE t.id = ?1 AND t.deleted_at IS NULL", SELECT_BASE);
    conn.query_row(&sql, params![id.value()], row_to_task)
        .optional()
        .map_err(crate::database::map_sql_err)
}

/// Sync core for the task-row UPDATE of everything but the status:
/// `status` / `completed_at` are written only by the status core
/// (`write_status_tx`, via [`set_status_tx`]), so a stale copy can't
/// revert a status change. Returns affected-row count; callers map 0 to
/// `NotFound`.
pub(crate) fn update_task_tx(conn: &rusqlite::Connection, item: &Task) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE task SET
            thread_id = ?2,
            parent_id = ?3,
            title = ?4,
            description = ?5,
            priority = ?6,
            sort_index = ?7,
            updated_at = ?8,
            deleted_at = ?9,
            author = ?10
         WHERE id = ?1 AND deleted_at IS NULL",
        params![
            item.id.value(),
            item.thread_id.as_ref().map(|t| t.value()),
            item.parent_id.map(|p| p.value()),
            item.title,
            item.description,
            priority_to_str(item.priority),
            item.sort_index,
            ts_to_string(item.updated_at),
            item.deleted_at.map(ts_to_string),
            item.author.map(author_to_str),
        ],
    )
}

fn status_to_str(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Ready => "ready",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Done => "done",
        TaskStatus::Canceled => "canceled",
        TaskStatus::Archived => "archived",
    }
}

fn str_to_status(s: &str) -> Result<TaskStatus, DomainError> {
    match s {
        "ready" => Ok(TaskStatus::Ready),
        "in_progress" => Ok(TaskStatus::InProgress),
        "blocked" => Ok(TaskStatus::Blocked),
        "done" => Ok(TaskStatus::Done),
        "canceled" => Ok(TaskStatus::Canceled),
        "archived" => Ok(TaskStatus::Archived),
        other => Err(DomainError::Invalid(format!(
            "unknown task status: {other}"
        ))),
    }
}

fn priority_to_str(p: TaskPriority) -> &'static str {
    match p {
        TaskPriority::Low => "low",
        TaskPriority::Medium => "medium",
        TaskPriority::High => "high",
        TaskPriority::Urgent => "urgent",
    }
}

fn str_to_priority(s: &str) -> Result<TaskPriority, DomainError> {
    match s {
        "low" => Ok(TaskPriority::Low),
        "medium" => Ok(TaskPriority::Medium),
        "high" => Ok(TaskPriority::High),
        "urgent" => Ok(TaskPriority::Urgent),
        other => Err(DomainError::Invalid(format!(
            "unknown task priority: {other}"
        ))),
    }
}

fn actor_to_str(a: TaskActorKind) -> &'static str {
    match a {
        TaskActorKind::User => "user",
        TaskActorKind::Agent => "agent",
        TaskActorKind::System => "system",
    }
}

fn str_to_actor(s: &str) -> Result<TaskActorKind, DomainError> {
    match s {
        "user" => Ok(TaskActorKind::User),
        "agent" => Ok(TaskActorKind::Agent),
        "system" => Ok(TaskActorKind::System),
        other => Err(DomainError::Invalid(format!("unknown actor kind: {other}"))),
    }
}

fn author_to_str(a: TaskAuthor) -> &'static str {
    match a {
        TaskAuthor::User => "user",
        TaskAuthor::Agent => "agent",
    }
}

fn str_to_author(s: &str) -> Result<TaskAuthor, DomainError> {
    match s {
        "user" => Ok(TaskAuthor::User),
        "agent" => Ok(TaskAuthor::Agent),
        other => Err(DomainError::Invalid(format!(
            "unknown task author: {other}"
        ))),
    }
}

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let id: i64 = row.get("id")?;
    let thread_id: Option<i64> = row.get("thread_id")?;
    let parent_id: Option<i64> = row.get("parent_id")?;
    let title: String = row.get("title")?;
    let description: String = row.get("description")?;
    let status: String = row.get("status")?;
    let priority: String = row.get("priority")?;
    let sort_index: i64 = row.get("sort_index")?;
    let created_by: String = row.get("created_by")?;
    let created_at: String = row.get("created_at")?;
    let updated_at: String = row.get("updated_at")?;
    let completed_at: Option<String> = row.get("completed_at")?;
    let deleted_at: Option<String> = row.get("deleted_at")?;
    let author: Option<String> = row.get("author")?;

    let note_count: i64 = row
        .get::<_, Option<i64>>("note_count")
        .ok()
        .flatten()
        .unwrap_or(0);

    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };

    Ok(Task {
        id: TaskId::new(id),
        thread_id: thread_id.map(ThreadId::new),
        parent_id: parent_id.map(TaskId::new),
        title,
        description,
        status: str_to_status(&status).map_err(map_err)?,
        priority: str_to_priority(&priority).map_err(map_err)?,
        sort_index,
        created_by: str_to_actor(&created_by).map_err(map_err)?,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
        updated_at: string_to_ts(&updated_at).map_err(map_err)?,
        completed_at: completed_at
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(map_err)?,
        deleted_at: deleted_at
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(map_err)?,
        note_count,
        author: author.and_then(|a| str_to_author(&a).ok()),
    })
}

const SELECT_BASE: &str =
    "SELECT t.*, COALESCE((SELECT COUNT(*) FROM task_note wn WHERE wn.task_id = t.id), 0) AS note_count
     FROM task t";

impl SqliteTaskStore {
    pub async fn list_all_for_backfill(&self) -> Result<Vec<Task>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!("{} ORDER BY t.created_at ASC", SELECT_BASE);
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map([], row_to_task)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    pub async fn list_recently_done(&self, limit: usize) -> Result<Vec<Task>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!(
                    "{} WHERE t.status = 'done' AND t.deleted_at IS NULL \
                       AND t.completed_at IS NOT NULL \
                     ORDER BY t.completed_at DESC LIMIT ?1",
                    SELECT_BASE
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![limit as i64], row_to_task)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }
}

#[async_trait]
impl TaskStore for SqliteTaskStore {
    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<Task>, DomainError> {
        let thread = *thread;
        self.db
            .call(move |conn| {
                let sql = format!(
                    "{} WHERE t.thread_id = ?1 AND t.deleted_at IS NULL \
                     ORDER BY t.sort_index ASC, t.created_at ASC",
                    SELECT_BASE
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![thread.value()], row_to_task)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn list_by_status_for_thread(
        &self,
        thread: &ThreadId,
        status: TaskStatus,
    ) -> Result<Vec<Task>, DomainError> {
        let thread = *thread;
        let status_str = status_to_str(status);
        self.db
            .call(move |conn| {
                let sql = format!(
                    "{} WHERE t.thread_id = ?1 AND t.status = ?2 AND t.deleted_at IS NULL \
                     ORDER BY t.sort_index ASC, t.created_at ASC",
                    SELECT_BASE
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![thread.value(), status_str], row_to_task)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn list_backlog(&self) -> Result<Vec<Task>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!(
                    "{} WHERE t.thread_id IS NULL AND t.deleted_at IS NULL \
                     ORDER BY t.sort_index ASC, t.created_at ASC",
                    SELECT_BASE
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map([], row_to_task)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn get(&self, id: TaskId) -> Result<Option<Task>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!("{} WHERE t.id = ?1", SELECT_BASE);
                let mut stmt = conn.prepare(&sql)?;
                let mut rows = stmt.query_map(params![id.value()], row_to_task)?;
                match rows.next() {
                    Some(r) => Ok(Some(r?)),
                    None => Ok(None),
                }
            })
            .await
    }

    async fn insert(&self, item: &Task) -> Result<TaskId, DomainError> {
        let owned = item.clone();
        let new_id = self
            .db
            .call(move |conn| insert_task_tx(conn, &owned))
            .await?;
        self.project_body_refs(item, new_id).await?;
        Ok(new_id)
    }

    /// Write the row's fields; an edit of the task's own fields logs
    /// `work_item.edited@1` in the same transaction (P3.10 — every edit
    /// reaches the log, so what reacts to edits, like the search index,
    /// sees them all). Status is never written here.
    async fn update(&self, item: &Task) -> Result<(), DomainError> {
        let item = item.clone();
        let edges_item = item.clone();
        let schemas = self.event_schemas.clone();
        self.db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "task_store");
                update_with_status_tx(tx, &ev, &item, None, Timestamp::now()).map(|_| ())
            })
            .await?;
        {
            let refs = &self.page_refs;
            let edges = task_edges(&edges_item);
            refs.replace_source_for_ref_types(
                KIND_WORK_ITEM,
                &work_item_id(edges_item.id),
                task_body_ref_types(),
                edges,
            )
            .await?;
        }
        Ok(())
    }

    /// Soft-delete the task and close its open effort in the same
    /// transaction: no claim outlives its task.
    async fn soft_delete(&self, id: TaskId) -> Result<(), DomainError> {
        let schemas = self.event_schemas.clone();
        self.db
            .transaction(move |tx| {
                let now = Timestamp::now();
                let thread: Option<Option<i64>> = tx
                    .query_row(
                        "UPDATE task SET deleted_at = ?2, updated_at = ?2
                         WHERE id = ?1 AND deleted_at IS NULL
                         RETURNING thread_id",
                        params![id.value(), ts_to_string(now)],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(crate::database::map_sql_err)?;
                if let Some(open) =
                    crate::effort_store::find_open_for_work_item_tx(tx, &work_item_ref(id))
                        .map_err(crate::database::map_sql_err)?
                {
                    let ev = EventCtx::system(&schemas, "task_service");
                    crate::effort_store::finish_tx(tx, &ev, open.id, None, None, now, false)?;
                }
                if let Some(thread) = thread {
                    // What indexes or links the task learns it's gone.
                    let ev = EventCtx::system(&schemas, "task_store");
                    let work_item = work_item_ref(id);
                    let anchors = match thread {
                        Some(t) => anchors_for_thread_tx(tx, ThreadId::new(t))?,
                        None => Anchors::default(),
                    };
                    let env = ev
                        .typed::<WorkItemDeleted>(&WorkItemDeletedV1 {
                            work_item: work_item.clone(),
                        })
                        .with_anchors(anchors)
                        .with_subject([work_item]);
                    ev.append(tx, &env)?;
                }
                Ok(())
            })
            .await?;
        {
            let refs = &self.page_refs;
            refs.replace_source_for_ref_types(
                KIND_WORK_ITEM,
                &work_item_id(id),
                task_body_ref_types(),
                vec![],
            )
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_store::SqliteStreamStore;
    use crate::thread_store::SqliteThreadStore;
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadStatus};

    fn ts() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    async fn fixture() -> (SqliteTaskStore, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let work = SqliteTaskStore::new(db);
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "oxplow".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/repo".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: ts(),
            updated_at: ts(),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::new(1),
            stream_id: s.id,
            title: "explore".into(),
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
            created_at: ts(),
            updated_at: ts(),
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        (work, t.id)
    }

    fn item(thread: Option<ThreadId>) -> Task {
        Task {
            id: TaskId::placeholder(),
            thread_id: thread,
            parent_id: None,
            title: "ship it".into(),
            description: String::new(),
            status: TaskStatus::Ready,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: ts(),
            updated_at: ts(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
        }
    }

    /// Moving an in_progress task takes its claim with it: the effort on
    /// the old thread closes and one opens on the new thread (another
    /// stream included); moved to the backlog, it just closes (tsk465).
    #[tokio::test]
    async fn moving_an_in_progress_task_moves_its_effort() {
        let (store, tid) = fixture().await;
        store
            .db
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                       VALUES (2, 'worktree', 'b', 'b', 'refs/heads/b', 'main', '/b',
                               '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z');
                     INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                       VALUES (2, 2, 'other', 'active',
                               '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z');",
                )
            })
            .await
            .unwrap();
        let mut filed = item(Some(tid));
        filed.status = TaskStatus::InProgress;
        let (id, first) = store.insert_logged(&filed).await.unwrap();
        let first = first.unwrap();

        let moved = store.move_task(id, Some(ThreadId::new(2))).await.unwrap();
        assert_eq!(moved.thread_id, Some(ThreadId::new(2)));
        let open_on = |thread: i64| {
            let db = store.db.clone();
            async move {
                db.call(move |c| {
                    c.query_row(
                        "SELECT count(*) FROM effort WHERE thread_id = ?1 AND ended_at IS NULL",
                        params![thread],
                        |r| r.get::<_, i64>(0),
                    )
                })
                .await
                .unwrap()
            }
        };
        assert_eq!(
            open_on(tid.value()).await,
            0,
            "the old thread's claim closed"
        );
        assert_eq!(open_on(2).await, 1, "the new thread holds the claim");
        let first_ended: Option<String> = store
            .db
            .call(move |c| {
                c.query_row(
                    "SELECT ended_at FROM effort WHERE id = ?1",
                    params![first.value()],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert!(first_ended.is_some());

        store.move_task(id, None).await.unwrap();
        assert_eq!(open_on(2).await, 0, "the backlog holds no claim");
    }

    /// A deleted task can't be edited (nothing is logged or re-projected),
    /// and deleting an in_progress task closes its effort — no claim
    /// outlives its task (review of P2.6b–P2.11, tsk464).
    #[tokio::test]
    async fn a_deleted_task_takes_no_edits_and_leaves_no_open_effort() {
        let (store, tid) = fixture().await;
        let mut filed = item(Some(tid));
        filed.status = TaskStatus::InProgress;
        let (id, effort) = store.insert_logged(&filed).await.unwrap();
        let effort = effort.unwrap();
        store.soft_delete(id).await.unwrap();

        let (ended, events): (Option<String>, Vec<String>) = store
            .db
            .call(move |c| {
                let ended = c.query_row(
                    "SELECT ended_at FROM effort WHERE id = ?1",
                    params![effort.value()],
                    |r| r.get(0),
                )?;
                let mut stmt = c.prepare("SELECT type FROM event_log ORDER BY seq")?;
                let types = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok((ended, types))
            })
            .await
            .unwrap();
        assert!(ended.is_some(), "the effort closed with its task");
        assert_eq!(
            events,
            vec![
                "effort.opened",
                "work_item.created",
                "effort.closed",
                "work_item.deleted"
            ]
        );

        let mut edit = filed.clone();
        edit.id = id;
        edit.title = "ghost".into();
        assert!(matches!(
            store.update_with_status(&edit, None).await,
            Err(DomainError::NotFound)
        ));
        let logged: i64 = store
            .db
            .call(|c| c.query_row("SELECT count(*) FROM event_log", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(logged, 4, "a refused edit logs nothing");
        let live = store
            .db
            .transaction(move |c| get_task_tx(c, id))
            .await
            .unwrap();
        assert!(live.is_none(), "get_task_tx sees live rows only");
    }

    /// A field write never touches status (review of P2.6): a copy read
    /// before a concurrent status change can't revert it.
    #[tokio::test]
    async fn a_stale_field_write_keeps_the_committed_status() {
        let (store, tid) = fixture().await;
        let id = store.insert(&item(Some(tid))).await.unwrap();
        let stale = store.get(id).await.unwrap().unwrap();
        let schemas = store.event_schemas.clone();
        store
            .db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "test");
                set_status_tx(tx, &ev, id, TaskStatus::Done, Timestamp::from_unix_ms(7))
            })
            .await
            .unwrap();
        let mut edit = stale;
        edit.title = "renamed".into();
        store.update(&edit).await.unwrap();
        let now = store.get(id).await.unwrap().unwrap();
        assert_eq!(now.title, "renamed");
        assert_eq!(now.status, TaskStatus::Done);
        assert_eq!(now.completed_at, Some(Timestamp::from_unix_ms(7)));
    }

    /// Field edits and a status change commit together; the edit logs
    /// `work_item.edited@1` naming what changed.
    #[tokio::test]
    async fn an_edit_with_a_status_change_logs_both() {
        let (store, tid) = fixture().await;
        let id = store.insert(&item(Some(tid))).await.unwrap();
        let mut row = store.get(id).await.unwrap().unwrap();
        row.title = "renamed".into();
        row.priority = TaskPriority::High;
        let after = store
            .update_with_status(&row, Some(TaskStatus::Blocked))
            .await
            .unwrap();
        assert_eq!(after.title, "renamed");
        assert_eq!(after.status, TaskStatus::Blocked);
        let events = store
            .db
            .call_mut(|c| crate::event_log_store::read_after_tx(c, 0, 10))
            .await
            .unwrap();
        let seen: Vec<&str> = events
            .iter()
            .map(|e| e.envelope.event_type.as_str())
            .collect();
        assert_eq!(seen, vec!["work_item.edited", "work_item.transitioned"]);
        assert_eq!(
            events[0].envelope.payload["fields"],
            serde_json::json!(["title", "priority"])
        );
        // Nothing changed: nothing logged.
        store.update_with_status(&after, None).await.unwrap();
        let again = store
            .db
            .call_mut(|c| crate::event_log_store::read_after_tx(c, 0, 10))
            .await
            .unwrap();
        assert_eq!(again.len(), 2);
    }

    /// P2.6.3 (tsk455): every status change logs exactly one
    /// `work_item.transitioned` — not only in_progress crossings, and for
    /// thread-less tasks too. Filing a task logs `work_item.created` with
    /// its status instead (tsk463).
    #[tokio::test]
    async fn every_status_change_logs_one_transition() {
        let (store, tid) = fixture().await;
        let loose = store.insert(&item(None)).await.unwrap();
        store.set_status(loose, TaskStatus::Blocked).await.unwrap();
        // Same status again: nothing to log.
        store.set_status(loose, TaskStatus::Blocked).await.unwrap();

        let attached = store.insert(&item(Some(tid))).await.unwrap();
        store.set_status(attached, TaskStatus::Done).await.unwrap();

        let mut filed = item(Some(tid));
        filed.status = TaskStatus::Blocked;
        let (born, effort) = store.insert_logged(&filed).await.unwrap();
        assert_eq!(effort, None, "only in_progress opens an effort");

        let events = store
            .db
            .call_mut(|c| crate::event_log_store::read_after_tx(c, 0, 20))
            .await
            .unwrap();
        let created: Vec<_> = events
            .iter()
            .filter(|e| e.envelope.event_type == "work_item.created")
            .collect();
        assert_eq!(created.len(), 1, "filing is a creation, not a transition");
        assert_eq!(
            created[0].envelope.payload["work_item"],
            work_item_ref(born)
        );
        assert_eq!(created[0].envelope.payload["status"], "blocked");
        let seen: Vec<(String, String, String, bool)> = events
            .iter()
            .filter(|e| e.envelope.event_type == "work_item.transitioned")
            .map(|e| {
                let p = &e.envelope.payload;
                (
                    p["work_item"].as_str().unwrap().to_string(),
                    p["from"].as_str().unwrap().to_string(),
                    p["to"].as_str().unwrap().to_string(),
                    e.envelope.anchors.stream_id.is_some(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            vec![
                (
                    work_item_ref(loose),
                    "ready".into(),
                    "blocked".into(),
                    false
                ),
                (work_item_ref(attached), "ready".into(), "done".into(), true),
            ]
        );
    }

    /// A status change computed from the stored row: `completed_at` follows
    /// `done`, and the change reports what it did.
    #[tokio::test]
    async fn set_status_tx_derives_the_row_and_reports_the_change() {
        let (store, tid) = fixture().await;
        let id = store.insert(&item(Some(tid))).await.unwrap();
        let schemas = store.event_schemas.clone();
        let change = store
            .db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "test");
                set_status_tx(
                    tx,
                    &ev,
                    id,
                    TaskStatus::InProgress,
                    Timestamp::from_unix_ms(5),
                )
            })
            .await
            .unwrap();
        assert_eq!(change.before.status, TaskStatus::Ready);
        assert_eq!(change.after.status, TaskStatus::InProgress);
        assert!(matches!(change.effort, EffortTransition::Opened(_)));
        let schemas = store.event_schemas.clone();
        let done = store
            .db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "test");
                set_status_tx(tx, &ev, id, TaskStatus::Done, Timestamp::from_unix_ms(9))
            })
            .await
            .unwrap();
        assert_eq!(done.after.completed_at, Some(Timestamp::from_unix_ms(9)));
        assert!(matches!(done.effort, EffortTransition::Finished(_)));
        let schemas = store.event_schemas.clone();
        let missing = store
            .db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "test");
                set_status_tx(
                    tx,
                    &ev,
                    TaskId::new(999),
                    TaskStatus::Done,
                    Timestamp::now(),
                )
            })
            .await;
        assert!(matches!(missing, Err(DomainError::NotFound)));
    }

    /// P2.5b (tsk428): filing a task straight into `in_progress` opens its
    /// effort in the insert's own transaction — both or neither.
    #[tokio::test]
    async fn insert_in_progress_opens_its_effort_in_the_same_transaction() {
        let (store, tid) = fixture().await;
        let mut it = item(Some(tid));
        it.status = TaskStatus::InProgress;
        let (id, effort) = store.insert_logged(&it).await.unwrap();
        let effort = effort.expect("in_progress on a thread opens an effort");
        let (work_item, ended): (String, Option<String>) = store
            .db
            .call(move |c| {
                c.query_row(
                    "SELECT work_item, ended_at FROM effort WHERE id = ?1",
                    params![effort.value()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!(work_item, work_item_ref(id));
        assert_eq!(ended, None);

        // The effort can't open, so the task row isn't written either.
        store
            .db
            .call(|c| {
                c.execute_batch(
                    "CREATE TRIGGER no_efforts BEFORE INSERT ON effort
                     BEGIN SELECT RAISE(ABORT, 'no efforts'); END;",
                )
            })
            .await
            .unwrap();
        assert!(store.insert_logged(&it).await.is_err());
        let rows: i64 = store
            .db
            .call(|c| c.query_row("SELECT count(*) FROM task", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(rows, 1, "the failed insert rolled back with its effort");
        let logged: Vec<String> = store
            .db
            .call_mut(|c| crate::event_log_store::read_after_tx(c, 0, 10))
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.envelope.event_type)
            .collect();
        assert_eq!(
            logged,
            vec!["effort.opened", "work_item.created"],
            "only the first insert's"
        );
    }

    #[tokio::test]
    async fn insert_then_get() {
        let (store, tid) = fixture().await;
        let it = item(Some(tid));
        let id = store.insert(&it).await.unwrap();
        let got = store.get(id).await.unwrap().unwrap();
        assert_eq!(got.id, id);
        assert_eq!(got.title, it.title);
    }

    #[tokio::test]
    async fn description_round_trips() {
        let (store, tid) = fixture().await;
        let mut it = item(Some(tid));
        it.description = "the detailed developer text".into();
        let id = store.insert(&it).await.unwrap();
        let got = store.get(id).await.unwrap().unwrap();
        assert_eq!(got.description, "the detailed developer text");

        let mut latest = got;
        latest.description = "edited".into();
        store.update(&latest).await.unwrap();
        let after = store.get(id).await.unwrap().unwrap();
        assert_eq!(after.description, "edited");
    }

    #[tokio::test]
    async fn list_for_thread_excludes_deleted() {
        let (store, tid) = fixture().await;
        let alive_id = store.insert(&item(Some(tid))).await.unwrap();
        let dead_id = store.insert(&item(Some(tid))).await.unwrap();
        store.soft_delete(dead_id).await.unwrap();
        let list = store.list_for_thread(&tid).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, alive_id);
    }

    #[tokio::test]
    async fn backlog_items_have_no_thread() {
        let (store, tid) = fixture().await;
        store.insert(&item(Some(tid))).await.unwrap();
        let backlog_id = store.insert(&item(None)).await.unwrap();

        let bl = store.list_backlog().await.unwrap();
        assert_eq!(bl.len(), 1);
        assert_eq!(bl[0].id, backlog_id);
    }

    #[tokio::test]
    async fn list_orders_by_sort_index() {
        let (store, tid) = fixture().await;
        let mut a = item(Some(tid));
        a.sort_index = 5;
        let mut b = item(Some(tid));
        b.sort_index = 1;
        let a_id = store.insert(&a).await.unwrap();
        let b_id = store.insert(&b).await.unwrap();
        let list = store.list_for_thread(&tid).await.unwrap();
        assert_eq!(list[0].id, b_id);
        assert_eq!(list[1].id, a_id);
    }

    #[tokio::test]
    async fn update_overwrites_existing() {
        let (store, tid) = fixture().await;
        let it = item(Some(tid));
        let id = store.insert(&it).await.unwrap();
        let mut latest = store.get(id).await.unwrap().unwrap();
        latest.title = "renamed".into();
        latest.status = TaskStatus::InProgress;
        store.update(&latest).await.unwrap();
        let got = store.get(id).await.unwrap().unwrap();
        assert_eq!(got.title, "renamed");
        // A plain update writes fields only; status moves through the
        // status core (`set_status_tx`).
        assert_eq!(got.status, TaskStatus::Ready);
    }

    /// Updating a row that was never inserted (or one whose id never
    /// matched anything) yields NotFound rather than silently doing
    /// nothing — callers can distinguish "wrote 0 rows" from "wrote
    /// 1 row" without an extra read.
    #[tokio::test]
    async fn update_missing_id_returns_not_found() {
        let (store, tid) = fixture().await;
        let mut it = item(Some(tid));
        it.id = TaskId::new(999_999);
        let err = store.update(&it).await.unwrap_err();
        assert!(
            matches!(err, DomainError::NotFound),
            "expected NotFound for missing id, got {err:?}"
        );
    }

    /// Soft-deleted rows are intentionally invisible to `update` —
    /// the WHERE clause filters on `deleted_at IS NULL`. This stops
    /// a malformed Task payload (with `deleted_at: None`) from
    /// silently un-soft-deleting the row.
    #[tokio::test]
    async fn update_on_soft_deleted_returns_not_found() {
        let (store, tid) = fixture().await;
        let id = store.insert(&item(Some(tid))).await.unwrap();
        store.soft_delete(id).await.unwrap();
        let mut latest = item(Some(ThreadId::new(1)));
        latest.id = id;
        latest.title = "ressurected".into();
        let err = store.update(&latest).await.unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
    }

    #[tokio::test]
    async fn insert_with_page_refs_projects_body_mentions() {
        use crate::page_ref_store::SqlitePageRefStore;
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "oxplow".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/repo".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: ts(),
            updated_at: ts(),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
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
            created_at: ts(),
            updated_at: ts(),
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();

        let page_refs = SqlitePageRefStore::new(db.clone());
        let store = SqliteTaskStore::new(db.clone());

        let mut it = item(Some(t.id));
        it.description = "see [[src/app.rs]] and blocks tsk99".into();
        let new_id = store.insert(&it).await.unwrap();

        let inbound = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(inbound
            .iter()
            .any(|e| e.source_id == format!("oxplow:{new_id}")));

        let mut latest = store.get(new_id).await.unwrap().unwrap();
        latest.description = "no refs anymore".into();
        store.update(&latest).await.unwrap();
        let inbound = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(inbound.is_empty(), "expected no backlinks; got {inbound:?}");
    }
}
