//! Thread notes: the per-thread capture pad an agent writes as it works
//! (`oxplow.knowledge.add_note` / `update_note`). Each note's `[[…]]` links are
//! its `page_ref` edges (source kind `thread_note`), and each change logs
//! `knowledge.note.written` / `deleted`.

use async_trait::async_trait;
use rusqlite::params;

use oxplow_domain::stores::ThreadNoteStore;
use oxplow_domain::{DomainError, NoteId, ThreadId, ThreadNote, Timestamp};

use crate::database::Database;
use crate::database::{string_to_ts, ts_to_string};
use crate::page_ref_projections::{note_edges, KIND_THREAD_NOTE};
use crate::page_ref_store::replace_source_tx;

#[derive(Clone)]
pub struct SqliteThreadNoteStore {
    db: Database,
}

/// A thread note's `knowledge.note.written` / `deleted` (how the search
/// index hears of it).
fn note_event(id: i64, thread: i64, deleted: bool) -> oxplow_domain::Envelope {
    use oxplow_domain::events::schema::{
        KnowledgeNoteDeleted, KnowledgeNoteDeletedV2, KnowledgeNoteWritten, KnowledgeNoteWrittenV2,
    };
    let note = format!("{KIND_THREAD_NOTE}:{}", NoteId::new(id));
    let thread = oxplow_domain::refs::build::thread_ref(ThreadId::new(thread));
    const SOURCE: &str = "system:notes";
    let env = if deleted {
        oxplow_domain::Envelope::typed::<KnowledgeNoteDeleted>(
            SOURCE,
            &KnowledgeNoteDeletedV2 {
                note: note.clone(),
                thread: thread.clone(),
            },
        )
    } else {
        oxplow_domain::Envelope::typed::<KnowledgeNoteWritten>(
            SOURCE,
            &KnowledgeNoteWrittenV2 {
                note: note.clone(),
                thread: thread.clone(),
            },
        )
    };
    env.with_subject([note, thread])
}

/// The event for a change to note `id`; `None` when there's no such note.
fn thread_note_event(
    conn: &rusqlite::Connection,
    id: i64,
    deleted: bool,
) -> rusqlite::Result<Option<oxplow_domain::Envelope>> {
    use rusqlite::OptionalExtension;
    let thread: Option<i64> = conn
        .query_row(
            "SELECT thread_id FROM thread_note WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(thread.map(|t| note_event(id, t, deleted)))
}

/// A note on `thread`, with its `page_ref` edges, on `conn` — a command's
/// transaction (`oxplow.knowledge.add_note`, P8.A6). Returns it and the
/// `knowledge.note.written` to log.
pub fn add_thread_note_tx(
    conn: &rusqlite::Connection,
    kinds: &oxplow_domain::refs::kind::KindRegistry,
    thread: ThreadId,
    body: &str,
    author: &str,
) -> Result<(ThreadNote, oxplow_domain::Envelope), DomainError> {
    let now = Timestamp::now();
    conn.execute(
        "INSERT INTO thread_note (thread_id, body, author, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![thread.value(), body, author, ts_to_string(now)],
    )
    .map_err(crate::database::map_sql_err)?;
    let id = conn.last_insert_rowid();
    let note_id = NoteId::new(id);
    replace_source_tx(
        conn,
        KIND_THREAD_NOTE,
        &note_id.to_string(),
        note_edges(kinds, KIND_THREAD_NOTE, &note_id.to_string(), body),
    )?;
    Ok((
        ThreadNote {
            id: note_id,
            thread_id: thread,
            body: body.to_string(),
            author: author.to_string(),
            created_at: now,
        },
        note_event(id, thread.value(), false),
    ))
}

/// Thread note `id`, on `conn`.
pub fn note_tx(conn: &rusqlite::Connection, id: NoteId) -> Result<Option<ThreadNote>, DomainError> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT * FROM thread_note WHERE id = ?1",
        params![id.value()],
        row_to_note,
    )
    .optional()
    .map_err(crate::database::map_sql_err)
}

/// Replace a note's body and its `page_ref` edges; the event to log.
pub fn update_note_tx(
    conn: &rusqlite::Connection,
    kinds: &oxplow_domain::refs::kind::KindRegistry,
    id: NoteId,
    body: &str,
) -> Result<Option<oxplow_domain::Envelope>, DomainError> {
    conn.execute(
        "UPDATE thread_note SET body = ?2 WHERE id = ?1",
        params![id.value(), body],
    )
    .map_err(crate::database::map_sql_err)?;
    replace_source_tx(
        conn,
        KIND_THREAD_NOTE,
        &id.to_string(),
        note_edges(kinds, KIND_THREAD_NOTE, &id.to_string(), body),
    )?;
    thread_note_event(conn, id.value(), false).map_err(crate::database::map_sql_err)
}

/// Delete a note and its `page_ref` edges; the event to log (read before
/// it goes).
pub fn delete_note_tx(
    conn: &rusqlite::Connection,
    id: NoteId,
) -> Result<Option<oxplow_domain::Envelope>, DomainError> {
    let event = thread_note_event(conn, id.value(), true).map_err(crate::database::map_sql_err)?;
    conn.execute("DELETE FROM thread_note WHERE id = ?1", params![id.value()])
        .map_err(crate::database::map_sql_err)?;
    replace_source_tx(conn, KIND_THREAD_NOTE, &id.to_string(), vec![])?;
    Ok(event)
}

impl SqliteThreadNoteStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Iterate every note's id + body for the boot-time backfill.
    pub async fn list_all_for_backfill(&self) -> Result<Vec<(String, String)>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT id, body FROM thread_note")?;
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

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadNote> {
    let created_at: String = row.get("created_at")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(ThreadNote {
        id: NoteId::new(row.get("id")?),
        thread_id: ThreadId::new(row.get("thread_id")?),
        body: row.get("body")?,
        author: row.get("author")?,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
    })
}

#[async_trait]
impl ThreadNoteStore for SqliteThreadNoteStore {
    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<ThreadNote>, DomainError> {
        let thread = *thread;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM thread_note WHERE thread_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![thread.value()], row_to_note)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page_ref_store::SqlitePageRefStore;
    use crate::stream_store::SqliteStreamStore;
    use crate::thread_store::SqliteThreadStore;
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadStatus};

    fn kinds() -> oxplow_domain::refs::kind::KindRegistry {
        oxplow_domain::refs::kind::core_kinds()
            .with_work_item_ids("oxplow", r"tsk\d+")
            .unwrap()
    }

    async fn fixture() -> (Database, ThreadId) {
        let db = Database::in_memory();
        let at = Timestamp::from_unix_ms(1);
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
            created_at: at,
            updated_at: at,
            archived_at: None,
        };
        SqliteStreamStore::new(db.clone()).upsert(&s).await.unwrap();
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
            created_at: at,
            updated_at: at,
            archived_at: None,
        };
        SqliteThreadStore::new(db.clone()).upsert(&t).await.unwrap();
        (db, t.id)
    }

    async fn add(
        db: &Database,
        thread: ThreadId,
        body: &str,
    ) -> (ThreadNote, oxplow_domain::Envelope) {
        let body = body.to_string();
        db.transaction(move |tx| add_thread_note_tx(tx, &kinds(), thread, &body, "agent"))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_note_round_trips_on_its_thread() {
        let (db, tid) = fixture().await;
        let (note, event) = add(&db, tid, "thread-level finding").await;
        assert_eq!(note.thread_id, tid);
        let listed = SqliteThreadNoteStore::new(db.clone())
            .list_for_thread(&tid)
            .await
            .unwrap();
        assert_eq!(listed, vec![note.clone()]);
        assert_eq!(event.event_type, "knowledge.note.written");
        assert_eq!(event.v, 2);
        assert_eq!(event.payload["note"], format!("thread_note:{}", note.id));
    }

    /// Its links are `thread_note` edges, restated by an update and gone
    /// with the note.
    #[tokio::test]
    async fn a_notes_links_follow_its_body() {
        let (db, tid) = fixture().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let (note, _) = add(&db, tid, "blocked by tsk99 see [[src/app.rs]]").await;
        let inbound = page_refs
            .list_backlinks("work_item", "oxplow:tsk99", None)
            .await
            .unwrap();
        assert!(
            inbound
                .iter()
                .any(|e| e.source_kind == "thread_note" && e.source_id == note.id.to_string()),
            "{inbound:?}"
        );

        let id = note.id;
        db.transaction(move |tx| update_note_tx(tx, &kinds(), id, "no refs"))
            .await
            .unwrap()
            .expect("an update logs its event");
        assert!(page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap()
            .is_empty());

        let deleted = db
            .transaction(move |tx| delete_note_tx(tx, id))
            .await
            .unwrap()
            .expect("a delete logs its event");
        assert_eq!(deleted.event_type, "knowledge.note.deleted");
        assert!(SqliteThreadNoteStore::new(db.clone())
            .list_for_thread(&tid)
            .await
            .unwrap()
            .is_empty());
    }
}
