//! Agents: the harness interfaces core keeps (`.context/agent-model.md`),
//! and — until the harness registry replaces it — the `AgentKind` enum.
//!
//! An agent session runs a **harness** (`harness::AgentHarness`): what
//! launches its process, what its hooks mean, how a person interacts with
//! it. Implementations are declared (`agent_harness` built-ins), many at
//! once, and looked up by the session's harness key
//! (`registry::HarnessRegistry`). An ACP agent's program is data
//! (`acp_adapter::AcpAdapter`). Driving an agent — oxplow prompting it — is
//! an interface only (`drive::Drive`): oxplow never drives an agent through
//! a surface licensed for interactive use.

pub mod acp_adapter;
pub mod drive;
pub mod harness;
pub mod registry;

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Type,
    schemars::JsonSchema,
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
    /// Runs in a terminal (a PTY), as opposed to ACP.
    pub fn is_terminal(self) -> bool {
        !matches!(self, AgentKind::Acp)
    }

    /// The kind [`AgentKind::as_str`] names.
    pub fn parse(s: &str) -> Option<Self> {
        [
            AgentKind::Claude,
            AgentKind::Codex,
            AgentKind::Opencode,
            AgentKind::Acp,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
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

/// How much a status asks of the person, for [`roll_up_status`]: what they
/// owe ranks first (an answer, then the next move after a dead turn), then
/// work in flight, then the quiet states.
fn attention(state: crate::AgentStatusState) -> u8 {
    use crate::AgentStatusState as S;
    match state {
        S::AwaitingUser => 5,
        S::Stalled => 4,
        S::Running => 3,
        S::Error => 2,
        S::Stopped => 1,
        S::Idle => 0,
    }
}

/// One status for many: a thread's from its sessions', a stream's from its
/// threads'. `awaiting_user > stalled > running > error > stopped > idle`;
/// `None` when there is nothing to roll up. The desktop's
/// `rollUpAgentStatus` and the `v_agent_status` model follow the same rule
/// (`fixtures/agent_status_rollup.json`).
pub fn roll_up_status(
    states: impl IntoIterator<Item = crate::AgentStatusState>,
) -> Option<crate::AgentStatusState> {
    states.into_iter().max_by_key(|s| attention(*s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentStatusState;

    /// The roll-up rule, as the shared fixture states it (the desktop's
    /// `rollUpAgentStatus` reads the same file).
    #[test]
    fn statuses_roll_up_as_the_shared_fixture_says() {
        #[derive(serde::Deserialize)]
        struct Case {
            states: Vec<AgentStatusState>,
            expect: Option<AgentStatusState>,
        }
        #[derive(serde::Deserialize)]
        struct Fixture {
            cases: Vec<Case>,
        }
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../fixtures/agent_status_rollup.json")).unwrap();
        for case in fixture.cases {
            assert_eq!(
                roll_up_status(case.states.iter().copied()),
                case.expect,
                "{:?}",
                case.states
            );
        }
    }
}
