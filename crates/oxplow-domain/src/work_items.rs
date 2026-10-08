//! The work-items capability (P5.C2, P7.A1; `.context/work-items.md`):
//! tasks, issues, tickets — whatever a provider tracks — behind one
//! interface. oxplow's own tasks are one provider (`oxplow`); an issue
//! tracker is another. Reads are SQL over `v_work_item`; writes are the
//! `work_item.*` commands, which the bus dispatches by the ref's provider
//! segment (`work_item:<provider>:<id>`) to that provider's
//! [`WorkItemVerbs`] — the same way for every list, oxplow's own included.

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

/// What kind of value a declared field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    /// One of `values`.
    Enum,
    Text,
    Number,
}

/// One of a work list's own fields: kept in an item's `native` under
/// `name`, declared so screens render and edit it without knowing which
/// list is active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct FieldDecl {
    /// Its key in `native` (snake_case).
    pub name: String,
    /// How a person names it.
    pub title: String,
    pub kind: FieldKind,
    /// An `enum`'s values, in the order they're offered (empty for any
    /// other kind).
    #[serde(default)]
    pub values: Vec<String>,
    /// Set by the list itself (who filed it, when it synced): shown, never
    /// edited — a `oxplow.work_item.update` that names it is refused.
    #[serde(default)]
    pub read_only: bool,
}

/// What's wrong with `fields`, if anything: names snake_case and unique,
/// an `enum` with values, any other kind without.
pub fn fields_problem(fields: &[FieldDecl]) -> Option<String> {
    let snake = |n: &str| {
        n.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && n.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    fields.iter().enumerate().find_map(|(i, f)| {
        if !snake(&f.name) {
            Some(format!("field `{}`: its name must be snake_case", f.name))
        } else if fields[..i].iter().any(|o| o.name == f.name) {
            Some(format!("field `{}` is declared twice", f.name))
        } else if f.kind == FieldKind::Enum && f.values.is_empty() {
            Some(format!("field `{}`: an enum needs its values", f.name))
        } else if f.kind != FieldKind::Enum && !f.values.is_empty() {
            Some(format!("field `{}`: only an enum has values", f.name))
        } else {
            None
        }
    })
}

/// The capability's verbs: the `work_item.<verb>` commands a work list
/// answers — create, update and transition always; the rest as its
/// features say (`links`, `comments`, `delete`, `ordering`: reorder,
/// `lists`: move).
pub const VERBS: [&str; 8] = [
    "create",
    "update",
    "transition",
    "link",
    "comment",
    "delete",
    "reorder",
    "move",
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
    /// Items can be deleted (`oxplow.work_item.delete`).
    #[serde(default)]
    pub delete: bool,
    /// A write sent twice with one idempotency key is done once, the
    /// second answered as the first (the protocol's
    /// `InvokeParams.idempotency_key`): the host may send a write again
    /// when its reply was lost.
    #[serde(default)]
    pub idempotent_writes: bool,
    /// Items have an order on their list (`oxplow.work_item.reorder`; read as
    /// `v_work_item.rank`).
    #[serde(default)]
    pub ordering: bool,
    /// Items are on a thread's list or the backlog, and move between them
    /// (`oxplow.work_item.move`; read as `v_work_item.thread_id`).
    #[serde(default)]
    pub lists: bool,
}

/// One of an item's links, as its provider records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkRecord {
    /// The item it points at (`work_item:<provider>:<id>`).
    pub target: String,
    /// `blocks`, `discovered_from`, `relates_to`, `duplicates`,
    /// `supersedes` or `replies_to`.
    pub link_type: String,
}

/// One of an item's comments, as its provider records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommentRecord {
    /// The provider's own id for it, unique on the item.
    pub id: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// RFC 3339; absent, when the host recorded it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// An item as its provider now has it — what `v_work_item` holds, and
/// what a provider's `work_item.recorded` event carries. `rank`, `links`
/// and `comments` are stated when the provider keeps them: absent, the
/// host keeps what it has; present, it's the whole set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    /// Its order on its list (ascending).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<f64>,
    /// Its links (from it to others), the whole set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<LinkRecord>>,
    /// Its comments, the whole set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comments: Option<Vec<CommentRecord>>,
    /// The list it's on, when the provider keeps lists (`lists`): stated,
    /// the host restates it; absent, it stays on the list its first record
    /// was filed from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<List>,
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

/// A provider's verbs: they run against its own system (its tables, a
/// process, a tracker) outside the bus's transaction, and are recorded
/// after they return; what they wrote reaches the interface through the
/// `work_item.recorded` events they answer with.
/// `input` is the `work_item.<verb>` input, less the host-side
/// `provider`; the implementation checks it against what the provider
/// declared and refuses anything else before calling. `idempotency_key`
/// is the write's key when the caller has one (an effect's step: the same
/// on every attempt); without one the provider's host mints one.
#[async_trait]
pub trait WorkItemVerbs: Send + Sync {
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
/// it supports, and the verbs the dispatching commands call.
#[derive(Clone)]
pub struct WorkItemsProvider {
    pub id: String,
    pub features: WorkItemsFeatures,
    pub verbs: Arc<dyn WorkItemVerbs>,
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
    /// Each list's `id_pattern`, compiled whole-match, the first time a
    /// loose id is resolved against it.
    matchers: Arc<std::sync::RwLock<BTreeMap<String, regex::Regex>>>,
}

impl WorkItemsRegistry {
    /// A registry whose active provider is what `active` says each time
    /// it's asked.
    pub fn new(active: ActiveSource) -> Self {
        Self {
            providers: Arc::default(),
            active,
            matchers: Arc::default(),
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
    /// The canonical ref of `raw`, a loose id of the active work list's
    /// (`tsk42` → `work_item:oxplow:tsk42`), or why it isn't one.
    pub fn loose_ref(&self, raw: &str) -> Result<String, String> {
        let active = self.active();
        let Some(pattern) = self.get(&active).ok().and_then(|p| p.id_pattern) else {
            return Err(format!(
                "`{raw}` isn't a work item ref (work_item:<provider>:<id>), and the active work \
                 list declares no id of its own"
            ));
        };
        let matched = {
            let cached = self.matchers.read().unwrap_or_else(|e| e.into_inner());
            cached.get(&pattern).map(|r| r.is_match(raw))
        };
        let matched = match matched {
            Some(m) => m,
            None => {
                let compiled = regex::Regex::new(&format!("^(?:{pattern})$"))
                    .map_err(|e| format!("the active work list's id pattern `{pattern}`: {e}"))?;
                let m = compiled.is_match(raw);
                self.matchers
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(pattern.clone(), compiled);
                m
            }
        };
        if matched {
            Ok(format!("work_item:{active}:{raw}"))
        } else {
            Err(format!(
                "`{raw}` isn't a work item ref (work_item:<provider>:<id>) or an id of the active \
                 work list (`{pattern}`)"
            ))
        }
    }

    pub fn active(&self) -> String {
        (self.active)()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_checked() {
        let field = |name: &str, kind: FieldKind, values: &[&str]| FieldDecl {
            name: name.into(),
            title: name.into(),
            kind,
            values: values.iter().map(|v| v.to_string()).collect(),
            read_only: false,
        };
        assert_eq!(
            fields_problem(&[field("priority", FieldKind::Enum, &["high", "low"])]),
            None
        );
        let problem = |fields: &[FieldDecl]| fields_problem(fields).unwrap_or_default();
        assert!(problem(&[field("Priority", FieldKind::Text, &[])]).contains("snake_case"));
        assert!(problem(&[
            field("points", FieldKind::Number, &[]),
            field("points", FieldKind::Number, &[])
        ])
        .contains("twice"));
        assert!(problem(&[field("size", FieldKind::Enum, &[])]).contains("needs its values"));
        assert!(problem(&[field("note", FieldKind::Text, &["x"])]).contains("only an enum"));
    }

    struct Nothing;

    #[async_trait]
    impl WorkItemVerbs for Nothing {
        async fn invoke(
            &self,
            _actor: &Actor,
            _verb: &str,
            _input: Value,
            _idempotency_key: Option<String>,
        ) -> Result<VerbOutcome, CommandError> {
            unreachable!("the registry never calls a verb")
        }

        async fn restart(&self) {}
    }

    fn named(id: &str) -> WorkItemsProvider {
        WorkItemsProvider {
            id: id.into(),
            features: WorkItemsFeatures::default(),
            verbs: Arc::new(Nothing),
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
            rank: None,
            links: None,
            comments: None,
            list: Some(List::Thread("thr3".into())),
        };
        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["ref"], "work_item:fake:W-1");
        assert_eq!(json["list"], serde_json::json!({ "thread": "thr3" }));
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

// ---- The verbs' inputs: one shape for every list ----
//
// What a `oxplow.work_item.<verb>` command takes, and what a list's verb
// receives (`WorkItemVerbs::invoke`): the `v_work_item` columns, a
// canonical `state` with the list's `native_state`, and `native` for its
// own fields.

/// Move an item to a canonical state, and optionally to one of its
/// provider's own states that maps to it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemTransitionInput {
    /// The item's ref (`work_item:oxplow:tsk42`, `work_item:issues:ENG-12`).
    #[serde(rename = "ref")]
    pub item_ref: String,
    pub to: CanonicalState,
    /// The provider's own state, which must map to `to` (oxplow:
    /// `archived` with `done` or `canceled`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_state: Option<String>,
}

/// A new item on the active tracker, optionally straight into
/// a state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemCreateInput {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The parent's ref, on the same provider (needs `hierarchy`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<String>,
    /// `todo` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<CanonicalState>,
    /// The provider's own state, which must map to `state` when both are
    /// given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_state: Option<String>,
    /// The tracker's own fields (oxplow: `{ priority? }`), as its
    /// `create` declares them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<Value>,
    /// The thread it's filed on (`thr3`): absent, an agent's own, or none
    /// for a person (oxplow's backlog).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

/// Edit an item's fields and, optionally, its state — one run. Absent
/// fields are left alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemUpdateInput {
    #[serde(rename = "ref")]
    pub item_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The parent's ref on the same provider, or `""` to detach.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<CanonicalState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_state: Option<String>,
    /// The provider's own fields to change (oxplow: `{ priority? }`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<Value>,
}

/// A typed link from one item to another of the same provider.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemLinkInput {
    /// The item linked from.
    #[serde(rename = "ref")]
    pub item_ref: String,
    /// The item linked to (the same provider's).
    pub target: String,
    /// The provider names its own types (oxplow: blocks, relates_to,
    /// discovered_from, duplicates, supersedes, replies_to).
    pub link_type: String,
}

/// A comment on an item (oxplow: a task note).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemCommentInput {
    #[serde(rename = "ref")]
    pub item_ref: String,
    /// Markdown.
    pub body: String,
}

/// Remove an item (oxplow: soft — the row stays, marked deleted).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemDeleteInput {
    #[serde(rename = "ref")]
    pub item_ref: String,
}

/// `oxplow.work_item.reorder`: put an item before or after another in its own
/// list (neither: at its end).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemReorderInput {
    /// The task's ref (`work_item:oxplow:tsk42`).
    #[serde(rename = "ref")]
    pub item_ref: String,
    /// Put it just before this item of the same list.
    #[serde(default)]
    pub before: Option<String>,
    /// Put it just after this item of the same list.
    #[serde(default)]
    pub after: Option<String>,
}

/// A work list: a thread's, or the project-wide backlog — where
/// `oxplow.work_item.move` takes an item, and where a record says it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum List {
    /// The project-wide backlog.
    Backlog,
    /// A thread's list (`thr3`).
    Thread(String),
}

/// `oxplow.work_item.move`: take an item to another list — its end, or next to
/// an item there.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemMoveInput {
    #[serde(rename = "ref")]
    pub item_ref: String,
    pub to: List,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
}
