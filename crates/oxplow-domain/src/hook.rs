//! Hook events + agent status + agent turn types.
//!
//! These cross the wire from the Claude Code hook subprocess into the
//! oxplow daemon, get persisted, and feed the write guard. Pure data —
//! no IO.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{AgentTurnId, ThreadId};
use crate::time::Timestamp;

/// Discriminant for hook events: the kinds the harnesses post, plus
/// `Interrupt`, which oxplow synthesizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HookKind {
    /// Renderer/agent paste, run-command, etc.
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    Stop,
    /// Synthesized by oxplow when the user hits Ctrl-C / Esc in a pane.
    Interrupt,
    /// A harness process started or resumed a session (Claude's command
    /// hook, Codex's hook, the ACP client). `source: "compact"` is a
    /// compaction inside a running session, not a start.
    SessionStart,
    /// A harness session ended — `reason: "clear"` for `/clear`.
    SessionEnd,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, schemars::JsonSchema,
)]
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
    /// notice. Never logged: `agent.status.changed` has no such state.
    Stalled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AgentStatus {
    pub thread_id: ThreadId,
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
    pub prompt: String,
    pub answer: Option<String>,
    pub session_id: Option<String>,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    /// The snapshot the stream's worktree was at when the turn opened;
    /// `start_snapshot_id → snapshot_id` is what the turn changed.
    /// `None` when the stream had no snapshot yet.
    pub start_snapshot_id: Option<i64>,
    /// The snapshot the worktree was at when the turn ended (its
    /// `turn_end` take); `None` while running.
    pub snapshot_id: Option<i64>,
}

/// How a turn ended (`agent.turn.ended`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    /// The agent finished (Stop).
    Completed,
    /// The person interrupted it.
    Interrupted,
    /// oxplow restarted with the turn still open; recovery closed it.
    Restart,
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
            state: AgentStatusState::Running,
            detail: Some("typing".into()),
            updated_at: Timestamp::from_unix_ms(1),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: AgentStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
