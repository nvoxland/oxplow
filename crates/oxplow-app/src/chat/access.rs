//! A thread's write access, kept current on its running sessions'
//! connections: after any thread command (a promote demotes the writer
//! too), every running chat session re-reads what its thread may write
//! and a connection that confines its agent follows — Codex's sandbox is
//! its write guard (`.context/agent-model.md` "Codex: its TUI on a thread
//! oxplow observes").

use std::sync::{Arc, Weak};

use oxplow_domain::events::schema::{CommandExecuted, EventType as _};
use oxplow_domain::{DomainError, StoredEvent};

use crate::event_pump::AsyncEventConsumer;
use crate::Services;

/// The commands that can change which thread of a stream is its writer.
const THREAD_COMMANDS: &str = "oxplow.thread.";

pub struct AccessSync {
    services: Weak<Services>,
}

/// Register [`AccessSync`] on `svc`'s pump (boot, before it spawns).
pub fn register(svc: &Arc<Services>) {
    svc.event_pump.register_async(Arc::new(AccessSync {
        services: Arc::downgrade(svc),
    }));
}

/// Whether `event` is a thread command that ran.
fn changes_access(event: &StoredEvent) -> bool {
    event
        .envelope
        .payload
        .get("command")
        .and_then(|c| c.as_str())
        .is_some_and(|c| c.starts_with(THREAD_COMMANDS))
}

#[async_trait::async_trait]
impl AsyncEventConsumer for AccessSync {
    fn name(&self) -> &'static str {
        "chat.access"
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == CommandExecuted::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        if let (true, Some(svc)) = (changes_access(event), self.services.upgrade()) {
            svc.chat.sync_access();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executed(command: &str) -> StoredEvent {
        let envelope = oxplow_domain::Envelope::new(
            CommandExecuted::TYPE,
            CommandExecuted::V,
            "human",
            serde_json::json!({ "command": command }),
        )
        .unwrap();
        StoredEvent {
            seq: 1,
            envelope,
            payload_expired_at: None,
        }
    }

    #[test]
    fn a_thread_command_changes_access_and_others_dont() {
        assert!(changes_access(&executed("oxplow.thread.promote")));
        assert!(changes_access(&executed("oxplow.thread.close")));
        assert!(!changes_access(&executed("oxplow.work_item.transition")));
    }
}
