//! Stores for the satellites of `task`: notes, links, events.
//!
//! Each is small enough to share this module rather than warranting
//! its own file. They share the timestamp + helper plumbing the
//! main task_store already establishes.

use async_trait::async_trait;
use rusqlite::params;

use oxplow_domain::stores::{TaskLinkStore, TaskNoteStore};
use oxplow_domain::{
    DomainError, NoteId, TaskId, TaskLink, TaskLinkId, TaskLinkType, TaskNote, ThreadId, Timestamp,
};

use crate::database::Database;
use crate::database::{string_to_ts, ts_to_string};
use crate::page_ref_projections::{
    link_edge, note_edges, task_link_ref_types, work_item_id, KIND_TASK_NOTE, KIND_WORK_ITEM,
};
use crate::page_ref_store::{
    replace_source_for_ref_types_tx, replace_source_tx, SqlitePageRefStore,
};

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

/// A note on task `item`, with its `page_ref` edges — the core of
/// `work_item.comment`, composing inside the bus's transaction.
pub fn add_task_note_tx(
    conn: &rusqlite::Connection,
    kinds: &oxplow_domain::refs::kind::KindRegistry,
    item: TaskId,
    body: &str,
    author: &str,
) -> Result<TaskNote, DomainError> {
    let now = Timestamp::now();
    conn.execute(
        "INSERT INTO task_note (task_id, body, author, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![item.value(), body, author, ts_to_string(now)],
    )
    .map_err(crate::database::map_sql_err)?;
    let id = NoteId::new(conn.last_insert_rowid());
    replace_source_tx(
        conn,
        KIND_TASK_NOTE,
        &id.to_string(),
        note_edges(kinds, KIND_TASK_NOTE, &id.to_string(), body),
    )?;
    Ok(TaskNote {
        id,
        task_id: item,
        body: body.to_string(),
        author: author.to_string(),
        created_at: now,
    })
}

/// A typed link from `from` to `to`, made in `thread`, restating `from`'s
/// link edges — the core of `work_item.link`.
pub fn create_link_tx(
    conn: &rusqlite::Connection,
    thread: ThreadId,
    from: TaskId,
    to: TaskId,
    link_type: TaskLinkType,
) -> Result<TaskLink, DomainError> {
    let now = Timestamp::now();
    conn.execute(
        "INSERT INTO task_link (thread_id, from_item_id, to_item_id, link_type, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            thread.value(),
            from.value(),
            to.value(),
            link_type_to_str(link_type),
            ts_to_string(now),
        ],
    )
    .map_err(crate::database::map_sql_err)?;
    let link = TaskLink {
        id: TaskLinkId::new(conn.last_insert_rowid()),
        thread_id: thread,
        from_item_id: from,
        to_item_id: to,
        link_type,
        created_at: now,
    };
    let mut stmt = conn
        .prepare("SELECT * FROM task_link WHERE from_item_id = ?1 ORDER BY created_at ASC")
        .map_err(crate::database::map_sql_err)?;
    let edges = stmt
        .query_map(params![from.value()], row_to_link)
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(crate::database::map_sql_err)?
        .iter()
        .map(link_edge)
        .collect();
    replace_source_for_ref_types_tx(
        conn,
        KIND_WORK_ITEM,
        &work_item_id(from),
        &task_link_ref_types(),
        edges,
    )?;
    Ok(link)
}

// ---------------- Task comments ----------------

#[derive(Clone)]
pub struct SqliteTaskNoteStore {
    db: Database,
}

impl SqliteTaskNoteStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Iterate every comment's id + body for the boot-time backfill.
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
}

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskNote> {
    let id: i64 = row.get("id")?;
    let task_id: i64 = row.get("task_id")?;
    let body: String = row.get("body")?;
    let author: String = row.get("author")?;
    let created_at: String = row.get("created_at")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(TaskNote {
        id: NoteId::new(id),
        task_id: TaskId::new(task_id),
        body,
        author,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
    })
}

#[async_trait]
impl TaskNoteStore for SqliteTaskNoteStore {
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
}

// ---------------- Task links ----------------

#[derive(Clone)]
pub struct SqliteTaskLinkStore {
    db: Database,
    page_refs: SqlitePageRefStore,
}

impl SqliteTaskLinkStore {
    pub fn new(db: Database) -> Self {
        Self {
            page_refs: SqlitePageRefStore::new(db.clone()),
            db,
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

    /// Re-emit `work_item_link:*` edges for all currently-stored outgoing
    /// links of `from_item`. Called after create/delete when
    /// `page_refs` is attached.
    async fn project_outgoing_links(&self, from_item: TaskId) -> Result<(), DomainError> {
        let refs = &self.page_refs;
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
            KIND_WORK_ITEM,
            &work_item_id(from_item),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The core kinds with oxplow's tasks as the work list (`tsk<n>`).
    fn tasks_kinds() -> oxplow_domain::refs::kind::KindRegistry {
        oxplow_domain::refs::kind::core_kinds()
            .with_work_item_ids("oxplow", r"tsk\d+")
            .unwrap()
    }
    use crate::stream_store::SqliteStreamStore;
    use crate::task_store::SqliteTaskStore;
    use crate::thread_store::SqliteThreadStore;
    use oxplow_domain::stores::{StreamStore, TaskStore, ThreadStore};
    use oxplow_domain::{
        Stream, StreamId, StreamKind, Task, TaskActorKind, TaskAuthor, TaskPriority, TaskStatus,
        Thread, ThreadStatus,
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
            acp_agent: None,
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

    async fn add_note(db: &Database, item: TaskId, body: &str, author: &str) -> TaskNote {
        let (body, author) = (body.to_string(), author.to_string());
        db.transaction(move |tx| add_task_note_tx(tx, &tasks_kinds(), item, &body, &author))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn note_for_item_round_trips() {
        let (db, _tid, item_id) = fixture().await;
        let store = SqliteTaskNoteStore::new(db.clone());
        let note = add_note(&db, item_id, "looking good", "user").await;
        assert_eq!(note.task_id, item_id);
        let listed = store.list_for_item(item_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].body, "looking good");
    }

    #[tokio::test]
    async fn a_task_comment_projects_its_body() {
        use crate::page_ref_store::SqlitePageRefStore;
        let (db, _tid, item_id) = fixture().await;
        let page_refs = SqlitePageRefStore::new(db.clone());

        let note = add_note(&db, item_id, "blocked by tsk99 see [[src/app.rs]]", "u").await;
        let inbound_task = page_refs
            .list_backlinks("work_item", "oxplow:tsk99", None)
            .await
            .unwrap();
        assert!(
            inbound_task
                .iter()
                .any(|e| e.source_kind == "task_note" && e.source_id == note.id.to_string()),
            "expected note to backlink tsk99; got {inbound_task:?}"
        );
        let inbound_file = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(inbound_file
            .iter()
            .any(|e| e.source_id == note.id.to_string()));
    }

    #[tokio::test]
    async fn link_create_delete_projects_page_ref_slice() {
        use crate::page_ref_store::SqlitePageRefStore;
        let (db, tid, from_id) = fixture().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let items = SqliteTaskStore::new(db.clone());
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

        let link = db
            .transaction(move |tx| create_link_tx(tx, tid, from_id, to_id, TaskLinkType::Blocks))
            .await
            .unwrap();

        let inbound_to = page_refs
            .list_backlinks("work_item", &format!("oxplow:{to_id}"), None)
            .await
            .unwrap();
        assert!(inbound_to
            .iter()
            .any(|e| e.source_id == format!("oxplow:{from_id}")
                && e.ref_type == "work_item_link:blocks"));

        let inbound_file = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(inbound_file
            .iter()
            .any(|e| e.source_id == format!("oxplow:{from_id}")));

        SqliteTaskLinkStore::new(db.clone())
            .delete(link.id)
            .await
            .unwrap();
        let inbound_to = page_refs
            .list_backlinks("work_item", &format!("oxplow:{to_id}"), None)
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
                .any(|e| e.source_id == format!("oxplow:{from_id}")),
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
        let store = SqliteTaskLinkStore::new(db.clone());
        db.transaction(move |tx| create_link_tx(tx, tid, from_id, to_id, TaskLinkType::Blocks))
            .await
            .unwrap();
        let outgoing = store.list_outgoing(from_id).await.unwrap();
        let incoming = store.list_incoming(to_id).await.unwrap();
        assert_eq!(outgoing.len(), 1);
        assert_eq!(incoming.len(), 1);
        assert_eq!(outgoing[0].link_type, TaskLinkType::Blocks);
    }
}
