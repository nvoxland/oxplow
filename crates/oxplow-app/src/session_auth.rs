//! Who an agent's request comes from (`.context/agent-model.md` "Caller
//! identity"). Every agent session gets its own bearer when it launches,
//! bound here to the session, its thread and stream, and its harness. The
//! control plane's hook, OTLP and MCP routes take the caller from the
//! bearer alone: nothing a request says about itself — no header, no
//! query — names who it is, so one agent can't act as another.
//!
//! A session holds one bearer for as long as the daemon runs: every launch
//! of it is handed the same one, because a launch can end up attaching to
//! the process already running (a tab shown again), which keeps the bearer
//! it started with. Closing the session revokes it. Tokens live in memory;
//! a session's process ends with the daemon, so nothing outlives a boot.

use std::collections::HashMap;
use std::sync::RwLock;

use base64::Engine as _;
use oxplow_domain::{AgentSessionId, StreamId, ThreadId};

/// The agent session a bearer stands for, as the launch bound it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub session: AgentSessionId,
    pub thread: ThreadId,
    pub stream: StreamId,
    /// Its harness's registry key: how its hooks' answers are rendered.
    pub harness: String,
}

/// The sessions' bearers.
#[derive(Debug, Default)]
pub struct SessionAuth {
    by_token: RwLock<HashMap<String, Principal>>,
}

impl SessionAuth {
    pub fn new() -> Self {
        Self::default()
    }

    /// `principal`'s session's bearer: the one it already holds, else a
    /// new one. A session that moved (another thread, harness) gets a new
    /// one, and the old one stops working.
    pub fn issue(&self, principal: Principal) -> String {
        let mut tokens = self.by_token.write().unwrap_or_else(|p| p.into_inner());
        if let Some(token) = tokens
            .iter()
            .find(|(_, p)| **p == principal)
            .map(|(t, _)| t.clone())
        {
            return token;
        }
        tokens.retain(|_, p| p.session != principal.session);
        let token = generate_token();
        tokens.insert(token.clone(), principal);
        token
    }

    /// The session `bearer` stands for, if it's a live one.
    pub fn authenticate(&self, bearer: &str) -> Option<Principal> {
        self.by_token
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(bearer)
            .cloned()
    }

    /// Retire `session`'s bearer: its process stopped.
    pub fn revoke(&self, session: AgentSessionId) {
        self.by_token
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|_, p| p.session != session);
    }
}

/// 32 random bytes, base64url without padding (43 characters).
fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(session: i64, thread: i64) -> Principal {
        Principal {
            session: AgentSessionId::new(session),
            thread: ThreadId::new(thread),
            stream: StreamId::new(1),
            harness: "claude".into(),
        }
    }

    #[test]
    fn an_issued_bearer_authenticates_as_its_session() {
        let auth = SessionAuth::new();
        let token = auth.issue(principal(3, 2));
        assert_eq!(token.len(), 43);
        assert_eq!(auth.authenticate(&token), Some(principal(3, 2)));
        assert_eq!(auth.authenticate("not-a-token"), None);
    }

    #[test]
    fn each_session_has_its_own_bearer() {
        let auth = SessionAuth::new();
        let a = auth.issue(principal(3, 2));
        let b = auth.issue(principal(4, 2));
        assert_ne!(a, b);
        assert_eq!(auth.authenticate(&a).map(|p| p.session.value()), Some(3));
        assert_eq!(auth.authenticate(&b).map(|p| p.session.value()), Some(4));
    }

    /// A relaunch (or a tab attaching again to the running process) gets
    /// the bearer the session holds, so the process keeps working.
    #[test]
    fn issuing_again_hands_back_the_sessions_bearer() {
        let auth = SessionAuth::new();
        let first = auth.issue(principal(3, 2));
        let second = auth.issue(principal(3, 2));
        assert_eq!(first, second);
        assert!(auth.authenticate(&first).is_some());
    }

    /// A session that moved to another thread holds a new bearer; the old
    /// one names who it no longer is.
    #[test]
    fn a_session_that_moved_gets_a_new_bearer() {
        let auth = SessionAuth::new();
        let first = auth.issue(principal(3, 2));
        let moved = auth.issue(principal(3, 9));
        assert_ne!(first, moved);
        assert_eq!(auth.authenticate(&first), None);
        assert_eq!(auth.authenticate(&moved).map(|p| p.thread.value()), Some(9));
    }

    #[test]
    fn a_revoked_session_authenticates_no_more() {
        let auth = SessionAuth::new();
        let token = auth.issue(principal(3, 2));
        let other = auth.issue(principal(4, 2));
        auth.revoke(AgentSessionId::new(3));
        assert_eq!(auth.authenticate(&token), None);
        assert!(auth.authenticate(&other).is_some());
    }
}
