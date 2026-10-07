//! The work-items capability (P5.C2, P7.A1; `.context/work-items.md`):
//! tasks, issues, tickets — whatever a provider tracks — behind one
//! interface. oxplow's own tasks are one provider (`oxplow`); an issue
//! tracker is another. Reads are SQL over `v_work_item`; writes are the
//! `work_item.*` commands, which the bus dispatches by the ref's provider
//! segment (`work_item:<provider>:<id>`): oxplow's in its transaction,
//! another provider's through its [`ExternalVerbs`].

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Actor, CommandCall, CommandError, Envelope};

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

/// The capability's verbs: the `work_item.<verb>` commands every provider
/// answers (`reorder` and `move` are oxplow's lists, not the capability's).
pub const VERBS: [&str; 6] = [
    "create",
    "update",
    "transition",
    "link",
    "comment",
    "delete",
];

/// What a provider supports beyond create, update and transition.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type,
)]
pub struct WorkItemsFeatures {
    /// Items nest (a parent ref).
    pub hierarchy: bool,
    pub comments: bool,
    pub links: bool,
    /// Items can be deleted (`work_item.delete`).
    #[serde(default)]
    pub delete: bool,
    /// A write sent twice with one idempotency key is done once, the
    /// second answered as the first (the protocol's
    /// `InvokeParams.idempotency_key`): the host may send a write again
    /// when its reply was lost.
    #[serde(default)]
    pub idempotent_writes: bool,
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

/// What one of a provider's verbs did, as the bus records it: the
/// handler's `result`, the events to log (the provider's
/// `work_item.recorded`), and the inverse — named by its **verb**
/// (`transition`), which the dispatching command turns back into
/// `work_item.<verb>` so an undo dispatches again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerbOutcome {
    pub result: Value,
    pub events: Vec<Envelope>,
    pub inverse: Option<CommandCall>,
}

/// A provider outside the bus's transaction: its verbs run against its
/// own system (a process, a tracker) and are recorded after they return.
/// `input` is the `work_item.<verb>` input, less the host-side
/// `provider`; the implementation checks it against what the provider
/// declared and refuses anything else before calling. `idempotency_key`
/// is the write's key when the caller has one (an effect's step: the same
/// on every attempt); without one the provider's host mints one.
#[async_trait]
pub trait ExternalVerbs: Send + Sync {
    async fn invoke(
        &self,
        actor: &Actor,
        verb: &str,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<VerbOutcome, CommandError>;

    /// End the provider's process: the next call starts a new one. What
    /// the conformance kit re-sends a keyed write across (tsk916) — a
    /// provider that keeps its keys only in memory would do it again.
    async fn restart(&self);
}

/// One source of work items: its ref segment (`oxplow`, `issues`), what
/// it supports, and — for a provider outside the bus's transaction — the
/// verbs the dispatching commands call. `None` is oxplow's own: its verbs
/// are the commands' `Tx` cores.
#[derive(Clone)]
pub struct WorkItemsProvider {
    pub id: String,
    pub features: WorkItemsFeatures,
    pub external: Option<Arc<dyn ExternalVerbs>>,
    /// What its own ids look like (a regex, matched whole: `tsk\d+`), so a
    /// loose id in a command resolves to its item while it's the active
    /// work list; `None` declares none.
    pub id_pattern: Option<String>,
    /// It takes any item's ref and keeps nothing (none): verbs naming
    /// another list's items reach it instead of being refused.
    pub sink: bool,
}

impl std::fmt::Debug for WorkItemsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkItemsProvider")
            .field("id", &self.id)
            .field("features", &self.features)
            .field("external", &self.external.is_some())
            .field("id_pattern", &self.id_pattern)
            .field("sink", &self.sink)
            .finish()
    }
}

/// oxplow's own provider name: `work_item:oxplow:tsk<n>`.
pub const OXPLOW: &str = "oxplow";

/// Why `id` can't name a provider or one of its instances — the segment
/// of its refs (`work_item:<id>:…`) and its command namespace — or `None`
/// when it can: lowercase letters, digits and underscores, starting with
/// a letter, and none of oxplow's own (`oxplow`, a core namespace). One
/// rule for a provider's id and an instance's (tsk840).
pub fn provider_id_problem(id: &str) -> Option<String> {
    let well_formed = id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !well_formed {
        Some(format!(
            "`{id}` must be lowercase letters, digits and underscores, starting with a letter"
        ))
    } else if id == OXPLOW || crate::events::schema::CORE_NAMESPACES.contains(&id) {
        Some(format!("`{id}` is reserved for oxplow"))
    } else {
        None
    }
}

#[derive(Default)]
struct Providers {
    by_id: BTreeMap<String, WorkItemsProvider>,
}

/// Where the active provider is read from: the config as it is now
/// (`activeProviders`), so a person's choice applies to the next `create`
/// — no copy for a reactor to refresh late (tsk1011).
pub type ActiveSource = Arc<dyn Fn() -> String + Send + Sync>;

/// The registered providers, by name. Cloning shares them.
#[derive(Clone)]
pub struct WorkItemsRegistry {
    providers: Arc<std::sync::RwLock<Providers>>,
    active: ActiveSource,
}

impl WorkItemsRegistry {
    /// A registry whose active provider is what `active` says each time
    /// it's asked.
    pub fn new(active: ActiveSource) -> Self {
        Self {
            providers: Arc::default(),
            active,
        }
    }

    pub fn register(&self, provider: WorkItemsProvider) {
        self.providers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .by_id
            .insert(provider.id.clone(), provider);
    }

    /// Remove a provider (an instance that stopped).
    pub fn unregister(&self, provider: &str) {
        self.providers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .by_id
            .remove(provider);
    }

    /// Every registered provider's name, sorted.
    pub fn names(&self) -> Vec<String> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .by_id
            .keys()
            .cloned()
            .collect()
    }

    pub fn get(&self, provider: &str) -> Result<WorkItemsProvider, WorkItemsError> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .by_id
            .get(provider)
            .cloned()
            .ok_or_else(|| WorkItemsError::UnknownProvider {
                provider: provider.to_string(),
                registered: self.names(),
            })
    }

    /// The provider `item_ref` belongs to.
    pub fn for_ref(&self, item_ref: &str) -> Result<WorkItemsProvider, WorkItemsError> {
        self.get(provider_of(item_ref)?)
    }

    /// The provider a `create` with no `provider` files on. Whether it is
    /// running is the caller's to check (`get`): an active provider that
    /// isn't is a failure naming it, never a silent fallback.
    pub fn active(&self) -> String {
        (self.active)()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(id: &str) -> WorkItemsProvider {
        WorkItemsProvider {
            id: id.into(),
            features: WorkItemsFeatures::default(),
            external: None,
            id_pattern: None,
            sink: false,
        }
    }

    #[test]
    fn a_ref_finds_its_provider_and_a_foreign_one_names_the_registered() {
        let registry = WorkItemsRegistry::new(Arc::new(|| OXPLOW.to_string()));
        registry.register(named("oxplow"));
        registry.register(named("fake"));
        assert_eq!(
            registry.for_ref("work_item:oxplow:tsk1").unwrap().id,
            "oxplow"
        );
        let err = registry.for_ref("work_item:issues:ENG-12").err().unwrap();
        assert_eq!(
            err.to_string(),
            "no work-items provider `issues`; registered: fake, oxplow"
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

    /// The active provider is what its source says each time it's asked;
    /// naming one doesn't check that it runs — a `create` on a missing
    /// active provider must fail naming it, which `get` does.
    #[test]
    fn the_active_provider_is_read_from_its_source_and_is_not_a_fallback() {
        let named_now = Arc::new(std::sync::Mutex::new(OXPLOW.to_string()));
        let source = named_now.clone();
        let registry = WorkItemsRegistry::new(Arc::new(move || source.lock().unwrap().clone()));
        registry.register(named("oxplow"));
        assert_eq!(registry.active(), "oxplow");
        *named_now.lock().unwrap() = "issues".into();
        assert_eq!(registry.active(), "issues");
        assert!(registry.get(&registry.active()).is_err());
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

    /// A provider that declares no `delete` reads as `false` (older
    /// declarations have no such flag).
    #[test]
    fn features_default_delete_to_false() {
        let f: WorkItemsFeatures = serde_json::from_value(serde_json::json!({
            "hierarchy": true, "comments": false, "links": false
        }))
        .unwrap();
        assert!(!f.delete);
    }
}
