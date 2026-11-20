//! What an observed session's actor (`session.rs`) needs of the agent it
//! watches: a thread on Codex's app-server daemon (`codex.rs`; tests script
//! their own). A connection speaks its own wire and hands the actor
//! [`Incoming`] messages in arrival order.

use std::path::Path;

use async_trait::async_trait;

use oxplow_domain::hook::TurnRun;

use super::model::{ChatUpdate, ToolCall};

/// Text kept per content block; a runaway command output is cut here.
pub const MAX_TEXT: usize = 64 * 1024;

pub(crate) fn cap(s: &str) -> String {
    if s.len() <= MAX_TEXT {
        return s.to_string();
    }
    let mut end = MAX_TEXT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [truncated]", &s[..end])
}

pub(crate) fn cap_value(v: serde_json::Value) -> serde_json::Value {
    match &v {
        serde_json::Value::String(s) if s.len() > MAX_TEXT => serde_json::Value::String(cap(s)),
        serde_json::Value::String(_) => v,
        _ if v.to_string().len() > MAX_TEXT => serde_json::json!({ "truncated": true }),
        _ => v,
    }
}

/// The agent's side of one session.
#[async_trait]
pub trait Connection: Send + Sync + 'static {
    /// Start the session, or continue the saved one it was built for: the
    /// agent's own id for it, what the next launch resumes. What a
    /// continued session replays reaches the incoming channel before this
    /// returns.
    async fn open(&self, cwd: &Path) -> Result<String, String>;
    /// Resolves when the agent has gone away.
    async fn closed(&self);
    /// Whether context can be appended to the agent's history right now
    /// without starting anything: the agent has such a channel and no turn
    /// is running. A context item arriving during a turn is read as more
    /// input and lengthens it, so the session injects only when this holds.
    fn can_inject(&self) -> bool {
        false
    }
    /// Append `text` to the agent's model-visible history as context. It
    /// starts no turn and answers nothing; the agent reads it at its next
    /// turn.
    async fn inject(&self, _text: &str) -> Result<(), String> {
        Err("this agent takes no injected context".into())
    }
    /// Name the agent's conversation (what its own UI lists it as). An
    /// agent that can't says so.
    async fn rename(&self, _name: &str) -> Result<(), String> {
        Err("this agent can't rename its conversation".into())
    }
}

/// What the agent sends, in arrival order.
pub enum Incoming {
    Update(Box<ChatUpdate>),
    /// The agent named the conversation (Codex's thread name).
    Named(String),
    /// The agent waits on the person (`Some`: what for) or no longer does
    /// (`None`). Observed only: whoever answers does it in the agent's own
    /// UI.
    Waiting(Option<String>),
    /// The agent summarized its context to make room (Codex's
    /// `contextCompaction` item). Observed only.
    Compacted,
    /// A child agent of the thread started or finished (Codex's collab
    /// agents). Observed only: nothing here calls a collab tool or starts,
    /// interrupts or closes a child.
    Subagent {
        change: SubagentChange,
        id: String,
        /// What kind it is (its role or path), when Codex says.
        kind: Option<String>,
        /// The harness's id of the turn that spawned it, when it said
        /// (Codex's turn id, a child's `rootTurnId`).
        spawned_in: Option<String>,
    },
    /// A turn of the thread began, under the agent's own id for it (Codex's
    /// `turn.id`).
    TurnStarted(String),
    /// A tool call a child agent finished, in the child's own thread:
    /// recorded as the child's, in the parent's session.
    ChildCall {
        child: oxplow_domain::agent::tool::Subagent,
        call: Box<ToolCall>,
    },
    /// The turn in flight finished (or failed).
    TurnEnded(Result<TurnEnd, String>),
}

/// A child agent's change of state.
#[derive(Debug, Clone, PartialEq)]
pub enum SubagentChange {
    Started,
    /// It finished (completed, interrupted, errored, shut down): what it
    /// said last, or why it ended.
    Finished(Option<String>),
}

/// How a turn ended.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnEnd {
    /// `end_turn`, `cancelled`, `completed`, `interrupted`, …
    pub stop_reason: String,
    /// Per-turn token counts, when the agent reports them.
    pub usage: Option<TurnTokens>,
    /// The model and effort the turn ran on, when the agent reports them.
    pub run: Option<TurnRun>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnTokens {
    /// Input not read from the cache.
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}
