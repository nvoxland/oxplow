//! Telling a session that what its agent would be told may have changed.
//!
//! A harness with no prompt hook (Codex) is given its context by
//! injection, between turns. The context changes while no turn runs — a
//! handoff sent to the thread, another agent arriving in its worktree, a
//! decision recorded — and the agent should have it before the person's
//! next prompt, not after that turn ends. This pump consumer bumps
//! [`Services::context_changes`](crate::Services) when the log records
//! one of the facts the context reads; each session that watches it builds
//! its context again, and the "once, then on change" baselines
//! (`AgentContext`) make an unchanged build inject nothing. No polling.

use oxplow_domain::{DomainError, StoredEvent};
use tokio::sync::watch;

use crate::event_pump::AsyncEventConsumer;

pub const NAME: &str = "agent.context_changes";

/// The command whose record is a decision the context lists.
const RECORD_DECISION: &str = "oxplow.effort.record_decision";

/// The event types whose facts the context reads: the handoffs and their
/// count, the agents open in a worktree, and the open effort and its
/// decisions.
pub fn changes_context(event_type: &str) -> bool {
    matches!(
        event_type,
        "handoff.sent"
            | "handoff.settled"
            | "agent.session.started"
            | "agent.session.ended"
            | "effort.opened"
            | "effort.closed"
            | "effort.finished"
            | "effort.linked"
            | "effort.retitled"
            | "effort.decision_reviewed"
            | "effort.decisions_inferred"
            | "command.executed"
    )
}

pub struct ContextChanges {
    pub changed: watch::Sender<u64>,
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
        // Any command is logged; only the one that records a decision
        // changes the context.
        if event.envelope.event_type == "command.executed"
            && payload["command"].as_str() != Some(RECORD_DECISION)
        {
            return Ok(());
        }
        self.changed.send_modify(|n| *n += 1);
        Ok(())
    }
}
