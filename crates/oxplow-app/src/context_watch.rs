//! Telling a session that what its agent would be told may have changed.
//!
//! A harness with no prompt hook (Codex) is given its context by
//! injection, between turns. The context changes while no turn runs —
//! another agent arriving in its worktree, a decision recorded, something put in a context basket or a basket set to go next — and the agent should have it before the person's
//! next prompt, not after that turn ends. While an enabled extension
//! declares an agent context (`context:`), whose query may read anything,
//! every command counts as such a change. This pump consumer bumps
//! [`Services::context_changes`](crate::Services) when the log records
//! one of the facts the context reads; each session that watches it builds
//! its context again, and the "once, then on change" baselines
//! (`AgentContext`) make an unchanged build inject nothing. No polling.

use oxplow_domain::{DomainError, StoredEvent};
use tokio::sync::watch;

use crate::event_pump::AsyncEventConsumer;

pub const NAME: &str = "agent.context_changes";

/// The commands whose runs change what the context lists: an item put in
/// or moved into a basket, a basket set to go with a thread's next prompt.
const CHANGING_COMMANDS: &[&str] = &[
    crate::commands::context_basket::ADD,
    crate::commands::context_basket::MOVE,
    crate::commands::context_basket::SEND_NEXT,
    crate::commands::context_basket::PUT_BACK,
];

/// The event types whose facts the context reads: the agents open in a
/// worktree, and the commands that change a basket.
pub fn changes_context(event_type: &str) -> bool {
    matches!(
        event_type,
        "agent.session.started" | "agent.session.ended" | "command.executed"
    )
}

pub struct ContextChanges {
    pub changed: watch::Sender<u64>,
    /// Whether an enabled extension declares an agent context now.
    pub contexts_declared: std::sync::Arc<dyn Fn() -> bool + Send + Sync>,
}

#[async_trait::async_trait]
impl AsyncEventConsumer for ContextChanges {
    fn name(&self) -> &'static str {
        NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        changes_context(event_type)
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let payload = &event.envelope.payload;
        // Any command is logged; only a few change the context, unless an
        // extension's context may read what any of them changed.
        if event.envelope.event_type == "command.executed"
            && !payload["command"]
                .as_str()
                .is_some_and(|c| CHANGING_COMMANDS.contains(&c))
            && !(self.contexts_declared)()
        {
            return Ok(());
        }
        self.changed.send_modify(|n| *n += 1);
        Ok(())
    }
}
