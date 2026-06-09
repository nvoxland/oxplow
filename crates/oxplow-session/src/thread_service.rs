//! Thread lifecycle service.
//!
//! Encodes the thread state machine: at most one Active thread per
//! stream (writer), zero or more Queued threads (read-only), and any
//! number of Closed threads (history). Transitions:
//!
//!   create        -> Queued (or Active if no other writer)
//!   promote       -> Queued -> Active (demoting any current Active to Queued)
//!   close         -> * -> Closed (sets closed_at)
//!   reopen        -> Closed -> Queued
//!   rename        -> title edit only
//!   set_prompt    -> custom_prompt edit
//!   reorder_queue -> rewrites sort_index for the queued bucket
//!   delete        -> hard delete (history-loss; UI normally closes)

use std::sync::Arc;

use thiserror::Error;
use tracing::info;

use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{AgentKind, DomainError, StreamId, Thread, ThreadId, ThreadStatus, Timestamp};

#[derive(Debug, Error)]
pub enum ThreadError {
    #[error("thread not found: {0}")]
    NotFound(ThreadId),
    #[error("thread is closed; reopen before mutating: {0}")]
    Closed(ThreadId),
    #[error("storage: {0}")]
    Storage(#[from] DomainError),
}

/// Cheap to clone — internal store is `Arc`.
#[derive(Clone)]
pub struct ThreadService {
    threads: Arc<dyn ThreadStore>,
}

impl ThreadService {
    pub fn new(threads: Arc<dyn ThreadStore>) -> Self {
        Self { threads }
    }

    /// Create a new thread on `stream`. If there is no current writer
    /// (active) thread on that stream, this thread becomes Active;
    /// otherwise it lands in the Queued bucket at the end.
    pub async fn create(
        &self,
        stream: &StreamId,
        title: impl Into<String>,
        pane_target: impl Into<String>,
        agent: AgentKind,
    ) -> Result<Thread, ThreadError> {
        let existing = self.threads.list_for_stream(stream).await?;
        let any_active = existing.iter().any(|t| t.status == ThreadStatus::Active);
        let next_sort = existing
            .iter()
            .filter(|t| t.status != ThreadStatus::Closed)
            .map(|t| t.sort_index)
            .max()
            .unwrap_or(-1)
            + 1;
        let now = Timestamp::now();
        let mut thread = Thread {
            id: ThreadId::placeholder(),
            stream_id: *stream,
            title: title.into(),
            status: if any_active {
                ThreadStatus::Queued
            } else {
                ThreadStatus::Active
            },
            sort_index: next_sort,
            pane_target: pane_target.into(),
            agent,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        thread.id = self.threads.upsert(&thread).await?;
        info!(thread_id = %thread.id, stream_id = %stream, "thread created");
        Ok(thread)
    }

    pub async fn rename(
        &self,
        id: &ThreadId,
        title: impl Into<String>,
    ) -> Result<Thread, ThreadError> {
        let mut t = self.load(id).await?;
        t.title = title.into();
        t.updated_at = Timestamp::now();
        self.threads.upsert(&t).await?;
        Ok(t)
    }

    pub async fn set_prompt(
        &self,
        id: &ThreadId,
        prompt: Option<String>,
    ) -> Result<Thread, ThreadError> {
        let mut t = self.load(id).await?;
        t.custom_prompt = prompt.filter(|s| !s.is_empty());
        t.updated_at = Timestamp::now();
        self.threads.upsert(&t).await?;
        Ok(t)
    }

    /// Promote a queued thread to active. Demotes any existing active
    /// thread on the same stream to queued first so the partial unique
    /// index never trips.
    pub async fn promote(&self, id: &ThreadId) -> Result<Thread, ThreadError> {
        let mut t = self.load(id).await?;
        if t.status == ThreadStatus::Closed {
            return Err(ThreadError::Closed(*id));
        }
        if t.status == ThreadStatus::Active {
            return Ok(t); // idempotent
        }
        // Demote whoever is currently active on this stream.
        let siblings = self.threads.list_for_stream(&t.stream_id).await?;
        for mut s in siblings
            .into_iter()
            .filter(|s| s.status == ThreadStatus::Active && s.id != t.id)
        {
            s.status = ThreadStatus::Queued;
            s.updated_at = Timestamp::now();
            self.threads.upsert(&s).await?;
        }
        t.status = ThreadStatus::Active;
        t.updated_at = Timestamp::now();
        self.threads.upsert(&t).await?;
        Ok(t)
    }

    pub async fn close(&self, id: &ThreadId) -> Result<Thread, ThreadError> {
        let mut t = self.load(id).await?;
        if t.status == ThreadStatus::Closed {
            return Ok(t);
        }
        let now = Timestamp::now();
        t.status = ThreadStatus::Closed;
        t.closed_at = Some(now);
        t.updated_at = now;
        self.threads.upsert(&t).await?;
        Ok(t)
    }

    pub async fn reopen(&self, id: &ThreadId) -> Result<Thread, ThreadError> {
        let mut t = self.load(id).await?;
        if t.status != ThreadStatus::Closed {
            return Ok(t);
        }
        t.status = ThreadStatus::Queued;
        t.closed_at = None;
        t.updated_at = Timestamp::now();
        self.threads.upsert(&t).await?;
        Ok(t)
    }

    /// Rewrite sort_index across queued threads for `stream`. Caller
    /// passes the desired ordering; closed threads keep their existing
    /// sort_index. Active is moved to position 0 if it appears in the
    /// list (UI shows it pinned at the top of the queue).
    pub async fn reorder_queue(
        &self,
        stream: &StreamId,
        order: &[ThreadId],
    ) -> Result<(), ThreadError> {
        let now = Timestamp::now();
        for (idx, id) in order.iter().enumerate() {
            let mut t = self.load(id).await?;
            if t.stream_id != *stream {
                continue;
            }
            t.sort_index = idx as i64;
            t.updated_at = now;
            self.threads.upsert(&t).await?;
        }
        Ok(())
    }

    pub async fn delete(&self, id: &ThreadId) -> Result<(), ThreadError> {
        self.threads.delete(id).await?;
        Ok(())
    }

    pub async fn list_for_stream(&self, stream: &StreamId) -> Result<Vec<Thread>, ThreadError> {
        Ok(self.threads.list_for_stream(stream).await?)
    }

    pub async fn list_closed(&self, stream: &StreamId) -> Result<Vec<Thread>, ThreadError> {
        let mut all = self.threads.list_for_stream(stream).await?;
        all.retain(|t| t.status == ThreadStatus::Closed);
        // Closed threads sorted most-recently-closed first.
        all.sort_by_key(|t| std::cmp::Reverse(t.closed_at));
        Ok(all)
    }

    pub async fn selected(&self, stream: &StreamId) -> Result<Option<ThreadId>, ThreadError> {
        Ok(self.threads.selected_for_stream(stream).await?)
    }

    /// Resolve the thread-id the agent should run under. Falls back
    /// through three layers:
    ///   1. The user's explicit selection (`selected_for_stream`).
    ///   2. The stream's writer (active) thread.
    ///   3. The first non-closed thread on the stream (queued / reader).
    ///
    /// Returns `None` only when the stream has no usable threads at all,
    /// which should not happen for a stream that boot-time seeded the
    /// "Default" thread.
    pub async fn selected_or_active(
        &self,
        stream: &StreamId,
    ) -> Result<Option<ThreadId>, ThreadError> {
        if let Some(id) = self.threads.selected_for_stream(stream).await? {
            return Ok(Some(id));
        }
        let mut all = self.threads.list_for_stream(stream).await?;
        // Prefer the writer (active) thread; fall back to the first
        // queued thread (sorted by ascending sort_index, which list_for_stream returns).
        all.sort_by_key(|t| (t.status != ThreadStatus::Active, t.sort_index));
        Ok(all.into_iter().next().map(|t| t.id))
    }

    pub async fn select(
        &self,
        stream: &StreamId,
        thread: Option<&ThreadId>,
    ) -> Result<(), ThreadError> {
        self.threads.set_selected_for_stream(stream, thread).await?;
        Ok(())
    }

    async fn load(&self, id: &ThreadId) -> Result<Thread, ThreadError> {
        self.threads
            .get(id)
            .await?
            .ok_or(ThreadError::NotFound(*id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteStreamStore, SqliteThreadStore};
    use oxplow_domain::stores::StreamStore;
    use oxplow_domain::{Stream, StreamKind};

    async fn fixture() -> (ThreadService, StreamId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
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
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        let svc = ThreadService::new(Arc::new(SqliteThreadStore::new(db)));
        (svc, s.id)
    }

    #[tokio::test]
    async fn selected_or_active_returns_writer_when_no_explicit_selection() {
        let (svc, sid) = fixture().await;
        let a = svc
            .create(&sid, "a", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let _b = svc
            .create(&sid, "b", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        // No `select()` call — fall-back path should return the active writer (`a`).
        assert_eq!(svc.selected(&sid).await.unwrap(), None);
        let id = svc.selected_or_active(&sid).await.unwrap();
        assert_eq!(id, Some(a.id));
    }

    #[tokio::test]
    async fn selected_or_active_prefers_explicit_selection() {
        let (svc, sid) = fixture().await;
        let _a = svc
            .create(&sid, "a", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let b = svc
            .create(&sid, "b", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        svc.select(&sid, Some(&b.id)).await.unwrap();
        let id = svc.selected_or_active(&sid).await.unwrap();
        assert_eq!(id, Some(b.id));
    }

    #[tokio::test]
    async fn selected_or_active_returns_none_when_stream_has_no_threads() {
        let (svc, sid) = fixture().await;
        assert_eq!(svc.selected_or_active(&sid).await.unwrap(), None);
    }

    #[tokio::test]
    async fn first_thread_is_active() {
        let (svc, sid) = fixture().await;
        let t = svc
            .create(&sid, "first", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        assert_eq!(t.status, ThreadStatus::Active);
    }

    #[tokio::test]
    async fn second_thread_is_queued() {
        let (svc, sid) = fixture().await;
        svc.create(&sid, "first", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let b = svc
            .create(&sid, "second", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        assert_eq!(b.status, ThreadStatus::Queued);
    }

    #[tokio::test]
    async fn promote_demotes_existing_active() {
        let (svc, sid) = fixture().await;
        let a = svc
            .create(&sid, "a", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let b = svc
            .create(&sid, "b", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let promoted = svc.promote(&b.id).await.unwrap();
        assert_eq!(promoted.status, ThreadStatus::Active);
        let list = svc.list_for_stream(&sid).await.unwrap();
        let a2 = list.iter().find(|t| t.id == a.id).unwrap();
        assert_eq!(a2.status, ThreadStatus::Queued);
    }

    #[tokio::test]
    async fn close_then_reopen_lands_in_queued() {
        let (svc, sid) = fixture().await;
        let a = svc
            .create(&sid, "a", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let closed = svc.close(&a.id).await.unwrap();
        assert_eq!(closed.status, ThreadStatus::Closed);
        assert!(closed.closed_at.is_some());
        let reopened = svc.reopen(&a.id).await.unwrap();
        assert_eq!(reopened.status, ThreadStatus::Queued);
        assert!(reopened.closed_at.is_none());
    }

    #[tokio::test]
    async fn rename_updates_title() {
        let (svc, sid) = fixture().await;
        let t = svc
            .create(&sid, "x", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let renamed = svc.rename(&t.id, "y").await.unwrap();
        assert_eq!(renamed.title, "y");
    }

    #[tokio::test]
    async fn set_prompt_round_trips_and_clears_on_empty() {
        let (svc, sid) = fixture().await;
        let t = svc
            .create(&sid, "x", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let with = svc
            .set_prompt(&t.id, Some("Be terse".into()))
            .await
            .unwrap();
        assert_eq!(with.custom_prompt.as_deref(), Some("Be terse"));
        let cleared = svc.set_prompt(&t.id, Some(String::new())).await.unwrap();
        assert_eq!(cleared.custom_prompt, None);
    }

    #[tokio::test]
    async fn reorder_queue_rewrites_sort_index() {
        let (svc, sid) = fixture().await;
        let a = svc
            .create(&sid, "a", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let b = svc
            .create(&sid, "b", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let c = svc
            .create(&sid, "c", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        // c, a, b
        svc.reorder_queue(&sid, &[c.id, a.id, b.id]).await.unwrap();
        let list = svc.list_for_stream(&sid).await.unwrap();
        let order: Vec<_> = list.iter().map(|t| t.id).collect();
        assert_eq!(order, vec![c.id, a.id, b.id]);
    }

    #[tokio::test]
    async fn list_closed_returns_only_closed_newest_first() {
        let (svc, sid) = fixture().await;
        let a = svc
            .create(&sid, "a", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let b = svc
            .create(&sid, "b", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        svc.close(&a.id).await.unwrap();
        svc.close(&b.id).await.unwrap();
        let closed = svc.list_closed(&sid).await.unwrap();
        assert_eq!(closed.len(), 2);
        assert_eq!(closed[0].id, b.id);
    }

    #[tokio::test]
    async fn select_round_trips() {
        let (svc, sid) = fixture().await;
        let t = svc
            .create(&sid, "x", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        assert_eq!(svc.selected(&sid).await.unwrap(), None);
        svc.select(&sid, Some(&t.id)).await.unwrap();
        assert_eq!(svc.selected(&sid).await.unwrap(), Some(t.id));
    }
}
