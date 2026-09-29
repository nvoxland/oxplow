use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::params;

use oxplow_domain::events::schema::{WorkItemTransitioned, WorkItemTransitionedV1};
use oxplow_domain::refs::build::{effort_ref, system_source, work_item_ref};
use oxplow_domain::stores::TaskStore;
use oxplow_domain::{
    Anchors, DomainError, EffortId, Envelope, EventSchemaRegistry, Task, TaskActorKind, TaskAuthor,
    TaskId, TaskPriority, TaskStatus, ThreadId, Timestamp,
};

use crate::database::Database;
use crate::database::{string_to_ts, ts_to_string};
use crate::event_log_store::append_tx;
use crate::page_ref_projections::{task_body_ref_types, task_edges, work_item_id, KIND_WORK_ITEM};
use crate::page_ref_store::SqlitePageRefStore;

#[derive(Clone)]
pub struct SqliteTaskStore {
    db: Database,
    page_refs: SqlitePageRefStore,
    /// Validates the `work_item.transitioned` envelopes this store logs.
    event_schemas: Arc<EventSchemaRegistry>,
}

/// Outcome of [`SqliteTaskStore::update_with_effort_transition`]: which
/// effort row the in_progress boundary crossing touched.
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

    /// Insert a task filed straight into `in_progress` on `thread` and
    /// open its effort — one transaction, so "in_progress ⟺ one open
    /// effort" holds from the first commit. Snapshot pins are backfilled
    /// after commit, as for [`Self::update_with_effort_transition`].
    pub async fn insert_with_effort(
        &self,
        item: &Task,
        thread: ThreadId,
    ) -> Result<(TaskId, EffortId), DomainError> {
        use crate::database::map_sql_err;
        let owned = Arc::new(item.clone());
        let (id, effort) = self
            .db
            .transaction(move |tx| {
                let id = insert_task_tx(tx, &owned).map_err(map_sql_err)?;
                let effort = crate::effort_store::start_tx(
                    tx,
                    &work_item_ref(id),
                    thread,
                    None,
                    Timestamp::now(),
                )
                .map_err(map_sql_err)?;
                Ok((id, effort))
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

    /// Persist a task row that just crossed the in_progress boundary
    /// AND open/finish its lifecycle effort — one transaction, so the
    /// invariant "in_progress ⟺ one open effort" can't be torn by a
    /// crash or a failed side-band step. Snapshot pins are deliberately
    /// NOT part of this: the caller requests the snapshot AFTER commit
    /// and backfills via `set_start_snapshot` / `set_end_snapshot`
    /// (effort attribution must never be gated on snapshot success).
    ///
    /// On entry an already-open effort is adopted instead of erroring —
    /// the V31 unique index makes a true double-open impossible, and
    /// adoption keeps the op idempotent under Busy retry.
    ///
    /// The same transaction appends `work_item.transitioned@1` to the
    /// event log (the outbox; `.context/data-model.md` "event_log") when
    /// `from` differs from the row's new status — so the log can never
    /// claim a transition the state doesn't show, or miss one it does.
    /// `item.status == InProgress` means entering; anything else leaves.
    pub async fn update_with_effort_transition(
        &self,
        item: &Task,
        thread: ThreadId,
        from: TaskStatus,
    ) -> Result<EffortTransition, DomainError> {
        use crate::database::map_sql_err;
        let item = Arc::new(item.clone());
        let schemas = self.event_schemas.clone();
        let entering = item.status == TaskStatus::InProgress;
        let outcome = self
            .db
            .transaction(move |tx| {
                let rows = update_task_tx(tx, &item).map_err(map_sql_err)?;
                if rows == 0 {
                    return Err(DomainError::NotFound);
                }
                let work_item = work_item_ref(item.id);
                let transition = if entering {
                    match crate::effort_store::find_open_for_work_item_tx(tx, &work_item)
                        .map_err(map_sql_err)?
                    {
                        Some(open) => Ok(EffortTransition::Opened(open.id)),
                        None => Ok(EffortTransition::Opened(
                            crate::effort_store::start_tx(
                                tx,
                                &work_item,
                                thread,
                                None,
                                Timestamp::now(),
                            )
                            .map_err(map_sql_err)?,
                        )),
                    }
                } else {
                    match crate::effort_store::find_open_for_work_item_tx(tx, &work_item)
                        .map_err(map_sql_err)?
                    {
                        Some(open) => {
                            let now = ts_to_string(Timestamp::now());
                            crate::effort_store::finish_tx(tx, open.id, None, None, &now)
                                .map_err(map_sql_err)?;
                            Ok(EffortTransition::Finished(open.id))
                        }
                        None => Ok(EffortTransition::NoOpenEffort),
                    }
                }?;
                if from != item.status {
                    let effort = match transition {
                        EffortTransition::Opened(e) | EffortTransition::Finished(e) => Some(e),
                        EffortTransition::NoOpenEffort => None,
                    };
                    let mut subject = vec![work_item.clone()];
                    subject.extend(effort.map(effort_ref));
                    let stream: Option<i64> = tx
                        .query_row(
                            "SELECT stream_id FROM threads WHERE id = ?1",
                            params![thread.value()],
                            |r| r.get(0),
                        )
                        .map_err(map_sql_err)?;
                    // No dedupe key: this producer is transactional, so a
                    // retried attempt has already rolled back and can't
                    // double-log. Keys are for at-least-once producers.
                    let env = Envelope::typed::<WorkItemTransitioned>(
                        system_source("task_service"),
                        &WorkItemTransitionedV1 {
                            work_item,
                            from,
                            to: item.status,
                            effort: effort.map(effort_ref),
                        },
                    )
                    .with_anchors(Anchors {
                        stream_id: stream.map(oxplow_domain::StreamId::new),
                        thread_id: Some(thread),
                        effort_id: effort,
                        ..Anchors::default()
                    })
                    .with_subject(subject);
                    append_tx(tx, &schemas, &env)?;
                }
                Ok(transition)
            })
            .await?;
        // The body-ref projection is the `page_ref.work_item` consumer's
        // job now: it reacts to the `work_item.transitioned` event this
        // transaction logged (oxplow-app `page_ref_consumers.rs`).
        Ok(outcome)
    }
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

/// One live task by id, for composition inside a transaction (the event
/// pump's consumers read through it).
pub fn get_task_tx(conn: &rusqlite::Connection, id: TaskId) -> Result<Option<Task>, DomainError> {
    use rusqlite::OptionalExtension;
    let sql = format!("{} WHERE t.id = ?1", SELECT_BASE);
    conn.query_row(&sql, params![id.value()], row_to_task)
        .optional()
        .map_err(crate::database::map_sql_err)
}

/// Sync core for the task-row UPDATE — connection-parameterized so it
/// composes inside a `Database::transaction` closure (the lifecycle
/// transition pairs it with effort open/finish). Returns affected-row
/// count; callers map 0 to `NotFound`.
pub(crate) fn update_task_tx(conn: &rusqlite::Connection, item: &Task) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE task SET
            thread_id = ?2,
            parent_id = ?3,
            title = ?4,
            description = ?5,
            status = ?6,
            priority = ?7,
            sort_index = ?8,
            updated_at = ?9,
            completed_at = ?10,
            deleted_at = ?11,
            author = ?12
         WHERE id = ?1 AND deleted_at IS NULL",
        params![
            item.id.value(),
            item.thread_id.as_ref().map(|t| t.value()),
            item.parent_id.map(|p| p.value()),
            item.title,
            item.description,
            status_to_str(item.status),
            priority_to_str(item.priority),
            item.sort_index,
            ts_to_string(item.updated_at),
            item.completed_at.map(ts_to_string),
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

    async fn update(&self, item: &Task) -> Result<(), DomainError> {
        let item = item.clone();
        let edges_item = item.clone();
        let rows_affected: usize = self
            .db
            .call(move |conn| update_task_tx(conn, &item))
            .await?;
        if rows_affected == 0 {
            return Err(DomainError::NotFound);
        }
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

    async fn soft_delete(&self, id: TaskId) -> Result<(), DomainError> {
        let now = ts_to_string(Timestamp::now());
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE task SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1",
                    params![id.value(), now],
                )?;
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

    /// P2.5b (tsk428): filing a task straight into `in_progress` opens its
    /// effort in the insert's own transaction — both or neither.
    #[tokio::test]
    async fn insert_in_progress_opens_its_effort_in_the_same_transaction() {
        let (store, tid) = fixture().await;
        let mut it = item(Some(tid));
        it.status = TaskStatus::InProgress;
        let (id, effort) = store.insert_with_effort(&it, tid).await.unwrap();
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

        // The effort can't open (the next task's ref already has an open
        // effort), so the task row isn't written either.
        let next = work_item_ref(TaskId::new(id.value() + 1));
        store
            .db
            .call(move |c| {
                c.execute(
                    "INSERT INTO effort (work_item, thread_id, started_at)
                       VALUES (?1, 1, '2026-01-01T00:00:00.000000Z')",
                    params![next],
                )
            })
            .await
            .unwrap();
        assert!(store.insert_with_effort(&it, tid).await.is_err());
        let rows: i64 = store
            .db
            .call(|c| c.query_row("SELECT count(*) FROM task", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(rows, 1, "the failed insert rolled back with its effort");
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
        assert_eq!(got.status, TaskStatus::InProgress);
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
