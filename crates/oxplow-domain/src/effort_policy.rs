//! The effort policy as an interface (`.context/work-tracking.md`): what
//! decides when a thread's effort opens, closes and links, as a reaction to
//! core's events. A policy only *composes* commands — core runs them, as
//! the policy's own effect actor — so every implementation (the built-in
//! `oxplow:commit-or-switch`, a provider process) meets the same gates.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::Anchors;
use crate::{CommandCall, DomainError};

/// What a policy sees of an event: an effect's shape (`id`, `type`, `v`,
/// `seq`, `source`, `subject`, `payload`) plus its anchors — the thread an
/// agent moved an item on, the turn a checkpoint is of, the effort a
/// landing is of.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub v: u32,
    pub seq: i64,
    pub source: String,
    pub subject: Vec<String>,
    pub payload: Value,
    pub anchors: Anchors,
}

/// The event types a policy is offered. (`capability.switched` for the
/// effort policy is core's to handle: it closes every open effort,
/// whichever policy is active after.)
pub const EVENTS: &[(&str, u32)] = &[
    ("work_item.state_changed", 1),
    ("effort.linked", 1),
    ("effort.opened", 2),
    ("thread.checkpoint", 1),
    ("effort.landed", 1),
];

/// An effort policy: the commands to run for an event.
#[async_trait]
pub trait EffortPolicy: Send + Sync {
    /// The implementation's id (what `activeProviders.effort_policy` names).
    fn id(&self) -> &str;
    /// The commands to run for `event`, in order; none to do nothing.
    async fn react(&self, event: &PolicyEvent) -> Result<Vec<CommandCall>, DomainError>;
}

/// The registered policies, by id: the ones the extensions declare and the
/// running provider instances together. A catalog reload restates the
/// declared ones ([`Self::set_declared`]) and never touches an instance
/// registered on its own ([`Self::register`]).
#[derive(Default)]
pub struct EffortPolicyRegistry {
    policies: RwLock<BTreeMap<String, Arc<dyn EffortPolicy>>>,
    /// The ids `set_declared` put there.
    declared: RwLock<BTreeSet<String>>,
}

impl EffortPolicyRegistry {
    /// Register a running instance under its id.
    pub fn register(&self, policy: Arc<dyn EffortPolicy>) {
        let id = policy.id().to_string();
        self.declared
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        self.policies
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, policy);
    }

    /// Unregister `id` (a stopped instance).
    pub fn unregister(&self, id: &str) {
        self.policies
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        self.declared
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    /// The declared policies are now `policies`: those declared before and
    /// not now go, these replace theirs, and a registered instance stays.
    pub fn set_declared(&self, policies: Vec<Arc<dyn EffortPolicy>>) {
        let mut map = self.policies.write().unwrap_or_else(|e| e.into_inner());
        let mut declared = self.declared.write().unwrap_or_else(|e| e.into_inner());
        for id in std::mem::take(&mut *declared) {
            map.remove(&id);
        }
        for policy in policies {
            let id = policy.id().to_string();
            declared.insert(id.clone());
            map.insert(id, policy);
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn EffortPolicy>> {
        self.policies
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// Whether `id` is a registered policy (a provider's id-clash check).
    pub fn has(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    pub fn ids(&self) -> Vec<String> {
        self.policies
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Named(&'static str);

    #[async_trait]
    impl EffortPolicy for Named {
        fn id(&self) -> &str {
            self.0
        }
        async fn react(&self, _: &PolicyEvent) -> Result<Vec<CommandCall>, DomainError> {
            Ok(Vec::new())
        }
    }

    /// A catalog reload restates the declared policies — one no longer
    /// declared goes — and never touches a registered instance.
    #[test]
    fn a_reload_restates_the_declared_and_keeps_an_instance() {
        let r = EffortPolicyRegistry::default();
        r.set_declared(vec![Arc::new(Named("oxplow")), Arc::new(Named("old"))]);
        r.register(Arc::new(Named("fake")));
        assert_eq!(r.ids(), vec!["fake", "old", "oxplow"]);
        r.set_declared(vec![Arc::new(Named("oxplow"))]);
        assert_eq!(r.ids(), vec!["fake", "oxplow"]);
        r.set_declared(Vec::new());
        assert_eq!(r.ids(), vec!["fake"]);
        assert!(r.has("fake"));
        r.unregister("fake");
        assert!(r.ids().is_empty());
    }

    /// The event a policy gets carries its anchors (an external policy
    /// reads them too).
    #[test]
    fn a_policy_event_carries_its_anchors() {
        let event = PolicyEvent {
            id: "e1".into(),
            event_type: "thread.checkpoint".into(),
            v: 1,
            seq: 3,
            source: "system:x".into(),
            subject: vec!["thread:thr1".into()],
            payload: serde_json::json!({}),
            anchors: Anchors {
                thread_id: Some(crate::ThreadId::new(1)),
                turn_id: Some(9),
                ..Anchors::default()
            },
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "thread.checkpoint");
        assert_eq!(json["anchors"]["turn_id"], 9);
    }
}
