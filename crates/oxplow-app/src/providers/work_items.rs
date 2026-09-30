//! An external provider's work items: [`ExternalWorkItems`] maps the
//! capability's calls onto the provider's declared commands, run through
//! the bus as `<id>.<verb>` — audited and policy-checked like oxplow's
//! own — and the events they return are logged, so the
//! `work_items.project` consumer projects the provider's items into
//! `work_item`.
//!
//! The verbs and their inputs are the work-items contract every provider
//! implements: `create { title, body, parent_ref? }`, `update { ref,
//! title?, body?, parent_ref? }` (`""` detaches), `transition { ref, to }`
//! (a canonical state or a native one), `link { ref, target, link_type }`
//! and `comment { ref, body }`.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use oxplow_domain::work_items::{
    provider_of, NewWorkItem, Transition, WorkItemPatch, WorkItemsError, WorkItemsFeatures,
    WorkItemsProvider,
};
use oxplow_domain::Actor;
use serde_json::{json, Value};

use super::registry::Instance;
use super::spec;
use crate::commands::CommandBus;

/// A provider's declared work-items features.
pub fn features_of(features: &Value) -> Result<WorkItemsFeatures, String> {
    serde_json::from_value(features.clone()).map_err(|e| e.to_string())
}

pub struct ExternalWorkItems {
    id: String,
    features: WorkItemsFeatures,
    bus: Weak<CommandBus>,
}

impl ExternalWorkItems {
    pub fn new(bus: &Arc<CommandBus>, instance: &Instance) -> Result<Self, String> {
        let decl = instance
            .declared
            .capabilities
            .iter()
            .find(|c| c.capability == spec::WORK_ITEMS)
            .ok_or("it doesn't declare the work_items capability")?;
        Ok(Self {
            id: instance.spec.id.clone(),
            features: features_of(&decl.features)?,
            bus: Arc::downgrade(bus),
        })
    }

    /// Refuse another provider's ref, naming this one.
    fn own(&self, item_ref: &str) -> Result<(), WorkItemsError> {
        let owner = provider_of(item_ref)?;
        if owner != self.id {
            return Err(WorkItemsError::Failed(format!(
                "`{item_ref}` belongs to provider `{owner}`, not `{}`",
                self.id
            )));
        }
        Ok(())
    }

    fn unsupported(&self, feature: &str) -> WorkItemsError {
        WorkItemsError::Unsupported {
            provider: self.id.clone(),
            feature: feature.into(),
        }
    }

    async fn run(&self, actor: &Actor, verb: &str, input: Value) -> Result<Value, WorkItemsError> {
        let bus = self
            .bus
            .upgrade()
            .ok_or_else(|| WorkItemsError::Failed("the command bus is gone".into()))?;
        bus.run(actor, &format!("{}.{verb}", self.id), input, false)
            .await
            .map(|outcome| outcome.result)
            .map_err(|e| WorkItemsError::Failed(e.to_string()))
    }
}

#[async_trait]
impl WorkItemsProvider for ExternalWorkItems {
    fn provider(&self) -> &str {
        &self.id
    }

    fn features(&self) -> WorkItemsFeatures {
        self.features
    }

    async fn create(&self, actor: &Actor, item: NewWorkItem) -> Result<String, WorkItemsError> {
        let mut input = json!({ "title": item.title, "body": item.body });
        if let Some(parent) = item.parent_ref {
            if !self.features.hierarchy {
                return Err(self.unsupported("hierarchy"));
            }
            self.own(&parent)?;
            input["parent_ref"] = parent.into();
        }
        let result = self.run(actor, "create", input).await?;
        result["ref"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| WorkItemsError::Failed(format!("{}.create returned no ref", self.id)))
    }

    async fn update(
        &self,
        actor: &Actor,
        item_ref: &str,
        patch: WorkItemPatch,
    ) -> Result<(), WorkItemsError> {
        self.own(item_ref)?;
        let mut input = json!({ "ref": item_ref });
        if let Some(title) = patch.title {
            input["title"] = title.into();
        }
        if let Some(body) = patch.body {
            input["body"] = body.into();
        }
        if let Some(parent) = patch.parent_ref {
            if !self.features.hierarchy {
                return Err(self.unsupported("hierarchy"));
            }
            input["parent_ref"] = parent.unwrap_or_default().into();
        }
        self.run(actor, "update", input).await.map(|_| ())
    }

    async fn transition(
        &self,
        actor: &Actor,
        item_ref: &str,
        to: Transition,
    ) -> Result<(), WorkItemsError> {
        self.own(item_ref)?;
        let to = match to {
            Transition::Canonical(state) => state.as_str().to_string(),
            Transition::Native(native) => native,
        };
        self.run(actor, "transition", json!({ "ref": item_ref, "to": to }))
            .await
            .map(|_| ())
    }

    async fn link(
        &self,
        actor: &Actor,
        from: &str,
        to: &str,
        link_type: &str,
    ) -> Result<(), WorkItemsError> {
        if !self.features.links {
            return Err(self.unsupported("links"));
        }
        self.own(from)?;
        self.run(
            actor,
            "link",
            json!({ "ref": from, "target": to, "link_type": link_type }),
        )
        .await
        .map(|_| ())
    }

    async fn comment(
        &self,
        actor: &Actor,
        item_ref: &str,
        body: &str,
    ) -> Result<(), WorkItemsError> {
        if !self.features.comments {
            return Err(self.unsupported("comments"));
        }
        self.own(item_ref)?;
        self.run(actor, "comment", json!({ "ref": item_ref, "body": body }))
            .await
            .map(|_| ())
    }
}
