//! What a chat session's actor (`session.rs`) needs of the agent it talks
//! to, whatever the protocol: an ACP agent over its stdio (`wire.rs`) or a
//! thread on Codex's app-server daemon (`codex.rs`). Each speaks its own
//! wire and hands the actor the same [`Incoming`] messages in arrival
//! order.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use oxplow_domain::hook::TurnRun;

use super::human_prompt::HumanPrompt;
use super::model::{ChatUpdate, PermissionAnswer, PermissionAsk, ToolCall};

/// The agent's side of one session.
#[async_trait]
pub trait Connection: Send + Sync + 'static {
    /// Start the session, or continue the saved one it was built for.
    /// What a continued session replays reaches the incoming channel
    /// before this returns.
    async fn open(&self, cwd: &Path) -> Result<Opened, String>;
    /// Send a person's prompt. The turn's end arrives as
    /// [`Incoming::PromptDone`], after everything the turn did.
    fn prompt(&self, prompt: HumanPrompt) -> Result<(), String>;
    /// Stop the turn in flight.
    fn cancel(&self);
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

/// A session that is up.
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
    /// The agent's own id for it: what the next launch resumes.
    pub session_id: String,
    /// The system prompt, when it must ride ahead of the first prompt
    /// (the agent had no other way to take it).
    pub preamble: Option<String>,
}

/// What the agent sends, in arrival order.
pub enum Incoming {
    Update(Box<ChatUpdate>),
    Permission {
        ask: Box<PermissionAsk>,
        reply: PermissionReply,
    },
    /// The agent asks oxplow to write a file for it (ACP's
    /// `fs/write_text_file`).
    WriteFile {
        path: PathBuf,
        content: String,
        reply: WriteReply,
    },
    /// The agent asks oxplow to read a file for it.
    ReadFile {
        path: PathBuf,
        line: Option<u32>,
        limit: Option<u32>,
        reply: ReadReply,
    },
    /// The agent named the conversation (Codex's thread name).
    Named(String),
    /// The agent waits on the person (`Some`: what for) or no longer does
    /// (`None`). Observed only: whoever answers does it in the agent's own
    /// UI.
    Waiting(Option<String>),
    /// The agent put a plan to the person (Codex's completed `plan` item).
    /// Recorded; whether the person accepts it is not observable.
    Plan(String),
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
    },
    /// A tool call a child agent finished, in the child's own thread:
    /// recorded as the child's, in the parent's session.
    ChildCall {
        child: oxplow_domain::agent::tool::Subagent,
        call: Box<ToolCall>,
    },
    /// The turn in flight finished (or failed).
    PromptDone(Result<TurnEnd, String>),
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

/// The answer to a permission request, sent once.
pub struct PermissionReply(Box<dyn FnOnce(&PermissionAnswer) + Send>);

impl PermissionReply {
    pub fn new(send: impl FnOnce(&PermissionAnswer) + Send + 'static) -> Self {
        Self(Box::new(send))
    }
    pub fn answer(self, a: &PermissionAnswer) {
        (self.0)(a)
    }
}

/// The answer to a write request: done, or refused with a reason that
/// reaches the model.
pub struct WriteReply(Box<dyn FnOnce(Result<(), String>) + Send>);

impl WriteReply {
    pub fn new(send: impl FnOnce(Result<(), String>) + Send + 'static) -> Self {
        Self(Box::new(send))
    }
    pub fn ok(self) {
        (self.0)(Ok(()))
    }
    pub fn deny(self, reason: &str) {
        (self.0)(Err(reason.to_string()))
    }
}

/// The answer to a read request: the text, or why not.
pub struct ReadReply(Box<dyn FnOnce(Result<String, String>) + Send>);

impl ReadReply {
    pub fn new(send: impl FnOnce(Result<String, String>) + Send + 'static) -> Self {
        Self(Box::new(send))
    }
    pub fn ok(self, content: String) {
        (self.0)(Ok(content))
    }
    pub fn err(self, message: &str) {
        (self.0)(Err(message.to_string()))
    }
}
