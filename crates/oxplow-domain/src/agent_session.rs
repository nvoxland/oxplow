//! An agent session: one agent slot a person opened on a thread.
//!
//! A thread is a line of the person's work; it needs no agent and may
//! have several. Each `agent_session` row is a slot — a terminal agent, or
//! (later) a one-shot action — with its harness, the
//! harness's resume id, and when it was opened and closed. The row is the
//! slot, not the process: a harness process ending leaves it open (its
//! tab keeps the ended notice and resume id); only closing the session,
//! its thread, or its stream's archive sets `closed_at`
//! (`.context/data-model.md` "agent_session").

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{AgentSessionId, ThreadId};
use crate::time::Timestamp;

/// What kind of slot a session is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    /// A harness in a terminal (a PTY).
    Terminal,
    /// A one-shot session on a programmatic surface (not built yet).
    Action,
}

impl SessionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionKind::Terminal => "terminal",
            SessionKind::Action => "action",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "terminal" => Some(SessionKind::Terminal),
            "action" => Some(SessionKind::Action),
            _ => None,
        }
    }
}

/// Why a session's slot was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SessionCloseReason {
    /// The person (or an approved agent proposal) closed it.
    Closed,
    /// Its thread was closed.
    ThreadClosed,
    /// Its stream was archived.
    StreamArchived,
}

impl SessionCloseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionCloseReason::Closed => "closed",
            SessionCloseReason::ThreadClosed => "thread_closed",
            SessionCloseReason::StreamArchived => "stream_archived",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "closed" => Some(SessionCloseReason::Closed),
            "thread_closed" => Some(SessionCloseReason::ThreadClosed),
            "stream_archived" => Some(SessionCloseReason::StreamArchived),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AgentSession {
    pub id: AgentSessionId,
    pub thread_id: ThreadId,
    pub kind: SessionKind,
    /// The harness that runs in it: its registry key
    /// (`agent::registry::HarnessRegistry`).
    pub harness: String,
    /// What the person called it; empty until renamed.
    pub title: String,
    /// The harness's own session id, to resume it; empty when unknown.
    pub resume_session_id: String,
    /// The host it runs on; `None` is the local machine.
    pub host: Option<String>,
    pub opened_at: Timestamp,
    pub closed_at: Option<Timestamp>,
    pub closed_reason: Option<SessionCloseReason>,
    pub updated_at: Timestamp,
    /// The OS pid of the process the daemon spawned for it, rewritten on
    /// every spawn; `None` for a session not yet spawned. A pid alone may be dead or reused:
    /// read it with `last_seen_at`.
    pub pid: Option<u32>,
    /// The newest hook seen from the session.
    pub last_seen_at: Option<Timestamp>,
    /// When the person deliberately stopped it; `None` once started again
    /// and after a crash or restart (which resume on activation).
    pub stopped_by_user_at: Option<Timestamp>,
    /// The name the harness gives the bound conversation; empty when it
    /// gave none.
    pub native_title: String,
    /// The next launch forks the bound conversation instead of resuming it.
    pub fork_next: bool,
}

/// How a slot came to be bound to a native conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BindingOrigin {
    /// Its first conversation, or one begun after it had none.
    Started,
    /// The harness moved it to another (a native `/resume`, `/branch`).
    Switched,
    /// A launch forked the one it was bound to.
    Forked,
    /// The person chose a saved one.
    Chosen,
}

impl BindingOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            BindingOrigin::Started => "started",
            BindingOrigin::Switched => "switched",
            BindingOrigin::Forked => "forked",
            BindingOrigin::Chosen => "chosen",
        }
    }
}

impl AgentSession {
    pub fn is_open(&self) -> bool {
        self.closed_at.is_none()
    }
}

/// What opening a session stores; the id and timestamps are the store's.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAgentSession {
    pub thread_id: ThreadId,
    pub kind: SessionKind,
    pub harness: String,
    pub title: String,
}

impl NewAgentSession {
    /// An untitled `kind` session of `harness` on `thread`.
    pub fn of(thread_id: ThreadId, kind: SessionKind, harness: impl Into<String>) -> Self {
        Self {
            thread_id,
            kind,
            harness: harness.into(),
            title: String::new(),
        }
    }

    /// An untitled terminal session of `harness` on `thread`.
    pub fn terminal(thread_id: ThreadId, harness: impl Into<String>) -> Self {
        Self::of(thread_id, SessionKind::Terminal, harness)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_reasons_round_trip_their_strings() {
        for k in [SessionKind::Terminal, SessionKind::Action] {
            assert_eq!(SessionKind::parse(k.as_str()), Some(k));
            assert_eq!(
                serde_json::to_string(&k).unwrap(),
                format!("\"{}\"", k.as_str())
            );
        }
        for r in [
            SessionCloseReason::Closed,
            SessionCloseReason::ThreadClosed,
            SessionCloseReason::StreamArchived,
        ] {
            assert_eq!(SessionCloseReason::parse(r.as_str()), Some(r));
        }
    }
}
