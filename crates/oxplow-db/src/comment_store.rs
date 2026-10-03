//! `SqliteCommentStore` — threaded annotations anchored to a text
//! selection. Mirrors the `task_satellite.rs` plumbing
//! (`spawn_blocking` + `with_conn`, ISO timestamp helpers) and uses
//! plain autoincrement integer ids.

use async_trait::async_trait;
use rusqlite::{params, OptionalExtension};

use oxplow_domain::stores::CommentStore;
use oxplow_domain::{
    Comment, CommentId, CommentIntent, CommentMessage, CommentMessageId, CommentStatus,
    CommentTarget, CommentThread, DomainError, StreamId, ThreadId, Timestamp,
};

use crate::database::Database;
use crate::database::{string_to_ts, ts_to_string};
use oxplow_domain::Envelope;

fn intent_to_str(i: CommentIntent) -> &'static str {
    match i {
        CommentIntent::Note => "note",
        CommentIntent::Followup => "followup",
    }
}

fn str_to_intent(s: &str) -> Result<CommentIntent, DomainError> {
    match s {
        "note" => Ok(CommentIntent::Note),
        "followup" => Ok(CommentIntent::Followup),
        other => Err(DomainError::Invalid(format!(
            "unknown comment intent: {other}"
        ))),
    }
}

fn status_to_str(s: CommentStatus) -> &'static str {
    match s {
        CommentStatus::Open => "open",
        CommentStatus::Resolved => "resolved",
    }
}

fn str_to_status(s: &str) -> Result<CommentStatus, DomainError> {
    match s {
        "open" => Ok(CommentStatus::Open),
        "resolved" => Ok(CommentStatus::Resolved),
        other => Err(DomainError::Invalid(format!(
            "unknown comment status: {other}"
        ))),
    }
}

fn map_err(e: DomainError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}

/// Parse a `[{ "kind", "id" }, …]` JSON column into refs. The columns
/// carry `DEFAULT '[]'` so this is always valid JSON in practice; a
/// malformed value degrades to an empty list rather than failing the
/// whole row load (the refs are typed context, not load-bearing state).
fn parse_refs(json: &str) -> Vec<CommentTarget> {
    serde_json::from_str(json).unwrap_or_default()
}

fn refs_to_json(refs: &[CommentTarget]) -> String {
    serde_json::to_string(refs).unwrap_or_else(|_| "[]".to_string())
}

/// Canonical `(kind,id)` refs mentioned *inside* `quote`, derived from
/// the shared [`oxplow_domain::refs::extract`] so the kind/id vocabulary
/// matches the `page_ref` graph and tab ids exactly. Inline mentions
/// like `tsk42`, `src/x.rs`, or `[[slug]]` become typed refs even
/// when the surface never rendered them as links.
fn refs_from_quote(
    kinds: &oxplow_domain::refs::kind::KindRegistry,
    quote: &str,
) -> Vec<CommentTarget> {
    use crate::page_ref_projections::{
        work_item_id, KIND_COMMIT, KIND_DIR, KIND_FILE, KIND_FINDING, KIND_WIKI, KIND_WORK_ITEM,
    };
    let r = oxplow_domain::refs::extract(kinds, quote);
    let mut out = Vec::new();
    let mut push = |kind: &str, id: String| {
        out.push(CommentTarget {
            kind: kind.to_string(),
            id,
        })
    };
    for f in r.files {
        push(KIND_FILE, f);
    }
    for d in r.dirs {
        push(KIND_DIR, d);
    }
    for w in r.wikis {
        push(KIND_WIKI, w);
    }
    for t in r.tasks {
        push(KIND_WORK_ITEM, work_item_id(oxplow_domain::TaskId::new(t)));
    }
    for f in r.findings {
        push(KIND_FINDING, f);
    }
    for c in r.commits {
        push(KIND_COMMIT, c);
    }
    out
}

/// Union the frontend-supplied `referenced_refs` (DOM links inside the
/// selection) with the refs the backend extracts from `quote`,
/// deduplicating on `(kind,id)` and preserving the provided refs first.
/// Centralizing this in the create path means no surface ever has to
/// reimplement ref parsing.
fn union_referenced_refs(
    kinds: &oxplow_domain::refs::kind::KindRegistry,
    provided: &[CommentTarget],
    quote: &str,
) -> Vec<CommentTarget> {
    let mut out = provided.to_vec();
    for r in refs_from_quote(kinds, quote) {
        if !out.iter().any(|e| e.kind == r.kind && e.id == r.id) {
            out.push(r);
        }
    }
    out
}

fn row_to_comment(row: &rusqlite::Row<'_>) -> rusqlite::Result<Comment> {
    let intent: String = row.get("intent")?;
    let status: String = row.get("status")?;
    let thread_id: Option<i64> = row.get("thread_id")?;
    let created_at: String = row.get("created_at")?;
    let updated_at: String = row.get("updated_at")?;
    let last_activity_at: String = row.get("last_activity_at")?;
    let resolved_at: Option<String> = row.get("resolved_at")?;
    Ok(Comment {
        id: CommentId::new(row.get("id")?),
        stream_id: StreamId::new(row.get::<_, i64>("stream_id")?),
        thread_id: thread_id.map(ThreadId::new),
        target_kind: row.get("target_kind")?,
        target_id: row.get("target_id")?,
        quote: row.get("quote")?,
        selectors_json: row.get("selectors_json")?,
        context_chain: parse_refs(&row.get::<_, String>("context_chain_json")?),
        referenced_refs: parse_refs(&row.get::<_, String>("referenced_refs_json")?),
        intent: str_to_intent(&intent).map_err(map_err)?,
        status: str_to_status(&status).map_err(map_err)?,
        orphaned: row.get::<_, i64>("orphaned")? != 0,
        author: row.get("author")?,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
        updated_at: string_to_ts(&updated_at).map_err(map_err)?,
        last_activity_at: string_to_ts(&last_activity_at).map_err(map_err)?,
        resolved_at: resolved_at
            .map(|s| string_to_ts(&s).map_err(map_err))
            .transpose()?,
    })
}

fn row_to_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<CommentMessage> {
    let created_at: String = row.get("created_at")?;
    Ok(CommentMessage {
        id: CommentMessageId::new(row.get("id")?),
        comment_id: CommentId::new(row.get("comment_id")?),
        author: row.get("author")?,
        body: row.get("body")?,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
    })
}

/// Load the messages for one comment, oldest-first.
fn load_messages(
    conn: &rusqlite::Connection,
    comment_id: i64,
) -> rusqlite::Result<Vec<CommentMessage>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM comment_message WHERE comment_id = ?1 ORDER BY created_at ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![comment_id], row_to_message)?;
    rows.collect()
}

/// Hydrate a set of comment rows into full threads. `where_clause` is
/// spliced after `WHERE` and must reference bound params `?1..`.
fn list_threads(
    conn: &rusqlite::Connection,
    where_clause: &str,
    args: &[&dyn rusqlite::ToSql],
) -> rusqlite::Result<Vec<CommentThread>> {
    let sql = format!(
        "SELECT * FROM comment WHERE {where_clause} ORDER BY last_activity_at DESC, id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let comments = stmt
        .query_map(args, row_to_comment)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(comments.len());
    for comment in comments {
        let messages = load_messages(conn, comment.id.value())?;
        out.push(CommentThread { comment, messages });
    }
    Ok(out)
}

#[derive(Clone)]
pub struct SqliteCommentStore {
    db: Database,
    vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
}

/// What a comment's event is logged as.
const SOURCE: &str = "system:comments";

/// `knowledge.comment.written@1` / `deleted@1` for comment `id` on
/// `target_kind` / `target_id` (P7.B6: how the search index hears of it).
fn comment_event(id: i64, target_kind: &str, target_id: &str, deleted: bool) -> Envelope {
    use oxplow_domain::events::schema::{
        KnowledgeCommentDeleted, KnowledgeCommentDeletedV1, KnowledgeCommentWritten,
        KnowledgeCommentWrittenV1,
    };
    let comment = format!("comment:{}", CommentId::new(id));
    let env = if deleted {
        Envelope::typed::<KnowledgeCommentDeleted>(
            SOURCE,
            &KnowledgeCommentDeletedV1 {
                comment: comment.clone(),
                target_kind: target_kind.into(),
                target_id: target_id.into(),
            },
        )
    } else {
        Envelope::typed::<KnowledgeCommentWritten>(
            SOURCE,
            &KnowledgeCommentWrittenV1 {
                comment: comment.clone(),
                target_kind: target_kind.into(),
                target_id: target_id.into(),
            },
        )
    };
    env.with_subject([comment])
}

/// The deletion events for the comments `filter` (a `WHERE` clause over
/// `comment`) matches — read before they go.
fn deleted(
    conn: &rusqlite::Connection,
    filter: &str,
    args: impl rusqlite::Params,
) -> rusqlite::Result<Vec<Envelope>> {
    let mut st = conn.prepare(&format!(
        "SELECT id, target_kind, target_id FROM comment WHERE {filter}"
    ))?;
    let rows = st.query_map(args, |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    rows.map(|r| r.map(|(id, kind, tid)| comment_event(id, &kind, &tid, true)))
        .collect()
}

/// The event for a change to comment `id`, from its row.
fn written(conn: &rusqlite::Connection, id: i64) -> rusqlite::Result<Vec<Envelope>> {
    let target: Option<(String, String)> = conn
        .query_row(
            "SELECT target_kind, target_id FROM comment WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(target
        .map(|(kind, tid)| comment_event(id, &kind, &tid, false))
        .into_iter()
        .collect())
}

/// What a new comment is (`knowledge.add_comment`, P8.A6).
#[derive(Debug, Clone)]
pub struct NewComment {
    pub stream: StreamId,
    pub thread: Option<ThreadId>,
    pub target: CommentTarget,
    pub quote: String,
    pub selectors_json: String,
    pub context_chain: Vec<CommentTarget>,
    pub referenced_refs: Vec<CommentTarget>,
    pub intent: CommentIntent,
    pub author: String,
    pub body: String,
}

fn sql(e: rusqlite::Error) -> DomainError {
    crate::database::map_sql_err(e)
}

/// Comment `id` with its messages, on `conn`.
pub fn get_tx(
    conn: &rusqlite::Connection,
    id: CommentId,
) -> Result<Option<CommentThread>, DomainError> {
    Ok(list_threads(conn, "id = ?1", &[&id.value()])
        .map_err(sql)?
        .into_iter()
        .next())
}

/// Create a comment with its first message, on `conn` — a command's
/// transaction. The inline refs its quote mentions join the provided
/// ones, so typed context survives a surface that didn't link them.
/// Returns it and the `knowledge.comment.written` to log.
pub fn create_tx(
    conn: &rusqlite::Connection,
    kinds: &oxplow_domain::refs::kind::KindRegistry,
    new: &NewComment,
) -> Result<(CommentThread, Vec<Envelope>), DomainError> {
    let referenced_refs = union_referenced_refs(kinds, &new.referenced_refs, &new.quote);
    let now = Timestamp::now();
    let now_s = ts_to_string(now);
    conn.execute(
        "INSERT INTO comment
               (stream_id, thread_id, target_kind, target_id, quote, selectors_json,
                context_chain_json, referenced_refs_json,
                intent, status, orphaned, author, created_at, updated_at, last_activity_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'open', 0, ?10, ?11, ?11, ?11)",
        params![
            new.stream.value(),
            new.thread.as_ref().map(|t| t.value()),
            new.target.kind,
            new.target.id,
            new.quote,
            new.selectors_json,
            refs_to_json(&new.context_chain),
            refs_to_json(&referenced_refs),
            intent_to_str(new.intent),
            new.author,
            now_s,
        ],
    )
    .map_err(sql)?;
    let comment_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO comment_message (comment_id, author, body, created_at)
             VALUES (?1, ?2, ?3, ?4)",
        params![comment_id, new.author, new.body, now_s],
    )
    .map_err(sql)?;
    let comment = Comment {
        id: CommentId::new(comment_id),
        stream_id: new.stream,
        thread_id: new.thread,
        target_kind: new.target.kind.clone(),
        target_id: new.target.id.clone(),
        quote: new.quote.clone(),
        selectors_json: new.selectors_json.clone(),
        context_chain: new.context_chain.clone(),
        referenced_refs,
        intent: new.intent,
        status: CommentStatus::Open,
        orphaned: false,
        author: new.author.clone(),
        created_at: now,
        updated_at: now,
        last_activity_at: now,
        resolved_at: None,
    };
    let messages = load_messages(conn, comment_id).map_err(sql)?;
    let event = comment_event(comment_id, &new.target.kind, &new.target.id, false);
    Ok((CommentThread { comment, messages }, vec![event]))
}

/// Reply on comment `comment`; bumps its last activity.
pub fn add_message_tx(
    conn: &rusqlite::Connection,
    comment: CommentId,
    author: &str,
    body: &str,
) -> Result<(CommentMessage, Vec<Envelope>), DomainError> {
    let now = Timestamp::now();
    let now_s = ts_to_string(now);
    conn.execute(
        "INSERT INTO comment_message (comment_id, author, body, created_at)
             VALUES (?1, ?2, ?3, ?4)",
        params![comment.value(), author, body, now_s],
    )
    .map_err(sql)?;
    let message_id = conn.last_insert_rowid();
    conn.execute(
        "UPDATE comment SET updated_at = ?2, last_activity_at = ?2 WHERE id = ?1",
        params![comment.value(), now_s],
    )
    .map_err(sql)?;
    Ok((
        CommentMessage {
            id: CommentMessageId::new(message_id),
            comment_id: comment,
            author: author.to_string(),
            body: body.to_string(),
            created_at: now,
        },
        written(conn, comment.value()).map_err(sql)?,
    ))
}

pub fn set_intent_tx(
    conn: &rusqlite::Connection,
    id: CommentId,
    intent: CommentIntent,
) -> Result<Vec<Envelope>, DomainError> {
    conn.execute(
        "UPDATE comment SET intent = ?2, updated_at = ?3 WHERE id = ?1",
        params![
            id.value(),
            intent_to_str(intent),
            ts_to_string(Timestamp::now())
        ],
    )
    .map_err(sql)?;
    written(conn, id.value()).map_err(sql)
}

/// Stamps `resolved_at` on resolve and clears it on reopen, so the
/// dashboard can bucket resolved comments by date.
pub fn set_status_tx(
    conn: &rusqlite::Connection,
    id: CommentId,
    status: CommentStatus,
) -> Result<Vec<Envelope>, DomainError> {
    let now = ts_to_string(Timestamp::now());
    let resolved_at = match status {
        CommentStatus::Resolved => Some(now.clone()),
        CommentStatus::Open => None,
    };
    conn.execute(
        "UPDATE comment SET status = ?2, updated_at = ?3, resolved_at = ?4 WHERE id = ?1",
        params![id.value(), status_to_str(status), now, resolved_at],
    )
    .map_err(sql)?;
    written(conn, id.value()).map_err(sql)
}

/// Re-attach a comment to a newly selected span: its quote and anchor,
/// no longer orphaned.
pub fn relink_tx(
    conn: &rusqlite::Connection,
    id: CommentId,
    quote: &str,
    selectors_json: &str,
) -> Result<Vec<Envelope>, DomainError> {
    conn.execute(
        "UPDATE comment
             SET quote = ?2, selectors_json = ?3, orphaned = 0, updated_at = ?4
             WHERE id = ?1",
        params![
            id.value(),
            quote,
            selectors_json,
            ts_to_string(Timestamp::now())
        ],
    )
    .map_err(sql)?;
    written(conn, id.value()).map_err(sql)
}

/// Delete a comment; its messages cascade.
pub fn delete_tx(conn: &rusqlite::Connection, id: CommentId) -> Result<Vec<Envelope>, DomainError> {
    let gone = deleted(conn, "id = ?1", params![id.value()]).map_err(sql)?;
    conn.execute("DELETE FROM comment WHERE id = ?1", params![id.value()])
        .map_err(sql)?;
    Ok(gone)
}

impl SqliteCommentStore {
    pub fn new(db: Database, vocabulary: oxplow_domain::vocabulary::VocabularyHandle) -> Self {
        Self { db, vocabulary }
    }

    /// Run `f` and log the events it returns, in one transaction (a retry
    /// may run `f` again).
    async fn logged<R, F>(&self, f: F) -> Result<R, DomainError>
    where
        F: Fn(&rusqlite::Connection) -> rusqlite::Result<(R, Vec<Envelope>)> + Send + 'static,
        R: Send + 'static,
    {
        let vocabulary = self.vocabulary.clone();
        self.db
            .transaction(move |tx| {
                let (out, events) = f(tx).map_err(crate::database::map_sql_err)?;
                for e in &events {
                    crate::event_log_store::append_tx(tx, &vocabulary.current(), e)?;
                }
                Ok(out)
            })
            .await
    }
}

#[async_trait]
impl CommentStore for SqliteCommentStore {
    async fn get(&self, id: CommentId) -> Result<Option<CommentThread>, DomainError> {
        self.db
            .call(move |conn| {
                Ok(list_threads(conn, "id = ?1", &[&id.value()])?
                    .into_iter()
                    .next())
            })
            .await
    }

    async fn list_for_target(
        &self,
        target: &CommentTarget,
    ) -> Result<Vec<CommentThread>, DomainError> {
        let target = target.clone();
        self.db
            .call(move |conn| {
                list_threads(
                    conn,
                    "target_kind = ?1 AND target_id = ?2",
                    &[&target.kind, &target.id],
                )
            })
            .await
    }

    async fn list_for_stream(&self, stream: &StreamId) -> Result<Vec<CommentThread>, DomainError> {
        let stream = *stream;
        self.db
            .call(move |conn| list_threads(conn, "stream_id = ?1", &[&stream.value()]))
            .await
    }

    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<CommentThread>, DomainError> {
        let thread = *thread;
        self.db
            .call(move |conn| list_threads(conn, "thread_id = ?1", &[&thread.value()]))
            .await
    }

    async fn set_anchor(
        &self,
        id: CommentId,
        selectors_json: &str,
        orphaned: bool,
    ) -> Result<(), DomainError> {
        let selectors_json = selectors_json.to_string();
        self.logged(move |conn| {
            conn.execute(
                "UPDATE comment SET selectors_json = ?2, orphaned = ?3, updated_at = ?4
                     WHERE id = ?1",
                params![
                    id.value(),
                    selectors_json,
                    orphaned as i64,
                    ts_to_string(Timestamp::now())
                ],
            )?;
            Ok(((), written(conn, id.value())?))
        })
        .await
    }

    async fn cleanup(&self, retention_days: i64) -> Result<u64, DomainError> {
        if retention_days <= 0 {
            return Ok(0);
        }
        self.logged(move |conn| {
            let cutoff = ts_to_string(Timestamp::from_unix_ms(
                Timestamp::now().unix_ms() - retention_days * 86_400_000,
            ));
            const SWEPT: &str = "(status = 'resolved' OR orphaned = 1) AND last_activity_at < ?1";
            let gone = deleted(conn, SWEPT, params![cutoff])?;
            let n = conn.execute(
                &format!("DELETE FROM comment WHERE {SWEPT}"),
                params![cutoff],
            )?;
            Ok((n as u64, gone))
        })
        .await
    }
}

/// The write cores in their own transaction, their events logged — what
/// a command does, for this module's tests.
#[cfg(test)]
impl SqliteCommentStore {
    async fn run<R: Send + 'static>(
        &self,
        f: impl Fn(&rusqlite::Connection) -> Result<(R, Vec<Envelope>), DomainError> + Send + 'static,
    ) -> Result<R, DomainError> {
        let vocabulary = self.vocabulary.clone();
        self.db
            .transaction(move |tx| {
                let (out, events) = f(tx)?;
                for e in &events {
                    crate::event_log_store::append_tx(tx, &vocabulary.current(), e)?;
                }
                Ok(out)
            })
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn create(
        &self,
        stream: &StreamId,
        thread: Option<&ThreadId>,
        target: &CommentTarget,
        quote: &str,
        selectors_json: &str,
        context_chain: &[CommentTarget],
        referenced_refs: &[CommentTarget],
        intent: CommentIntent,
        author: &str,
        body: &str,
    ) -> Result<CommentThread, DomainError> {
        let new = NewComment {
            stream: *stream,
            thread: thread.copied(),
            target: target.clone(),
            quote: quote.into(),
            selectors_json: selectors_json.into(),
            context_chain: context_chain.to_vec(),
            referenced_refs: referenced_refs.to_vec(),
            intent,
            author: author.into(),
            body: body.into(),
        };
        let vocabulary = self.vocabulary.current();
        self.run(move |c| create_tx(c, &vocabulary.kinds, &new))
            .await
    }

    async fn add_message(
        &self,
        comment: CommentId,
        author: &str,
        body: &str,
    ) -> Result<CommentMessage, DomainError> {
        let (author, body) = (author.to_string(), body.to_string());
        self.run(move |c| add_message_tx(c, comment, &author, &body))
            .await
    }

    async fn set_status(&self, id: CommentId, status: CommentStatus) -> Result<(), DomainError> {
        self.run(move |c| Ok(((), set_status_tx(c, id, status)?)))
            .await
    }

    async fn relink(
        &self,
        id: CommentId,
        quote: &str,
        selectors_json: &str,
    ) -> Result<(), DomainError> {
        let (quote, selectors) = (quote.to_string(), selectors_json.to_string());
        self.run(move |c| Ok(((), relink_tx(c, id, &quote, &selectors)?)))
            .await
    }

    async fn delete(&self, id: CommentId) -> Result<(), DomainError> {
        self.run(move |c| Ok(((), delete_tx(c, id)?))).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_store::SqliteStreamStore;
    use crate::thread_store::SqliteThreadStore;
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{Stream, StreamKind, Thread, ThreadStatus};

    fn now() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    async fn fixture() -> (Database, StreamId, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());

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
        (db, s.id, t.id)
    }

    fn target() -> CommentTarget {
        CommentTarget {
            kind: "wiki".into(),
            id: "some-page".into(),
        }
    }

    #[tokio::test]
    async fn create_round_trips_with_first_message() {
        let (db, stream, thread) = fixture().await;
        let store =
            SqliteCommentStore::new(db, oxplow_domain::vocabulary::VocabularyHandle::core());
        let created = store
            .create(
                &stream,
                Some(&thread),
                &target(),
                "the selected words",
                "{\"from\":1,\"to\":5}",
                &[],
                &[],
                CommentIntent::Followup,
                "user",
                "what about this?",
            )
            .await
            .unwrap();
        assert_eq!(created.comment.stream_id, stream);
        assert_eq!(created.comment.thread_id.as_ref(), Some(&thread));
        assert_eq!(created.comment.intent, CommentIntent::Followup);
        assert_eq!(created.comment.status, CommentStatus::Open);
        assert!(!created.comment.orphaned);
        assert_eq!(created.messages.len(), 1);
        assert_eq!(created.messages[0].body, "what about this?");

        let listed = store.list_for_target(&target()).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].comment.id, created.comment.id);
    }

    #[tokio::test]
    async fn create_round_trips_context_chain_and_referenced_refs() {
        let (db, stream, thread) = fixture().await;
        let store =
            SqliteCommentStore::new(db, oxplow_domain::vocabulary::VocabularyHandle::core());
        let context_chain = vec![
            CommentTarget {
                kind: "commit".into(),
                id: "abc1234".into(),
            },
            CommentTarget {
                kind: "page".into(),
                id: "git-dashboard".into(),
            },
        ];
        let referenced_refs = vec![CommentTarget {
            kind: "file".into(),
            id: "src/app.rs".into(),
        }];
        let created = store
            .create(
                &stream,
                Some(&thread),
                &CommentTarget {
                    kind: "file".into(),
                    id: "src/app.rs".into(),
                },
                "fn main",
                "[{\"type\":\"TextQuoteSelector\",\"exact\":\"fn main\"}]",
                &context_chain,
                &referenced_refs,
                CommentIntent::Followup,
                "user",
                "why is this here?",
            )
            .await
            .unwrap();
        // The create return value carries the typed context…
        assert_eq!(created.comment.context_chain, context_chain);
        assert_eq!(created.comment.referenced_refs, referenced_refs);
        // …and so does a fresh load from the DB (proves the JSON columns
        // serialize + parse round-trip).
        let loaded = store.get(created.comment.id).await.unwrap().unwrap();
        assert_eq!(loaded.comment.context_chain, context_chain);
        assert_eq!(loaded.comment.referenced_refs, referenced_refs);
        assert_eq!(
            loaded.comment.selectors_json,
            "[{\"type\":\"TextQuoteSelector\",\"exact\":\"fn main\"}]"
        );
    }

    #[tokio::test]
    async fn create_unions_refs_extracted_from_quote_into_referenced_refs() {
        let (db, stream, thread) = fixture().await;
        let store =
            SqliteCommentStore::new(db, oxplow_domain::vocabulary::VocabularyHandle::core());
        // The frontend captured one DOM-link ref; the quote *also* names
        // `tsk42` and `src/app.rs` inline (not rendered as links).
        let provided = vec![CommentTarget {
            kind: "wiki".into(),
            id: "architecture".into(),
        }];
        let created = store
            .create(
                &stream,
                Some(&thread),
                &target(),
                "see tsk42 and src/app.rs for context",
                "[]",
                &[],
                &provided,
                CommentIntent::Followup,
                "user",
                "body",
            )
            .await
            .unwrap();
        let refs = &created.comment.referenced_refs;
        // FE-provided ref is preserved…
        assert!(
            refs.iter()
                .any(|r| r.kind == "wiki" && r.id == "architecture"),
            "provided ref dropped: {refs:?}",
        );
        // …and the quote's inline mentions are unioned in as typed refs.
        assert!(
            refs.iter()
                .any(|r| r.kind == "work_item" && r.id == "oxplow:tsk42"),
            "task ref not extracted: {refs:?}",
        );
        assert!(
            refs.iter()
                .any(|r| r.kind == "file" && r.id == "src/app.rs"),
            "file ref not extracted: {refs:?}",
        );
        // And it round-trips from the DB, not just the create return value.
        let loaded = store.get(created.comment.id).await.unwrap().unwrap();
        assert_eq!(&loaded.comment.referenced_refs, refs);
    }

    #[tokio::test]
    async fn create_dedups_quote_refs_against_provided() {
        let (db, stream, thread) = fixture().await;
        let store =
            SqliteCommentStore::new(db, oxplow_domain::vocabulary::VocabularyHandle::core());
        // FE already supplied tsk42; the quote names it again. It must
        // appear exactly once.
        let provided = vec![CommentTarget {
            kind: "work_item".into(),
            id: "oxplow:tsk42".into(),
        }];
        let created = store
            .create(
                &stream,
                Some(&thread),
                &target(),
                "tsk42 again",
                "[]",
                &[],
                &provided,
                CommentIntent::Note,
                "user",
                "body",
            )
            .await
            .unwrap();
        let n = created
            .comment
            .referenced_refs
            .iter()
            .filter(|r| r.kind == "work_item" && r.id == "oxplow:tsk42")
            .count();
        assert_eq!(
            n, 1,
            "tsk42 duplicated: {:?}",
            created.comment.referenced_refs
        );
    }

    #[tokio::test]
    async fn resolved_at_set_on_resolve_cleared_on_reopen() {
        let (db, stream, thread) = fixture().await;
        let store =
            SqliteCommentStore::new(db, oxplow_domain::vocabulary::VocabularyHandle::core());
        let c = store
            .create(
                &stream,
                Some(&thread),
                &target(),
                "words",
                "{\"from\":1,\"to\":3}",
                &[],
                &[],
                CommentIntent::Note,
                "user",
                "hmm",
            )
            .await
            .unwrap();
        // Open comments carry no resolved_at.
        assert!(c.comment.resolved_at.is_none());

        store
            .set_status(c.comment.id, CommentStatus::Resolved)
            .await
            .unwrap();
        let resolved = store.get(c.comment.id).await.unwrap().unwrap();
        assert!(
            resolved.comment.resolved_at.is_some(),
            "resolving should stamp resolved_at",
        );

        // Reopening clears it again.
        store
            .set_status(c.comment.id, CommentStatus::Open)
            .await
            .unwrap();
        let reopened = store.get(c.comment.id).await.unwrap().unwrap();
        assert!(
            reopened.comment.resolved_at.is_none(),
            "reopening should clear resolved_at",
        );
    }

    #[tokio::test]
    async fn relink_rewrites_quote_and_clears_orphan() {
        let (db, stream, thread) = fixture().await;
        let store =
            SqliteCommentStore::new(db, oxplow_domain::vocabulary::VocabularyHandle::core());
        let c = store
            .create(
                &stream,
                Some(&thread),
                &target(),
                "old words",
                "{\"from\":1,\"to\":9}",
                &[],
                &[],
                CommentIntent::Note,
                "user",
                "note",
            )
            .await
            .unwrap();
        // Mark it orphaned (quote vanished).
        store
            .set_anchor(c.comment.id, c.comment.selectors_json.as_str(), true)
            .await
            .unwrap();
        assert!(
            store
                .get(c.comment.id)
                .await
                .unwrap()
                .unwrap()
                .comment
                .orphaned
        );

        store
            .relink(c.comment.id, "new words", "{\"from\":20,\"to\":29}")
            .await
            .unwrap();
        let after = store.get(c.comment.id).await.unwrap().unwrap().comment;
        assert_eq!(after.quote, "new words");
        assert_eq!(after.selectors_json, "{\"from\":20,\"to\":29}");
        assert!(!after.orphaned);
    }

    #[tokio::test]
    async fn thread_grows_and_orders_oldest_first() {
        let (db, stream, thread) = fixture().await;
        let store =
            SqliteCommentStore::new(db, oxplow_domain::vocabulary::VocabularyHandle::core());
        let c = store
            .create(
                &stream,
                Some(&thread),
                &target(),
                "q",
                "{}",
                &[],
                &[],
                CommentIntent::Followup,
                "user",
                "first",
            )
            .await
            .unwrap();
        store
            .add_message(c.comment.id, "agent", "second")
            .await
            .unwrap();
        store
            .add_message(c.comment.id, "user", "third")
            .await
            .unwrap();
        let got = store.get(c.comment.id).await.unwrap().unwrap();
        let bodies: Vec<_> = got.messages.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(bodies, vec!["first", "second", "third"]);
    }

    #[tokio::test]
    async fn needs_response_tracks_authorship() {
        let (db, stream, thread) = fixture().await;
        let store =
            SqliteCommentStore::new(db, oxplow_domain::vocabulary::VocabularyHandle::core());
        let c = store
            .create(
                &stream,
                Some(&thread),
                &target(),
                "q",
                "{}",
                &[],
                &[],
                CommentIntent::Followup,
                "user",
                "please look",
            )
            .await
            .unwrap();
        // Fresh follow-up with only a user message → needs response.
        assert!(store
            .get(c.comment.id)
            .await
            .unwrap()
            .unwrap()
            .needs_response());
        // Agent replies → answered.
        store
            .add_message(c.comment.id, "agent", "done")
            .await
            .unwrap();
        assert!(!store
            .get(c.comment.id)
            .await
            .unwrap()
            .unwrap()
            .needs_response());
        // User follows up again → needs response once more.
        store
            .add_message(c.comment.id, "user", "one more thing")
            .await
            .unwrap();
        assert!(store
            .get(c.comment.id)
            .await
            .unwrap()
            .unwrap()
            .needs_response());
        // Resolving clears it regardless of authorship.
        store
            .set_status(c.comment.id, CommentStatus::Resolved)
            .await
            .unwrap();
        assert!(!store
            .get(c.comment.id)
            .await
            .unwrap()
            .unwrap()
            .needs_response());
    }

    #[tokio::test]
    async fn note_intent_never_needs_response() {
        let (db, stream, thread) = fixture().await;
        let store =
            SqliteCommentStore::new(db, oxplow_domain::vocabulary::VocabularyHandle::core());
        let c = store
            .create(
                &stream,
                Some(&thread),
                &target(),
                "q",
                "{}",
                &[],
                &[],
                CommentIntent::Note,
                "user",
                "just thinking out loud",
            )
            .await
            .unwrap();
        assert!(!store
            .get(c.comment.id)
            .await
            .unwrap()
            .unwrap()
            .needs_response());
    }

    #[tokio::test]
    async fn list_for_stream_and_thread() {
        let (db, stream, thread) = fixture().await;
        let store =
            SqliteCommentStore::new(db, oxplow_domain::vocabulary::VocabularyHandle::core());
        store
            .create(
                &stream,
                Some(&thread),
                &target(),
                "q",
                "{}",
                &[],
                &[],
                CommentIntent::Note,
                "user",
                "a",
            )
            .await
            .unwrap();
        store
            .create(
                &stream,
                Some(&thread),
                &CommentTarget {
                    kind: "file".into(),
                    id: "src/x.rs".into(),
                },
                "q2",
                "{}",
                &[],
                &[],
                CommentIntent::Followup,
                "user",
                "b",
            )
            .await
            .unwrap();
        assert_eq!(store.list_for_stream(&stream).await.unwrap().len(), 2);
        assert_eq!(store.list_for_thread(&thread).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn set_anchor_marks_orphaned() {
        let (db, stream, thread) = fixture().await;
        let store =
            SqliteCommentStore::new(db, oxplow_domain::vocabulary::VocabularyHandle::core());
        let c = store
            .create(
                &stream,
                Some(&thread),
                &target(),
                "q",
                "{}",
                &[],
                &[],
                CommentIntent::Note,
                "user",
                "a",
            )
            .await
            .unwrap();
        store
            .set_anchor(c.comment.id, "{\"from\":9,\"to\":9}", true)
            .await
            .unwrap();
        let got = store.get(c.comment.id).await.unwrap().unwrap();
        assert!(got.comment.orphaned);
        assert_eq!(got.comment.selectors_json, "{\"from\":9,\"to\":9}");
    }

    #[tokio::test]
    async fn delete_cascades_messages() {
        let (db, stream, thread) = fixture().await;
        let store = SqliteCommentStore::new(
            db.clone(),
            oxplow_domain::vocabulary::VocabularyHandle::core(),
        );
        let c = store
            .create(
                &stream,
                Some(&thread),
                &target(),
                "q",
                "{}",
                &[],
                &[],
                CommentIntent::Note,
                "user",
                "a",
            )
            .await
            .unwrap();
        store.add_message(c.comment.id, "agent", "b").await.unwrap();
        store.delete(c.comment.id).await.unwrap();
        assert!(store.get(c.comment.id).await.unwrap().is_none());
        let remaining: i64 = db
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) FROM comment_message", [], |r| r.get(0))
            })
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[tokio::test]
    async fn cleanup_sweeps_resolved_and_orphaned_past_cutoff() {
        let (db, stream, thread) = fixture().await;
        let store = SqliteCommentStore::new(
            db.clone(),
            oxplow_domain::vocabulary::VocabularyHandle::core(),
        );
        // An open comment must survive cleanup.
        store
            .create(
                &stream,
                Some(&thread),
                &target(),
                "keep",
                "{}",
                &[],
                &[],
                CommentIntent::Note,
                "user",
                "a",
            )
            .await
            .unwrap();
        let resolved = store
            .create(
                &stream,
                Some(&thread),
                &target(),
                "old",
                "{}",
                &[],
                &[],
                CommentIntent::Note,
                "user",
                "b",
            )
            .await
            .unwrap();
        store
            .set_status(resolved.comment.id, CommentStatus::Resolved)
            .await
            .unwrap();
        // Force its last_activity_at far into the past.
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE comment SET last_activity_at = '2000-01-01T00:00:00Z' WHERE id = ?1",
                params![resolved.comment.id.value()],
            )?;
            Ok(())
        })
        .unwrap();

        let deleted = store.cleanup(14).await.unwrap();
        assert_eq!(deleted, 1);
        assert!(store.get(resolved.comment.id).await.unwrap().is_none());
        assert_eq!(store.list_for_stream(&stream).await.unwrap().len(), 1);

        // retention_days = 0 disables pruning.
        assert_eq!(store.cleanup(0).await.unwrap(), 0);
    }
}
