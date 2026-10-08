use oxplow_domain::vocabulary::VocabularyHandle;
use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::params;

use oxplow_domain::stores::TaskStore;
use oxplow_domain::{
    DomainError, Task, TaskActorKind, TaskAuthor, TaskId, TaskPriority, TaskStatus, ThreadId,
    Timestamp,
};

use crate::database::Database;
use crate::database::{string_to_ts, ts_to_string};
use crate::page_ref_projections::{task_body_ref_types, task_edges, work_item_id, KIND_WORK_ITEM};
use crate::page_ref_store::SqlitePageRefStore;

#[derive(Clone)]
pub struct SqliteTaskStore {
    db: Database,
    page_refs: SqlitePageRefStore,
    /// The ref kinds a task body's mentions may name.
    vocabulary: VocabularyHandle,
}

/// A status change made by [`set_status_tx`]: the row before and after.
/// A task's status is a record; it never opens or closes an effort (the
/// effort policy reacts to it, `.context/work-tracking.md`).
#[derive(Debug, Clone, PartialEq)]
pub struct StatusChange {
    pub before: Task,
    pub after: Task,
}

impl SqliteTaskStore {
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

    /// Re-project a task's body mentions into `page_ref` (the task-body
    /// slice; the effort slice is the effort store's).
    async fn project_body_refs(&self, item: &Task, id: TaskId) -> Result<(), DomainError> {
        let mut placed = item.clone();
        placed.id = id;
        let vocabulary = self.vocabulary.current();
        self.page_refs
            .replace_source_for_ref_types(
                KIND_WORK_ITEM,
                &work_item_id(id),
                task_body_ref_types(),
                task_edges(&vocabulary.kinds, &placed),
            )
            .await
    }

    /// Move a task to another thread (or the backlog, `None`) at the end
    /// of its list. Returns the moved row.
    pub async fn move_task(&self, id: TaskId, dest: Option<ThreadId>) -> Result<Task, DomainError> {
        let moved = self
            .db
            .transaction(move |tx| {
                let item = get_task_tx(tx, id)?.ok_or(DomainError::NotFound)?;
                if item.thread_id == dest {
                    return Ok(item);
                }
                Ok(place_task_tx(tx, id, dest, Placement::End, Timestamp::now())?.task)
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
        self.db
            .transaction(move |tx| set_status_tx(tx, id, to, Timestamp::now()))
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
        let after = self
            .db
            .transaction(move |tx| update_with_status_tx(tx, &owned, status, Timestamp::now()))
            .await?;
        self.project_body_refs(&after, after.id).await?;
        Ok(after)
    }
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
    project_work_item_tx(conn, item.id).map_err(crate::database::map_sql_err)?;
    Ok(())
}

/// Write `item`'s fields (never its status) and, when `status` is given,
/// move the task there ([`set_status_tx`], which reads the committed
/// status inside this transaction). `NotFound` for a missing or deleted
/// row. Returns the row as committed.
pub fn update_with_status_tx(
    conn: &rusqlite::Connection,
    item: &Task,
    status: Option<TaskStatus>,
    now: Timestamp,
) -> Result<Task, DomainError> {
    if update_task_tx(conn, item).map_err(crate::database::map_sql_err)? == 0 {
        return Err(DomainError::NotFound);
    }
    if let Some(to) = status {
        set_status_tx(conn, item.id, to, now)?;
    }
    get_task_tx(conn, item.id)?.ok_or(DomainError::NotFound)
}

/// Insert `item` (its `id` ignored) in the caller's transaction. Returns
/// its id.
pub fn insert_tx(conn: &rusqlite::Connection, item: &Task) -> Result<TaskId, DomainError> {
    insert_task_tx(conn, item).map_err(crate::database::map_sql_err)
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

/// Soft-delete task `id` at `now` in the caller's transaction — the core
/// of `oxplow.work_item.delete`: the row's `deleted_at`, its `work_item` row
/// and its body's `page_ref` edges dropped. `NotFound` when it's missing
/// or already deleted.
pub fn soft_delete_tx(
    conn: &rusqlite::Connection,
    id: TaskId,
    now: Timestamp,
) -> Result<(), DomainError> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "UPDATE task SET deleted_at = ?2, updated_at = ?2
             WHERE id = ?1 AND deleted_at IS NULL
             RETURNING thread_id",
        params![id.value(), ts_to_string(now)],
        |r| r.get::<_, Option<i64>>(0),
    )
    .optional()
    .map_err(crate::database::map_sql_err)?
    .ok_or(DomainError::NotFound)?;
    project_work_item_tx(conn, id).map_err(crate::database::map_sql_err)?;
    crate::page_ref_store::replace_source_for_ref_types_tx(
        conn,
        KIND_WORK_ITEM,
        &work_item_id(id),
        &task_body_ref_types(),
        vec![],
    )?;
    Ok(())
}

/// Where in a list a task goes: its end, or next to another task there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    End,
    Before(TaskId),
    After(TaskId),
}

/// What [`place_task_tx`] did: the task as it now stands, and where it
/// was (its list, and a neighbour there) — what an undo puts back.
#[derive(Debug, Clone)]
pub struct Placed {
    pub task: Task,
    pub from_thread: Option<ThreadId>,
    pub from_place: Placement,
}

/// A list's live task ids in order: a thread's, or the backlog's (`None`).
fn list_ids_tx(
    conn: &rusqlite::Connection,
    thread: Option<ThreadId>,
) -> Result<Vec<TaskId>, DomainError> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM task WHERE thread_id IS ?1 AND deleted_at IS NULL
             ORDER BY sort_index ASC, created_at ASC",
        )
        .map_err(crate::database::map_sql_err)?;
    let ids = stmt
        .query_map(params![thread.map(|t| t.value())], |r| r.get::<_, i64>(0))
        .map_err(crate::database::map_sql_err)?
        .collect::<rusqlite::Result<Vec<i64>>>()
        .map_err(crate::database::map_sql_err)?;
    Ok(ids.into_iter().map(TaskId::new).collect())
}

/// Put task `id` in `dest`'s list (`None` = the backlog) at `place`,
/// renumbering that list's `sort_index` — the core of `oxplow.work_item.reorder`
/// (the same list) and `oxplow.work_item.move` (another). `place` must
/// name a task in that list.
pub fn place_task_tx(
    conn: &rusqlite::Connection,
    id: TaskId,
    dest: Option<ThreadId>,
    place: Placement,
    now: Timestamp,
) -> Result<Placed, DomainError> {
    let mut item = get_task_tx(conn, id)?.ok_or(DomainError::NotFound)?;
    let from_thread = item.thread_id;
    let from_list = list_ids_tx(conn, from_thread)?;
    let at = from_list.iter().position(|t| *t == id);
    let from_place = match at {
        Some(i) if i > 0 => Placement::After(from_list[i - 1]),
        Some(i) if i + 1 < from_list.len() => Placement::Before(from_list[i + 1]),
        _ => Placement::End,
    };
    let mut list: Vec<TaskId> = list_ids_tx(conn, dest)?
        .into_iter()
        .filter(|t| *t != id)
        .collect();
    let index = match place {
        Placement::End => list.len(),
        Placement::Before(other) | Placement::After(other) => {
            let i = list.iter().position(|t| *t == other).ok_or_else(|| {
                DomainError::Invalid(format!("{other} isn't in the list {id} is going to"))
            })?;
            if matches!(place, Placement::After(_)) {
                i + 1
            } else {
                i
            }
        }
    };
    list.insert(index, id);
    for (i, t) in list.iter().enumerate() {
        if *t != id {
            let renumbered = conn
                .execute(
                    "UPDATE task SET sort_index = ?2 WHERE id = ?1 AND sort_index != ?2",
                    params![t.value(), i as i64],
                )
                .map_err(crate::database::map_sql_err)?;
            // Its work_item row carries `sort_index` in `native`: restate
            // it with the task row, so `v_work_item` never disagrees.
            if renumbered > 0 {
                project_work_item_tx(conn, *t).map_err(crate::database::map_sql_err)?;
            }
        }
    }
    item.thread_id = dest;
    item.sort_index = index as i64;
    item.updated_at = now;
    if update_task_tx(conn, &item).map_err(crate::database::map_sql_err)? == 0 {
        return Err(DomainError::NotFound);
    }
    Ok(Placed {
        task: item,
        from_thread,
        from_place,
    })
}

/// Move task `id` to `to` at `now`, reading the row in the same
/// transaction — the core of `oxplow.work_item.transition`.
pub fn set_status_tx(
    conn: &rusqlite::Connection,
    id: TaskId,
    to: TaskStatus,
    now: Timestamp,
) -> Result<StatusChange, DomainError> {
    let before = get_task_tx(conn, id)?.ok_or(DomainError::NotFound)?;
    let mut after = before.clone();
    after.set_status(to, now);
    write_status_tx(conn, &after)?;
    Ok(StatusChange { before, after })
}

/// Restate task `id`'s `work_item` row (P5.C1, the oxplow provider's):
/// every task write calls it in its own transaction, so the two never
/// disagree. Canonical state: `ready` → `todo`; `archived` → `done` when
/// it was completed, else `canceled`; the rest map by name. The native
/// status stays in `native_state`, the oxplow-only fields in `native`; the
/// interface's own columns (`thread_id`, `rank`, `closed_at`) are the
/// task's list, sort index and close. A hard delete (a cascade from the
/// thread or stream) is the `task` table's trigger's (V115); links and
/// comments follow `task_link` / `task_note` by trigger (V17).
pub(crate) fn project_work_item_tx(
    conn: &rusqlite::Connection,
    id: TaskId,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO work_item (ref, provider, title, body, state, native_state, native,
                                parent_ref, thread_id, rank, closed_at,
                                created_at, updated_at, deleted_at)
         SELECT 'work_item:oxplow:tsk' || t.id, 'oxplow', t.title, t.description,
                CASE t.status
                    WHEN 'ready' THEN 'todo'
                    WHEN 'archived' THEN
                        CASE WHEN t.completed_at IS NULL THEN 'canceled' ELSE 'done' END
                    ELSE t.status
                END,
                t.status,
                json_object('priority', t.priority, 'thread_id', t.thread_id,
                            'sort_index', t.sort_index, 'author', t.author,
                            'completed_at', t.completed_at),
                CASE WHEN t.parent_id IS NULL THEN NULL
                     ELSE 'work_item:oxplow:tsk' || t.parent_id END,
                t.thread_id, t.sort_index,
                CASE WHEN t.status = 'done' OR t.status = 'canceled' OR t.status = 'archived'
                     THEN coalesce(t.completed_at, t.updated_at) END,
                t.created_at, t.updated_at, t.deleted_at
         FROM task t WHERE t.id = ?1
         ON CONFLICT(ref) DO UPDATE SET
            title = excluded.title, body = excluded.body, state = excluded.state,
            native_state = excluded.native_state, native = excluded.native,
            parent_ref = excluded.parent_ref, thread_id = excluded.thread_id,
            rank = excluded.rank,
            -- When it first closed; reopened, it's open again.
            closed_at = CASE WHEN excluded.closed_at IS NULL THEN NULL
                             ELSE coalesce(work_item.closed_at, excluded.closed_at) END,
            updated_at = excluded.updated_at, deleted_at = excluded.deleted_at",
        params![id.value()],
    )?;
    Ok(())
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
    let id = TaskId::new(conn.last_insert_rowid());
    project_work_item_tx(conn, id)?;
    Ok(id)
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
    let rows = conn.execute(
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
    )?;
    project_work_item_tx(conn, item.id)?;
    Ok(rows)
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

    /// Write the row's fields; status is never written here.
    async fn update(&self, item: &Task) -> Result<(), DomainError> {
        let item = item.clone();
        let edges_item = item.clone();
        self.db
            .transaction(move |tx| {
                update_with_status_tx(tx, &item, None, Timestamp::now()).map(|_| ())
            })
            .await?;
        {
            let refs = &self.page_refs;
            let vocabulary = self.vocabulary.current();
            let edges = task_edges(&vocabulary.kinds, &edges_item);
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

    /// Soft-delete the task (see [`soft_delete_tx`]).
    async fn soft_delete(&self, id: TaskId) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| {
                match soft_delete_tx(tx, id, Timestamp::now()) {
                    // Deleting a deleted (or missing) task is a no-op here.
                    Err(DomainError::NotFound) => Ok(()),
                    other => other,
                }
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_store::SqliteStreamStore;
    use crate::thread_store::SqliteThreadStore;
    use oxplow_domain::refs::build::work_item_ref;
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

    /// tsk572: a soft-deleted parent isn't a live work item, so its
    /// children in `v_work_item` have no parent — the relationship holds.
    #[tokio::test]
    async fn a_deleted_parent_leaves_its_children_without_one() {
        let (store, tid) = fixture().await;
        let epic = store.insert(&item(Some(tid))).await.unwrap();
        let mut child = item(None);
        child.parent_id = Some(epic);
        let child = store.insert(&child).await.unwrap();
        store.soft_delete(epic).await.unwrap();
        let child_ref = work_item_ref(child);
        let (parent, dangling): (Option<String>, i64) = store
            .db
            .call(move |c| {
                c.query_row(
                    "SELECT (SELECT parent_ref FROM v_work_item WHERE ref = ?1),
                            (SELECT count(*) FROM v_work_item w WHERE w.parent_ref IS NOT NULL
                               AND NOT EXISTS (SELECT 1 FROM v_work_item p
                                               WHERE p.ref = w.parent_ref))",
                    [child_ref],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!(parent, None);
        assert_eq!(dangling, 0);
    }

    /// The oxplow provider's `work_item` rows (P5.C1) are written with the
    /// task rows: every live task has one, and a deleted one's is marked
    /// deleted, or gone with a cascade.
    #[tokio::test]
    async fn every_task_has_its_work_item_row() {
        let (store, tid) = fixture().await;
        let epic = store.insert(&item(Some(tid))).await.unwrap();
        let mut child = item(None);
        child.title = "child".into();
        child.parent_id = Some(epic);
        let child = store.insert(&child).await.unwrap();
        store
            .set_status(epic, TaskStatus::InProgress)
            .await
            .unwrap();
        let mut edited = store.get(child).await.unwrap().unwrap();
        edited.title = "renamed".into();
        edited.description = "the body".into();
        store.update(&edited).await.unwrap();
        let gone = store.insert(&item(Some(tid))).await.unwrap();
        store.soft_delete(gone).await.unwrap();
        let shipped = store.insert(&item(Some(tid))).await.unwrap();
        store.set_status(shipped, TaskStatus::Done).await.unwrap();
        store
            .set_status(shipped, TaskStatus::Archived)
            .await
            .unwrap();
        let dropped = store.insert(&item(Some(tid))).await.unwrap();
        store
            .set_status(dropped, TaskStatus::Archived)
            .await
            .unwrap();

        let rows = store
            .db
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT ref, provider, title, body, state, native_state, parent_ref,
                            deleted_at IS NOT NULL
                     FROM work_item ORDER BY ref",
                )?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok(format!(
                            "{}|{}|{}|{}|{}|{}|{}|{}",
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                            r.get::<_, String>(5)?,
                            r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                            r.get::<_, bool>(7)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        let r = |id: TaskId| work_item_ref(id);
        let mut expected = vec![
            format!("{}|oxplow|ship it||in_progress|in_progress||false", r(epic)),
            format!(
                "{}|oxplow|renamed|the body|todo|ready|{}|false",
                r(child),
                r(epic)
            ),
            format!("{}|oxplow|ship it||todo|ready||true", r(gone)),
            format!("{}|oxplow|ship it||done|archived||false", r(shipped)),
            format!("{}|oxplow|ship it||canceled|archived||false", r(dropped)),
        ];
        expected.sort();
        assert_eq!(rows, expected);

        // A cascade (the stream or thread deleted outright) takes the
        // work item with the task.
        store
            .db
            .call(|c| c.execute("DELETE FROM threads", []))
            .await
            .unwrap();
        let left: (i64, i64) = store
            .db
            .call(|c| {
                c.query_row(
                    "SELECT (SELECT count(*) FROM task), (SELECT count(*) FROM work_item)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!(left, (0, 0), "the backlog child went with its parent");
    }

    /// A deleted task can't be edited. The store logs nothing of its own:
    /// a list's events are core's, logged by the `oxplow.work_item.*`
    /// commands for every list alike.
    #[tokio::test]
    async fn a_deleted_task_takes_no_edits_and_the_store_logs_nothing() {
        let (store, tid) = fixture().await;
        let mut filed = item(Some(tid));
        filed.status = TaskStatus::InProgress;
        let id = store.insert(&filed).await.unwrap();
        store.set_status(id, TaskStatus::Blocked).await.unwrap();
        store.soft_delete(id).await.unwrap();

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
        assert_eq!(logged, 0);
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
        store
            .db
            .transaction(move |tx| {
                set_status_tx(tx, id, TaskStatus::Done, Timestamp::from_unix_ms(7))
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

    /// Field edits and a status change commit together.
    #[tokio::test]
    async fn an_edit_with_a_status_change_commits_both() {
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
        assert_eq!(after.priority, TaskPriority::High);
        assert_eq!(after.status, TaskStatus::Blocked);
        assert_eq!(store.get(id).await.unwrap().unwrap(), after);
    }

    /// A status change computed from the stored row: `completed_at` follows
    /// `done`, and the change reports what it did.
    #[tokio::test]
    async fn set_status_tx_derives_the_row_and_reports_the_change() {
        let (store, tid) = fixture().await;
        let id = store.insert(&item(Some(tid))).await.unwrap();
        let change = store
            .db
            .transaction(move |tx| {
                set_status_tx(tx, id, TaskStatus::InProgress, Timestamp::from_unix_ms(5))
            })
            .await
            .unwrap();
        assert_eq!(change.before.status, TaskStatus::Ready);
        assert_eq!(change.after.status, TaskStatus::InProgress);
        let done = store
            .db
            .transaction(move |tx| {
                set_status_tx(tx, id, TaskStatus::Done, Timestamp::from_unix_ms(9))
            })
            .await
            .unwrap();
        assert_eq!(done.after.completed_at, Some(Timestamp::from_unix_ms(9)));
        let missing = store
            .db
            .transaction(move |tx| {
                set_status_tx(tx, TaskId::new(999), TaskStatus::Done, Timestamp::now())
            })
            .await;
        assert!(matches!(missing, Err(DomainError::NotFound)));
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
