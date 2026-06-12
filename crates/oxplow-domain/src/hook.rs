//! Hook events + agent status + agent turn types.
//!
//! These cross the wire from the Claude Code hook subprocess into the
//! oxplow daemon, get persisted, and feed every Stop / write-guard /
//! filing decision. Pure data — no IO.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{AgentTurnId, HookEventId, StreamId, TaskId, ThreadId};
use crate::time::Timestamp;

/// Discriminant for hook events. Matches the kinds Claude Code emits
/// plus a few oxplow-internal synthetic kinds (Interrupt, AgentBoot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HookKind {
    /// Renderer/agent paste, run-command, etc.
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    Stop,
    SubagentStop,
    /// Synthesized by oxplow when the user hits Ctrl-C / Esc in a pane.
    Interrupt,
    /// Agent boot sentinel — fires once per session_id when oxplow first
    /// observes traffic for it.
    AgentBoot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct HookEvent {
    pub id: HookEventId,
    pub thread_id: Option<ThreadId>,
    pub stream_id: Option<StreamId>,
    pub kind: HookKind,
    pub session_id: Option<String>,
    /// Raw envelope from the hook subprocess, JSON-encoded. The
    /// pipeline parses this lazily — the persisted form is verbatim.
    pub payload_json: String,
    pub received_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatusState {
    Idle,
    Running,
    AwaitingUser,
    Stopped,
    Error,
    /// Derived-only: the hook log says Running but no event has
    /// arrived for longer than the stall threshold. Claude Code emits
    /// no hook when a turn dies on an API error and the process drops
    /// back to its prompt, so a wall-clock check is the only way to
    /// notice. Never persisted to the agent_status table.
    Stalled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AgentStatus {
    pub thread_id: ThreadId,
    pub pane_target: String,
    pub state: AgentStatusState,
    pub detail: Option<String>,
    pub updated_at: Timestamp,
}

/// One open or closed agent turn. Open rows render as live in-progress
/// entries in the Work panel; the Stop hook closes the row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AgentTurn {
    pub id: AgentTurnId,
    pub thread_id: ThreadId,
    pub task_id: Option<TaskId>,
    pub prompt: String,
    pub answer: Option<String>,
    pub session_id: Option<String>,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_kind_serializes_as_snake_case() {
        let json = serde_json::to_string(&HookKind::PreToolUse).unwrap();
        assert_eq!(json, "\"pre_tool_use\"");
    }

    #[test]
    fn agent_status_round_trips() {
        let s = AgentStatus {
            thread_id: ThreadId::new(1),
            pane_target: "working".into(),
            state: AgentStatusState::Running,
            detail: Some("typing".into()),
            updated_at: Timestamp::from_unix_ms(1),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: AgentStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
