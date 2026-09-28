//! Agent implementation identifiers.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum AgentKind {
    #[default]
    Claude,
    Codex,
    Opencode,
    /// An agent spoken to over the Agent Client Protocol (tsk335); which one
    /// is the thread's `acp_agent`.
    Acp,
}

impl AgentKind {
    /// Runs in a terminal (a PTY / tmux pane), as opposed to ACP.
    pub fn is_terminal(self) -> bool {
        !matches!(self, AgentKind::Acp)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Opencode => "opencode",
            AgentKind::Acp => "acp",
        }
    }
}
