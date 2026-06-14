//! Per-thread PTY output liveness.
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
//! quiet too). It's deliberately tiny: one timestamp per thread,
//! overwritten on every output burst, never persisted. The terminal
//! forwarder writes it; [`crate::agent_stall_watch`] reads it.
//!
//! Keyed by `ThreadId` rather than pane_target on purpose: the agent's
//! working and talking panes are both signs the thread is alive, and
//! the watchdog only needs "is this thread's agent still emitting
//! anything". Shell panes are not thread-scoped and never record here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use oxplow_domain::{ThreadId, Timestamp};

/// Cloneable handle to the shared last-output-per-thread map. Cheap to
/// clone (an `Arc`); all clones see the same state.
#[derive(Clone, Default)]
pub struct OutputActivity {
    inner: Arc<Mutex<HashMap<ThreadId, Timestamp>>>,
}

impl OutputActivity {
    pub fn new() -> Self {
        Self::default()
    }

    /// Note that `thread` produced PTY output at `at`. Keeps the latest
    /// timestamp seen — out-of-order or stale records never move it
    /// backwards.
    pub fn record(&self, thread: ThreadId, at: Timestamp) {
        let mut m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let slot = m.entry(thread).or_insert(at);
        if at > *slot {
            *slot = at;
        }
    }

    /// The most recent output timestamp for `thread`, if any has been
    /// recorded since boot.
    pub fn last(&self, thread: &ThreadId) -> Option<Timestamp> {
        let m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        m.get(thread).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_unix_ms(ms)
    }

    #[test]
    fn unrecorded_thread_is_none() {
        let a = OutputActivity::new();
        assert_eq!(a.last(&ThreadId::new(1)), None);
    }

    #[test]
    fn record_then_last_returns_it() {
        let a = OutputActivity::new();
        a.record(ThreadId::new(1), at(100));
        assert_eq!(a.last(&ThreadId::new(1)), Some(at(100)));
    }

    #[test]
    fn keeps_the_latest_timestamp() {
        let a = OutputActivity::new();
        a.record(ThreadId::new(1), at(100));
        a.record(ThreadId::new(1), at(50)); // stale, must not regress
        a.record(ThreadId::new(1), at(200));
        assert_eq!(a.last(&ThreadId::new(1)), Some(at(200)));
    }

    #[test]
    fn threads_are_independent() {
        let a = OutputActivity::new();
        a.record(ThreadId::new(1), at(100));
        a.record(ThreadId::new(2), at(300));
        assert_eq!(a.last(&ThreadId::new(1)), Some(at(100)));
        assert_eq!(a.last(&ThreadId::new(2)), Some(at(300)));
    }
}
