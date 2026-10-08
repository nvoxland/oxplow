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

/// A note on task `item` — the core of `oxplow.work_item.comment`,
/// composing inside the bus's transaction.
pub fn add_task_note_tx(
    conn: &rusqlite::Connection,
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
    Ok(TaskNote {
        id,
        task_id: item,
        body: body.to_string(),
        author: author.to_string(),
        created_at: now,
    })
}

/// A typed link from `from` to `to`, made in `thread` — the core of
/// `oxplow.work_item.link`.
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
    Ok(TaskLink {
        id: TaskLinkId::new(conn.last_insert_rowid()),
        thread_id: thread,
        from_item_id: from,
        to_item_id: to,
        link_type,
        created_at: now,
    })
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
}

impl SqliteTaskLinkStore {
    pub fn new(db: Database) -> Self {
        Self { db }
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
        self.db
            .call(move |conn| {
                conn.execute("DELETE FROM task_link WHERE id = ?1", params![id.value()])?;
                Ok(())
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
        db.transaction(move |tx| add_task_note_tx(tx, item, &body, &author))
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

    /// A comment and a link reach the work-item interface (its
    /// `work_item_comment` / `work_item_link` rows, by trigger) and write
    /// no page refs: an item's page refs are core's, restated from the
    /// interface for every list.
    #[tokio::test]
    async fn a_comment_and_a_link_reach_the_interface_and_write_no_page_refs() {
        let (db, tid, from_id) = fixture().await;
        let to_id = SqliteTaskStore::new(db.clone())
            .insert(&Task {
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
            })
            .await
            .unwrap();
        add_note(&db, from_id, "blocked by tsk99 see [[src/app.rs]]", "u").await;
        db.transaction(move |tx| create_link_tx(tx, tid, from_id, to_id, TaskLinkType::Blocks))
            .await
            .unwrap();
        let counts: (i64, i64, i64) = db
            .call(|c| {
                c.query_row(
                    "SELECT (SELECT count(*) FROM work_item_comment),
                            (SELECT count(*) FROM work_item_link),
                            (SELECT count(*) FROM page_ref)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!(counts, (1, 1, 0));
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
