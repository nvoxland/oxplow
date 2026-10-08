//! The processes behind agent sessions (`.context/agent-model.md`). An
//! `agent_session` row is the slot a person opened; its process — a PTY
//! running a harness, or an ACP agent — is started on demand by the UI and
//! stopped here when its slot closes. Commands own the slot (closing it is
//! a command); this owns the process.

use std::sync::Arc;

use oxplow_domain::AgentSessionId;

use crate::acp::manager::AcpManager;
use crate::terminal_sessions::{agent_pane_key, TerminalSessionRegistry};

/// Stops what runs in an agent session, whichever kind it is.
#[derive(Clone)]
pub struct SessionProcesses {
    acp: Arc<AcpManager>,
    terminals: TerminalSessionRegistry,
}

impl SessionProcesses {
    pub fn new(acp: Arc<AcpManager>, terminals: TerminalSessionRegistry) -> Self {
        Self { acp, terminals }
    }

    /// Stop agent session `session`'s process: its ACP agent (closed from
    /// this moment) or its PTY (killed on the runtime right after).
    /// Nothing running is fine — there is nothing to stop.
    pub fn kill(&self, session: AgentSessionId) {
        let _ = self.acp.close(&session);
        let terminals = self.terminals.clone();
        tokio::spawn(async move {
            terminals.close_key(&agent_pane_key(session)).await;
        });
    }
}
