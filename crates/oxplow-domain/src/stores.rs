//! Store traits.
//!
//! Service crates depend on these traits, never on concrete impls.
//! `oxplow-db` implements them against rusqlite; tests can supply
//! in-memory fakes. The traits are async even though the SQLite impl
//! is sync — this matches Tauri's tokio-multi-thread runtime where DB
//! calls go through `spawn_blocking`. From the caller's POV the
//! await point is the same regardless of impl.

use async_trait::async_trait;

use crate::comment::{CommentTarget, CommentThread};
use crate::hook::{AgentStatus, AgentTurn};
use crate::ids::{AgentTurnId, CommentId, StreamId, ThreadId};
use crate::stream::Stream;
use crate::thread::{Thread, ThreadNote};
use crate::DomainError;

#[async_trait]
pub trait StreamStore: Send + Sync {
    async fn list(&self) -> Result<Vec<Stream>, DomainError>;
    async fn get(&self, id: &StreamId) -> Result<Option<Stream>, DomainError>;
    /// Insert or update a stream. When `stream.id` is the placeholder
    /// (`StreamId::placeholder()`) a fresh autoincrement id is allocated;
    /// otherwise the explicit id is inserted / updated in place. Returns
    /// the effective id.
    async fn upsert(&self, stream: &Stream) -> Result<StreamId, DomainError>;
    /// Record the branch `id`'s workspace has checked out (`branch`,
    /// `branch_ref`, `updated_at` only — nothing else a concurrent write
    /// may have changed). Whether the stream exists.
    async fn set_branch(&self, id: &StreamId, branch: &str) -> Result<bool, DomainError>;
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
    /// Insert or update a thread. When `thread.id` is the placeholder
    /// (`ThreadId::placeholder()`) a fresh autoincrement id is allocated;
    /// otherwise the explicit id is inserted / updated in place. Returns
    /// the effective id.
    async fn upsert(&self, thread: &Thread) -> Result<ThreadId, DomainError>;
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
pub trait ThreadNoteStore: Send + Sync {
    // Written by `oxplow.knowledge.add_note` / `update_note`
    // (`oxplow_db::thread_note_store`).
    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<ThreadNote>, DomainError>;
}

#[async_trait]
/// A thread's agent status is its newest logged `agent.status.changed`;
/// the log is the only record (writes go through the hook ingest).
pub trait AgentStatusStore: Send + Sync {
    async fn get(&self, thread: &ThreadId) -> Result<Option<AgentStatus>, DomainError>;
    /// The status of every thread that has logged one.
    async fn list_all(&self) -> Result<Vec<AgentStatus>, DomainError>;
}

#[async_trait]
pub trait AgentTurnStore: Send + Sync {
    /// Open a turn. When `turn.id` is the placeholder a fresh
    /// autoincrement id is allocated; returns the effective id.
    async fn open(&self, turn: &AgentTurn) -> Result<AgentTurnId, DomainError>;
    /// Close an open turn, logging `agent.turn.ended` with `outcome` in
    /// the same transaction. Returns whether this call closed it (`false`
    /// when it was already closed — nothing is logged then).
    async fn close(
        &self,
        id: &AgentTurnId,
        answer: Option<String>,
        outcome: crate::hook::TurnOutcome,
    ) -> Result<bool, DomainError>;
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
    // Writing a comment is a `knowledge.*_comment` command (P8.A6), over
    // `oxplow_db::comment_store::*_tx`.
    async fn get(&self, id: CommentId) -> Result<Option<CommentThread>, DomainError>;
    async fn list_for_target(
        &self,
        target: &CommentTarget,
    ) -> Result<Vec<CommentThread>, DomainError>;
    async fn list_for_stream(&self, stream: &StreamId) -> Result<Vec<CommentThread>, DomainError>;
    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<CommentThread>, DomainError>;

    /// Delete `resolved` and `orphaned` threads whose last activity is
    /// older than `retention_days`. Returns the number deleted.
    async fn cleanup(&self, retention_days: i64) -> Result<u64, DomainError>;
}
