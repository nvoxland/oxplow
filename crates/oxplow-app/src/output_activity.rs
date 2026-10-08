//! Per-session PTY output liveness.
//!
//! The hook log is the authoritative record of *state changes*, but
//! hooks are sparse within a single turn: a long turn streams tokens to
//! the terminal for many minutes while emitting no Pre/PostToolUse
//! between tool calls. Read against the wall clock, that frozen log
//! looks like death even though the agent is plainly working (tsk141).
//!
//! This tracker captures the missing cadence signal — the last time the
//! agent's PTY produced output — so the stall watchdog can tell a busy
//! long turn (output still advancing) from a genuinely-dead one (output
//! quiet too). It's deliberately tiny: one timestamp per agent session,
//! overwritten on every output burst, never persisted. The terminal
//! forwarder writes it; [`crate::agent_stall_watch`] reads it.
//!
//! Keyed by agent session: each session's PTY is its own agent, and two
//! sessions in one thread live and die apart. Shell panes, and agent
//! panes no session claims, never record here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use oxplow_domain::{AgentSessionId, Timestamp};

/// Cloneable handle to the shared last-output-per-session map. Cheap to
/// clone (an `Arc`); all clones see the same state.
#[derive(Clone, Default)]
pub struct OutputActivity {
    inner: Arc<Mutex<HashMap<AgentSessionId, Timestamp>>>,
}

impl OutputActivity {
    pub fn new() -> Self {
        Self::default()
    }

    /// Note that `session`'s agent produced PTY output at `at`. Keeps the latest
    /// timestamp seen — out-of-order or stale records never move it
    /// backwards.
    pub fn record(&self, session: AgentSessionId, at: Timestamp) {
        let mut m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let slot = m.entry(session).or_insert(at);
        if at > *slot {
            *slot = at;
        }
    }

    /// The most recent output timestamp for `session`, if any has been
    /// recorded since boot.
    pub fn last(&self, session: &AgentSessionId) -> Option<Timestamp> {
        let m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        m.get(session).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_unix_ms(ms)
    }

    #[test]
    fn an_unrecorded_session_is_none() {
        let a = OutputActivity::new();
        assert_eq!(a.last(&AgentSessionId::new(1)), None);
    }

    #[test]
    fn record_then_last_returns_it() {
        let a = OutputActivity::new();
        a.record(AgentSessionId::new(1), at(100));
        assert_eq!(a.last(&AgentSessionId::new(1)), Some(at(100)));
    }

    #[test]
    fn keeps_the_latest_timestamp() {
        let a = OutputActivity::new();
        a.record(AgentSessionId::new(1), at(100));
        a.record(AgentSessionId::new(1), at(50)); // stale, must not regress
        a.record(AgentSessionId::new(1), at(200));
        assert_eq!(a.last(&AgentSessionId::new(1)), Some(at(200)));
    }

    #[test]
    fn sessions_are_independent() {
        let a = OutputActivity::new();
        a.record(AgentSessionId::new(1), at(100));
        a.record(AgentSessionId::new(2), at(300));
        assert_eq!(a.last(&AgentSessionId::new(1)), Some(at(100)));
        assert_eq!(a.last(&AgentSessionId::new(2)), Some(at(300)));
    }
}
