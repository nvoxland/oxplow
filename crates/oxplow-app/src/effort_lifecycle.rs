//! The effort-lifecycle consumer (P2.6.2, tsk454; `.context/data-model.md`
//! "event_log"): the post-commit half of opening and closing an effort,
//! driven by the `effort.opened@1` / `effort.closed@1` events the effort
//! store logs in the same transaction as the write.
//!
//! It runs on the event pump as an async consumer, so the work is durable:
//! a crash between the commit and the snapshot re-delivers the event on
//! the next run instead of leaving the effort unpinned. The work itself
//! lives on [`TaskService`] (`on_effort_opened` / `on_effort_closed`),
//! which is idempotent under re-delivery.

use async_trait::async_trait;
use oxplow_domain::{DomainError, EffortId, StoredEvent};

use crate::event_pump::AsyncEventConsumer;
use crate::task_service::TaskService;

pub struct EffortLifecycleConsumer {
    tasks: TaskService,
}

impl EffortLifecycleConsumer {
    pub fn new(tasks: TaskService) -> Self {
        Self { tasks }
    }
}

/// The effort an `effort.*` event is about (its `effort` payload ref).
fn effort_of(event: &StoredEvent) -> Result<EffortId, DomainError> {
    let r = event.envelope.payload["effort"]
        .as_str()
        .unwrap_or_default();
    r.strip_prefix("effort:")
        .and_then(EffortId::try_from_str)
        .ok_or_else(|| {
            DomainError::Invalid(format!(
                "{} seq {}: `effort` is not an effort ref: `{r}`",
                event.envelope.event_type, event.seq
            ))
        })
}

#[async_trait]
impl AsyncEventConsumer for EffortLifecycleConsumer {
    fn name(&self) -> &'static str {
        "effort.lifecycle"
    }

    fn handles(&self, event_type: &str) -> bool {
        matches!(event_type, "effort.opened" | "effort.closed")
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let effort = effort_of(event)?;
        let retroactive = event.envelope.payload["retroactive"]
            .as_bool()
            .unwrap_or(false);
        match event.envelope.event_type.as_str() {
            // A retroactive open is closed in the same transaction; only its
            // close has work (the metrics).
            "effort.opened" if !retroactive => self.tasks.on_effort_opened(effort).await,
            "effort.closed" => self.tasks.on_effort_closed(effort, retroactive).await,
            _ => Ok(()),
        }
    }
}
