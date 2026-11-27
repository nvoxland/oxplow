//! Thread reads and the per-stream selection pointer. A thread's
//! lifecycle — create, rename, prompt, mark, close, reopen —
//! is the `thread.*` commands (`oxplow-app/src/commands/thread.rs`), so
//! every surface goes through the command bus.

use std::sync::Arc;

use thiserror::Error;

use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{DeviceId, DomainError, StreamId, Thread, ThreadId};

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
        all.retain(|t| !t.is_open());
        // Closed threads sorted most-recently-closed first.
        all.sort_by_key(|t| std::cmp::Reverse(t.closed_at));
        Ok(all)
    }

    /// The thread `device` chose in `stream`; a reader with no device
    /// (an agent, a job, a script) has chosen none.
    pub async fn selected(
        &self,
        device: Option<&DeviceId>,
        stream: &StreamId,
    ) -> Result<Option<ThreadId>, ThreadError> {
        match device {
            Some(device) => Ok(self.threads.selected_for_stream(device, stream).await?),
            None => Ok(None),
        }
    }

    /// The thread `device` is on in `stream`: its explicit selection
    /// (`selected_for_stream`), else the stream's first open thread in its
    /// order — what a reader with no device gets, never one device's
    /// selection. `None` only when the stream has no open thread, which
    /// should not happen for a stream that seeded its first one.
    pub async fn selected_or_first(
        &self,
        device: Option<&DeviceId>,
        stream: &StreamId,
    ) -> Result<Option<ThreadId>, ThreadError> {
        // `list_for_stream` returns them in their order.
        let open: Vec<Thread> = self
            .threads
            .list_for_stream(stream)
            .await?
            .into_iter()
            .filter(Thread::is_open)
            .collect();
        // A choice that was closed since is no longer the device's thread.
        if let Some(id) = self.selected(device, stream).await? {
            if open.iter().any(|t| t.id == id) {
                return Ok(Some(id));
            }
        }
        Ok(open.first().map(|t| t.id))
    }

    /// `device` shows `thread` in `stream`.
    pub async fn select(
        &self,
        device: &DeviceId,
        stream: &StreamId,
        thread: Option<&ThreadId>,
    ) -> Result<(), ThreadError> {
        self.threads
            .set_selected_for_stream(device, stream, thread)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteStreamStore, SqliteThreadStore};
    use oxplow_domain::stores::StreamStore;
    use oxplow_domain::{Stream, StreamKind, Timestamp};

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
            host: oxplow_domain::HostId::LOCAL,
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
    async fn add(f: &Fixture, title: &str, open: bool, at: i64) -> Thread {
        let mut t = Thread {
            id: ThreadId::placeholder(),
            stream_id: f.stream,
            title: title.into(),
            sort_index: at,
            summary: String::new(),
            summary_updated_at: None,
            closed_at: (!open).then(|| Timestamp::from_unix_ms(at + 1)),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
            mark: None,
        };
        t.id = f.store.upsert(&t).await.unwrap();
        t
    }

    /// With no selection, the first open thread in the stream's order —
    /// a closed one before it is passed over.
    #[tokio::test]
    async fn selected_or_first_is_the_first_open_thread_without_a_selection() {
        let f = fixture().await;
        add(&f, "closed", false, 0).await;
        let a = add(&f, "a", true, 1).await;
        add(&f, "b", true, 2).await;
        let mac = DeviceId::generate();
        assert_eq!(f.svc.selected(Some(&mac), &f.stream).await.unwrap(), None);
        assert_eq!(
            f.svc
                .selected_or_first(Some(&mac), &f.stream)
                .await
                .unwrap(),
            Some(a.id)
        );
    }

    #[tokio::test]
    async fn selected_or_first_prefers_explicit_selection() {
        let f = fixture().await;
        add(&f, "a", true, 0).await;
        let a_id = f.store.list_for_stream(&f.stream).await.unwrap()[0].id;
        let b = add(&f, "b", true, 1).await;
        let (mac, tablet) = (DeviceId::generate(), DeviceId::generate());
        f.svc.select(&mac, &f.stream, Some(&b.id)).await.unwrap();
        assert_eq!(
            f.svc
                .selected_or_first(Some(&mac), &f.stream)
                .await
                .unwrap(),
            Some(b.id)
        );
        // Another device, or a reader with none, has the first open thread.
        assert_eq!(
            f.svc
                .selected_or_first(Some(&tablet), &f.stream)
                .await
                .unwrap(),
            Some(a_id)
        );
        assert_eq!(
            f.svc.selected_or_first(None, &f.stream).await.unwrap(),
            Some(a_id)
        );
    }

    /// A device's choice that was closed since (on another device, say) is no
    /// longer its thread: the first open one is.
    #[tokio::test]
    async fn selected_or_first_passes_over_a_closed_choice() {
        let f = fixture().await;
        let a = add(&f, "a", true, 0).await;
        let closed = add(&f, "closed", false, 1).await;
        let mac = DeviceId::generate();
        f.svc
            .select(&mac, &f.stream, Some(&closed.id))
            .await
            .unwrap();
        assert_eq!(
            f.svc
                .selected_or_first(Some(&mac), &f.stream)
                .await
                .unwrap(),
            Some(a.id)
        );
    }

    #[tokio::test]
    async fn selected_or_first_returns_none_when_stream_has_no_threads() {
        let f = fixture().await;
        assert_eq!(
            f.svc.selected_or_first(None, &f.stream).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn list_closed_returns_only_closed_newest_first() {
        let f = fixture().await;
        add(&f, "open", true, 0).await;
        let a = add(&f, "a", false, 1).await;
        let b = add(&f, "b", false, 2).await;
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
        let t = add(&f, "x", true, 0).await;
        let mac = DeviceId::generate();
        assert_eq!(f.svc.selected(Some(&mac), &f.stream).await.unwrap(), None);
        f.svc.select(&mac, &f.stream, Some(&t.id)).await.unwrap();
        assert_eq!(
            f.svc.selected(Some(&mac), &f.stream).await.unwrap(),
            Some(t.id)
        );
        assert_eq!(f.svc.selected(None, &f.stream).await.unwrap(), None);
    }
}
