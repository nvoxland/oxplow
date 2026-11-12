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
    /// What opened it; `None` for a turn recorded before sources were.
    pub source: Option<TurnSource>,
}

/// What a hook's body means, as its harness read it: the harness adapter
/// says (core reads no harness's body keys). Each part is empty when the
/// harness said nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookReading {
    /// Where the harness keeps the session's transcript.
    pub transcript: Option<String>,
    /// A `Stop` / `SubagentStop`'s final message: the agent's own last words.
    pub final_message: Option<String>,
    /// A `Notification`: the agent waits on the person (a permission, a
    /// form, input).
    pub waits_on_person: bool,
    /// A `Notification`'s text.
    pub notice: Option<String>,
    /// A `SessionStart` that is a context compaction posted mid-turn: the
    /// same process and turn go on.
    pub compaction: bool,
    /// A `PostCompact`'s summary of the context, when the harness gives one.
    pub compact_summary: Option<String>,
    /// What triggered a `PostCompact` (`auto`, `manual`), as the harness
    /// names it.
    pub compact_trigger: Option<String>,
    /// A `SessionEnd`'s reason, as the harness names it.
    pub end_reason: Option<String>,
    /// A `SessionEnd` that clears the conversation: the next session starts
    /// a new one.
    pub clears_conversation: bool,
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
    /// The work the agent had started that was still running when the
    /// turn ended (Claude Code's Stop `background_tasks`); `None` when the
    /// harness says nothing, empty when it says none.
    pub left_running: Option<Vec<RunningWork>>,
    /// How the harness says the turn ended, when it is not simply finished
    /// (an abort is `Interrupted`, a failure `Errored`); `None` reads as
    /// `Completed`.
    pub outcome: Option<TurnOutcome>,
    /// The harness's own words for an error that ended it (`name: message`).
    pub error: Option<String>,
}

/// A job or subagent a turn left running, as its harness reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
pub struct RunningWork {
    /// The harness's id for it: the one its report back names (Claude's
    /// task id, a subagent's id).
    pub id: String,
    /// What it is, in the harness's word (`subagent`, `shell`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// What it is doing, as the harness described it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// What opened a turn (`agent_turn.source`), as its harness read the
/// prompt hook. Only `person` is a person's words; the others are what the
/// harness delivered to the agent on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnSource {
    /// A person's prompt.
    Person,
    /// A job the agent started in the background reported back (Claude
    /// Code's task notification: a background shell or subagent finished).
    BackgroundJob,
    /// A subagent handed its report back to its session.
    Subagent,
    /// Another agent session's message.
    Peer,
    /// The harness opened a turn and said nothing of why.
    Unknown,
}

impl TurnSource {
    /// Its stored spelling (`agent_turn.source`).
    pub fn as_str(self) -> &'static str {
        match self {
            TurnSource::Person => "person",
            TurnSource::BackgroundJob => "background_job",
            TurnSource::Subagent => "subagent",
            TurnSource::Peer => "peer",
            TurnSource::Unknown => "unknown",
        }
    }

    /// The source a stored spelling names; `None` for a turn recorded
    /// before sources were (NULL) or an unknown spelling.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "person" => TurnSource::Person,
            "background_job" => TurnSource::BackgroundJob,
            "subagent" => TurnSource::Subagent,
            "peer" => TurnSource::Peer,
            "unknown" => TurnSource::Unknown,
            _ => return None,
        })
    }
}

/// How a turn's cause is known (`agent_turn.cause_basis`): the harness
/// named it (`reported`), or oxplow matched it from what it saw
/// (`matched`). Best effort either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CauseBasis {
    Reported,
    Matched,
}

impl CauseBasis {
    /// Its stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            CauseBasis::Reported => "reported",
            CauseBasis::Matched => "matched",
        }
    }
}

/// How a turn ended (`agent.turn.ended`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    /// The agent finished (Stop).
    Completed,
    /// The person interrupted it (an abort in the harness).
    Interrupted,
    /// The harness ended it with an error (an API failure, a refused
    /// request): `agent_turn.error` holds the harness's own words.
    Errored,
    /// Its harness session ended with the turn still open (an exit sends
    /// no Stop).
    SessionEnded,
    /// A new harness process started with the turn still open: the one
    /// that ran it is gone.
    SessionRestarted,
    /// oxplow closed the agent session with the turn still open.
    SessionClosed,
    /// oxplow restarted with the turn still open; recovery closed it.
    Restart,
}

impl TurnOutcome {
    /// Its stored spelling (`agent_turn.outcome`).
    pub fn as_str(self) -> &'static str {
        match self {
            TurnOutcome::Completed => "completed",
            TurnOutcome::Interrupted => "interrupted",
            TurnOutcome::Errored => "errored",
            TurnOutcome::SessionEnded => "session_ended",
            TurnOutcome::SessionRestarted => "session_restarted",
            TurnOutcome::SessionClosed => "session_closed",
            TurnOutcome::Restart => "restart",
        }
    }
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
