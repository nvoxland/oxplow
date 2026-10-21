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

/// The colour a person marks a thread's tab with (`oxplow.thread.mark`):
/// a fold in its tab's corner, for grouping threads their own way. It
/// changes nothing else.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ThreadMark {
    Amber,
    Red,
    Green,
    Blue,
    Violet,
}

impl ThreadMark {
    pub const ALL: [ThreadMark; 5] = [
        ThreadMark::Amber,
        ThreadMark::Red,
        ThreadMark::Green,
        ThreadMark::Blue,
        ThreadMark::Violet,
    ];

    /// The stored and wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadMark::Amber => "amber",
            ThreadMark::Red => "red",
            ThreadMark::Green => "green",
            ThreadMark::Blue => "blue",
            ThreadMark::Violet => "violet",
        }
    }

    pub fn parse(s: &str) -> Option<ThreadMark> {
        ThreadMark::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

/// A note on a thread: the per-thread capture pad an agent writes as it
/// works (`oxplow.knowledge.add_note`). Its ref is `oxplow:oxplow/thread_note?id=not<n>`.
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
    #[specta(type = crate::JsNumber)]
    pub sort_index: i64,
    pub summary: String,
    pub summary_updated_at: Option<Timestamp>,
    /// When the thread was closed; `None` while it is open. Open or
    /// closed is all a thread's state is.
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
    /// The person's colour for its tab; `None` when unmarked.
    pub mark: Option<ThreadMark>,
}

impl Thread {
    /// Whether the thread is open (not closed).
    pub fn is_open(&self) -> bool {
        self.closed_at.is_none()
    }

    /// A new, open thread titled `title` on `stream`, not yet stored
    /// (its id is the placeholder).
    pub fn seed(stream_id: StreamId, title: impl Into<String>, now: Timestamp) -> Self {
        Thread {
            id: ThreadId::placeholder(),
            stream_id,
            title: title.into(),
            sort_index: 0,
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
            mark: None,
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
    fn a_mark_reads_back_from_its_name() {
        for m in ThreadMark::ALL {
            assert_eq!(ThreadMark::parse(m.as_str()), Some(m));
            assert_eq!(
                serde_json::to_string(&m).unwrap(),
                format!("\"{}\"", m.as_str())
            );
        }
        assert_eq!(ThreadMark::parse("plaid"), None);
    }
}
