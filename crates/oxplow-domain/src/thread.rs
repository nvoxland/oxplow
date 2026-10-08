//! Thread domain type (was "batch" in earlier TS code).
//!
//! A thread is a line of the person's work inside a stream. It needs no
//! agent and may have several (its `agent_session`s, see
//! [`crate::agent_session`]). Multiple threads can run concurrently per
//! stream; one is "selected" at a time for UI focus.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{NoteId, StreamId, ThreadId};
use crate::time::Timestamp;

/// Thread lifecycle status — mirrors the TS `ThreadState` shape.
///
/// `Active` is the writer thread for its stream (only one per stream
/// can mutate the worktree at a time). `Queued` is a non-writer
/// thread sharing the same worktree in read-only mode. `Closed` is
/// terminated; closed threads are excluded from the rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    Active,
    Queued,
    Closed,
}

impl ThreadStatus {
    /// True when this thread is the writer for its stream — i.e.
    /// the only thread allowed to mutate the shared worktree.
    pub fn is_writer(&self) -> bool {
        matches!(self, ThreadStatus::Active)
    }
}

/// A note on a thread: the per-thread capture pad an agent writes as it
/// works (`oxplow.knowledge.add_note`). Its ref is `thread_note:not<n>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ThreadNote {
    pub id: NoteId,
    pub thread_id: ThreadId,
    pub body: String,
    pub author: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Thread {
    pub id: ThreadId,
    pub stream_id: StreamId,
    pub title: String,
    pub status: ThreadStatus,
    pub sort_index: i64,
    pub summary: String,
    pub summary_updated_at: Option<Timestamp>,
    /// Timestamp when the thread was closed (status transitions to
    /// `Closed`). `None` for active/queued threads.
    pub closed_at: Option<Timestamp>,
    /// Per-thread custom prompt appended to the agent's system message.
    /// `None` when unset; `Some("")` is distinct (empty override).
    pub custom_prompt: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Set when the thread was archived as part of its stream's
    /// "Remove…" action. Archived threads are filtered out of
    /// `ThreadStore::list_for_stream`.
    pub archived_at: Option<Timestamp>,
}

impl Thread {
    /// A new, active thread titled `title` on `stream`, not yet stored
    /// (its id is the placeholder).
    pub fn seed(stream_id: StreamId, title: impl Into<String>, now: Timestamp) -> Self {
        Thread {
            id: ThreadId::placeholder(),
            stream_id,
            title: title.into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let now = Timestamp::from_unix_ms(1_700_000_000_000);
        let t = Thread {
            id: ThreadId::new(1),
            ..Thread::seed(StreamId::new(1), "explore", now)
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: Thread = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn status_uses_snake_case() {
        assert_eq!(
            serde_json::to_string(&ThreadStatus::Active).unwrap(),
            "\"active\""
        );
        assert_eq!(
            serde_json::to_string(&ThreadStatus::Queued).unwrap(),
            "\"queued\""
        );
        assert_eq!(
            serde_json::to_string(&ThreadStatus::Closed).unwrap(),
            "\"closed\""
        );
    }

    #[test]
    fn only_active_is_writer() {
        assert!(ThreadStatus::Active.is_writer());
        assert!(!ThreadStatus::Queued.is_writer());
        assert!(!ThreadStatus::Closed.is_writer());
    }
}
