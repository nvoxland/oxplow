//! The effort reactors (P2.6b, tsk451): what happens once an effort's
//! close is fully handled — the durable `effort.finished@1` the
//! effort-lifecycle consumer logs after pinning the bracket. Each runs on
//! the event pump as its own async consumer, so a slow one (a model call)
//! never delays the others or a status change's settle, and a restart
//! delivers whatever was logged but not yet handled. They replace the
//! in-memory `EffortFinished` broadcast, which dropped on lag and was never
//! sent for synthesized or recovered efforts.
//!
//! - `effort.evidence` — rebuild the effort's evidence rows.
//! - `effort.decisions` — infer the decisions it made (a model call;
//!   failures are logged, the review packet just shows none).
//!
//! They hold `Services` weakly: the pump that runs them is part of it.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use oxplow_domain::{DomainError, EffortId, StoredEvent};

use crate::event_pump::AsyncEventConsumer;
use crate::Services;

#[derive(Clone, Copy)]
enum Reaction {
    Evidence,
    Decisions,
}

struct EffortReactor {
    reaction: Reaction,
    services: Weak<Services>,
}

/// Register the reactors on `svc`'s pump (boot, before it spawns).
pub fn register(svc: &Arc<Services>) {
    for reaction in [Reaction::Evidence, Reaction::Decisions] {
        svc.event_pump.register_async(Arc::new(EffortReactor {
            reaction,
            services: Arc::downgrade(svc),
        }));
    }
}

fn effort_of(event: &StoredEvent) -> Result<EffortId, DomainError> {
    let r = event.envelope.payload["effort"]
        .as_str()
        .unwrap_or_default();
    r.strip_prefix("effort:")
        .and_then(EffortId::try_from_str)
        .ok_or_else(|| DomainError::Invalid(format!("effort.finished seq {}: `{r}`", event.seq)))
}

#[async_trait]
impl AsyncEventConsumer for EffortReactor {
    fn name(&self) -> &'static str {
        match self.reaction {
            Reaction::Evidence => "effort.evidence",
            Reaction::Decisions => "effort.decisions",
        }
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == "effort.finished"
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        // Shutting down: the checkpoint stays, a restart delivers it.
        let Some(svc) = self.services.upgrade() else {
            return Err(DomainError::Busy("services are shutting down".into()));
        };
        let effort = effort_of(event)?;
        match self.reaction {
            Reaction::Evidence => crate::effort_evidence::refresh(&svc, effort.value()).await,
            Reaction::Decisions => {
                match crate::inferred_decisions::infer_for_effort(&svc, effort.value()).await {
                    Ok(outcome) => tracing::debug!(%effort, ?outcome, "inferred decisions"),
                    Err(error) => tracing::warn!(%effort, %error, "inferring decisions failed"),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::stores::TaskStore as _;

    /// A synthesized effort — recorded for a task that was never opened —
    /// now reaches every reactor, where the in-memory `EffortFinished` never
    /// fired for it.
    #[tokio::test]
    async fn a_synthesized_effort_reaches_every_reactor() {
        let f = crate::test_fixtures::services_with_effort().await;
        register(&f.svc);
        let mut loose = f.svc.task_store.get(f.task).await.unwrap().unwrap();
        loose.id = oxplow_domain::TaskId::placeholder();
        loose.status = oxplow_domain::TaskStatus::Ready;
        let never_opened = f.svc.task_store.insert(&loose).await.unwrap();
        f.svc
            .tasks
            .record_effort(
                &f.svc.effort_store,
                &oxplow_domain::refs::build::work_item_ref(never_opened),
                &f.thread,
                &[],
                Some("did it without opening".into()),
                &[],
                None,
            )
            .await
            .unwrap();
        f.svc.event_pump.run_once().await.unwrap();

        let events = f.svc.event_log_store.read_after(0, 200).await.unwrap();
        let finished = events
            .iter()
            .find(|e| {
                e.envelope.event_type == "effort.finished"
                    && e.envelope.payload["retroactive"] == true
            })
            .expect("the synthesized effort finished");
        for reactor in ["effort.evidence", "effort.decisions"] {
            let cp = f
                .svc
                .event_log_store
                .checkpoint(reactor.into())
                .await
                .unwrap();
            assert!(
                cp >= finished.seq,
                "{reactor} reached {cp} < {}",
                finished.seq
            );
        }
    }
}
