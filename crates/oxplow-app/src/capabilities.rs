//! Which implementation of each capability is active
//! (`.context/work-tracking.md` "Capabilities").
//!
//! Core declares the capabilities (`oxplow_domain::capability`). The
//! implementations come from three places, all held by one
//! [`CapabilityRegistry`]:
//!
//! - **core**: the fixed ones (the VCS) and, for every optional
//!   capability, [`NONE`] — nothing implements it;
//! - **extensions**: `implementations:` naming a built-in in core's
//!   standard library ([`BUILT_INS`], `entry: oxplow:oxwiki`), the way a
//!   collector names `oxplow:junit`. `oxplow-foundation` declares the
//!   defaults;
//! - **provider instances** while they run.
//!
//! [`CapabilityRegistry::resolve`] picks the active one: a person's own
//! choice (`oxplow.activeProviders.<capability>` in `.oxplow/local.yaml`),
//! else the project's (the same key in `.oxplow/project.yaml`), else the
//! capability's default. A choice that isn't
//! available falls back: an optional capability to [`NONE`], a required
//! one to its default built-in, which core always has. The registry
//! writes what it holds, with the active row and why, to
//! `v_capability_provider`.

use oxplow_domain::diagnostic::Diagnostic;
use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use oxplow_config::OxplowConfig;
use oxplow_db::capability_store::{list_tx, reset_tx};
use oxplow_db::CapabilityProvider;
pub use oxplow_domain::capability::ChosenBy;
use oxplow_domain::capability::{self, NONE};
use oxplow_domain::events::schema::{CapabilitySwitched, CapabilitySwitchedV3, EventType as _};
use oxplow_domain::events::Envelope;
use oxplow_domain::stores::StreamStore as _;
use oxplow_domain::vocabulary::VocabularyHandle;
use oxplow_domain::DomainError;
use serde_json::Value;

/// Core's standard library of implementations, by entry name, with the
/// capability each implements. An extension declares one by naming it;
/// core resolves the name, like a collector's `oxplow:junit`.
pub const BUILT_INS: &[BuiltIn] = &[
    BuiltIn {
        entry: "oxplow:snapshots",
        capability: "snapshots",
        provider: None,
        title: "Keep every version",
        features: &["contents"],
        config_schema: None,
    },
    // The same capture, recording what changed and keeping no bytes.
    BuiltIn {
        entry: "oxplow:snapshot-hashes",
        capability: "snapshots",
        provider: None,
        title: "Track changes only",
        features: &[],
        config_schema: None,
    },
    // Agent harnesses: what runs in an agent session.
    harness(
        "oxplow:claude-code",
        "Claude Code",
        &[
            "terminal",
            "permission_prompts",
            "resume",
            "transcript",
            "telemetry",
            "runtime_text",
            "subagents",
            "prompt_context",
            "tool_context",
            "conversation_list",
            "conversation_fork",
        ],
    ),
    // Codex's TUI on a thread of its daemon that oxplow opens and
    // observes: its approvals are the TUI's, and the TUI owns the prompt,
    // so context reaches it as items injected into the thread's history
    // between turns (`prompt_context`), never as a prompt.
    harness(
        "oxplow:codex-cli",
        "Codex",
        &[
            "terminal",
            "permission_prompts",
            "resume",
            "runtime_text",
            "subagents",
            "prompt_context",
            "conversation_list",
            "conversation_fork",
            "native_rename",
        ],
    ),
    // Its bridge appends a prompt hook's context to the user's message and a
    // tool hook's to the tool's result.
    harness(
        "oxplow:opencode",
        "opencode",
        &[
            "terminal",
            "permission_prompts",
            "resume",
            "runtime_text",
            "subagents",
            "prompt_context",
            "tool_context",
            "conversation_list",
            "conversation_fork",
        ],
    ),
];

/// An agent harness built-in.
const fn harness(
    entry: &'static str,
    title: &'static str,
    features: &'static [&'static str],
) -> BuiltIn {
    BuiltIn {
        entry,
        capability: "agent_harness",
        provider: None,
        title,
        features,
        config_schema: None,
    }
}

/// One built-in implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltIn {
    pub entry: &'static str,
    pub capability: &'static str,
    /// The provider id its items' refs carry, which a declaration of it
    /// must use; `None` for a built-in whose implementation has no id of
    /// its own.
    pub provider: Option<&'static str>,
    pub title: &'static str,
    /// The features it has — core's to say, since it's core's code.
    pub features: &'static [&'static str],
    /// The JSON Schema a declaration's `config:` must meet; `None` takes
    /// no config.
    pub config_schema: Option<&'static str>,
}

/// A built-in's features, as an implementation declares them.
fn built_in_features(b: &BuiltIn) -> Value {
    Value::Object(
        b.features
            .iter()
            .map(|f| (f.to_string(), Value::Bool(true)))
            .collect(),
    )
}

/// The built-in `entry` names, if core has it.
pub fn built_in(entry: &str) -> Option<&'static BuiltIn> {
    BUILT_INS.iter().find(|b| b.entry == entry)
}

/// How an implementation is loaded — never how it's called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Core's own (the VCS).
    Core,
    /// A built-in in core's standard library, by its entry.
    BuiltIn(&'static str),
    /// An AI provider: the Rust kind its declaration names (`provider:`,
    /// one of `oxplow_ai_rig::KINDS`), core's code.
    ProviderKind(&'static str),
    /// A provider instance's process.
    External,
    /// A script in its extension (`implementations:` with a file as its
    /// `entry`), called through the capability's verbs
    /// (`providers::script`).
    Script,
    /// Nothing implements it.
    None,
}

impl Source {
    fn as_str(&self) -> &'static str {
        match self {
            Source::Core => "core",
            Source::BuiltIn(_) | Source::ProviderKind(_) => "builtin",
            Source::External => "external",
            Source::Script => "script",
            Source::None => "none",
        }
    }
}

/// One implementation of a capability.
#[derive(Debug, Clone, PartialEq)]
pub struct Implementation {
    pub capability: String,
    /// Its id: what `activeProviders` names (`oxplow`, `issues`, `none`).
    pub id: String,
    pub title: String,
    /// The extension declaring it; `None` for core's.
    pub extension: Option<String>,
    pub source: Source,
    /// The features it declares.
    pub features: Value,
    /// Whether a setting offers it now (`when:` in its declaration; true
    /// without one). One that isn't is declared but off: listed, never used.
    pub offered: bool,
    /// Its logo as a data URI, when its declaration has an `icon:`.
    pub icon: Option<String>,
    /// What its declaration configures (`config:`, checked against its
    /// built-in's schema): an ACP adapter's command, a provider's base URL.
    pub config: Value,
    /// One sentence saying what it is, from its declaration.
    pub description: Option<String>,
}

impl Implementation {
    /// Whether it declares `feature`.
    pub fn has_feature(&self, feature: &str) -> bool {
        self.features
            .get(feature)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// The commands only it offers: a provider instance's namespace
    /// (`<id>.*`); nothing for core's and the built-ins.
    fn surface(&self) -> Option<String> {
        matches!(self.source, Source::External).then(|| format!("{}.*", self.id))
    }

    /// The "nothing implements it" of an optional capability: it takes
    /// everything and keeps nothing, so it declares every feature the
    /// capability has — what needs one isn't refused while it's none.
    pub fn none(capability: &str) -> Self {
        let features = capability::spec(capability)
            .map(|c| c.features)
            .unwrap_or_default()
            .iter()
            .map(|f| (f.to_string(), Value::Bool(true)))
            .collect();
        Self {
            capability: capability.into(),
            id: NONE.into(),
            title: "None".into(),
            extension: None,
            source: Source::None,
            features: Value::Object(features),
            offered: true,
            // Nothing in text is none's: it keeps no items.
            icon: None,
            config: Value::Object(Default::default()),
            description: None,
        }
    }
}

/// A capability's active implementation and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub id: String,
    pub chosen_by: ChosenBy,
    /// What was chosen, when it fell back from it.
    pub wanted: Option<String>,
}

/// What's active, per capability: its implementation and features — what
/// a declared need is checked against (a page, an advisory).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Active {
    by_capability: std::collections::BTreeMap<String, (String, Vec<String>)>,
    /// What implementations that aren't active own, kept from offering:
    /// a provider instance's command namespace (`<namespace>.*`).
    hidden_commands: Vec<Hidden>,
    /// The shared models with a live store (`model:<ns>/<model>` needs).
    live_models: Arc<std::collections::BTreeSet<String>>,
}

/// A command its owner, not being active, keeps from offering.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hidden {
    pattern: String,
    owner: String,
    capability: String,
}

impl Hidden {
    fn matches(&self, name: &str) -> bool {
        match self.pattern.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => self.pattern == name,
        }
    }

    fn message(&self, name: &str) -> String {
        let title =
            capability::spec(&self.capability).map_or(self.capability.as_str(), |c| c.title);
        format!(
            "`{name}` is {}'s, which isn't the active {} (choose one in Settings → Capabilities).",
            self.owner,
            title.to_lowercase()
        )
    }
}

impl Active {
    /// Why `spec` isn't offered — its owner isn't active, or a need is
    /// unmet — or `None` when it is.
    pub fn refusal(&self, spec: &oxplow_domain::CommandSpec) -> Option<String> {
        self.command_refusal(&spec.id, &spec.needs)
    }

    /// Why the command `name` needing `needs` isn't offered, or `None`.
    pub fn command_refusal(&self, name: &str, needs: &[String]) -> Option<String> {
        if let Some(h) = self.hidden_commands.iter().find(|h| h.matches(name)) {
            return Some(h.message(name));
        }
        let unmet = self.unmet(needs);
        (!unmet.is_empty()).then(|| needs_message(&unmet))
    }

    /// The needs in `needs` it doesn't meet: a capability whose active
    /// implementation is none, or a feature the active one doesn't have
    /// (none has every feature: it takes everything, keeping nothing).
    pub fn unmet(&self, needs: &[String]) -> Vec<String> {
        needs
            .iter()
            // A scope is always there; what may use it is a
            // separate question (`.context/commands.md`).
            .filter(|need| {
                oxplow_domain::scope::scope(oxplow_domain::service::need_scope(need)).is_none()
            })
            .filter(|need| match oxplow_domain::capability::model_need(need) {
                Some(model) => !self.live_models.contains(model),
                None => true,
            })
            .filter(|need| {
                if oxplow_domain::capability::model_need(need).is_some() {
                    return true;
                }
                let (id, feature) = match need.split_once('.') {
                    Some((id, f)) => (id, Some(f)),
                    None => (need.as_str(), None),
                };
                match (self.by_capability.get(id), feature) {
                    (None, _) => true,
                    (Some((_, features)), Some(f)) => !features.iter().any(|x| x == f),
                    (Some((active, _)), None) => active == NONE,
                }
            })
            .cloned()
            .collect()
    }
}

/// What a person reads when `unmet` needs keep something from showing.
pub fn needs_message(unmet: &[String]) -> String {
    let named: Vec<String> = unmet
        .iter()
        .map(|need| {
            if let Some(model) = oxplow_domain::capability::model_need(need) {
                return format!("a store of `{model}` (an extension that keeps them)");
            }
            let (id, feature) = match need.split_once('.') {
                Some((id, f)) => (id, Some(f)),
                None => (need.as_str(), None),
            };
            let title = capability::spec(id).map_or(id, |c| c.title);
            match feature {
                Some(f) => format!("{title} with {f}"),
                None => title.to_string(),
            }
        })
        .collect();
    format!(
        "Needs: {} (choose one in Settings → Capabilities).",
        named.join(", ")
    )
}

#[derive(Clone)]
struct RegistryState {
    core: Vec<Implementation>,
    declared: Vec<Implementation>,
    external: Vec<Implementation>,
    /// Config choices committed with this published registry generation.
    published_config: Option<Arc<OxplowConfig>>,
    /// The shared models with a live store in this generation: what a
    /// `model:<ns>/<model>` need asks for.
    live_models: Arc<std::collections::BTreeSet<String>>,
    revision: Arc<()>,
}

impl RegistryState {
    fn implementations(&self) -> Vec<Implementation> {
        let mut all = self.core.clone();
        all.extend(self.declared.iter().cloned());
        all.extend(self.external.iter().cloned());
        all
    }
}

/// Every implementation in one registry snapshot: core, declarations and
/// running instances. A read never resolves features from another snapshot.
pub struct CapabilityRegistry {
    state: RwLock<RegistryState>,
    /// What `capability.switched` is validated against.
    vocabulary: VocabularyHandle,
    /// The project's catalog: the runtime owner filter. A registry built
    /// alone belongs to no project and offers everything it holds. This does
    /// not change the frozen registry state or candidate preparation.
    catalog: Option<(
        Arc<crate::extension::extension_catalog::ExtensionCatalog>,
        std::path::PathBuf,
    )>,
    /// The project's activation boundary; its published config applies once
    /// a generation has been activated.
    activation: Arc<crate::extension::extension_activation::ExtensionActivation>,
}

/// Frozen capability declarations, selections and database rows for one catalog
/// candidate. Existing external instances are captured, never started here.
#[derive(Clone)]
pub struct PreparedCapabilities {
    generation: String,
    base: Arc<()>,
    candidate: RegistryState,
    active: Active,
    rows: Vec<CapabilityProvider>,
    resolved: Vec<(&'static str, Resolved)>,
}

/// A checked replacement holding the registry against competing writers.
/// Created and consumed on the publication worker; no lock crosses an await.
pub(crate) struct CapabilityReservation<'a> {
    state: std::sync::RwLockWriteGuard<'a, RegistryState>,
    candidate: Option<RegistryState>,
}

impl CapabilityReservation<'_> {
    /// Retain the writer through the remaining maps and follow policies.
    pub(crate) fn apply(&mut self) {
        *self.state = self
            .candidate
            .take()
            .expect("capability reservation applied once");
    }
}

impl PreparedCapabilities {
    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub fn active(&self) -> &Active {
        &self.active
    }

    /// The `v_capability_provider` rows this candidate will hold.
    pub fn rows(&self) -> &[CapabilityProvider] {
        &self.rows
    }

    pub(crate) fn selected(&self, capability: &str) -> Option<&str> {
        self.resolved
            .iter()
            .find(|(name, _)| *name == capability)
            .map(|(_, selection)| selection.id.as_str())
    }

    /// Whether the supplied host config still resolves exactly the captured
    /// selections, including fallback choices and their provenance.
    pub fn matches_config(&self, config: &OxplowConfig) -> bool {
        let (rows, resolved) =
            CapabilityRegistry::rows_from(&self.candidate.implementations(), config);
        rows == self.rows && resolved == self.resolved
    }

    /// Restate candidate rows and switch events in the caller's transaction.
    /// No runtime adapters, registry swaps or external execution happen here.
    /// Storage/rollback failures require aborting the outer transaction.
    pub fn restate_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        vocabulary: &oxplow_domain::vocabulary::Vocabulary,
    ) -> Result<(), DomainError> {
        tx.execute_batch("SAVEPOINT candidate_extension_capabilities")
            .map_err(oxplow_db::map_sql_err)?;
        match restate(tx, vocabulary, &self.rows, &self.resolved) {
            Ok(()) => tx
                .execute_batch("RELEASE candidate_extension_capabilities")
                .map_err(oxplow_db::map_sql_err),
            Err(error) => {
                tx.execute_batch(
                    "ROLLBACK TO candidate_extension_capabilities; RELEASE candidate_extension_capabilities",
                )
                .map_err(oxplow_db::map_sql_err)?;
                Err(error)
            }
        }
    }
}

impl CapabilityRegistry {
    /// A registry with core's fixed implementations (`vcs`)
    /// and [`NONE`] for each optional capability.
    pub fn new(fixed: Vec<Implementation>, vocabulary: VocabularyHandle) -> Self {
        let mut core = fixed;
        core.extend(
            capability::CAPABILITIES
                .iter()
                .filter(|c| c.optional)
                .map(|c| Implementation::none(c.id)),
        );
        // A required capability's default is core's own, always there —
        // what it falls back to whatever is disabled.
        core.extend(
            capability::CAPABILITIES
                .iter()
                .filter(|c| c.choosable && !c.optional)
                .filter_map(|c| {
                    let b = BUILT_INS.iter().find(|b| b.capability == c.id)?;
                    Some(Implementation {
                        capability: c.id.into(),
                        id: c.default.into(),
                        title: b.title.into(),
                        extension: None,
                        source: Source::BuiltIn(b.entry),
                        features: built_in_features(b),
                        offered: true,
                        icon: None,
                        config: Value::Object(Default::default()),
                        description: None,
                    })
                }),
        );
        Self {
            state: RwLock::new(RegistryState {
                core,
                declared: Vec::new(),
                external: Vec::new(),
                published_config: None,
                live_models: Arc::default(),
                revision: Arc::new(()),
            }),
            vocabulary,
            catalog: None,
            activation: Arc::new(
                crate::extension::extension_activation::ExtensionActivation::new(),
            ),
        }
    }

    /// Replace what extensions declare (on boot, and when the catalog
    /// changes).
    pub fn set_declared(&self, declared: Vec<Implementation>) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.declared = declared;
        state.revision = Arc::new(());
    }

    /// A provider instance started (`true`) or stopped.
    pub fn set_external(&self, implementation: Implementation, running: bool) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state
            .external
            .retain(|i| !(i.capability == implementation.capability && i.id == implementation.id));
        if running {
            state.external.push(implementation);
        }
        state.revision = Arc::new(());
    }

    /// Prepare declarations and selections from one catalog and one registry
    /// snapshot. Preparation neither registers adapters nor publishes rows.
    pub fn prepare(
        &self,
        snapshot: &crate::extension::extension_catalog::CatalogSnapshot,
        config: &OxplowConfig,
        settings: &Settings,
    ) -> Result<PreparedCapabilities, crate::vocabulary_reactor::VocabularyErrors> {
        self.prepare_candidate(snapshot, config, None, settings)
    }

    /// Prepare declarations and selections using a host-staged external
    /// capability map instead of the currently published instances.
    pub fn prepare_with_external(
        &self,
        snapshot: &crate::extension::extension_catalog::CatalogSnapshot,
        config: &OxplowConfig,
        external: Vec<Implementation>,
        settings: &Settings,
    ) -> Result<PreparedCapabilities, crate::vocabulary_reactor::VocabularyErrors> {
        self.prepare_candidate(snapshot, config, Some(external), settings)
    }

    fn prepare_candidate(
        &self,
        snapshot: &crate::extension::extension_catalog::CatalogSnapshot,
        config: &OxplowConfig,
        external: Option<Vec<Implementation>>,
        settings: &Settings,
    ) -> Result<PreparedCapabilities, crate::vocabulary_reactor::VocabularyErrors> {
        let errors: crate::vocabulary_reactor::VocabularyErrors = snapshot
            .extensions()
            .iter()
            .filter(|extension| extension.enabled && !extension.errors.is_empty())
            .map(|extension| (extension.name.clone(), extension.errors.clone()))
            .collect();
        if !errors.is_empty() {
            return Err(errors);
        }
        if let Some(external) = &external {
            let mut errors: Vec<Diagnostic> = Vec::new();
            let mut identities = BTreeSet::new();
            for implementation in external {
                if implementation.source != Source::External {
                    errors.push(Diagnostic::error(
                        "activation.external_capability_source",
                        "external capability metadata must use the external source",
                    ));
                }
                if capability::spec(&implementation.capability).is_none() {
                    errors.push(Diagnostic::error(
                        "activation.external_capability_unknown",
                        "external capability metadata names an unknown capability",
                    ));
                }
                if implementation.id.is_empty()
                    || implementation.id.len() > 256
                    || !implementation
                        .id
                        .bytes()
                        .all(|byte| byte.is_ascii_graphic())
                {
                    errors.push(Diagnostic::error(
                        "activation.external_capability_id",
                        "external capability metadata has an invalid id",
                    ));
                }
                if !identities.insert((
                    implementation.capability.as_str(),
                    implementation.id.as_str(),
                )) {
                    errors.push(Diagnostic::error(
                        "activation.external_capability_duplicate",
                        "external capability metadata has a duplicate identity",
                    ));
                }
            }
            if !errors.is_empty() {
                return Err(crate::vocabulary_reactor::VocabularyErrors::from([(
                    "<host>".to_string(),
                    errors,
                )]));
            }
        }
        let mut candidate = self.state.read().unwrap_or_else(|e| e.into_inner()).clone();
        let base = candidate.revision.clone();
        if let Some(external) = external {
            candidate.external = external;
        }
        candidate.declared = declared_by(snapshot.extensions(), settings);
        candidate.published_config = Some(Arc::new(config.clone()));
        candidate.revision = Arc::new(());
        let all = candidate.implementations();
        let (rows, resolved) = Self::rows_from(&all, config);
        let models = crate::models::ModelRegistry::of(snapshot.extensions());
        candidate.live_models = Arc::new(
            models
                .models
                .keys()
                .filter(|m| !models.live_stores(m).is_empty())
                .cloned()
                .collect(),
        );
        let active = Self::snapshot_from(&all, config, candidate.live_models.clone());
        Ok(PreparedCapabilities {
            generation: snapshot.generation().to_string(),
            base,
            candidate,
            active,
            rows,
            resolved,
        })
    }

    pub(crate) fn reserve_prepared(
        &self,
        prepared: &PreparedCapabilities,
        config: &OxplowConfig,
    ) -> Result<CapabilityReservation<'_>, oxplow_domain::CommandError> {
        if !prepared.matches_config(config) {
            return Err(oxplow_domain::CommandError::Unavailable {
                message: "capability choices changed while their replacement was staged".into(),
                retry_after_ms: None,
            });
        }
        let state = self.state.write().unwrap_or_else(|e| e.into_inner());
        if !Arc::ptr_eq(&state.revision, &prepared.base) {
            return Err(oxplow_domain::CommandError::Unavailable {
                message: "the capability registry changed while its replacement was staged".into(),
                retry_after_ms: None,
            });
        }
        Ok(CapabilityReservation {
            state,
            candidate: Some(prepared.candidate.clone()),
        })
    }

    /// Bind live reads to one project's catalog and activation boundary.
    pub(crate) fn with_project(
        mut self,
        catalog: Arc<crate::extension::extension_catalog::ExtensionCatalog>,
        root: std::path::PathBuf,
        activation: Arc<crate::extension::extension_activation::ExtensionActivation>,
    ) -> Self {
        self.catalog = Some((catalog, root));
        self.activation = activation;
        self
    }

    /// Every currently available implementation, core's first. After catalog
    /// adoption, extension-owned implementations disappear immediately when
    /// their active owner is disabled or removed; the frozen map is unchanged.
    pub fn implementations(&self) -> Vec<Implementation> {
        let mut implementations = self
            .state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .implementations();
        let Some((catalog, root)) = &self.catalog else {
            return implementations;
        };
        let snapshot = catalog.live_snapshot(root);
        implementations.retain(|implementation| {
            implementation.extension.as_ref().is_none_or(|owner| {
                snapshot
                    .extensions()
                    .iter()
                    .any(|extension| extension.name == *owner && extension.enabled)
            })
        });
        implementations
    }

    /// `capability`'s implementation `id`, if available.
    pub fn get(&self, capability: &str, id: &str) -> Option<Implementation> {
        self.implementations()
            .into_iter()
            .find(|i| i.capability == capability && i.id == id)
    }

    /// `capability`'s active implementation under `config`, and why.
    pub fn resolve(&self, config: &OxplowConfig, capability: &str) -> Resolved {
        let config = self.read_effective_config(config);
        Self::resolve_from(&self.implementations(), &config, capability)
    }

    fn resolve_from(all: &[Implementation], config: &OxplowConfig, capability: &str) -> Resolved {
        let Some(spec) = capability::spec(capability) else {
            return Resolved {
                id: NONE.into(),
                chosen_by: ChosenBy::Fallback,
                wanted: None,
            };
        };
        if spec.many {
            // Every declared one serves; the default is the one used when
            // nothing names one.
            return Resolved {
                id: spec.default.into(),
                chosen_by: ChosenBy::Default,
                wanted: None,
            };
        }
        let (wanted, chosen_by) = if !spec.choosable {
            (spec.default.to_string(), ChosenBy::Default)
        } else if let Some(id) = config.active_providers.get(capability) {
            (
                id.clone(),
                match config
                    .active_provider_origins
                    .get(capability)
                    .map(String::as_str)
                {
                    Some("local") => ChosenBy::Local,
                    Some("global") => ChosenBy::Global,
                    _ => ChosenBy::Project,
                },
            )
        } else {
            (spec.default.to_string(), ChosenBy::Default)
        };
        // One a setting turned off isn't there to choose.
        if all
            .iter()
            .any(|i| i.offered && i.capability == capability && i.id == wanted)
        {
            return Resolved {
                id: wanted,
                chosen_by,
                wanted: None,
            };
        }
        Resolved {
            // A required capability's default is core's: always there.
            id: if spec.optional {
                NONE.to_string()
            } else {
                spec.default.to_string()
            },
            chosen_by: ChosenBy::Fallback,
            wanted: Some(wanted),
        }
    }

    /// What's active under `config`, for checking needs.
    pub fn snapshot(&self, config: &OxplowConfig) -> Active {
        let config = self.read_effective_config(config);
        let live_models = self
            .state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .live_models
            .clone();
        Self::snapshot_from(&self.implementations(), &config, live_models)
    }

    fn snapshot_from(
        all: &[Implementation],
        config: &OxplowConfig,
        live_models: Arc<std::collections::BTreeSet<String>>,
    ) -> Active {
        let mut by_capability = std::collections::BTreeMap::new();
        for spec in capability::CAPABILITIES {
            if spec.many {
                // Met while any implementation is declared and offered (one
                // a setting turned off serves nothing); a feature while any
                // of them has it.
                let served: Vec<&Implementation> = all
                    .iter()
                    .filter(|i| i.offered && i.capability == spec.id)
                    .collect();
                if served.is_empty() {
                    continue;
                }
                let mut features: Vec<String> = served
                    .iter()
                    .filter_map(|i| i.features.as_object())
                    .flat_map(|m| {
                        m.iter()
                            .filter(|(_, v)| v.as_bool() == Some(true))
                            .map(|(k, _)| k.clone())
                    })
                    .collect();
                features.sort();
                features.dedup();
                by_capability.insert(spec.id.to_string(), (spec.default.to_string(), features));
                continue;
            }
            let id = Self::resolve_from(all, config, spec.id).id;
            let features = all
                .iter()
                .find(|i| i.capability == spec.id && i.id == id)
                .and_then(|i| i.features.as_object().cloned())
                .map(|m| {
                    m.into_iter()
                        .filter(|(_, v)| v.as_bool() == Some(true))
                        .map(|(k, _)| k)
                        .collect()
                })
                .unwrap_or_default();
            by_capability.insert(spec.id.to_string(), (id, features));
        }
        // What a running provider instance owns — its command namespace —
        // kept from offering unless it's the active one.
        let hidden_commands = all
            .iter()
            .filter(|i| matches!(i.source, Source::External))
            .filter(|i| {
                !by_capability
                    .get(&i.capability)
                    .is_some_and(|(a, _)| a == &i.id)
            })
            .filter_map(|i| {
                i.surface().map(|pattern| Hidden {
                    pattern,
                    owner: i.title.clone(),
                    capability: i.capability.clone(),
                })
            })
            .collect();
        Active {
            by_capability,
            hidden_commands,
            live_models,
        }
    }

    /// The active task tracker's id recognizer under `config`: its id and
    /// declared `id_pattern`, if it declares one (none doesn't).
    /// The config the last generation published; before the first one
    /// (boot's `initialize`) `fallback`, the config as it is now.
    fn read_effective_config(&self, fallback: &OxplowConfig) -> Arc<OxplowConfig> {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .published_config
            .clone()
            .unwrap_or_else(|| Arc::new(fallback.clone()))
    }

    /// `capability`'s active implementation id under `config`.
    pub fn active(&self, config: &OxplowConfig, capability: &str) -> String {
        self.resolve(config, capability).id
    }

    /// Restate `v_capability_provider` from what the registry holds: every
    /// implementation, the active one marked with why, and a choice that
    /// isn't available as its own row (`available = 0`). A capability whose
    /// active implementation differs from the one the rows had logs
    /// `capability.switched@2`, in the same transaction — so a change is
    /// logged once, across restarts too; the very first statement, with
    /// nothing active before, logs nothing.
    pub async fn publish(
        &self,
        config: &OxplowConfig,
        db: &oxplow_db::Database,
    ) -> Result<(), DomainError> {
        let (rows, now) = self.rows(config);
        let vocabulary = self.vocabulary.current();
        db.transaction(move |tx| restate(tx, &vocabulary, &rows, &now))
            .await
    }

    /// [`Self::publish`] at construction, before there's a runtime: so the
    /// first read of `v_capability_provider` (and of `v_oxplow_task`, which
    /// shows the active list's items) sees what's active.
    pub fn publish_now(
        &self,
        config: &OxplowConfig,
        db: &oxplow_db::Database,
    ) -> Result<(), DomainError> {
        let (rows, now) = self.rows(config);
        let vocabulary = self.vocabulary.current();
        db.transaction_now(|tx| restate(tx, &vocabulary, &rows, &now))
    }

    /// The rows `v_capability_provider` holds under `config`, and each
    /// capability's resolved implementation.
    fn rows(
        &self,
        config: &OxplowConfig,
    ) -> (Vec<CapabilityProvider>, Vec<(&'static str, Resolved)>) {
        Self::rows_from(&self.implementations(), config)
    }

    fn rows_from(
        all: &[Implementation],
        config: &OxplowConfig,
    ) -> (Vec<CapabilityProvider>, Vec<(&'static str, Resolved)>) {
        let mut rows: Vec<CapabilityProvider> = Vec::new();
        let mut now = Vec::new();
        for spec in capability::CAPABILITIES {
            let resolved = Self::resolve_from(all, config, spec.id);
            for i in all.iter().filter(|i| i.capability == spec.id) {
                // Many serve at once: every declared one is active — but one
                // a setting turned off (`when:`) is listed, off, saying so.
                let active = i.offered && (spec.many || i.id == resolved.id);
                let chosen_by = if !i.offered {
                    Some("setting".to_string())
                } else if spec.many {
                    Some("declared".to_string())
                } else {
                    active.then(|| resolved.chosen_by.as_str().to_string())
                };
                rows.push(CapabilityProvider {
                    capability: i.capability.clone(),
                    provider: i.id.clone(),
                    extension: i.extension.clone(),
                    features: i.features.clone(),
                    active,
                    title: i.title.clone(),
                    source: i.source.as_str().into(),
                    available: true,
                    chosen_by,
                    capability_title: spec.title.into(),
                    choosable: spec.choosable,
                    optional: spec.optional,
                    description: i.description.clone(),
                });
            }
            if let Some(wanted) = &resolved.wanted {
                if !rows
                    .iter()
                    .any(|r| r.capability == spec.id && &r.provider == wanted)
                {
                    rows.push(CapabilityProvider {
                        capability: spec.id.into(),
                        provider: wanted.clone(),
                        extension: None,
                        features: Value::Object(Default::default()),
                        active: false,
                        title: wanted.clone(),
                        source: "unknown".into(),
                        available: false,
                        chosen_by: None,
                        capability_title: spec.title.into(),
                        choosable: spec.choosable,
                        optional: spec.optional,
                        description: None,
                    });
                }
            }
            now.push((spec.id, resolved));
        }
        (rows, now)
    }
}

/// Restate the rows and log each switch (`capability.switched@2`), in one
/// transaction.
fn restate(
    tx: &rusqlite::Transaction<'_>,
    vocabulary: &oxplow_domain::vocabulary::Vocabulary,
    rows: &[CapabilityProvider],
    now: &[(&str, Resolved)],
) -> Result<(), DomainError> {
    let before = list_tx(tx)?;
    reset_tx(tx, rows)?;
    for switch in switches(&before, now) {
        let envelope = Envelope::new(
            CapabilitySwitched::TYPE,
            CapabilitySwitched::V,
            "system",
            serde_json::to_value(&switch)
                .map_err(|e| DomainError::Invalid(format!("capability.switched: {e}")))?,
        )?;
        oxplow_db::event_log_store::append_tx(tx, vocabulary, &envelope)?;
    }
    Ok(())
}

/// The capabilities whose active implementation in `before` (the rows as
/// they were) isn't the one `now` resolves; one with no active row before
/// (the first statement) isn't a switch.
fn switches(before: &[CapabilityProvider], now: &[(&str, Resolved)]) -> Vec<CapabilitySwitchedV3> {
    now.iter()
        // Many serve at once: nothing is chosen, so nothing switches.
        .filter(|(capability, _)| !capability::spec(capability).is_some_and(|s| s.many))
        .filter_map(|(capability, resolved)| {
            let from = before
                .iter()
                .find(|r| r.active && r.capability == *capability)?;
            (from.provider != resolved.id).then(|| CapabilitySwitchedV3 {
                capability: capability.to_string(),
                from: from.provider.clone(),
                to: resolved.id.clone(),
                chosen_by: resolved.chosen_by,
            })
        })
        .collect()
}

/// Each namespace's effective configuration, for the implementations a
/// setting offers (`when:`): [`offering_implementations`]. One missing
/// offers nothing (it fails closed, as every read of a setting does).
pub type Settings = std::collections::BTreeMap<String, Value>;

/// The implementations `extensions` declare, as the registry holds them,
/// each offered as `settings` say.
pub fn declared_by(
    extensions: &[crate::extension::extensions::Extension],
    settings: &Settings,
) -> Vec<Implementation> {
    extensions
        .iter()
        .filter(|e| e.enabled)
        .flat_map(|e| {
            e.implementations.iter().filter_map(|d| {
                let offered = d.when.as_ref().is_none_or(|when| {
                    settings
                        .get(&e.namespace)
                        .is_some_and(|values| when.holds(values))
                });
                declared(e, d).map(|mut i| {
                    i.offered = offered;
                    i
                })
            })
        })
        .collect()
}

/// What `d` declares, offered.
fn declared(
    e: &crate::extension::extensions::Extension,
    d: &crate::extension::extensions::implementations::ImplementationDecl,
) -> Option<Implementation> {
    if let Some(kind) = &d.provider {
        return provider_implementation(e, d, kind);
    }
    if let Some(script) = &d.script {
        return Some(script_implementation(e, d, script));
    }
    let b = built_in(d.entry.as_deref()?)?;
    Some(Implementation {
        capability: d.capability.clone(),
        id: d.id.clone(),
        title: d.title.clone().unwrap_or_else(|| b.title.to_string()),
        extension: Some(e.name.clone()),
        source: Source::BuiltIn(b.entry),
        features: built_in_features(b),
        offered: true,
        icon: d.icon.clone(),
        config: d.config.clone(),
        description: d.description.clone(),
    })
}

/// A script implementation, as the registry holds it: what it declares of
/// itself — its features, a task tracker's fields, stages and id pattern —
/// as an external provider's instance row says it.
fn script_implementation(
    e: &crate::extension::extensions::Extension,
    d: &crate::extension::extensions::implementations::ImplementationDecl,
    script: &crate::extension::extensions::implementations::ScriptImplementation,
) -> Implementation {
    let features = script
        .declared
        .capabilities
        .iter()
        .find(|c| c.capability == d.capability)
        .map(|c| c.features.clone())
        .filter(|f| !f.is_null())
        .unwrap_or_else(|| serde_json::json!({}));
    Implementation {
        capability: d.capability.clone(),
        id: d.id.clone(),
        title: d.title.clone().unwrap_or_else(|| d.id.clone()),
        extension: Some(e.name.clone()),
        source: Source::Script,
        features,
        offered: true,
        icon: d.icon.clone(),
        config: d.config.clone(),
        description: d.description.clone(),
    }
}

/// An AI provider, as the registry holds it: the Rust kind it is. Its one
/// feature is whether the kind answers typed questions natively.
fn provider_implementation(
    e: &crate::extension::extensions::Extension,
    d: &crate::extension::extensions::implementations::ImplementationDecl,
    kind: &str,
) -> Option<Implementation> {
    let kind = oxplow_ai_rig::kind(kind).ok()?;
    Some(Implementation {
        capability: d.capability.clone(),
        id: d.id.clone(),
        title: d.title.clone().unwrap_or_else(|| d.id.clone()),
        extension: Some(e.name.clone()),
        source: Source::ProviderKind(kind.name),
        features: serde_json::json!({ "decide_native": kind.decides_natively }),
        offered: true,
        icon: d.icon.clone(),
        config: d.config.clone(),
        description: d.description.clone(),
    })
}

/// Every skill and slash command the agent gets now: core's, and each
/// extension's that's offered (one with skills declares `agent`: it doesn't
/// load otherwise) — what it needs is active, and,
/// when an implementation lists it, that implementation is the active one,
/// and, when a setting offers it (`when:`), the extension's effective
/// setting is one of its values. A name another extension already took is
/// left out (logged).
pub async fn agent_text(svc: &crate::Services) -> oxplow_domain::agent::text::AgentText {
    let project_dir = svc.worktrees.project_dir();
    let extensions = svc.extension_catalog.get(project_dir).to_vec();
    let config = crate::config_service::read_config(&svc.config);
    let settings = offering_settings(svc, &extensions).await;
    offered_text(
        &svc.capabilities,
        &config,
        &extensions,
        &settings,
        |path, file| crate::extension::extensions::read_extension_file(project_dir, path, file),
    )
}

/// The effective configuration of each namespace among `extensions` with an
/// implementation a setting offers (`when:`), as stored now: what one
/// publication's preparation reads once, so every reader of the declared
/// implementations ([`declared_by`]) sees the same answer. One that can't be
/// read is left out, so what it offers isn't (it fails closed).
pub(crate) async fn offering_implementations(
    svc: &crate::Services,
    extensions: &[crate::extension::extensions::Extension],
) -> Settings {
    offering_implementations_tx(&svc.layout.project_dir, &svc.global_config_dir, extensions)
}

/// [`offering_implementations`] in the caller's transaction: what the
/// publication checks, after it commits, against what it was prepared with.
pub(crate) fn offering_implementations_tx(
    project: &std::path::Path,
    global: &std::path::Path,
    extensions: &[crate::extension::extensions::Extension],
) -> Settings {
    let mut out = Settings::new();
    for e in extensions
        .iter()
        .filter(|e| e.enabled && e.implementations.iter().any(|i| i.when.is_some()))
    {
        let Some(declaration) = &e.configuration else {
            continue;
        };
        if out.contains_key(&e.namespace) {
            continue;
        }
        match crate::extension::extension_configuration::effective_values(
            project,
            global,
            &e.namespace,
            declaration,
        ) {
            Ok(values) => {
                out.insert(e.namespace.clone(), values);
            }
            Err(error) => {
                let namespace = &e.namespace;
                tracing::warn!(%namespace, %error, "its settings can't be read; the implementations they offer are off");
            }
        }
    }
    out
}

/// The effective configuration of each namespace among `extensions` with a
/// skill or an agent context a setting offers (`when:`), as stored now. One
/// that can't be read (a layer at an old schema version, say) is left out,
/// so what its settings offer isn't: it fails closed, as every read of it
/// does.
pub(crate) async fn offering_settings(
    svc: &crate::Services,
    extensions: &[crate::extension::extensions::Extension],
) -> std::collections::BTreeMap<String, Value> {
    let mut wanted: Vec<(
        String,
        crate::extension::extensions::config_schema::ConfigurationFile,
    )> = Vec::new();
    for ext in extensions.iter().filter(|e| e.enabled) {
        if !(ext.skills.iter().any(|s| s.when.is_some())
            || ext.contexts.iter().any(|c| c.when.is_some()))
            || wanted.iter().any(|(ns, _)| ns == &ext.namespace)
        {
            continue;
        }
        if let Some(declaration) = &ext.configuration {
            wanted.push((ext.namespace.clone(), declaration.clone()));
        }
    }
    if wanted.is_empty() {
        return Default::default();
    }
    wanted.into_iter().filter_map(|(namespace, declaration)| {
        match crate::extension::extension_configuration::effective_values(&svc.layout.project_dir, &svc.global_config_dir, &namespace, &declaration) {
            Ok(values) => Some((namespace, values)),
            Err(error) => { tracing::warn!(%namespace, %error, "its settings can't be read; the skills they offer are left out"); None }
        }
    }).collect()
}

/// Rewrite the skills and commands of the agent runtimes already on disk
/// to what's offered now, each registered harness its own, in the project
/// and in every stream's worktree on this machine: at boot (an agent that
/// outlived an upgrade), when the extensions change, and on a
/// switch.
pub async fn refresh_agent_text(svc: &crate::Services) {
    // The offered capabilities and the extensions' files are read from one
    // generation; an unavailable one is refreshed by the next publication.
    let Ok(text) = svc.sql.read_generation(agent_text(svc)).await else {
        return;
    };
    let project_dir = &svc.layout.project_dir;
    let workspaces: Vec<std::path::PathBuf> = match svc.stream_store.list().await {
        Ok(streams) => streams
            .iter()
            .filter(|s| s.host == oxplow_domain::HostId::LOCAL)
            .map(|s| crate::worktrees::workspace_path(project_dir, &s.worktree_path))
            .collect(),
        Err(error) => {
            tracing::warn!(%error, "listing the streams' worktrees for the agent's skills failed");
            Vec::new()
        }
    };
    let roots = oxplow_domain::agent::harness::RuntimeRoots {
        project_dir: project_dir.clone(),
        workspaces,
    };
    for harness in svc.harnesses.all() {
        if let Err(error) = harness.refresh_text(&roots, &text).await {
            tracing::warn!(%error, harness = harness.id(), "refreshing the agent's skills failed");
        }
    }
}

/// On each `capability.switched`: [`refresh_agent_text`], and the
/// vocabulary rebuilt (it reads the active task tracker's ids).
pub struct AgentTextRefresh {
    services: std::sync::Weak<crate::Services>,
}

/// Register [`AgentTextRefresh`] on `svc`'s pump (boot, before it spawns).
pub fn register(svc: &std::sync::Arc<crate::Services>) {
    svc.event_pump
        .register_async(std::sync::Arc::new(AgentTextRefresh {
            services: std::sync::Arc::downgrade(svc),
        }));
}

#[async_trait::async_trait]
impl crate::event_pump::AsyncEventConsumer for AgentTextRefresh {
    fn name(&self) -> &'static str {
        "capabilities.agent_text"
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == CapabilitySwitched::TYPE
    }

    async fn handle(&self, _event: &oxplow_domain::StoredEvent) -> Result<(), DomainError> {
        if let Some(svc) = self.services.upgrade() {
            refresh_agent_text(&svc).await;
        }
        Ok(())
    }
}

/// [`agent_text`] over `extensions`, reading each file with `read`.
/// `settings` is the effective configuration of each namespace, by namespace:
/// a skill with `when:` is offered only while its namespace's holds it.
pub fn offered_text(
    registry: &CapabilityRegistry,
    config: &OxplowConfig,
    extensions: &[crate::extension::extensions::Extension],
    settings: &std::collections::BTreeMap<String, Value>,
    // `(extension's path, file)`: a file of a skill, read through its
    // extension's folder.
    read: impl Fn(&str, &str) -> Option<String>,
) -> oxplow_domain::agent::text::AgentText {
    use crate::extension::extensions::skills::SkillKind;
    let active = registry.snapshot(config);
    let mut text = oxplow_agent_text::core_text();
    for ext in extensions.iter().filter(|e| e.enabled) {
        for skill in &ext.skills {
            let owner = ext
                .implementations
                .iter()
                .find(|d| d.skills.contains(&skill.name));
            let owner_active = owner.is_none_or(|d| {
                registry.active(config, &d.capability) == d.id
                    && registry
                        .get(&d.capability, &d.id)
                        .is_some_and(|i| i.extension.as_deref() == Some(ext.name.as_str()))
            });
            let set = skill.when.as_ref().is_none_or(|when| {
                settings
                    .get(&ext.namespace)
                    .is_some_and(|values| when.holds(values))
            });
            if !owner_active || !set || !active.unmet(&skill.needs).is_empty() {
                continue;
            }
            if text.names(&skill.name) {
                tracing::warn!(extension = %ext.name, skill = %skill.name, "another extension's skill has this name; leaving it out");
                continue;
            }
            let Some(body) = read(&ext.path, &skill.file) else {
                continue;
            };
            let item = oxplow_domain::agent::text::Text {
                name: skill.name.clone(),
                body,
                references: Vec::new(),
            };
            match skill.kind {
                SkillKind::Skill => text.skills.push(item),
                SkillKind::Command => text.commands.push(item),
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin(capability: &str, id: &str, entry: &'static str) -> Implementation {
        Implementation {
            capability: capability.into(),
            id: id.into(),
            title: id.into(),
            extension: Some("oxplow-bundled".into()),
            source: Source::BuiltIn(entry),
            features: Value::Null,
            offered: true,
            icon: None,
            config: Value::Object(Default::default()),
            description: None,
        }
    }

    fn registry(declared: bool) -> CapabilityRegistry {
        let r = CapabilityRegistry::new(Vec::new(), VocabularyHandle::core());
        if declared {
            r.set_declared(vec![
                builtin("snapshots", "oxplow", "oxplow:snapshots"),
                builtin("snapshots", "hashes", "oxplow:snapshot-hashes"),
                builtin("acme_store", "acme", "oxplow:acme"),
            ]);
        }
        r
    }

    /// A config whose effective choices are `project`'s, then `local`'s
    /// over them, each with the level it came from.
    fn config(project: &[(&str, &str)], local: &[(&str, &str)]) -> OxplowConfig {
        let mut c =
            oxplow_config::load_project_config(tempfile::tempdir().unwrap().path()).unwrap();
        for (level, pairs) in [("project", project), ("local", local)] {
            for (capability, id) in pairs {
                c.active_providers
                    .insert(capability.to_string(), id.to_string());
                c.active_provider_origins
                    .insert(capability.to_string(), level.to_string());
            }
        }
        c
    }

    /// The effective choice names its level; no choice is the default.
    #[test]
    fn a_choice_is_the_effective_levels_then_the_default() {
        let r = registry(true);
        let res = |c: &OxplowConfig| r.resolve(c, "snapshots");
        assert_eq!(
            res(&config(&[], &[])),
            Resolved {
                id: "oxplow".into(),
                chosen_by: ChosenBy::Default,
                wanted: None
            }
        );
        assert_eq!(
            res(&config(&[("snapshots", "hashes")], &[])).chosen_by,
            ChosenBy::Project
        );
        let both = config(&[("snapshots", "hashes")], &[("snapshots", "oxplow")]);
        assert_eq!(
            (res(&both).id.as_str(), res(&both).chosen_by),
            ("oxplow", ChosenBy::Local)
        );
    }

    /// A choice that isn't available falls back to the capability's
    /// default, which core always has, so disabling the extension that
    /// declares the defaults leaves snapshots working. A capability core
    /// doesn't know is `none`.
    #[test]
    fn an_unavailable_choice_falls_back() {
        let r = registry(true);
        let gone = r.resolve(&config(&[("snapshots", "issues")], &[]), "snapshots");
        assert_eq!(
            gone,
            Resolved {
                id: "oxplow".into(),
                chosen_by: ChosenBy::Fallback,
                wanted: Some("issues".into())
            }
        );
        let disabled = registry(false);
        let none = config(&[], &[]);
        let snapshots = disabled.resolve(&none, "snapshots");
        assert_eq!(
            (snapshots.id.as_str(), snapshots.chosen_by),
            ("oxplow", ChosenBy::Default)
        );
        let hashes = disabled.resolve(&config(&[("snapshots", "hashes")], &[]), "snapshots");
        assert_eq!(
            (hashes.id.as_str(), hashes.chosen_by),
            ("oxplow", ChosenBy::Fallback)
        );
        assert_eq!(disabled.active(&none, "vcs"), "git");
        assert_eq!(disabled.active(&none, "no_such_capability"), NONE);
    }

    /// A provider instance is available while it runs.
    #[test]
    fn a_running_instance_is_available() {
        let r = registry(true);
        let c = config(&[("snapshots", "issues")], &[]);
        let issues = Implementation {
            capability: "snapshots".into(),
            id: "issues".into(),
            title: "Issues".into(),
            extension: Some("tracker".into()),
            source: Source::External,
            features: Value::Null,
            offered: true,
            icon: None,
            config: Value::Object(Default::default()),
            description: None,
        };
        r.set_external(issues.clone(), true);
        assert_eq!(r.active(&c, "snapshots"), "issues");
        r.set_external(issues, false);
        assert_eq!(r.active(&c, "snapshots"), "oxplow");
    }

    /// A capability many implementations serve: every declared one is
    /// active (`declared`), the default is the one used when nothing names
    /// one, and nothing is ever a switch.
    #[tokio::test]
    async fn a_many_capability_marks_every_row_active_and_never_switches() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &fx.svc;
        let c = config(&[], &[]);
        svc.capabilities.set_declared(vec![
            builtin("agent_harness", "claude", "oxplow:claude-code"),
            builtin("agent_harness", "codex", "oxplow:codex-cli"),
        ]);
        svc.capabilities.publish(&c, &svc.db).await.unwrap();
        svc.capabilities
            .set_declared(vec![builtin("agent_harness", "codex", "oxplow:codex-cli")]);
        svc.capabilities.publish(&c, &svc.db).await.unwrap();
        let rows = svc
            .sql
            .query_sql(
                "SELECT provider, active, chosen_by FROM v_capability_provider
                  WHERE capability = 'agent_harness' ORDER BY provider",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&rows.rows).unwrap(),
            serde_json::json!([["codex", 1, "declared"]])
        );
        let resolved = svc.capabilities.resolve(&c, "agent_harness");
        assert_eq!(
            (resolved.id.as_str(), resolved.chosen_by),
            ("claude", ChosenBy::Default)
        );
        let switched = svc
            .db
            .read(|tx| {
                tx.query_row(
                    "SELECT count(*) FROM event_log WHERE type = 'capability.switched'
                       AND json_extract(payload, '$.capability') = 'agent_harness'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(switched, 0);
    }

    /// A need on a many-capability is met when any implementation is
    /// declared, and a need on one of its features when any declares it.
    #[test]
    fn a_feature_need_on_a_many_capability_is_met_by_the_union() {
        let r = registry(false);
        let c = config(&[], &[]);
        let needs = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            r.snapshot(&c).unmet(&needs(&["agent_harness"])),
            needs(&["agent_harness"])
        );
        let with = |id: &str, features: Value| Implementation {
            features,
            ..builtin("agent_harness", id, "oxplow:claude-code")
        };
        r.set_declared(vec![
            with("claude", serde_json::json!({ "terminal": true })),
            with("api", serde_json::json!({ "programmatic": true })),
        ]);
        let snap = r.snapshot(&c);
        assert!(snap
            .unmet(&needs(&[
                "agent_harness",
                "agent_harness.programmatic",
                "agent_harness.terminal"
            ]))
            .is_empty());
        assert_eq!(
            snap.unmet(&needs(&["agent_harness.resume"])),
            needs(&["agent_harness.resume"])
        );
    }

    /// With `oxplow-bundled` disabled, the defaults still run: snapshots
    /// are `oxplow-foundation`'s (core's own snapshots default is the
    /// active row). The task tracker has no implementation of core's own:
    /// it resolves to none.
    #[tokio::test]
    async fn disabling_the_bundled_extension_leaves_core_usable() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &fx.svc;
        std::fs::create_dir_all(svc.layout.project_dir.join(".oxplow")).unwrap();
        std::fs::write(
            oxplow_config::config_path(&svc.layout.project_dir),
            crate::test_fixtures::optional_shipped_disabled(),
        )
        .unwrap();
        svc.extension_catalog.changed();
        crate::test_fixtures::activate_with_problems(svc).await;
        let config = crate::config_service::read_config(&svc.config);
        let snapshots = svc.capabilities.resolve(&config, "snapshots");
        assert_eq!(
            (snapshots.id.as_str(), snapshots.chosen_by),
            ("oxplow", ChosenBy::Default)
        );
        let rows = svc
            .sql
            .query_sql(
                "SELECT provider, active, available, source FROM v_capability_provider
                  WHERE capability = 'snapshots'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&rows.rows).unwrap(),
            serde_json::json!([["hashes", 0, 1, "builtin"], ["oxplow", 1, 1, "builtin"]])
        );
    }

    /// The rows say which is active and why, and a choice that isn't
    /// available is listed as such.
    #[tokio::test]
    async fn the_rows_say_what_is_active_and_why() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let c = config(&[("snapshots", "issues")], &[]);
        fx.svc.capabilities.publish(&c, &fx.svc.db).await.unwrap();
        let out = fx
            .svc
            .sql
            .query_sql(
                "SELECT capability, provider, active, chosen_by, available FROM v_capability_provider
                  WHERE capability = 'snapshots' ORDER BY capability, provider",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            serde_json::json!([
                ["snapshots", "hashes", 0, null, 1],
                ["snapshots", "issues", 0, null, 0],
                ["snapshots", "oxplow", 1, "fallback", 1],
            ])
        );
    }

    /// A change of what's active is logged once, as
    /// `capability.switched`: not the first statement, nor an unchanged
    /// one.
    #[tokio::test]
    async fn a_change_of_the_active_one_is_logged() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &fx.svc;
        let default = config(&[], &[]);
        let off = config(&[("snapshots", "hashes")], &[]);
        // (Startup logged its own choices; only what follows is checked.)
        let startup = svc
            .db
            .read(|tx| {
                tx.query_row("SELECT coalesce(max(seq), 0) FROM event_log", [], |r| {
                    r.get::<_, i64>(0)
                })
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        for c in [&default, &default, &off, &off] {
            svc.capabilities.publish(c, &svc.db).await.unwrap();
        }
        let switches: Vec<String> = svc
            .db
            .read(move |tx| {
                let mut stmt = tx
                    .prepare(
                        "SELECT payload FROM event_log WHERE type = 'capability.switched'
                          AND seq > ?1 ORDER BY seq",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = stmt
                    .query_map([startup], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let switches: Vec<Value> = switches
            .iter()
            .map(|p| serde_json::from_str(p).unwrap())
            .collect();
        assert_eq!(
            switches,
            vec![serde_json::json!({
                "capability": "snapshots",
                "from": "oxplow",
                "to": "hashes",
                "chosen_by": "project",
            })]
        );
    }

    /// A feature the chosen snapshots implementation doesn't declare hides
    /// what needs it.
    #[test]
    fn a_missing_feature_hides_what_needs_it() {
        let r = registry(true);
        let needs = |n: &str| vec![n.to_string()];
        let keeping = r.snapshot(&config(&[], &[]));
        assert_eq!(
            keeping.command_refusal("x", &needs("snapshots.contents")),
            None
        );
        let hashes = r.snapshot(&config(&[("snapshots", "hashes")], &[]));
        assert!(hashes
            .command_refusal("x", &needs("snapshots.contents"))
            .is_some());
    }

    /// A provider instance's own commands are offered only while it's the
    /// active implementation.
    #[test]
    fn an_instance_namespace_follows_the_active_list() {
        let r = registry(true);
        r.set_external(
            Implementation {
                capability: "snapshots".into(),
                id: "fake".into(),
                title: "Fake".into(),
                extension: Some("tracker".into()),
                source: Source::External,
                features: Value::Null,
                offered: true,
                icon: None,
                config: serde_json::json!({}),
                description: None,
            },
            true,
        );
        let default = r.snapshot(&config(&[], &[]));
        assert!(default
            .command_refusal("fake.sync_now", &[])
            .is_some_and(|m| m.contains("Fake's")));
        let fake = r.snapshot(&config(&[("snapshots", "fake")], &[]));
        assert_eq!(fake.command_refusal("fake.sync_now", &[]), None);
    }

    /// A runtime already on disk gets what's offered (boot refreshes it
    /// for an agent that outlived an upgrade): core's skills.
    #[tokio::test]
    async fn the_runtimes_skills_follow_what_is_offered() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &fx.svc;
        let plugin = svc.layout.project_dir.join(".oxplow/runtime/claude-plugin");
        let (skills, commands) = (plugin.join("skills"), plugin.join("commands"));
        std::fs::create_dir_all(skills.join("oxplow-extension")).unwrap();
        std::fs::create_dir_all(&commands).unwrap();
        std::fs::write(skills.join("oxplow-extension/SKILL.md"), "stale").unwrap();
        // A harness that keeps its skills beside a stream's worktree
        // (Codex) has them refreshed there.
        let stream = svc.stream_store.list().await.unwrap().remove(0);
        let in_worktree =
            crate::worktrees::workspace_path(&svc.layout.project_dir, &stream.worktree_path)
                .join(".agents/skills/oxplow-extension");
        std::fs::create_dir_all(&in_worktree).unwrap();
        std::fs::write(in_worktree.join("SKILL.md"), "stale").unwrap();
        std::fs::write(in_worktree.join(".oxplow"), "").unwrap();
        refresh_agent_text(svc).await;
        let current = oxplow_agent_text::core_text()
            .skill_body("oxplow-extension")
            .unwrap()
            .to_string();
        for skill in [skills.join("oxplow-extension"), in_worktree] {
            assert_eq!(
                std::fs::read_to_string(skill.join("SKILL.md")).unwrap(),
                current,
                "{skill:?}"
            );
        }
        assert!(commands.join("configure.md").is_file());
    }
}

#[cfg(test)]
mod contribution_tests {
    /// A refresh publishes every enabled extension's contributions as
    /// `v_contribution`, a disabled one's taken out.
    #[tokio::test]
    async fn a_refresh_publishes_the_contributions() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let dir = fx.svc.layout.project_dir.join(".oxplow/extensions/acme");
        std::fs::create_dir_all(dir.join("skills")).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            "manifest: 2\nname: acme\nversion: 1.2.0\nintent:\n  purpose: p\n  examples: [{ name: a }]\npermissions: [agent.write]\nskills:\n  - { name: deploy, file: skills/deploy.md }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("skills/deploy.md"),
            "---\nname: deploy\ndescription: d\n---\nShip it.\n",
        )
        .unwrap();
        fx.svc.extension_catalog.changed();
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        let rows = fx
            .svc
            .sql
            .query_sql(
                "SELECT uri, kind, version, global_name FROM v_contribution WHERE extension = 'acme'",
                Vec::new(),
                None,
            )
            .await
            .unwrap()
            .rows;
        use oxplow_db::SqlCell::Text;
        assert_eq!(
            rows,
            vec![vec![
                Text("oxplow:oxplow/skill?skill=acme/deploy".into()),
                Text("skill".into()),
                Text("1.2.0".into()),
                Text("deploy".into()),
            ]]
        );
        // Bundled extensions' contributions are listed too, at oxplow's version.
        let bundled = fx
            .svc
            .sql
            .query_sql(
                "SELECT count(*) FROM v_contribution WHERE origin = 'bundled' AND version = ?1",
                vec![oxplow_db::SqlCell::Text(env!("CARGO_PKG_VERSION").into())],
                None,
            )
            .await
            .unwrap()
            .rows;
        assert!(
            matches!(bundled[0][0], oxplow_db::SqlCell::Int(n) if n > 0),
            "{bundled:?}"
        );
    }

    /// What a built-in harness declares is what it does: a harness lists
    /// its saved conversations exactly when it declares `conversation_list`
    /// (the others answer "unsupported", never a guess or an empty list),
    /// and every feature it declares is one the vocabulary knows, so a
    /// failure is attributable to a version of a declared capability.
    #[tokio::test]
    async fn a_built_in_harness_answers_unsupported_exactly_for_what_it_doesnt_declare() {
        use oxplow_domain::agent::harness::{ConversationQuery, HarnessError};
        let vocabulary: Vec<&str> = oxplow_domain::capability::CAPABILITIES
            .iter()
            .find(|c| c.id == "agent_harness")
            .map(|c| c.features.to_vec())
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut checked = 0;
        for b in super::BUILT_INS
            .iter()
            .filter(|b| b.capability == "agent_harness")
        {
            for f in b.features {
                assert!(
                    vocabulary.contains(f),
                    "{} declares unknown feature {f}",
                    b.entry
                );
            }
            let id = b.entry.trim_start_matches("oxplow:");
            let h = oxplow_harnesses::built_in(b.entry, id, b.title).expect("a built-in harness");
            let query = ConversationQuery {
                cwd: dir.path().to_path_buf(),
                home: Some(dir.path().to_path_buf()),
                search_path: vec![],
                limit: 1,
            };
            let unsupported = matches!(
                h.conversations(&query).await,
                Err(HarnessError::Unsupported { .. })
            );
            assert_eq!(
                unsupported,
                !b.features.contains(&"conversation_list"),
                "{} lists its conversations iff it declares conversation_list",
                b.entry
            );
            checked += 1;
        }
        assert_eq!(checked, 3);
    }
}

#[cfg(test)]
mod setting_tests {
    use oxplow_config::settings::Level;
    use oxplow_domain::Actor;
    use serde_json::{json, Value};

    use crate::test_fixtures::ThreadFixture;

    /// `acme`: a pick-one `mode` (default `a`), a skill offered while it's
    /// `a` and one while it's `b`, and its native settings write.
    fn declare(fx: &ThreadFixture) {
        let dir = fx.svc.layout.project_dir.join(".oxplow/extensions/acme");
        std::fs::create_dir_all(dir.join("skills")).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            "manifest: 2
name: acme
intent:
  purpose: p
  examples: [{ name: a }]
permissions: [agent.write]
configuration:
  version: 1
  schema:
    type: object
    additionalProperties: false
    properties:
      mode: { type: string, enum: [a, b] }
  defaults: { mode: a }
skills:
  - { name: on-a, file: skills/on-a.md, when: { config: /mode, in: [a] } }
  - { name: on-b, file: skills/on-b.md, when: { config: /mode, in: [b] } }
  - { name: always, file: skills/always.md }
",
        )
        .unwrap();
        for name in ["on-a", "on-b", "always"] {
            std::fs::write(
                dir.join(format!("skills/{name}.md")),
                format!("---\nname: {name}\ndescription: d\n---\nGo.\n"),
            )
            .unwrap();
        }
        fx.svc.extension_catalog.changed();
    }

    async fn offered(fx: &ThreadFixture) -> Vec<String> {
        let mut names: Vec<String> = fx
            .svc
            .sql
            .read_generation(super::agent_text(&fx.svc))
            .await
            .unwrap()
            .skills
            .iter()
            .map(|s| s.name.clone())
            .filter(|n| ["on-a", "on-b", "always"].contains(&n.as_str()))
            .collect();
        names.sort();
        names
    }

    async fn store(fx: &ThreadFixture, level: Level, values: Value) {
        let extensions = fx
            .svc
            .extension_catalog
            .live_snapshot(&fx.svc.layout.project_dir);
        crate::settings::store(
            &fx.svc.layout.project_dir,
            &fx.svc.global_config_dir,
            fx.svc.secrets.clone(),
            extensions.extensions(),
        )
        .unwrap()
        .set("acme.mode", values["mode"].clone(), level)
        .unwrap();
    }

    /// A skill with `when:` is offered only while the extension's effective
    /// setting is one of its values: the default, then a project layer over
    /// it, then a personal layer over the project's.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_skill_is_offered_by_the_effective_setting() {
        let fx = crate::test_fixtures::services_with_thread().await;
        declare(&fx);
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert_eq!(offered(&fx).await, ["always", "on-a"], "the default is a");

        store(&fx, Level::Project, json!({ "mode": "b" })).await;
        assert_eq!(offered(&fx).await, ["always", "on-b"], "the project's b");

        store(&fx, Level::Local, json!({ "mode": "a" })).await;
        assert_eq!(
            offered(&fx).await,
            ["always", "on-a"],
            "the person's a over the project's b"
        );
    }

    /// A settings write through the extension's own command republishes, and
    /// the runtimes already on disk get the skills the new value offers: no
    /// restart.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_settings_write_rewrites_the_agents_skills() {
        let fx = crate::test_fixtures::services_with_thread().await;
        declare(&fx);
        let skills = fx
            .svc
            .layout
            .project_dir
            .join(".oxplow/runtime/claude-plugin/skills");
        std::fs::create_dir_all(&skills).unwrap();
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        super::refresh_agent_text(&fx.svc).await;
        assert!(skills.join("on-a/SKILL.md").is_file());
        assert!(!skills.join("on-b").exists());

        fx.svc.extension_activation.spawn(&fx.svc);
        fx.svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.config.set",
                json!({ "key": "acme.mode", "value": "b", "level": "project" }),
                false,
            )
            .await
            .unwrap();
        // What the event pump does when the write's `command.executed` arrives.
        fx.svc
            .extension_activation
            .request_config_refresh(
                &fx.svc.db,
                &fx.svc.layout.project_dir,
                &fx.svc.global_config_dir,
            )
            .await
            .unwrap();
        assert!(
            crate::test_fixtures::until(std::time::Duration::from_secs(20), || async {
                skills.join("on-b/SKILL.md").is_file()
            })
            .await,
            "the write republished and rewrote the skills"
        );
        assert!(!skills.join("on-a").exists(), "on-a is no longer offered");
    }

    /// oxtask's task skills (`next-task`, `track-work`, `work-tasks`) are
    /// offered while the task model has a live store, as oxtask's workflow
    /// setting says; with the workflow off none of them is.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_task_skills_follow_the_task_workflow() {
        let fx = crate::test_fixtures::services_developing_oxplow().await;
        for name in ["oxplow-bundled", "oxtask"] {
            crate::extension::extension_deploy::deploy(&fx.svc, name, true)
                .await
                .expect("deploys from its source");
        }
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        let offered = || async {
            let text = fx
                .svc
                .sql
                .read_generation(super::agent_text(&fx.svc))
                .await
                .unwrap();
            let mut names: Vec<String> = text
                .skills
                .iter()
                .chain(&text.commands)
                .map(|s| s.name.clone())
                .filter(|n| ["track-work", "work-tasks", "next-task"].contains(&n.as_str()))
                .collect();
            names.sort();
            names
        };
        assert_eq!(offered().await, ["next-task", "track-work"], "the default");
        let extensions = fx
            .svc
            .extension_catalog
            .live_snapshot(&fx.svc.layout.project_dir);
        let store = crate::settings::store(
            &fx.svc.layout.project_dir,
            &fx.svc.global_config_dir,
            fx.svc.secrets.clone(),
            extensions.extensions(),
        )
        .unwrap();
        for (workflow, skills) in [
            ("person_files", vec!["next-task", "work-tasks"]),
            ("custom", vec!["next-task"]),
            ("off", vec![]),
        ] {
            store
                .set("oxtask.tasks.workflow", json!(workflow), Level::Project)
                .unwrap();
            assert_eq!(offered().await, skills, "{workflow}");
        }
    }
}
