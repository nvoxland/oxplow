//! Stores for the satellites of `task`: notes, links, events.
//!
//! Each is small enough to share this module rather than warranting
//! its own file. They share the timestamp + helper plumbing the
//! main task_store already establishes.

use async_trait::async_trait;
use rusqlite::params;

use crate::ids::OxtaskId;
use oxplow_db::{map_sql_err, string_to_ts, ts_to_string, Database};
use oxplow_domain::{DomainError, NoteId, Timestamp};

use crate::model::{OxtaskLinkType, OxtaskNote};

fn link_type_to_str(t: OxtaskLinkType) -> &'static str {
    match t {
        OxtaskLinkType::Blocks => "blocks",
        OxtaskLinkType::RelatesTo => "relates_to",
        OxtaskLinkType::DiscoveredFrom => "discovered_from",
        OxtaskLinkType::Duplicates => "duplicates",
        OxtaskLinkType::Supersedes => "supersedes",
        OxtaskLinkType::RepliesTo => "replies_to",
        OxtaskLinkType::About => "about",
    }
}

/// A note on task `item` — the core of `oxplow.task.comment`,
/// composing inside the bus's transaction.
pub fn add_oxtask_note_tx(
    conn: &rusqlite::Connection,
    item: OxtaskId,
    body: &str,
    author: &str,
) -> Result<OxtaskNote, DomainError> {
    let now = Timestamp::now();
    conn.execute(
        "INSERT INTO oxtask_note (task_id, body, author, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![item.value(), body, author, ts_to_string(now)],
    )
    .map_err(map_sql_err)?;
    let id = NoteId::new(conn.last_insert_rowid());
    Ok(OxtaskNote {
        id,
        task_id: item,
        body: body.to_string(),
        author: author.to_string(),
        created_at: now,
    })
}

/// A typed link from task `from` to any ref `to` (another item, or what
/// the item is `about`) — the core of
/// `oxplow.task.link`. Linking twice is one link: whether this one
/// is new.
pub fn create_link_tx(
    conn: &rusqlite::Connection,
    from: OxtaskId,
    to: &str,
    link_type: OxtaskLinkType,
) -> Result<bool, DomainError> {
    let added = conn
        .execute(
            "INSERT OR IGNORE INTO oxtask_link (from_item_id, to_uri, link_type, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                from.value(),
                to,
                link_type_to_str(link_type),
                ts_to_string(Timestamp::now()),
            ],
        )
        .map_err(map_sql_err)?;
    Ok(added > 0)
}

/// Remove `from`'s link of `link_type` to `to` — the core of
/// `oxplow.task.unlink`: whether there was one.
pub fn delete_link_tx(
    conn: &rusqlite::Connection,
    from: OxtaskId,
    to: &str,
    link_type: OxtaskLinkType,
) -> Result<bool, DomainError> {
    let removed = conn
        .execute(
            "DELETE FROM oxtask_link WHERE from_item_id = ?1 AND to_uri = ?2 AND link_type = ?3",
            params![from.value(), to, link_type_to_str(link_type)],
        )
        .map_err(map_sql_err)?;
    Ok(removed > 0)
}

// ---------------- Oxtask comments ----------------

#[derive(Clone)]
pub struct SqliteOxtaskNoteStore {
    db: Database,
}

impl SqliteOxtaskNoteStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<OxtaskNote> {
    let id: i64 = row.get("id")?;
    let task_id: i64 = row.get("task_id")?;
    let body: String = row.get("body")?;
    let author: String = row.get("author")?;
    let created_at: String = row.get("created_at")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(OxtaskNote {
        id: NoteId::new(id),
        task_id: OxtaskId::new(task_id),
        body,
        author,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
    })
}

#[async_trait]
pub trait OxtaskNoteStore: Send + Sync {
    // A note on a task is a task comment: the `oxplow.task.comment`
    // command ([`add_oxtask_note_tx`]).
    async fn list_for_item(&self, item: OxtaskId) -> Result<Vec<OxtaskNote>, DomainError>;
}

#[async_trait]
impl OxtaskNoteStore for SqliteOxtaskNoteStore {
    async fn list_for_item(&self, item: OxtaskId) -> Result<Vec<OxtaskNote>, DomainError> {
        crate::db::read(&self.db, move |conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM oxtask_note WHERE task_id = ?1 ORDER BY created_at ASC")?;
            let rows = stmt.query_map(params![item.value()], row_to_note)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use oxplow_domain::ThreadId;

    use crate::model::{Oxtask, OxtaskActorKind, OxtaskAuthor, OxtaskPriority, OxtaskStatus};
    use crate::store::{OxtaskStore, SqliteOxtaskStore};
    use oxplow_db::{SqliteStreamStore, SqliteThreadStore};
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{Stream, StreamId, StreamKind, Thread};

    fn now() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    async fn fixture() -> (Database, ThreadId, OxtaskId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let items = SqliteOxtaskStore::new(db.clone());

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
            host: oxplow_domain::HostId::LOCAL,
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
            sort_index: 0,
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now(),
            updated_at: now(),
            archived_at: None,
            mark: None,
        };
        threads.upsert(&t).await.unwrap();

        let item = Oxtask {
            id: OxtaskId::placeholder(),
            parent_id: None,
            title: "x".into(),
            description: String::new(),
            status: OxtaskStatus::Ready,
            priority: OxtaskPriority::Medium,
            created_by: OxtaskActorKind::User,
            created_at: now(),
            updated_at: now(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(OxtaskAuthor::User),
        };
        let item_id = items.insert(&item).await.unwrap();
        (db, t.id, item_id)
    }

    async fn add_note(db: &Database, item: OxtaskId, body: &str, author: &str) -> OxtaskNote {
        let (body, author) = (body.to_string(), author.to_string());
        db.transaction(move |tx| add_oxtask_note_tx(tx, item, &body, &author))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn note_for_item_round_trips() {
        let (db, _tid, item_id) = fixture().await;
        let store = SqliteOxtaskNoteStore::new(db.clone());
        let note = add_note(&db, item_id, "looking good", "user").await;
        assert_eq!(note.task_id, item_id);
        let listed = store.list_for_item(item_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].body, "looking good");
    }

    /// A comment and a link are the task's, in its record — and write
    /// nothing of core's (no `task_*` rows, no page refs): what a verb
    /// did reaches the interface through the record it answers with.
    #[tokio::test]
    async fn a_comment_and_a_link_are_in_the_record_and_nothing_else() {
        let (db, _tid, from_id) = fixture().await;
        let to_id = SqliteOxtaskStore::new(db.clone())
            .insert(&Oxtask {
                id: OxtaskId::placeholder(),
                parent_id: None,
                title: "y".into(),
                description: String::new(),
                status: OxtaskStatus::Ready,
                priority: OxtaskPriority::Medium,
                created_by: OxtaskActorKind::User,
                created_at: now(),
                updated_at: now(),
                completed_at: None,
                deleted_at: None,
                note_count: 0,
                author: Some(OxtaskAuthor::User),
            })
            .await
            .unwrap();
        add_note(&db, from_id, "blocked by tsk99 see [[src/app.rs]]", "u").await;
        db.transaction(move |tx| {
            create_link_tx(
                tx,
                from_id,
                &crate::refs::task_ref(to_id),
                OxtaskLinkType::Blocks,
            )
        })
        .await
        .unwrap();
        let counts: (i64, i64, i64) = crate::db::read(&db, |c| {
            c.query_row(
                "SELECT (SELECT count(*) FROM task_comment),
                            (SELECT count(*) FROM task_link),
                            (SELECT count(*) FROM page_ref)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .await
        .unwrap();
        assert_eq!(counts, (0, 0, 0));
        let record = crate::db::read(&db, move |c| {
            Ok(crate::record::record_tx(c, from_id).unwrap())
        })
        .await
        .unwrap();
        assert_eq!(
            record.links,
            Some(vec![oxplow_domain::tasks::LinkRecord {
                target: crate::refs::task_ref(to_id),
                link_type: "blocks".into(),
            }])
        );
        let comments = record.comments.unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].body, "blocked by tsk99 see [[src/app.rs]]");
    }
}
