//! oxplow's own vocabulary for an agent's tool calls
//! (`.context/agent-model.md` "Tool vocabulary"). Every harness names its
//! tools its own way — Claude Code's `Edit` and `Bash`, Codex's
//! `apply_patch` and `shell`, an ACP agent's tool kinds — and maps each
//! call onto a [`ToolUse`] (`AgentHarness::tool_use`). Core reads only
//! the [`ToolKind`] and the fields here: the write guard, a turn's
//! claimed files, the agent's status and test-run collection never name a
//! harness's tool.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What a tool call does, as oxplow reads it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// Reads a file.
    Read,
    /// Writes, edits, deletes or moves files — the call names them.
    Edit,
    /// Runs a shell command.
    Shell,
    /// Starts a subagent, which runs tools of its own.
    Subagent,
    /// Asks the person a question and waits for the answer.
    Ask,
    /// Puts a plan to the person and waits for approval.
    Plan,
    /// Searches files (names or contents).
    Search,
    /// Fetches or searches the web.
    Fetch,
    /// Calls an MCP server's tool.
    Mcp,
    /// Anything else (a to-do list, a mode switch, thinking).
    #[default]
    Other,
}

impl ToolKind {
    /// Whether a call of this kind can change the worktree: a file edit, a
    /// shell command, or a subagent (which runs its own tools). Every other
    /// kind only reads or talks.
    pub fn changes_worktree(self) -> bool {
        matches!(self, ToolKind::Edit | ToolKind::Shell | ToolKind::Subagent)
    }

    /// Whether the call waits on the person (a question, a plan).
    pub fn waits_on_person(self) -> bool {
        matches!(self, ToolKind::Ask | ToolKind::Plan)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ToolKind::Read => "read",
            ToolKind::Edit => "edit",
            ToolKind::Shell => "shell",
            ToolKind::Subagent => "subagent",
            ToolKind::Ask => "ask",
            ToolKind::Plan => "plan",
            ToolKind::Search => "search",
            ToolKind::Fetch => "fetch",
            ToolKind::Mcp => "mcp",
            ToolKind::Other => "other",
        }
    }
}

/// A subagent an agent session started (Claude Code's `Agent`/`Task`, a
/// Codex spawned agent, an opencode child session): not a session of its
/// own — it has no bearer and no process oxplow knows — but who a tool
/// call or a hand-back came from inside its parent session.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Subagent {
    /// The harness's id for it (Claude's `agent_id`, opencode's child
    /// session id), or the starting call's id when the harness gives none.
    pub id: String,
    /// What kind of subagent the harness says it is (`general-purpose`,
    /// `explore`), when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// A plan the agent proposes to the person before it acts (Claude's
/// `ExitPlanMode`, Codex's `plan` item). Whether the person accepted it is
/// in no harness's hooks, so it isn't known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    pub text: String,
    /// The file the harness saved it in, when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// One tool call, as its harness maps it: the harness's own name for the
/// tool, and what oxplow reads of it. On the wire, these fields; one left
/// out is empty.
/// A message one agent session sends another, as its harness reports the
/// call that sends it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct SentMessage {
    /// The recipient, in the harness's words (a session's name).
    pub to: String,
    /// The message's text.
    pub text: String,
    /// Whether the sender asked to be told when the recipient is idle
    /// (Claude Code's `notify_when_idle`).
    #[serde(default)]
    pub notify_when_idle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ToolUse {
    /// The harness's name for the tool (`Edit`, `apply_patch`), as it
    /// reports it — shown and recorded, never matched by core.
    pub name: String,
    pub kind: ToolKind,
    /// The files it names, as given (absolute or worktree-relative). An
    /// edit's are the files it changes.
    pub paths: Vec<String>,
    /// A shell call's command line, whole.
    pub command: Option<String>,
    /// A short summary for a person: a search's pattern, a fetch's URL, a
    /// question. A shell call's is its command.
    pub detail: Option<String>,
    /// The harness's id for this call, the same on its start and finish.
    pub call_id: Option<String>,
    /// How long it ran, in milliseconds, as the harness timed it (Claude
    /// Code's PostToolUse `duration_ms`), on a finished call.
    pub duration_ms: Option<u64>,
    /// A message the call sends to another agent session (Claude Code's
    /// `SendMessage`): who to, and its text as the receiver will see it.
    pub sent: Option<SentMessage>,
    /// The harness's id of the background job this call started (Claude
    /// Code's Bash `backgroundTaskId`, on its PostToolUse): the job is
    /// known from this call on, the id its Stop and its report name.
    pub started_job: Option<String>,
    /// Whether it succeeded, when the harness said (a finished call).
    pub ok: Option<bool>,
    /// A shell call's exit code, when the harness said.
    pub exit_code: Option<i64>,
    /// What an `Ask` asks: its first question.
    pub question: Option<String>,
    /// What a `Plan` call proposes: its text, and the file it was saved in.
    pub plan: Option<Plan>,
    /// The call's input as the harness gave it, recorded by hash with the
    /// call (Claude's `tool_input`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    /// What the call returned once it finished, recorded by hash (Claude's
    /// `tool_response`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
    /// The subagent that made the call, when it wasn't the session's own
    /// agent (the harness says, or core brackets it by the open subagent
    /// call).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent: Option<Subagent>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_edits_shell_commands_and_subagents_change_the_worktree() {
        for kind in [ToolKind::Edit, ToolKind::Shell, ToolKind::Subagent] {
            assert!(kind.changes_worktree(), "{kind:?}");
        }
        for kind in [
            ToolKind::Read,
            ToolKind::Ask,
            ToolKind::Plan,
            ToolKind::Search,
            ToolKind::Fetch,
            ToolKind::Mcp,
            ToolKind::Other,
        ] {
            assert!(!kind.changes_worktree(), "{kind:?}");
        }
        assert!(ToolKind::Ask.waits_on_person() && ToolKind::Plan.waits_on_person());
        assert!(!ToolKind::Shell.waits_on_person());
    }

    #[test]
    fn a_kind_is_its_snake_case_name_on_the_wire() {
        for kind in [ToolKind::Edit, ToolKind::Subagent, ToolKind::Mcp] {
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::json!(kind.as_str())
            );
        }
    }
}
