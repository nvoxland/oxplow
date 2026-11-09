use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::params;

use crate::ids::TaskId;
use oxplow_db::{map_sql_err, string_to_ts, ts_to_string, Database};
use oxplow_domain::{DomainError, Timestamp};

use crate::model::{Task, TaskActorKind, TaskAuthor, TaskPriority, TaskStatus};

/// oxplow's task rows. It writes nothing of core's: what a write did
/// reaches the work-item interface through the `work_item.recorded` its
/// verb answers with ([`crate::verbs`]).
#[derive(Clone)]
pub struct SqliteTaskStore {
    db: Database,
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
    pub fn new(db: Database) -> Self {
        Self { db }
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
        .map_err(map_sql_err)?;
    if rows == 0 {
        return Err(DomainError::NotFound);
    }
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
    if update_task_tx(conn, item).map_err(map_sql_err)? == 0 {
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
    insert_task_tx(conn, item).map_err(map_sql_err)
}

/// Soft-delete task `id` at `now` in the caller's transaction — the core
/// of `oxplow.work_item.delete`: the row's `deleted_at`. `NotFound` when
/// it's missing or already deleted.
pub fn soft_delete_tx(
    conn: &rusqlite::Connection,
    id: TaskId,
    now: Timestamp,
) -> Result<(), DomainError> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "UPDATE task SET deleted_at = ?2, updated_at = ?2
             WHERE id = ?1 AND deleted_at IS NULL
             RETURNING id",
        params![id.value(), ts_to_string(now)],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .map_err(map_sql_err)?
    .ok_or(DomainError::NotFound)?;
    Ok(())
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

/// Sync core for the task-row INSERT; returns the new id.
pub(crate) fn insert_task_tx(conn: &rusqlite::Connection, item: &Task) -> rusqlite::Result<TaskId> {
    conn.execute(
        "INSERT INTO task (
            parent_id, title, description,
            status, priority, created_by, created_at, updated_at,
            completed_at, deleted_at, author
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            item.parent_id.map(|p| p.value()),
            item.title,
            item.description,
            status_to_str(item.status),
            priority_to_str(item.priority),
            actor_to_str(item.created_by),
            ts_to_string(item.created_at),
            ts_to_string(item.updated_at),
            item.completed_at.map(ts_to_string),
            item.deleted_at.map(ts_to_string),
            item.author.map(author_to_str),
        ],
    )?;
    let id = TaskId::new(conn.last_insert_rowid());
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
        .map_err(map_sql_err)
}

/// Sync core for the task-row UPDATE of everything but the status:
/// `status` / `completed_at` are written only by the status core
/// (`write_status_tx`, via [`set_status_tx`]), so a stale copy can't
/// revert a status change. Returns affected-row count; callers map 0 to
/// `NotFound`.
pub(crate) fn update_task_tx(conn: &rusqlite::Connection, item: &Task) -> rusqlite::Result<usize> {
    let rows = conn.execute(
        "UPDATE task SET
            parent_id = ?2,
            title = ?3,
            description = ?4,
            priority = ?5,
            updated_at = ?6,
            deleted_at = ?7,
            author = ?8
         WHERE id = ?1 AND deleted_at IS NULL",
        params![
            item.id.value(),
            item.parent_id.map(|p| p.value()),
            item.title,
            item.description,
            priority_to_str(item.priority),
            ts_to_string(item.updated_at),
            item.deleted_at.map(ts_to_string),
            item.author.map(author_to_str),
        ],
    )?;
    Ok(rows)
}

fn status_to_str(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Backlog => "backlog",
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
        "backlog" => Ok(TaskStatus::Backlog),
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
    let parent_id: Option<i64> = row.get("parent_id")?;
    let title: String = row.get("title")?;
    let description: String = row.get("description")?;
    let status: String = row.get("status")?;
    let priority: String = row.get("priority")?;
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
        parent_id: parent_id.map(TaskId::new),
        title,
        description,
        status: str_to_status(&status).map_err(map_err)?,
        priority: str_to_priority(&priority).map_err(map_err)?,
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

/// The task rows, read and written.
#[async_trait]
pub trait TaskStore: Send + Sync {
    async fn get(&self, id: TaskId) -> Result<Option<Task>, DomainError>;
    /// Insert a new task; assigns and returns the autoincrement id.
    async fn insert(&self, item: &Task) -> Result<TaskId, DomainError>;
    /// Update an existing task by id.
    async fn update(&self, item: &Task) -> Result<(), DomainError>;
    async fn soft_delete(&self, id: TaskId) -> Result<(), DomainError>;
}

#[async_trait]
impl TaskStore for SqliteTaskStore {
    async fn get(&self, id: TaskId) -> Result<Option<Task>, DomainError> {
        crate::db::read(&self.db, move |conn| {
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
        let new_id = crate::db::write(&self.db, move |conn| insert_task_tx(conn, &owned)).await?;
        Ok(new_id)
    }

    /// Write the row's fields; status is never written here.
    async fn update(&self, item: &Task) -> Result<(), DomainError> {
        let item = item.clone();
        self.db
            .transaction(move |tx| {
                update_with_status_tx(tx, &item, None, Timestamp::now()).map(|_| ())
            })
            .await?;
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
    use crate::refs::work_item_ref;

    fn ts() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    async fn fixture() -> SqliteTaskStore {
        SqliteTaskStore::new(Database::in_memory())
    }

    fn item() -> Task {
        Task {
            id: TaskId::placeholder(),
            parent_id: None,
            title: "ship it".into(),
            description: String::new(),
            status: TaskStatus::Ready,
            priority: TaskPriority::Medium,
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
        let store = fixture().await;
        let epic = store.insert(&item()).await.unwrap();
        let mut child = item();
        child.parent_id = Some(epic);
        let child = store.insert(&child).await.unwrap();
        store.soft_delete(epic).await.unwrap();
        let child_ref = work_item_ref(child);
        let (parent, dangling): (Option<String>, i64) = crate::db::read(&store.db, move |c| {
            c.query_row(
                "SELECT (SELECT parent_uri FROM v_work_item WHERE uri = ?1),
                            (SELECT count(*) FROM v_work_item w WHERE w.parent_uri IS NOT NULL
                               AND NOT EXISTS (SELECT 1 FROM v_work_item p
                                               WHERE p.uri = w.parent_uri))",
                [child_ref],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .await
        .unwrap();
        assert_eq!(parent, None);
        assert_eq!(dangling, 0);
    }

    /// A task's record is the item as the interface holds it: its
    /// canonical and native state (archived rides on done or canceled), its
    /// parent, its deletion.
    #[tokio::test]
    async fn a_tasks_record_is_the_item_as_the_interface_holds_it() {
        let store = fixture().await;
        let epic = store.insert(&item()).await.unwrap();
        let mut child = item();
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
        let gone = store.insert(&item()).await.unwrap();
        store.soft_delete(gone).await.unwrap();
        let shipped = store.insert(&item()).await.unwrap();
        store.set_status(shipped, TaskStatus::Done).await.unwrap();
        store
            .set_status(shipped, TaskStatus::Archived)
            .await
            .unwrap();
        let dropped = store.insert(&item()).await.unwrap();
        store
            .set_status(dropped, TaskStatus::Archived)
            .await
            .unwrap();

        let ids = [epic, child, gone, shipped, dropped];
        let (mut rows, interface) = crate::db::read(&store.db, move |c| {
            let rows = ids
                .iter()
                .map(|&id| {
                    let w = crate::record::record_tx(c, id).unwrap();
                    format!(
                        "{}|{}|{}|{}|{}|{}|{}",
                        w.uri,
                        w.title,
                        w.body,
                        w.state.as_str(),
                        w.native_state,
                        w.parent_uri.unwrap_or_default(),
                        w.deleted
                    )
                })
                .collect::<Vec<_>>();
            let interface: i64 = c.query_row("SELECT count(*) FROM work_item", [], |r| r.get(0))?;
            Ok((rows, interface))
        })
        .await
        .unwrap();
        rows.sort();
        // The store writes nothing of the interface's: its verbs answer
        // with these records.
        assert_eq!(interface, 0);
        let r = |id: TaskId| work_item_ref(id);
        let mut expected = vec![
            format!("{}|ship it||in_progress|in_progress||false", r(epic)),
            format!("{}|renamed|the body|todo|ready|{}|false", r(child), r(epic)),
            format!("{}|ship it||todo|ready||true", r(gone)),
            format!("{}|ship it||done|archived||false", r(shipped)),
            format!("{}|ship it||canceled|archived||false", r(dropped)),
        ];
        expected.sort();
        assert_eq!(rows, expected);
    }

    /// A deleted task can't be edited. The store logs nothing of its own:
    /// a list's events are core's, logged by the `oxplow.work_item.*`
    /// commands for every list alike.
    #[tokio::test]
    async fn a_deleted_task_takes_no_edits_and_the_store_logs_nothing() {
        let store = fixture().await;
        let mut filed = item();
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
        let logged: i64 = crate::db::read(&store.db, |c| {
            c.query_row("SELECT count(*) FROM event_log", [], |r| r.get(0))
        })
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
        let store = fixture().await;
        let id = store.insert(&item()).await.unwrap();
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
        let store = fixture().await;
        let id = store.insert(&item()).await.unwrap();
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
        let store = fixture().await;
        let id = store.insert(&item()).await.unwrap();
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
        let store = fixture().await;
        let it = item();
        let id = store.insert(&it).await.unwrap();
        let got = store.get(id).await.unwrap().unwrap();
        assert_eq!(got.id, id);
        assert_eq!(got.title, it.title);
    }

    #[tokio::test]
    async fn description_round_trips() {
        let store = fixture().await;
        let mut it = item();
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
    async fn update_overwrites_existing() {
        let store = fixture().await;
        let it = item();
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
        let store = fixture().await;
        let mut it = item();
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
        let store = fixture().await;
        let id = store.insert(&item()).await.unwrap();
        store.soft_delete(id).await.unwrap();
        let mut latest = item();
        latest.id = id;
        latest.title = "ressurected".into();
        let err = store.update(&latest).await.unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
    }
}
