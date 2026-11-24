//! The processes behind agent sessions (`.context/agent-model.md`). An
//! `agent_session` row is the slot a person opened; its process — a PTY
//! running a harness, or a Codex observer — is started on demand by the UI and
//! stopped here when its slot closes. Commands own the slot (closing it is
//! a command); this owns the process.

use std::sync::Arc;

use oxplow_domain::AgentSessionId;

use crate::agent_observer::manager::ObserverManager;
use crate::session_auth::SessionAuth;
use crate::terminal_sessions::{agent_pane_key, TerminalSessionRegistry};

/// Stops what runs in an agent session, whichever kind it is.
#[derive(Clone)]
pub struct SessionProcesses {
    observer: Arc<ObserverManager>,
    terminals: TerminalSessionRegistry,
    auth: Arc<SessionAuth>,
}

impl SessionProcesses {
    pub fn new(
        observer: Arc<ObserverManager>,
        terminals: TerminalSessionRegistry,
        auth: Arc<SessionAuth>,
    ) -> Self {
        Self {
            observer,
            terminals,
            auth,
        }
    }

    /// Stop agent session `session`'s process: its Codex observer (closed from
    /// this moment) or its PTY (killed on the runtime right after), and
    /// retire its bearer. Nothing running is fine — there is nothing to
    /// stop.
    pub fn kill(&self, session: AgentSessionId) {
        self.invalidate(session);
        let processes = self.clone();
        tokio::spawn(async move {
            let _launch = processes.terminals.lock_agent_launch(session).await;
            // A launch already in flight can have issued a new bearer or
            // opened an observer since the slot closed.
            processes.invalidate(session);
            processes
                .terminals
                .close_key(&agent_pane_key(session))
                .await;
        });
    }

    fn invalidate(&self, session: AgentSessionId) {
        self.auth.revoke(session);
        let _ = self.observer.close(&session);
    }
}
