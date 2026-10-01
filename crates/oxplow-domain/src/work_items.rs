//! The work-items capability (P5.C2, `.context/work-items.md`): tasks,
//! issues, tickets — whatever a provider tracks — behind one interface.
//! oxplow's own tasks are one provider (`oxplow`); an issue tracker is
//! another. Reads are SQL over `v_work_item`; writes go through a
//! provider, which a [`WorkItemsRegistry`] picks by the ref's provider
//! segment (`work_item:<provider>:<id>`).

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Actor;

/// The states every provider maps its own to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalState {
    Todo,
    InProgress,
    Blocked,
    Done,
    Canceled,
}

impl CanonicalState {
    pub const ALL: [CanonicalState; 5] = [
        CanonicalState::Todo,
        CanonicalState::InProgress,
        CanonicalState::Blocked,
        CanonicalState::Done,
        CanonicalState::Canceled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CanonicalState::Todo => "todo",
            CanonicalState::InProgress => "in_progress",
            CanonicalState::Blocked => "blocked",
            CanonicalState::Done => "done",
            CanonicalState::Canceled => "canceled",
        }
    }
}

/// Where to move an item: a canonical state, or one of the provider's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Transition {
    Canonical(CanonicalState),
    Native(String),
}

/// What a provider supports beyond create, update and transition.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type,
)]
pub struct WorkItemsFeatures {
    /// Items nest (a parent ref).
    pub hierarchy: bool,
    pub comments: bool,
    pub links: bool,
    /// Moving an item to `in_progress` opens its effort itself (oxplow's
    /// tasks do), so `effort.open` must not open a second.
    pub in_progress_opens_effort: bool,
}

/// A new item.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NewWorkItem {
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Refused unless the provider has `hierarchy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<String>,
}

/// Fields to change; absent ones are left alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkItemPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// `Some(None)` detaches from the parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<Option<String>>,
}

/// An item as its provider now has it — what `v_work_item` holds, and
/// what a provider's `work_item.recorded` event carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemRecord {
    /// `work_item:<provider>:<id>`.
    #[serde(rename = "ref")]
    pub item_ref: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub state: CanonicalState,
    pub native_state: String,
    /// Provider-specific fields.
    #[serde(default)]
    pub native: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<String>,
    /// Gone at the provider.
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkItemsError {
    #[error("`{0}` isn't a work item ref (work_item:<provider>:<id>)")]
    NotARef(String),
    #[error("no work-items provider `{provider}`; registered: {}", registered.join(", "))]
    UnknownProvider {
        provider: String,
        registered: Vec<String>,
    },
    #[error("{provider} work items don't support {feature}")]
    Unsupported { provider: String, feature: String },
    #[error("{0}")]
    Failed(String),
}

/// The provider segment of `work_item:<provider>:<id>`.
pub fn provider_of(item_ref: &str) -> Result<&str, WorkItemsError> {
    item_ref
        .strip_prefix("work_item:")
        .and_then(|rest| rest.split_once(':'))
        .filter(|(p, id)| !p.is_empty() && !id.is_empty())
        .map(|(p, _)| p)
        .ok_or_else(|| WorkItemsError::NotARef(item_ref.to_string()))
}

/// One source of work items.
#[async_trait]
pub trait WorkItemsProvider: Send + Sync {
    /// The ref's provider segment (`oxplow`, `linear`).
    fn provider(&self) -> &str;
    fn features(&self) -> WorkItemsFeatures;
    /// The new item's ref.
    async fn create(&self, actor: &Actor, item: NewWorkItem) -> Result<String, WorkItemsError>;
    async fn update(
        &self,
        actor: &Actor,
        item_ref: &str,
        patch: WorkItemPatch,
    ) -> Result<(), WorkItemsError>;
    async fn transition(
        &self,
        actor: &Actor,
        item_ref: &str,
        to: Transition,
    ) -> Result<(), WorkItemsError>;
    /// A typed link (`blocks`, `relates_to`, …) from one item to another.
    async fn link(
        &self,
        actor: &Actor,
        from: &str,
        to: &str,
        link_type: &str,
    ) -> Result<(), WorkItemsError>;
    async fn comment(
        &self,
        actor: &Actor,
        item_ref: &str,
        body: &str,
    ) -> Result<(), WorkItemsError>;
}

/// The registered providers, by name. Cloning shares them.
#[derive(Clone, Default)]
pub struct WorkItemsRegistry {
    providers: Arc<std::sync::RwLock<BTreeMap<String, Arc<dyn WorkItemsProvider>>>>,
}

impl WorkItemsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, provider: Arc<dyn WorkItemsProvider>) {
        self.providers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(provider.provider().to_string(), provider);
    }

    /// Remove a provider (an instance that stopped).
    pub fn unregister(&self, provider: &str) {
        self.providers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(provider);
    }

    /// Every registered provider's name, sorted.
    pub fn names(&self) -> Vec<String> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    pub fn get(&self, provider: &str) -> Result<Arc<dyn WorkItemsProvider>, WorkItemsError> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(provider)
            .cloned()
            .ok_or_else(|| WorkItemsError::UnknownProvider {
                provider: provider.to_string(),
                registered: self.names(),
            })
    }

    /// The provider `item_ref` belongs to.
    pub fn for_ref(&self, item_ref: &str) -> Result<Arc<dyn WorkItemsProvider>, WorkItemsError> {
        self.get(provider_of(item_ref)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Named(&'static str);

    #[async_trait]
    impl WorkItemsProvider for Named {
        fn provider(&self) -> &str {
            self.0
        }
        fn features(&self) -> WorkItemsFeatures {
            WorkItemsFeatures::default()
        }
        async fn create(&self, _: &Actor, _: NewWorkItem) -> Result<String, WorkItemsError> {
            Ok(String::new())
        }
        async fn update(&self, _: &Actor, _: &str, _: WorkItemPatch) -> Result<(), WorkItemsError> {
            Ok(())
        }
        async fn transition(
            &self,
            _: &Actor,
            _: &str,
            _: Transition,
        ) -> Result<(), WorkItemsError> {
            Ok(())
        }
        async fn link(&self, _: &Actor, _: &str, _: &str, _: &str) -> Result<(), WorkItemsError> {
            Ok(())
        }
        async fn comment(&self, _: &Actor, _: &str, _: &str) -> Result<(), WorkItemsError> {
            Ok(())
        }
    }

    #[test]
    fn a_ref_finds_its_provider_and_a_foreign_one_names_the_registered() {
        let registry = WorkItemsRegistry::new();
        registry.register(Arc::new(Named("oxplow")));
        registry.register(Arc::new(Named("fake")));
        assert_eq!(
            registry
                .for_ref("work_item:oxplow:tsk1")
                .unwrap()
                .provider(),
            "oxplow"
        );
        let err = registry.for_ref("work_item:linear:ENG-12").err().unwrap();
        assert_eq!(
            err.to_string(),
            "no work-items provider `linear`; registered: fake, oxplow"
        );
        assert!(matches!(
            registry.for_ref("tsk1").err().unwrap(),
            WorkItemsError::NotARef(_)
        ));
        assert!(matches!(
            provider_of("work_item::x"),
            Err(WorkItemsError::NotARef(_))
        ));
    }

    #[test]
    fn a_record_round_trips_under_its_wire_names() {
        let record = WorkItemRecord {
            item_ref: "work_item:fake:W-1".into(),
            title: "t".into(),
            body: String::new(),
            state: CanonicalState::InProgress,
            native_state: "doing".into(),
            native: serde_json::json!({ "points": 3 }),
            parent_ref: None,
            deleted: false,
        };
        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["ref"], "work_item:fake:W-1");
        assert_eq!(json["state"], "in_progress");
        assert_eq!(
            serde_json::from_value::<WorkItemRecord>(json).unwrap(),
            record
        );
    }
}
