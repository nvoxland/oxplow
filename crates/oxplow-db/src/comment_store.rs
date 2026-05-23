//! `SqliteCommentStore` — threaded annotations anchored to a text
//! selection. Mirrors the `task_satellite.rs` plumbing
//! (`spawn_blocking` + `with_conn`, ISO timestamp helpers) and uses
//! plain autoincrement integer ids.

use async_trait::async_trait;
use rusqlite::params;

use oxplow_domain::stores::CommentStore;
use oxplow_domain::{
    Comment, CommentId, CommentIntent, CommentMessage, CommentMessageId, CommentStatus,
    CommentTarget, CommentThread, DomainError, StreamId, ThreadId, Timestamp,
};

use crate::database::Database;

fn ts_to_string(ts: Timestamp) -> String {
    serde_json::to_string(&ts)
        .unwrap()
        .trim_matches('"')
        .to_string()
}

fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DomainError::Invalid(format!("bad timestamp: {e}")))
}

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

fn row_to_comment(row: &rusqlite::Row<'_>) -> rusqlite::Result<Comment> {
    let intent: String = row.get("intent")?;
    let status: String = row.get("status")?;
    let thread_id: Option<String> = row.get("thread_id")?;
    let created_at: String = row.get("created_at")?;
    let updated_at: String = row.get("updated_at")?;
    let last_activity_at: String = row.get("last_activity_at")?;
    let resolved_at: Option<String> = row.get("resolved_at")?;
    Ok(Comment {
        id: CommentId::new(row.get("id")?),
        stream_id: StreamId::from(row.get::<_, String>("stream_id")?),
        thread_id: thread_id.map(ThreadId::from),
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
}

impl SqliteCommentStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl CommentStore for SqliteCommentStore {
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
        let db = self.db.clone();
        let stream = stream.clone();
        let thread = thread.cloned();
        let target = target.clone();
        let quote = quote.to_string();
        let selectors_json = selectors_json.to_string();
        let context_chain = context_chain.to_vec();
        let referenced_refs = referenced_refs.to_vec();
        let author = author.to_string();
        let body = body.to_string();
        tokio::task::spawn_blocking(move || -> Result<CommentThread, DomainError> {
            let now = Timestamp::now();
            let now_s = ts_to_string(now);
            let context_chain_json = refs_to_json(&context_chain);
            let referenced_refs_json = refs_to_json(&referenced_refs);
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO comment
                       (stream_id, thread_id, target_kind, target_id, quote, selectors_json,
                        context_chain_json, referenced_refs_json,
                        intent, status, orphaned, author, created_at, updated_at, last_activity_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'open', 0, ?10, ?11, ?11, ?11)",
                    params![
                        stream.as_str(),
                        thread.as_ref().map(|t| t.as_str()),
                        target.kind,
                        target.id,
                        quote,
                        selectors_json,
                        context_chain_json,
                        referenced_refs_json,
                        intent_to_str(intent),
                        author,
                        now_s,
                    ],
                )?;
                let comment_id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO comment_message (comment_id, author, body, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![comment_id, author, body, now_s],
                )?;
                let comment = Comment {
                    id: CommentId::new(comment_id),
                    stream_id: stream.clone(),
                    thread_id: thread.clone(),
                    target_kind: target.kind.clone(),
                    target_id: target.id.clone(),
                    quote: quote.clone(),
                    selectors_json: selectors_json.clone(),
                    context_chain: context_chain.clone(),
                    referenced_refs: referenced_refs.clone(),
                    intent,
                    status: CommentStatus::Open,
                    orphaned: false,
                    author: author.clone(),
                    created_at: now,
                    updated_at: now,
                    last_activity_at: now,
                    resolved_at: None,
                };
                let messages = load_messages(conn, comment_id)?;
                Ok(CommentThread { comment, messages })
            })
        })
        .await
        .unwrap()
    }

    async fn add_message(
        &self,
        comment: CommentId,
        author: &str,
        body: &str,
    ) -> Result<CommentMessage, DomainError> {
        let db = self.db.clone();
        let author = author.to_string();
        let body = body.to_string();
        tokio::task::spawn_blocking(move || -> Result<CommentMessage, DomainError> {
            let now = Timestamp::now();
            let now_s = ts_to_string(now);
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO comment_message (comment_id, author, body, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![comment.value(), author, body, now_s],
                )?;
                let message_id = conn.last_insert_rowid();
                conn.execute(
                    "UPDATE comment SET updated_at = ?2, last_activity_at = ?2 WHERE id = ?1",
                    params![comment.value(), now_s],
                )?;
                Ok(CommentMessage {
                    id: CommentMessageId::new(message_id),
                    comment_id: comment,
                    author: author.clone(),
                    body: body.clone(),
                    created_at: now,
                })
            })
        })
        .await
        .unwrap()
    }

    async fn get(&self, id: CommentId) -> Result<Option<CommentThread>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                Ok(list_threads(conn, "id = ?1", &[&id.value()])?
                    .into_iter()
                    .next())
            })
        })
        .await
        .unwrap()
    }

    async fn list_for_target(
        &self,
        target: &CommentTarget,
    ) -> Result<Vec<CommentThread>, DomainError> {
        let db = self.db.clone();
        let target = target.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                list_threads(
                    conn,
                    "target_kind = ?1 AND target_id = ?2",
                    &[&target.kind, &target.id],
                )
            })
        })
        .await
        .unwrap()
    }

    async fn list_for_stream(&self, stream: &StreamId) -> Result<Vec<CommentThread>, DomainError> {
        let db = self.db.clone();
        let stream = stream.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| list_threads(conn, "stream_id = ?1", &[&stream.as_str()]))
        })
        .await
        .unwrap()
    }

    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<CommentThread>, DomainError> {
        let db = self.db.clone();
        let thread = thread.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| list_threads(conn, "thread_id = ?1", &[&thread.as_str()]))
        })
        .await
        .unwrap()
    }

    async fn set_intent(&self, id: CommentId, intent: CommentIntent) -> Result<(), DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "UPDATE comment SET intent = ?2, updated_at = ?3 WHERE id = ?1",
                    params![
                        id.value(),
                        intent_to_str(intent),
                        ts_to_string(Timestamp::now())
                    ],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    async fn set_status(&self, id: CommentId, status: CommentStatus) -> Result<(), DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                // Stamp resolved_at on →resolved, clear it on →open, so
                // the dashboard can bucket resolved comments by date.
                let now = ts_to_string(Timestamp::now());
                let resolved_at = match status {
                    CommentStatus::Resolved => Some(now.clone()),
                    CommentStatus::Open => None,
                };
                conn.execute(
                    "UPDATE comment SET status = ?2, updated_at = ?3, resolved_at = ?4 WHERE id = ?1",
                    params![id.value(), status_to_str(status), now, resolved_at],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    async fn set_anchor(
        &self,
        id: CommentId,
        selectors_json: &str,
        orphaned: bool,
    ) -> Result<(), DomainError> {
        let db = self.db.clone();
        let selectors_json = selectors_json.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
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
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    async fn relink(
        &self,
        id: CommentId,
        quote: &str,
        selectors_json: &str,
    ) -> Result<(), DomainError> {
        let db = self.db.clone();
        let quote = quote.to_string();
        let selectors_json = selectors_json.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
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
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    async fn delete(&self, id: CommentId) -> Result<(), DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                // comment_message rows cascade via FK.
                conn.execute("DELETE FROM comment WHERE id = ?1", params![id.value()])?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    async fn cleanup(&self, retention_days: i64) -> Result<u64, DomainError> {
        if retention_days <= 0 {
            return Ok(0);
        }
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let cutoff = ts_to_string(Timestamp::from_unix_ms(
                Timestamp::now().unix_ms() - retention_days * 86_400_000,
            ));
            db.with_conn(|conn| {
                let n = conn.execute(
                    "DELETE FROM comment
                     WHERE (status = 'resolved' OR orphaned = 1)
                       AND last_activity_at < ?1",
                    params![cutoff],
                )?;
                Ok(n as u64)
            })
        })
        .await
        .unwrap()
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
            id: StreamId::from("s-1"),
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
            id: ThreadId::from("b-1"),
            stream_id: s.id.clone(),
            title: "t".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
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
        let store = SqliteCommentStore::new(db);
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
        let store = SqliteCommentStore::new(db);
        let context_chain = vec![
            CommentTarget {
                kind: "git-commit".into(),
                id: "abc1234".into(),
            },
            CommentTarget {
                kind: "git-dashboard".into(),
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
    async fn resolved_at_set_on_resolve_cleared_on_reopen() {
        let (db, stream, thread) = fixture().await;
        let store = SqliteCommentStore::new(db);
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
        let store = SqliteCommentStore::new(db);
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
        let store = SqliteCommentStore::new(db);
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
        let store = SqliteCommentStore::new(db);
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
        let store = SqliteCommentStore::new(db);
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
        let store = SqliteCommentStore::new(db);
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
        let store = SqliteCommentStore::new(db);
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
        let store = SqliteCommentStore::new(db.clone());
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
        let store = SqliteCommentStore::new(db.clone());
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
