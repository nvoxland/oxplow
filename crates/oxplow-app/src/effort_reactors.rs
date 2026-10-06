//! The effort reactors: what happens once an effort's close is fully
//! handled — the durable `effort.finished` the effort-lifecycle consumer
//! logs after pinning the bracket. Each runs on the event pump as its own
//! async consumer, so a slow one (a model call) never delays the others,
//! and a restart delivers whatever was logged but not yet handled.
//!
//! - `effort.evidence` — rebuild the effort's evidence rows.
//! - `effort.decisions` — infer the decisions it made (a model call;
//!   failures are logged, the review packet just shows none).
//! - `effort.commits` — link the commits made since it started that hold its
//!   work to its task (tsk1035: committed before the task closed).
//!
//! They hold `Services` weakly: the pump that runs them is part of it.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use oxplow_domain::{DomainError, StoredEvent};

use crate::event_pump::AsyncEventConsumer;
use crate::Services;

#[derive(Clone, Copy)]
enum Reaction {
    Evidence,
    Decisions,
    Commits,
}

struct EffortReactor {
    reaction: Reaction,
    services: Weak<Services>,
}

/// Register the reactors on `svc`'s pump (boot, before it spawns).
pub fn register(svc: &Arc<Services>) {
    for reaction in [Reaction::Evidence, Reaction::Decisions, Reaction::Commits] {
        svc.event_pump.register_async(Arc::new(EffortReactor {
            reaction,
            services: Arc::downgrade(svc),
        }));
    }
}

#[async_trait]
impl AsyncEventConsumer for EffortReactor {
    fn name(&self) -> &'static str {
        match self.reaction {
            Reaction::Evidence => "effort.evidence",
            Reaction::Decisions => "effort.decisions",
            Reaction::Commits => "effort.commits",
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
        let effort = crate::effort_lifecycle::effort_of(event)?;
        match self.reaction {
            Reaction::Evidence => crate::effort_evidence::refresh(&svc, effort.value()).await,
            Reaction::Decisions => {
                match crate::inferred_decisions::infer_for_effort(&svc, effort.value()).await {
                    Ok(outcome) => tracing::debug!(%effort, ?outcome, "inferred decisions"),
                    Err(error) => tracing::warn!(%effort, %error, "inferring decisions failed"),
                }
            }
            Reaction::Commits => crate::commit_links::link_effort(&svc, &effort).await?,
        }
        Ok(())
    }
}
