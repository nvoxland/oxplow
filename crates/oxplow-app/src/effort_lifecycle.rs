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
use oxplow_db::SqliteEventLogStore;
use oxplow_domain::events::schema::{EffortFinished, EffortFinishedV2};
use oxplow_domain::refs::build::{snapshot_ref, system_source};
use oxplow_domain::{DomainError, EffortId, Envelope, StoredEvent};

use crate::event_pump::AsyncEventConsumer;
use crate::task_service::TaskService;

/// The consumer's name (its checkpoint key; what callers settle on).
pub const NAME: &str = "effort.lifecycle";

pub struct EffortLifecycleConsumer {
    tasks: TaskService,
    log: SqliteEventLogStore,
    /// To let the claim reactor catch up before a close reconciles.
    pump: Option<std::sync::Weak<crate::event_pump::EventPump>>,
}

/// How long a close waits for pending edit claims before it reconciles.
const CLAIM_SETTLE: std::time::Duration = std::time::Duration::from_secs(10);

impl EffortLifecycleConsumer {
    pub fn new(tasks: TaskService, log: SqliteEventLogStore) -> Self {
        Self {
            tasks,
            log,
            pump: None,
        }
    }

    pub fn with_pump(mut self, pump: std::sync::Weak<crate::event_pump::EventPump>) -> Self {
        self.pump = Some(pump);
        self
    }

    /// A close compares what the effort claimed with what changed; the
    /// claims of its last edits may still be on the `effort.claim`
    /// reactor's queue. Let it catch up first (bounded: a timeout
    /// reconciles with what's there, the claim still lands and clears the
    /// path from the unattributed list).
    async fn settle_claims(&self) {
        if let Some(pump) = self.pump.as_ref().and_then(|p| p.upgrade()) {
            pump.settle(&[crate::tool_call_reactors::EFFORT_CLAIM], CLAIM_SETTLE)
                .await;
        }
    }

    /// Log `effort.finished@1` for a close this consumer finished handling.
    /// Its dedupe key makes a re-delivery's second append a no-op.
    async fn log_finished(
        &self,
        closed: &StoredEvent,
        effort: &oxplow_db::Effort,
        retroactive: bool,
    ) -> Result<(), DomainError> {
        let env = Envelope::typed::<EffortFinished>(
            system_source(NAME),
            &EffortFinishedV2 {
                effort: closed.envelope.payload["effort"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                work_item: effort.work_item.clone(),
                end_snapshot: effort.end_snapshot_id.map(snapshot_ref),
                retroactive,
            },
        )
        .with_anchors(oxplow_domain::Anchors {
            snapshot_id: effort.end_snapshot_id,
            ..closed.envelope.anchors.clone()
        })
        .with_subject(closed.envelope.subject.clone())
        .with_cause(closed.envelope.id.clone())
        .with_dedupe_key(format!("effort.finished:{}", effort.id));
        match self.log.append(env).await {
            Ok(_) | Err(DomainError::Constraint(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// The effort an `effort.*` event is about (its `effort` payload ref): what
/// every consumer of `effort.closed` / `effort.finished` reads (tsk1025).
pub(crate) fn effort_of(event: &StoredEvent) -> Result<EffortId, DomainError> {
    let r = event.envelope.payload["effort"]
        .as_str()
        .unwrap_or_default();
    oxplow_domain::refs::build::effort_of_ref(r).ok_or_else(|| {
        DomainError::Invalid(format!(
            "{} seq {}: `effort` is not an effort ref: `{r}`",
            event.envelope.event_type, event.seq
        ))
    })
}

#[async_trait]
impl AsyncEventConsumer for EffortLifecycleConsumer {
    fn name(&self) -> &'static str {
        NAME
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
            "effort.closed" => {
                self.settle_claims().await;
                match self.tasks.on_effort_closed(effort, retroactive).await? {
                    Some(finished) => self.log_finished(event, &finished, retroactive).await,
                    None => Ok(()),
                }
            }
            _ => Ok(()),
        }
    }
}
