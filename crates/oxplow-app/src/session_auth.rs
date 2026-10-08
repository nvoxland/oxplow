//! Who an agent's request comes from (`.context/agent-model.md` "Caller
//! identity"). Every agent session gets its own bearer when it launches,
//! bound here to the session, its thread and stream, and its harness. The
//! control plane's hook, OTLP and MCP routes take the caller from the
//! bearer alone: nothing a request says about itself — no header, no
//! query — names who it is, so one agent can't act as another.
//!
//! Tokens live in memory. A session's process ends with the daemon, and its
//! next launch mints a new one, so nothing needs to outlive a boot.

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

    /// A new bearer for `principal`'s session. A session holds one at a
    /// time: minting again (a relaunch) retires the one before.
    pub fn mint(&self, principal: Principal) -> String {
        let token = generate_token();
        let mut tokens = self.by_token.write().unwrap_or_else(|p| p.into_inner());
        tokens.retain(|_, p| p.session != principal.session);
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
    fn a_minted_bearer_authenticates_as_its_session() {
        let auth = SessionAuth::new();
        let token = auth.mint(principal(3, 2));
        assert_eq!(token.len(), 43);
        assert_eq!(auth.authenticate(&token), Some(principal(3, 2)));
        assert_eq!(auth.authenticate("not-a-token"), None);
    }

    #[test]
    fn each_session_has_its_own_bearer() {
        let auth = SessionAuth::new();
        let a = auth.mint(principal(3, 2));
        let b = auth.mint(principal(4, 2));
        assert_ne!(a, b);
        assert_eq!(auth.authenticate(&a).map(|p| p.session.value()), Some(3));
        assert_eq!(auth.authenticate(&b).map(|p| p.session.value()), Some(4));
    }

    #[test]
    fn minting_again_retires_the_earlier_bearer() {
        let auth = SessionAuth::new();
        let first = auth.mint(principal(3, 2));
        let second = auth.mint(principal(3, 2));
        assert_eq!(auth.authenticate(&first), None);
        assert!(auth.authenticate(&second).is_some());
    }

    #[test]
    fn a_revoked_session_authenticates_no_more() {
        let auth = SessionAuth::new();
        let token = auth.mint(principal(3, 2));
        let other = auth.mint(principal(4, 2));
        auth.revoke(AgentSessionId::new(3));
        assert_eq!(auth.authenticate(&token), None);
        assert!(auth.authenticate(&other).is_some());
    }
}
