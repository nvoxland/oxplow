//! Thread reads and the per-stream selection pointer. A thread's
//! lifecycle — create, rename, prompt, promote, close, reopen, reorder —
//! is the `thread.*` commands (`oxplow-app/src/commands/thread.rs`), so
//! every surface goes through the command bus.

use std::sync::Arc;

use thiserror::Error;

use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{DomainError, StreamId, Thread, ThreadId, ThreadStatus};

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteStreamStore, SqliteThreadStore};
    use oxplow_domain::stores::StreamStore;
    use oxplow_domain::{AgentKind, Stream, StreamKind, Timestamp};

    struct Fixture {
        svc: ThreadService,
        store: SqliteThreadStore,
        stream: StreamId,
    }

    async fn fixture() -> Fixture {
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
        let store = SqliteThreadStore::new(db);
        Fixture {
            svc: ThreadService::new(Arc::new(store.clone())),
            store,
            stream: s.id,
        }
    }

    /// A thread row as the `thread.*` commands would leave it.
    async fn add(f: &Fixture, title: &str, status: ThreadStatus, at: i64) -> Thread {
        let mut t = Thread {
            id: ThreadId::placeholder(),
            stream_id: f.stream,
            title: title.into(),
            status,
            sort_index: at,
            pane_target: "working".into(),
            agent: AgentKind::Claude,
            acp_agent: None,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: (status == ThreadStatus::Closed).then(|| Timestamp::from_unix_ms(at + 1)),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        t.id = f.store.upsert(&t).await.unwrap();
        t
    }

    #[tokio::test]
    async fn selected_or_active_returns_writer_when_no_explicit_selection() {
        let f = fixture().await;
        let a = add(&f, "a", ThreadStatus::Active, 0).await;
        add(&f, "b", ThreadStatus::Queued, 1).await;
        assert_eq!(f.svc.selected(&f.stream).await.unwrap(), None);
        assert_eq!(
            f.svc.selected_or_active(&f.stream).await.unwrap(),
            Some(a.id)
        );
    }

    #[tokio::test]
    async fn selected_or_active_prefers_explicit_selection() {
        let f = fixture().await;
        add(&f, "a", ThreadStatus::Active, 0).await;
        let b = add(&f, "b", ThreadStatus::Queued, 1).await;
        f.svc.select(&f.stream, Some(&b.id)).await.unwrap();
        assert_eq!(
            f.svc.selected_or_active(&f.stream).await.unwrap(),
            Some(b.id)
        );
    }

    #[tokio::test]
    async fn selected_or_active_returns_none_when_stream_has_no_threads() {
        let f = fixture().await;
        assert_eq!(f.svc.selected_or_active(&f.stream).await.unwrap(), None);
    }

    #[tokio::test]
    async fn list_closed_returns_only_closed_newest_first() {
        let f = fixture().await;
        add(&f, "open", ThreadStatus::Active, 0).await;
        let a = add(&f, "a", ThreadStatus::Closed, 1).await;
        let b = add(&f, "b", ThreadStatus::Closed, 2).await;
        let closed: Vec<ThreadId> = f
            .svc
            .list_closed(&f.stream)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(closed, [b.id, a.id]);
    }

    #[tokio::test]
    async fn select_round_trips() {
        let f = fixture().await;
        let t = add(&f, "x", ThreadStatus::Active, 0).await;
        assert_eq!(f.svc.selected(&f.stream).await.unwrap(), None);
        f.svc.select(&f.stream, Some(&t.id)).await.unwrap();
        assert_eq!(f.svc.selected(&f.stream).await.unwrap(), Some(t.id));
    }
}
