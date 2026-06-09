//! Stores for the satellites of `task`: notes, links, events.
//!
//! Each is small enough to share this module rather than warranting
//! its own file. They share the timestamp + helper plumbing the
//! main task_store already establishes.

use async_trait::async_trait;
use rusqlite::params;

use oxplow_domain::stores::{TaskEventStore, TaskLinkStore, TaskNoteStore};
use oxplow_domain::{
    DomainError, NoteId, TaskActorKind, TaskEvent, TaskId, TaskLink, TaskLinkId, TaskLinkType,
    TaskNote, ThreadId, Timestamp,
};

use crate::database::Database;
use crate::page_ref_projections::{
    link_edge, note_edges, task_link_ref_types, KIND_TASK, KIND_TASK_NOTE,
};
use crate::page_ref_store::SqlitePageRefStore;

fn ts_to_string(ts: Timestamp) -> String {
    serde_json::to_string(&ts)
        .expect("Timestamp serializes to JSON")
        .trim_matches('"')
        .to_string()
}

fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DomainError::Invalid(format!("bad timestamp: {e}")))
}

fn link_type_to_str(t: TaskLinkType) -> &'static str {
    match t {
        TaskLinkType::Blocks => "blocks",
        TaskLinkType::RelatesTo => "relates_to",
        TaskLinkType::DiscoveredFrom => "discovered_from",
        TaskLinkType::Duplicates => "duplicates",
        TaskLinkType::Supersedes => "supersedes",
        TaskLinkType::RepliesTo => "replies_to",
    }
}

fn str_to_link_type(s: &str) -> Result<TaskLinkType, DomainError> {
    match s {
        "blocks" => Ok(TaskLinkType::Blocks),
        "relates_to" => Ok(TaskLinkType::RelatesTo),
        "discovered_from" => Ok(TaskLinkType::DiscoveredFrom),
        "duplicates" => Ok(TaskLinkType::Duplicates),
        "supersedes" => Ok(TaskLinkType::Supersedes),
        "replies_to" => Ok(TaskLinkType::RepliesTo),
        other => Err(DomainError::Invalid(format!("unknown link type: {other}"))),
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

// ---------------- Work notes ----------------

#[derive(Clone)]
pub struct SqliteTaskNoteStore {
    db: Database,
    page_refs: Option<SqlitePageRefStore>,
}

impl SqliteTaskNoteStore {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            page_refs: None,
        }
    }

    pub fn with_page_refs(mut self, store: SqlitePageRefStore) -> Self {
        self.page_refs = Some(store);
        self
    }

    /// Iterate every note id + body for the boot-time backfill.
    pub async fn list_all_for_backfill(&self) -> Result<Vec<(String, String)>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT id, body FROM task_note")?;
                let rows = stmt.query_map([], |r| {
                    Ok((
                        NoteId::new(r.get::<_, i64>(0)?).to_string(),
                        r.get::<_, String>(1)?,
                    ))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn project_note(&self, id: &str, body: &str) -> Result<(), DomainError> {
        let Some(refs) = &self.page_refs else {
            return Ok(());
        };
        let edges = note_edges(id, body);
        refs.replace_source(KIND_TASK_NOTE, id, edges).await
    }
}

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskNote> {
    let id: i64 = row.get("id")?;
    let task_id: Option<i64> = row.get("task_id")?;
    let thread_id: Option<i64> = row.get("thread_id")?;
    let body: String = row.get("body")?;
    let author: String = row.get("author")?;
    let created_at: String = row.get("created_at")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(TaskNote {
        id: NoteId::new(id),
        task_id: task_id.map(TaskId::new),
        thread_id: thread_id.map(ThreadId::new),
        body,
        author,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
    })
}

#[async_trait]
impl TaskNoteStore for SqliteTaskNoteStore {
    async fn add_for_item(
        &self,
        item: TaskId,
        body: &str,
        author: &str,
    ) -> Result<TaskNote, DomainError> {
        let body_owned = body.to_string();
        let author = author.to_string();
        let note = self
            .db
            .call(move |conn| {
                let now = Timestamp::now();
                conn.execute(
                    "INSERT INTO task_note (task_id, body, author, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![item.value(), body_owned, author, ts_to_string(now)],
                )?;
                let id = NoteId::new(conn.last_insert_rowid());
                Ok(TaskNote {
                    id,
                    task_id: Some(item),
                    thread_id: None,
                    body: body_owned,
                    author,
                    created_at: now,
                })
            })
            .await?;
        self.project_note(&note.id.to_string(), &note.body).await?;
        Ok(note)
    }

    async fn add_for_thread(
        &self,
        thread: &ThreadId,
        body: &str,
        author: &str,
    ) -> Result<TaskNote, DomainError> {
        let thread = *thread;
        let body_owned = body.to_string();
        let author = author.to_string();
        let note = self
            .db
            .call(move |conn| {
                let now = Timestamp::now();
                conn.execute(
                    "INSERT INTO task_note (thread_id, body, author, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![thread.value(), body_owned, author, ts_to_string(now)],
                )?;
                let id = NoteId::new(conn.last_insert_rowid());
                Ok(TaskNote {
                    id,
                    task_id: None,
                    thread_id: Some(thread),
                    body: body_owned,
                    author,
                    created_at: now,
                })
            })
            .await?;
        self.project_note(&note.id.to_string(), &note.body).await?;
        Ok(note)
    }

    async fn list_for_item(&self, item: TaskId) -> Result<Vec<TaskNote>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM task_note WHERE task_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![item.value()], row_to_note)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<TaskNote>, DomainError> {
        let thread = *thread;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM task_note WHERE thread_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![thread.value()], row_to_note)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn update_body(&self, id: &NoteId, body: &str) -> Result<(), DomainError> {
        let id_clone = *id;
        let body_clone = body.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE task_note SET body = ?2 WHERE id = ?1",
                    params![id_clone.value(), body_clone],
                )?;
                Ok(())
            })
            .await?;
        self.project_note(&id.to_string(), body).await?;
        Ok(())
    }

    async fn delete(&self, id: &NoteId) -> Result<(), DomainError> {
        let id_clone = *id;
        self.db
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM task_note WHERE id = ?1",
                    params![id_clone.value()],
                )?;
                Ok(())
            })
            .await?;
        if let Some(refs) = &self.page_refs {
            refs.replace_source(KIND_TASK_NOTE, &id.to_string(), vec![])
                .await?;
        }
        Ok(())
    }
}

// ---------------- Task links ----------------

#[derive(Clone)]
pub struct SqliteTaskLinkStore {
    db: Database,
    page_refs: Option<SqlitePageRefStore>,
}

impl SqliteTaskLinkStore {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            page_refs: None,
        }
    }

    /// Distinct `from_item_id` values across every link row. Used
    /// by the page-ref backfill so we can re-project each owning
    /// task's slice exactly once.
    pub async fn list_distinct_from_items(&self) -> Result<Vec<TaskId>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT DISTINCT from_item_id FROM task_link")?;
                let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
                rows.map(|r| r.map(TaskId::new))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    pub fn with_page_refs(mut self, store: SqlitePageRefStore) -> Self {
        self.page_refs = Some(store);
        self
    }

    /// Re-emit `task_link:*` edges for all currently-stored outgoing
    /// links of `from_item`. Called after create/delete when
    /// `page_refs` is attached.
    async fn project_outgoing_links(&self, from_item: TaskId) -> Result<(), DomainError> {
        let Some(refs) = &self.page_refs else {
            return Ok(());
        };
        let links: Vec<TaskLink> = self
            .db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM task_link WHERE from_item_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![from_item.value()], row_to_link)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        let edges: Vec<_> = links.iter().map(link_edge).collect();
        refs.replace_source_for_ref_types(
            KIND_TASK,
            &from_item.to_string(),
            task_link_ref_types(),
            edges,
        )
        .await
    }
}

fn row_to_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskLink> {
    let id: i64 = row.get("id")?;
    let thread_id: i64 = row.get("thread_id")?;
    let from_item_id: i64 = row.get("from_item_id")?;
    let to_item_id: i64 = row.get("to_item_id")?;
    let link_type: String = row.get("link_type")?;
    let created_at: String = row.get("created_at")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(TaskLink {
        id: TaskLinkId::new(id),
        thread_id: ThreadId::new(thread_id),
        from_item_id: TaskId::new(from_item_id),
        to_item_id: TaskId::new(to_item_id),
        link_type: str_to_link_type(&link_type).map_err(map_err)?,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
    })
}

#[async_trait]
impl TaskLinkStore for SqliteTaskLinkStore {
    async fn create(
        &self,
        thread: &ThreadId,
        from: TaskId,
        to: TaskId,
        link_type: TaskLinkType,
    ) -> Result<TaskLink, DomainError> {
        let thread_clone = *thread;
        let link = self
            .db
            .call(move |conn| {
                let now = Timestamp::now();
                conn.execute(
                    "INSERT INTO task_link (thread_id, from_item_id, to_item_id, link_type, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        thread_clone.value(),
                        from.value(),
                        to.value(),
                        link_type_to_str(link_type),
                        ts_to_string(now),
                    ],
                )?;
                let new_id = conn.last_insert_rowid();
                Ok(TaskLink {
                    id: TaskLinkId::new(new_id),
                    thread_id: thread_clone,
                    from_item_id: from,
                    to_item_id: to,
                    link_type,
                    created_at: now,
                })
            })
            .await?;
        self.project_outgoing_links(from).await?;
        Ok(link)
    }

    async fn list_outgoing(&self, item: TaskId) -> Result<Vec<TaskLink>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM task_link WHERE from_item_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![item.value()], row_to_link)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn list_incoming(&self, item: TaskId) -> Result<Vec<TaskLink>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM task_link WHERE to_item_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![item.value()], row_to_link)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn delete(&self, id: TaskLinkId) -> Result<(), DomainError> {
        let from_item: Option<TaskId> = self
            .db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT from_item_id FROM task_link WHERE id = ?1")?;
                let mut rows = stmt.query_map(params![id.value()], |r| r.get::<_, i64>(0))?;
                Ok(rows.next().transpose()?.map(TaskId::new))
            })
            .await?;
        self.db
            .call(move |conn| {
                conn.execute("DELETE FROM task_link WHERE id = ?1", params![id.value()])?;
                Ok(())
            })
            .await?;
        if let Some(from) = from_item {
            self.project_outgoing_links(from).await?;
        }
        Ok(())
    }
}

// ---------------- Task events ----------------

#[derive(Clone)]
pub struct SqliteTaskEventStore {
    db: Database,
}

impl SqliteTaskEventStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskEvent> {
    let id: String = row.get("id")?;
    let thread_id: i64 = row.get("thread_id")?;
    let item_id: Option<i64> = row.get("item_id")?;
    let event_type: String = row.get("event_type")?;
    let actor_kind: String = row.get("actor_kind")?;
    let actor_id: String = row.get("actor_id")?;
    let payload_json: String = row.get("payload_json")?;
    let created_at: String = row.get("created_at")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(TaskEvent {
        id,
        thread_id: ThreadId::new(thread_id),
        item_id: item_id.map(TaskId::new),
        event_type,
        actor_kind: str_to_actor(&actor_kind).map_err(map_err)?,
        actor_id,
        payload_json,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
    })
}

#[async_trait]
impl TaskEventStore for SqliteTaskEventStore {
    async fn append(&self, event: &TaskEvent) -> Result<(), DomainError> {
        let event = event.clone();
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO task_event
                       (id, thread_id, item_id, event_type, actor_kind, actor_id, payload_json, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        event.id,
                        event.thread_id.value(),
                        event.item_id.map(|i| i.value()),
                        event.event_type,
                        actor_to_str(event.actor_kind),
                        event.actor_id,
                        event.payload_json,
                        ts_to_string(event.created_at),
                    ],
                )?;
                Ok(())
            })
            .await
    }

    async fn list_for_item(&self, item: TaskId) -> Result<Vec<TaskEvent>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM task_event WHERE item_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![item.value()], row_to_event)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<TaskEvent>, DomainError> {
        let thread = *thread;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM task_event WHERE thread_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![thread.value()], row_to_event)?;
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
        Stream, StreamId, StreamKind, Task, TaskAuthor, TaskPriority, TaskStatus, Thread,
        ThreadStatus,
    };

    fn now() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    async fn fixture() -> (Database, ThreadId, TaskId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let items = SqliteTaskStore::new(db.clone());

        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/r".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: now(),
            updated_at: now(),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();

        let t = Thread {
            id: ThreadId::new(1),
            stream_id: s.id,
            title: "t".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now(),
            updated_at: now(),
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();

        let item = Task {
            id: TaskId::placeholder(),
            thread_id: Some(t.id),
            parent_id: None,
            title: "x".into(),
            description: String::new(),
            status: TaskStatus::Ready,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now(),
            updated_at: now(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
        };
        let item_id = items.insert(&item).await.unwrap();
        (db, t.id, item_id)
    }

    #[tokio::test]
    async fn note_for_item_round_trips() {
        let (db, _tid, item_id) = fixture().await;
        let store = SqliteTaskNoteStore::new(db);
        let note = store
            .add_for_item(item_id, "looking good", "user")
            .await
            .unwrap();
        assert_eq!(note.task_id, Some(item_id));
        assert!(note.thread_id.is_none());
        let listed = store.list_for_item(item_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].body, "looking good");
    }

    #[tokio::test]
    async fn note_for_thread_round_trips() {
        let (db, tid, _item_id) = fixture().await;
        let store = SqliteTaskNoteStore::new(db);
        let note = store
            .add_for_thread(&tid, "thread-level finding", "agent")
            .await
            .unwrap();
        assert!(note.task_id.is_none());
        assert_eq!(note.thread_id.as_ref(), Some(&tid));
        let listed = store.list_for_thread(&tid).await.unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[tokio::test]
    async fn note_delete_removes() {
        let (db, _tid, item_id) = fixture().await;
        let store = SqliteTaskNoteStore::new(db);
        let note = store.add_for_item(item_id, "x", "u").await.unwrap();
        store.delete(&note.id).await.unwrap();
        assert!(store.list_for_item(item_id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn note_with_page_refs_projects_body() {
        use crate::page_ref_store::SqlitePageRefStore;
        let (db, tid, item_id) = fixture().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let store = SqliteTaskNoteStore::new(db).with_page_refs(page_refs.clone());

        let note = store
            .add_for_item(item_id, "blocked by tsk99 see [[src/app.rs]]", "u")
            .await
            .unwrap();
        let inbound_task = page_refs
            .list_backlinks("task", "tsk99", None)
            .await
            .unwrap();
        assert!(
            inbound_task
                .iter()
                .any(|e| e.source_kind == "task-note" && e.source_id == note.id.to_string()),
            "expected note to backlink tsk99; got {inbound_task:?}"
        );
        let inbound_file = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(inbound_file
            .iter()
            .any(|e| e.source_id == note.id.to_string()));

        store.update_body(&note.id, "no refs").await.unwrap();
        let inbound_file = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(inbound_file
            .iter()
            .all(|e| e.source_id != note.id.to_string()));

        store.delete(&note.id).await.unwrap();
        let inbound_task = page_refs
            .list_backlinks("task", "tsk99", None)
            .await
            .unwrap();
        assert!(inbound_task
            .iter()
            .all(|e| e.source_id != note.id.to_string()));

        let tnote = store
            .add_for_thread(&tid, "see [[src/lib.rs]]", "u")
            .await
            .unwrap();
        let inbound_lib = page_refs
            .list_backlinks("file", "src/lib.rs", None)
            .await
            .unwrap();
        assert!(inbound_lib
            .iter()
            .any(|e| e.source_id == tnote.id.to_string()));
    }

    #[tokio::test]
    async fn link_create_delete_projects_page_ref_slice() {
        use crate::page_ref_store::SqlitePageRefStore;
        let (db, tid, from_id) = fixture().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let items = SqliteTaskStore::new(db.clone()).with_page_refs(page_refs.clone());
        let mut sender = items.get(from_id).await.unwrap().unwrap();
        sender.description = "see [[src/app.rs]]".into();
        items.update(&sender).await.unwrap();

        let to = Task {
            id: TaskId::placeholder(),
            thread_id: Some(tid),
            parent_id: None,
            title: "y".into(),
            description: String::new(),
            status: TaskStatus::Ready,
            priority: TaskPriority::Medium,
            sort_index: 1,
            created_by: TaskActorKind::User,
            created_at: now(),
            updated_at: now(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
        };
        let to_id = items.insert(&to).await.unwrap();

        let links = SqliteTaskLinkStore::new(db.clone()).with_page_refs(page_refs.clone());
        let link = links
            .create(&tid, from_id, to_id, TaskLinkType::Blocks)
            .await
            .unwrap();

        let inbound_to = page_refs
            .list_backlinks("task", &to_id.to_string(), None)
            .await
            .unwrap();
        assert!(inbound_to
            .iter()
            .any(|e| e.source_id == from_id.to_string() && e.ref_type == "task_link:blocks"));

        let inbound_file = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(inbound_file
            .iter()
            .any(|e| e.source_id == from_id.to_string()));

        links.delete(link.id).await.unwrap();
        let inbound_to = page_refs
            .list_backlinks("task", &to_id.to_string(), None)
            .await
            .unwrap();
        assert!(inbound_to.is_empty(), "link backlink should clear");
        let inbound_file = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(
            inbound_file
                .iter()
                .any(|e| e.source_id == from_id.to_string()),
            "body-mention slice must survive link deletion"
        );
    }

    #[tokio::test]
    async fn link_round_trip_and_directionality() {
        let (db, tid, from_id) = fixture().await;
        let items = SqliteTaskStore::new(db.clone());
        let to = Task {
            id: TaskId::placeholder(),
            thread_id: Some(tid),
            parent_id: None,
            title: "y".into(),
            description: String::new(),
            status: TaskStatus::Ready,
            priority: TaskPriority::Medium,
            sort_index: 1,
            created_by: TaskActorKind::User,
            created_at: now(),
            updated_at: now(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
        };
        let to_id = items.insert(&to).await.unwrap();
        let store = SqliteTaskLinkStore::new(db);
        store
            .create(&tid, from_id, to_id, TaskLinkType::Blocks)
            .await
            .unwrap();
        let outgoing = store.list_outgoing(from_id).await.unwrap();
        let incoming = store.list_incoming(to_id).await.unwrap();
        assert_eq!(outgoing.len(), 1);
        assert_eq!(incoming.len(), 1);
        assert_eq!(outgoing[0].link_type, TaskLinkType::Blocks);
    }

    #[tokio::test]
    async fn event_append_and_list() {
        let (db, tid, item_id) = fixture().await;
        let store = SqliteTaskEventStore::new(db);
        let evt = TaskEvent {
            id: "evt-1".into(),
            thread_id: tid,
            item_id: Some(item_id),
            event_type: "transition".into(),
            actor_kind: TaskActorKind::Agent,
            actor_id: "claude".into(),
            payload_json: "{\"to\":\"in_progress\"}".into(),
            created_at: now(),
        };
        store.append(&evt).await.unwrap();
        let item_events = store.list_for_item(item_id).await.unwrap();
        let thread_events = store.list_for_thread(&tid).await.unwrap();
        assert_eq!(item_events.len(), 1);
        assert_eq!(thread_events.len(), 1);
        assert_eq!(thread_events[0].event_type, "transition");
    }
}
