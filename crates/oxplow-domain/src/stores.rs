//! Store traits.
//!
//! Service crates depend on these traits, never on concrete impls.
//! `oxplow-db` implements them against rusqlite; tests can supply
//! in-memory fakes. The traits are async even though the SQLite impl
//! is sync — this matches Tauri's tokio-multi-thread runtime where DB
//! calls go through `spawn_blocking`. From the caller's POV the
//! await point is the same regardless of impl.

use async_trait::async_trait;

use crate::comment::{CommentIntent, CommentMessage, CommentStatus, CommentTarget, CommentThread};
use crate::hook::{AgentStatus, AgentStatusState, AgentTurn, HookEvent, HookKind};
use crate::ids::{AgentTurnId, CommentId, NoteId, StreamId, TaskId, TaskLinkId, ThreadId};
use crate::stream::Stream;
use crate::task::{Task, TaskEvent, TaskLink, TaskLinkType, TaskNote};
use crate::thread::Thread;
use crate::DomainError;

#[async_trait]
pub trait StreamStore: Send + Sync {
    async fn list(&self) -> Result<Vec<Stream>, DomainError>;
    async fn get(&self, id: &StreamId) -> Result<Option<Stream>, DomainError>;
    async fn upsert(&self, stream: &Stream) -> Result<(), DomainError>;
    async fn delete(&self, id: &StreamId) -> Result<(), DomainError>;
    /// Soft-delete: stamp `archived_at` so the row drops out of
    /// `list()` but stays referenced from history (efforts, snapshots,
    /// page_visit). Idempotent — re-archiving an already-archived row
    /// is a no-op.
    async fn archive(&self, id: &StreamId) -> Result<(), DomainError>;
    async fn primary(&self) -> Result<Option<Stream>, DomainError>;
    /// Returns the runtime-state pointer to the currently-selected
    /// stream id, if any. Survives restarts; null until set.
    async fn current_id(&self) -> Result<Option<StreamId>, DomainError>;
    /// Sets (or clears) the current-stream pointer.
    async fn set_current(&self, id: Option<&StreamId>) -> Result<(), DomainError>;
}

#[async_trait]
pub trait ThreadStore: Send + Sync {
    async fn list_for_stream(&self, stream: &StreamId) -> Result<Vec<Thread>, DomainError>;
    async fn get(&self, id: &ThreadId) -> Result<Option<Thread>, DomainError>;
    async fn upsert(&self, thread: &Thread) -> Result<(), DomainError>;
    async fn delete(&self, id: &ThreadId) -> Result<(), DomainError>;
    /// Soft-delete: stamp `archived_at`. Excluded from
    /// `list_for_stream` after this fires.
    async fn archive(&self, id: &ThreadId) -> Result<(), DomainError>;
    /// Per-stream selected-thread pointer. None means nothing selected.
    async fn selected_for_stream(&self, stream: &StreamId)
        -> Result<Option<ThreadId>, DomainError>;
    async fn set_selected_for_stream(
        &self,
        stream: &StreamId,
        thread: Option<&ThreadId>,
    ) -> Result<(), DomainError>;
}

#[async_trait]
pub trait TaskStore: Send + Sync {
    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<Task>, DomainError>;
    async fn list_backlog(&self) -> Result<Vec<Task>, DomainError>;
    async fn get(&self, id: TaskId) -> Result<Option<Task>, DomainError>;
    /// Insert a new task; assigns and returns the autoincrement id.
    async fn insert(&self, item: &Task) -> Result<TaskId, DomainError>;
    /// Update an existing task by id.
    async fn update(&self, item: &Task) -> Result<(), DomainError>;
    async fn soft_delete(&self, id: TaskId) -> Result<(), DomainError>;
}

#[async_trait]
pub trait TaskNoteStore: Send + Sync {
    async fn add_for_item(
        &self,
        item: TaskId,
        body: &str,
        author: &str,
    ) -> Result<TaskNote, DomainError>;
    async fn add_for_thread(
        &self,
        thread: &ThreadId,
        body: &str,
        author: &str,
    ) -> Result<TaskNote, DomainError>;
    async fn list_for_item(&self, item: TaskId) -> Result<Vec<TaskNote>, DomainError>;
    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<TaskNote>, DomainError>;
    /// Replace the body of an existing note. Used by
    /// `oxplow__record_query_finding` to fill in a note that was
    /// pre-allocated empty by `oxplow__delegate_query`.
    async fn update_body(&self, id: &NoteId, body: &str) -> Result<(), DomainError>;
    async fn delete(&self, id: &NoteId) -> Result<(), DomainError>;
}

#[async_trait]
pub trait TaskLinkStore: Send + Sync {
    async fn create(
        &self,
        thread: &ThreadId,
        from: TaskId,
        to: TaskId,
        link_type: TaskLinkType,
    ) -> Result<TaskLink, DomainError>;
    async fn list_outgoing(&self, item: TaskId) -> Result<Vec<TaskLink>, DomainError>;
    async fn list_incoming(&self, item: TaskId) -> Result<Vec<TaskLink>, DomainError>;
    async fn delete(&self, id: TaskLinkId) -> Result<(), DomainError>;
}

#[async_trait]
pub trait TaskEventStore: Send + Sync {
    async fn append(&self, event: &TaskEvent) -> Result<(), DomainError>;
    async fn list_for_item(&self, item: TaskId) -> Result<Vec<TaskEvent>, DomainError>;
    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<TaskEvent>, DomainError>;
}

#[async_trait]
pub trait HookEventStore: Send + Sync {
    async fn append(&self, event: &HookEvent) -> Result<(), DomainError>;
    /// Most recent first, capped at `limit` (default 200).
    async fn list_recent(
        &self,
        thread: Option<&ThreadId>,
        limit: usize,
    ) -> Result<Vec<HookEvent>, DomainError>;
    async fn list_by_kind(
        &self,
        kind: HookKind,
        limit: usize,
    ) -> Result<Vec<HookEvent>, DomainError>;
}

#[async_trait]
pub trait AgentStatusStore: Send + Sync {
    async fn upsert(
        &self,
        thread: &ThreadId,
        pane_target: &str,
        state: AgentStatusState,
        detail: Option<String>,
    ) -> Result<AgentStatus, DomainError>;
    async fn get(
        &self,
        thread: &ThreadId,
        pane_target: &str,
    ) -> Result<Option<AgentStatus>, DomainError>;
    async fn list_all(&self) -> Result<Vec<AgentStatus>, DomainError>;
}

#[async_trait]
pub trait AgentTurnStore: Send + Sync {
    async fn open(&self, turn: &AgentTurn) -> Result<(), DomainError>;
    async fn close(&self, id: &AgentTurnId, answer: Option<String>) -> Result<(), DomainError>;
    async fn get(&self, id: &AgentTurnId) -> Result<Option<AgentTurn>, DomainError>;
    async fn list_open(&self, thread: &ThreadId) -> Result<Vec<AgentTurn>, DomainError>;
    /// Every open agent_turn across every thread. Used by daemon
    /// recovery on boot to close orphans the previous process left
    /// behind.
    async fn list_all_open(&self) -> Result<Vec<AgentTurn>, DomainError>;
    async fn list_for_thread(
        &self,
        thread: &ThreadId,
        limit: usize,
    ) -> Result<Vec<AgentTurn>, DomainError>;
}

/// Threaded comments anchored to a text selection on any page.
/// Reads return whole [`CommentThread`]s (anchor + messages).
#[async_trait]
pub trait CommentStore: Send + Sync {
    /// Create a comment anchored to `target` with its first message.
    /// `context_chain` is the ancestor regions the selection sat inside
    /// (innermost→outermost, excluding `target`); `referenced_refs` are
    /// the canonical refs found inside the selection.
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
    ) -> Result<CommentThread, DomainError>;

    /// Append a reply to an existing thread; bumps `last_activity_at`.
    async fn add_message(
        &self,
        comment: CommentId,
        author: &str,
        body: &str,
    ) -> Result<CommentMessage, DomainError>;

    async fn get(&self, id: CommentId) -> Result<Option<CommentThread>, DomainError>;
    async fn list_for_target(
        &self,
        target: &CommentTarget,
    ) -> Result<Vec<CommentThread>, DomainError>;
    async fn list_for_stream(&self, stream: &StreamId) -> Result<Vec<CommentThread>, DomainError>;
    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<CommentThread>, DomainError>;

    async fn set_intent(&self, id: CommentId, intent: CommentIntent) -> Result<(), DomainError>;
    async fn set_status(&self, id: CommentId, status: CommentStatus) -> Result<(), DomainError>;
    /// Persist a re-resolved selectors array (and whether it's orphaned).
    async fn set_anchor(
        &self,
        id: CommentId,
        selectors_json: &str,
        orphaned: bool,
    ) -> Result<(), DomainError>;
    /// Re-attach an orphaned comment to a freshly-selected span: replace
    /// both the `quote` and the `selectors_json` and clear `orphaned`.
    /// (The old quote no longer matches, so unlike `set_anchor` this
    /// rewrites the durable anchor text too.)
    async fn relink(
        &self,
        id: CommentId,
        quote: &str,
        selectors_json: &str,
    ) -> Result<(), DomainError>;
    async fn delete(&self, id: CommentId) -> Result<(), DomainError>;

    /// Delete `resolved` and `orphaned` threads whose last activity is
    /// older than `retention_days`. Returns the number deleted.
    async fn cleanup(&self, retention_days: i64) -> Result<u64, DomainError>;
}
