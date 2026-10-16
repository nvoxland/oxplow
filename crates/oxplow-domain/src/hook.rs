//! Hook events + agent status + agent turn types.
//!
//! These cross the wire from the Claude Code hook subprocess into the
//! oxplow daemon, get persisted, and feed the write guard. Pure data —
//! no IO.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{AgentSessionId, AgentTurnId, StreamId, ThreadId};
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
    /// The harness told the person something (Claude's `Notification`):
    /// a permission prompt waits on them.
    Notification,
    /// The harness asks the person to allow a tool (Claude's
    /// `PermissionRequest`). Observed only: the answer is always `Ack`, so
    /// the harness's own dialog decides.
    PermissionRequest,
    /// The harness put a plan to the person outside a tool call (Codex's
    /// `plan` item): `tool.plan` carries it. Claude's `ExitPlanMode` is a
    /// tool call and arrives as a `PreToolUse`.
    PlanProposed,
    /// A subagent of the session started (Claude's `SubagentStart`).
    SubagentStart,
    /// A subagent of the session finished (Claude's `SubagentStop`, or a
    /// hand-back).
    SubagentStop,
    /// The harness summarized the session's context (Claude's
    /// `PostCompact`; opencode's `session.compacted`, through its bridge;
    /// Codex's `contextCompaction` item, from the chat host).
    PostCompact,
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
    /// The thread's stream.
    pub stream_id: StreamId,
    /// The agent session it is the status of; `None` for activity no
    /// session claims (an agent oxplow didn't start).
    pub agent_session_id: Option<AgentSessionId>,
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
    /// The agent session it ran in; `None` for a turn no session claims
    /// (a hook from an agent oxplow didn't start).
    pub agent_session_id: Option<AgentSessionId>,
    pub prompt: String,
    pub answer: Option<String>,
    /// The harness's own session id, when it reported one.
    pub session_id: Option<String>,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    /// The snapshot the stream's worktree was at when the turn opened;
    /// `start_snapshot_id → snapshot_id` is what the turn changed.
    /// `None` when the stream had no snapshot yet.
    #[specta(type = Option<crate::JsNumber>)]
    pub start_snapshot_id: Option<i64>,
    /// The snapshot the worktree was at when the turn ended (its
    /// `turn_end` take); `None` while running.
    #[specta(type = Option<crate::JsNumber>)]
    pub snapshot_id: Option<i64>,
}

/// What a turn ran on and how full its context was, as its harness
/// reported it with the turn's end. Each part is `None` when the harness
/// said nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnRun {
    /// The model's id, as the harness names it.
    pub model: Option<String>,
    /// The reasoning effort, as the harness names it (`high`, `medium`, …).
    pub effort: Option<String>,
    /// Tokens in the context when the turn ended: the input of its last
    /// request, the cached part included.
    pub context_tokens: Option<i64>,
    /// The model's context window, in tokens.
    pub context_window: Option<i64>,
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
            stream_id: StreamId::new(1),
            agent_session_id: Some(AgentSessionId::new(2)),
            state: AgentStatusState::Running,
            detail: Some("typing".into()),
            updated_at: Timestamp::from_unix_ms(1),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: AgentStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
