//! Which implementation of each capability is active
//! (`.context/work-tracking.md` "Swappable pieces").
//!
//! Core declares the capabilities (`oxplow_domain::capability`). The
//! implementations come from three places, all held by one
//! [`CapabilityRegistry`]:
//!
//! - **core**: the fixed ones (the VCS, knowledge) and, for every optional
//!   capability, [`NONE`] — nothing implements it;
//! - **extensions**: `implementations:` naming a built-in in core's
//!   standard library ([`BUILT_INS`], `entry: oxplow:tasks`), the way a
//!   collector names `oxplow:junit`. `oxplow-bundled` declares the
//!   defaults; disabling it takes them away;
//! - **provider instances** while they run.
//!
//! [`CapabilityRegistry::resolve`] picks the active one: a person's own
//! choice (`.oxplow/personal.yaml`), else the project's
//! (`activeProviders`), else the capability's default. A choice that isn't
//! available falls back: an optional capability to [`NONE`], a required
//! one to its default built-in, which core always has. The registry
//! writes what it holds, with the active row and why, to
//! `v_capability_provider`.

use std::sync::RwLock;

use oxplow_config::OxplowConfig;
use oxplow_db::capability_store::{list_tx, reset_tx};
use oxplow_db::CapabilityProvider;
pub use oxplow_domain::capability::ChosenBy;
use oxplow_domain::capability::{self, NONE};
use oxplow_domain::events::schema::{CapabilitySwitched, CapabilitySwitchedV1, EventType as _};
use oxplow_domain::events::Envelope;
use oxplow_domain::vocabulary::VocabularyHandle;
use oxplow_domain::DomainError;
use serde_json::Value;

/// Core's standard library of implementations, by entry name, with the
/// capability each implements. An extension declares one by naming it;
/// core resolves the name, like a collector's `oxplow:junit`.
pub const BUILT_INS: &[BuiltIn] = &[
    BuiltIn {
        entry: "oxplow:tasks",
        capability: "work_items",
        title: "oxplow's tasks",
        features: &[
            "hierarchy",
            "comments",
            "links",
            "delete",
            "ordering",
            "lists",
        ],
        tools: &["list_tasks", "get_task", "read_task_options"],
        id_pattern: Some(r"tsk\d+"),
        fields: &[
            BuiltInField {
                name: "priority",
                title: "Priority",
                kind: oxplow_domain::work_items::FieldKind::Enum,
                values: &["urgent", "high", "medium", "low"],
                read_only: false,
            },
            // Who filed it: set from the actor that created it.
            BuiltInField {
                name: "author",
                title: "Filed by",
                kind: oxplow_domain::work_items::FieldKind::Enum,
                values: &["user", "agent"],
                read_only: true,
            },
        ],
    },
    BuiltIn {
        entry: "oxplow:commit-or-switch",
        capability: "effort_policy",
        title: "A commit lands it, or the task switches",
        features: &[],
        tools: &[],
        id_pattern: None,
        fields: &[],
    },
    BuiltIn {
        entry: "oxplow:snapshots",
        capability: "snapshots",
        title: "Keep every version",
        features: &["contents"],
        tools: &[],
        id_pattern: None,
        fields: &[],
    },
];

/// One built-in implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltIn {
    pub entry: &'static str,
    pub capability: &'static str,
    pub title: &'static str,
    /// The features it has — core's to say, since it's core's code.
    pub features: &'static [&'static str],
    /// The MCP tools only it offers (its own agent surface): offered while
    /// it's the active implementation, hidden otherwise.
    pub tools: &'static [&'static str],
    /// What its items' own ids look like (a work list's; a regex matched
    /// whole): how a loose id in text or a command is one of its items.
    pub id_pattern: Option<&'static str>,
    /// Its own fields (a work list's, kept in `native`).
    pub fields: &'static [BuiltInField],
}

/// A built-in's field, as core's table declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltInField {
    pub name: &'static str,
    pub title: &'static str,
    pub kind: oxplow_domain::work_items::FieldKind,
    pub values: &'static [&'static str],
    pub read_only: bool,
}

/// A built-in's fields, as an implementation declares them.
fn built_in_fields(b: &BuiltIn) -> Value {
    serde_json::to_value(
        b.fields
            .iter()
            .map(|f| oxplow_domain::work_items::FieldDecl {
                name: f.name.into(),
                title: f.title.into(),
                kind: f.kind,
                values: f.values.iter().map(|v| v.to_string()).collect(),
                read_only: f.read_only,
            })
            .collect::<Vec<_>>(),
    )
    .expect("fields serialize")
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
    /// Core's own (the VCS, knowledge).
    Core,
    /// A built-in in core's standard library, by its entry.
    BuiltIn(&'static str),
    /// A provider instance's process.
    External,
    /// Nothing implements it.
    None,
}

impl Source {
    fn as_str(&self) -> &'static str {
        match self {
            Source::Core => "core",
            Source::BuiltIn(_) => "builtin",
            Source::External => "external",
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
    /// Its own fields, as it declares them (a JSON array of `FieldDecl`).
    pub fields: Value,
    /// What its items' own ids look like (a work list's), when it says.
    pub id_pattern: Option<String>,
}

impl Implementation {
    /// What only it offers: a built-in's declared tools; a provider
    /// instance's command namespace (`<id>.*`).
    fn surface(&self) -> (Vec<String>, Vec<String>) {
        match &self.source {
            Source::BuiltIn(entry) => built_in(entry).map_or_else(Default::default, |b| {
                (Vec::new(), b.tools.iter().map(|t| t.to_string()).collect())
            }),
            Source::External => (vec![format!("{}.*", self.id)], Vec::new()),
            Source::Core | Source::None => Default::default(),
        }
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
            fields: Value::Array(Vec::new()),
            // Nothing in text is none's: it keeps no items.
            id_pattern: None,
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
/// a declared need is checked against (a lens, an advisory).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Active {
    by_capability: std::collections::BTreeMap<String, (String, Vec<String>)>,
    /// What implementations that aren't active own, kept from offering:
    /// command names (or `<namespace>.*`) and tool names.
    hidden_commands: Vec<Hidden>,
    hidden_tools: Vec<Hidden>,
}

/// A command or tool its owner, not being active, keeps from offering.
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
            "`{name}` is {}'s, which isn't the active {} (choose one in Settings → Pieces).",
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

    /// Why the MCP tool `name` isn't offered, or `None` when it is.
    pub fn tool_refusal(&self, name: &str) -> Option<String> {
        self.hidden_tools
            .iter()
            .find(|h| h.matches(name))
            .map(|h| h.message(name))
    }

    pub fn offers_tool(&self, name: &str) -> bool {
        self.tool_refusal(name).is_none()
    }

    /// The needs in `needs` it doesn't meet: a capability whose active
    /// implementation is none, or a feature the active one doesn't have
    /// (none has every feature: it takes everything, keeping nothing).
    pub fn unmet(&self, needs: &[String]) -> Vec<String> {
        needs
            .iter()
            // A host capability is always there; what may use it is a
            // separate question (`.context/commands.md`).
            .filter(|need| oxplow_domain::host_capability::host_capability(need).is_none())
            .filter(|need| {
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
        "Needs: {} (choose one in Settings → Pieces).",
        named.join(", ")
    )
}

/// Every implementation oxplow has now: core's, the ones extensions
/// declare, and running provider instances'.
pub struct CapabilityRegistry {
    core: RwLock<Vec<Implementation>>,
    declared: RwLock<Vec<Implementation>>,
    external: RwLock<Vec<Implementation>>,
    /// What `capability.switched` is validated against.
    vocabulary: VocabularyHandle,
}

impl CapabilityRegistry {
    /// A registry with core's fixed implementations (`vcs`, `knowledge`)
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
                        fields: built_in_fields(b),
                        id_pattern: b.id_pattern.map(str::to_string),
                    })
                }),
        );
        Self {
            core: RwLock::new(core),
            declared: RwLock::default(),
            external: RwLock::default(),
            vocabulary,
        }
    }

    /// Add one of core's own (built after the registry: knowledge).
    pub fn add_core(&self, implementation: Implementation) {
        self.core
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(implementation);
    }

    /// Replace what extensions declare (on boot, and when the catalog
    /// changes).
    pub fn set_declared(&self, declared: Vec<Implementation>) {
        *self.declared.write().unwrap_or_else(|e| e.into_inner()) = declared;
    }

    /// A provider instance started (`true`) or stopped.
    pub fn set_external(&self, implementation: Implementation, running: bool) {
        let mut external = self.external.write().unwrap_or_else(|e| e.into_inner());
        external
            .retain(|i| !(i.capability == implementation.capability && i.id == implementation.id));
        if running {
            external.push(implementation);
        }
    }

    /// Every implementation, core's first.
    pub fn implementations(&self) -> Vec<Implementation> {
        let mut all = self.core.read().unwrap_or_else(|e| e.into_inner()).clone();
        all.extend(
            self.declared
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .cloned(),
        );
        all.extend(
            self.external
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .cloned(),
        );
        all
    }

    /// `capability`'s implementation `id`, if available.
    pub fn get(&self, capability: &str, id: &str) -> Option<Implementation> {
        self.implementations()
            .into_iter()
            .find(|i| i.capability == capability && i.id == id)
    }

    /// `capability`'s active implementation under `config`, and why.
    pub fn resolve(&self, config: &OxplowConfig, capability: &str) -> Resolved {
        let Some(spec) = capability::spec(capability) else {
            return Resolved {
                id: NONE.into(),
                chosen_by: ChosenBy::Fallback,
                wanted: None,
            };
        };
        let (wanted, chosen_by) = if !spec.choosable {
            (spec.default.to_string(), ChosenBy::Default)
        } else if let Some(id) = config.personal_active_providers.get(capability) {
            (id.clone(), ChosenBy::Personal)
        } else if let Some(id) = config.active_providers.get(capability) {
            (id.clone(), ChosenBy::Project)
        } else {
            (spec.default.to_string(), ChosenBy::Default)
        };
        if self.get(capability, &wanted).is_some() {
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
        let mut by_capability = std::collections::BTreeMap::new();
        for spec in capability::CAPABILITIES {
            let id = self.active(config, spec.id);
            let features = self
                .get(spec.id, &id)
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
        // What each implementation owns, kept from offering unless it's
        // the active one: every built-in's (declared or not — the bundled
        // extension off is the same as another one chosen) and every
        // running instance's.
        let mut owners: Vec<(Implementation, bool)> = BUILT_INS
            .iter()
            .map(|b| Implementation {
                capability: b.capability.into(),
                id: b.entry.into(),
                title: b.title.into(),
                extension: None,
                source: Source::BuiltIn(b.entry),
                features: Value::Null,
                fields: serde_json::Value::Array(Vec::new()),
                id_pattern: None,
            })
            .map(|i| {
                let active = by_capability
                    .get(&i.capability)
                    .and_then(|(id, _)| self.get(&i.capability, id))
                    .is_some_and(|a| a.source == i.source);
                (i, active)
            })
            .collect();
        owners.extend(
            self.external
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|i| {
                    (
                        i.clone(),
                        by_capability
                            .get(&i.capability)
                            .is_some_and(|(a, _)| a == &i.id),
                    )
                }),
        );
        let mut hidden_commands = Vec::new();
        let mut hidden_tools = Vec::new();
        for (owner, _) in owners.iter().filter(|(_, active)| !active) {
            let (commands, tools) = owner.surface();
            let hide = |pattern: String| Hidden {
                pattern,
                owner: owner.title.clone(),
                capability: owner.capability.clone(),
            };
            hidden_commands.extend(commands.into_iter().map(hide));
            hidden_tools.extend(tools.into_iter().map(hide));
        }
        Active {
            by_capability,
            hidden_commands,
            hidden_tools,
        }
    }

    /// The active work list's id recognizer under `config`: its id and
    /// declared `id_pattern`, if it declares one (none doesn't).
    pub fn work_item_ids(&self, config: &OxplowConfig) -> Option<(String, String)> {
        let id = self.active(config, "work_items");
        let pattern = self.get("work_items", &id)?.id_pattern?;
        Some((id, pattern))
    }

    /// `capability`'s active implementation id under `config`.
    pub fn active(&self, config: &OxplowConfig, capability: &str) -> String {
        self.resolve(config, capability).id
    }

    /// Restate `v_capability_provider` from what the registry holds: every
    /// implementation, the active one marked with why, and a choice that
    /// isn't available as its own row (`available = 0`). A capability whose
    /// active implementation differs from the one the rows had logs
    /// `capability.switched@1`, in the same transaction — so a change is
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
    /// first read of `v_capability_provider` (and of `v_work_item`, which
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
        let all = self.implementations();
        let mut rows: Vec<CapabilityProvider> = Vec::new();
        let mut now = Vec::new();
        for spec in capability::CAPABILITIES {
            let resolved = self.resolve(config, spec.id);
            for i in all.iter().filter(|i| i.capability == spec.id) {
                let active = i.id == resolved.id;
                rows.push(CapabilityProvider {
                    capability: i.capability.clone(),
                    provider: i.id.clone(),
                    extension: i.extension.clone(),
                    features: i.features.clone(),
                    active,
                    title: i.title.clone(),
                    source: i.source.as_str().into(),
                    available: true,
                    chosen_by: active.then(|| resolved.chosen_by.as_str().to_string()),
                    capability_title: spec.title.into(),
                    choosable: spec.choosable,
                    optional: spec.optional,
                    fields: i.fields.clone(),
                    id_pattern: i.id_pattern.clone(),
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
                        fields: Value::Array(Vec::new()),
                        id_pattern: None,
                    });
                }
            }
            now.push((spec.id, resolved));
        }
        (rows, now)
    }
}

/// Restate the rows and log each switch (`capability.switched@1`), in one
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
fn switches(before: &[CapabilityProvider], now: &[(&str, Resolved)]) -> Vec<CapabilitySwitchedV1> {
    now.iter()
        .filter_map(|(capability, resolved)| {
            let from = before
                .iter()
                .find(|r| r.active && r.capability == *capability)?;
            (from.provider != resolved.id).then(|| CapabilitySwitchedV1 {
                capability: capability.to_string(),
                from: from.provider.clone(),
                to: resolved.id.clone(),
                chosen_by: resolved.chosen_by,
            })
        })
        .collect()
}

/// The implementations `extensions` declare, as the registry holds them.
pub fn declared_by(extensions: &[crate::extensions::Extension]) -> Vec<Implementation> {
    extensions
        .iter()
        .filter(|e| e.enabled)
        .flat_map(|e| {
            e.implementations.iter().filter_map(|d| {
                let b = built_in(&d.entry)?;
                Some(Implementation {
                    capability: d.capability.clone(),
                    id: d.id.clone(),
                    title: d.title.clone().unwrap_or_else(|| b.title.to_string()),
                    extension: Some(e.name.clone()),
                    source: Source::BuiltIn(b.entry),
                    features: built_in_features(b),
                    fields: built_in_fields(b),
                    id_pattern: b.id_pattern.map(str::to_string),
                })
            })
        })
        .collect()
}

/// Every skill and slash command the agent gets now: core's, and each
/// consented extension's that's offered — what it needs is active, and,
/// when an implementation lists it, that implementation is the active one.
/// A name another extension already took is left out (logged).
pub fn agent_text(svc: &crate::Services) -> oxplow_plugin::AgentText {
    let project_dir = svc.worktrees.project_dir();
    let extensions =
        crate::advisories::consented(&svc.approvals, &svc.extension_catalog.get(project_dir));
    let config = crate::config_service::read_config(&svc.config);
    offered_text(&svc.capabilities, &config, &extensions, |ext, file| {
        crate::extensions::read_extension_file(project_dir, ext, file)
    })
}

/// Rewrite the skills and commands of the agent runtimes already on disk
/// to what's offered now: at boot (an agent that outlived an upgrade,
/// tsk376), when the extensions change, and on a switch.
pub fn refresh_agent_text(svc: &crate::Services) {
    if let Err(error) = oxplow_plugin::refresh_skills(&svc.layout.project_dir, &agent_text(svc)) {
        tracing::warn!(%error, "refreshing the agent's skills failed");
    }
}

/// On each `capability.switched`: [`refresh_agent_text`], and the
/// vocabulary rebuilt (it reads the active work list's ids).
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
            refresh_agent_text(&svc);
            // The vocabulary reads the active list's ids.
            svc.vocabulary_service.sync().await?;
        }
        Ok(())
    }
}

/// [`agent_text`] over `extensions`, reading each file with `read`.
pub fn offered_text(
    registry: &CapabilityRegistry,
    config: &OxplowConfig,
    extensions: &[crate::extensions::Extension],
    read: impl Fn(&str, &str) -> Option<String>,
) -> oxplow_plugin::AgentText {
    use crate::extensions::skills::SkillKind;
    let active = registry.snapshot(config);
    let mut text = oxplow_plugin::AgentText::core();
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
            if !owner_active || !active.unmet(&skill.needs).is_empty() {
                continue;
            }
            if text.names(&skill.name) {
                tracing::warn!(extension = %ext.name, skill = %skill.name, "another extension's skill has this name; leaving it out");
                continue;
            }
            let Some(body) = read(&ext.name, &skill.file) else {
                continue;
            };
            let item = oxplow_plugin::Text {
                name: skill.name.clone(),
                body,
            };
            match skill.kind {
                SkillKind::Skill => text.skills.push(item),
                SkillKind::Command => text.commands.push(item),
            }
        }
    }
    text
}

/// Restate the registry's declared implementations from the project's
/// extensions and publish the rows.
pub async fn refresh(svc: &crate::Services) -> Result<(), DomainError> {
    let extensions = svc.extension_catalog.get(&svc.layout.project_dir);
    svc.capabilities.set_declared(declared_by(&extensions));
    let config = crate::config_service::read_config(&svc.config);
    svc.capabilities.publish(&config, &svc.db).await
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
            fields: serde_json::Value::Array(Vec::new()),
            id_pattern: None,
        }
    }

    fn registry(declared: bool) -> CapabilityRegistry {
        let r = CapabilityRegistry::new(Vec::new(), VocabularyHandle::core());
        if declared {
            r.set_declared(vec![
                builtin("work_items", "oxplow", "oxplow:tasks"),
                builtin("effort_policy", "oxplow", "oxplow:commit-or-switch"),
            ]);
        }
        r
    }

    fn config(project: &[(&str, &str)], personal: &[(&str, &str)]) -> OxplowConfig {
        let mut c =
            oxplow_config::load_project_config(tempfile::tempdir().unwrap().path()).unwrap();
        let map = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        c.active_providers = map(project);
        c.personal_active_providers = map(personal);
        c
    }

    /// A person's choice over the project's over the default.
    #[test]
    fn a_choice_is_the_persons_then_the_projects_then_the_default() {
        let r = registry(true);
        let res = |c: &OxplowConfig| r.resolve(c, "effort_policy");
        assert_eq!(
            res(&config(&[], &[])),
            Resolved {
                id: "oxplow".into(),
                chosen_by: ChosenBy::Default,
                wanted: None
            }
        );
        assert_eq!(
            res(&config(&[("effort_policy", "none")], &[])).chosen_by,
            ChosenBy::Project
        );
        let both = config(&[("effort_policy", "none")], &[("effort_policy", "oxplow")]);
        assert_eq!(
            (res(&both).id.as_str(), res(&both).chosen_by),
            ("oxplow", ChosenBy::Personal)
        );
    }

    /// A choice that isn't available falls back: an optional capability to
    /// none, a required one to its default — which core always has, so
    /// disabling the extension that declares the defaults leaves snapshots
    /// working.
    #[test]
    fn an_unavailable_choice_falls_back() {
        let r = registry(true);
        let gone = r.resolve(&config(&[("work_items", "issues")], &[]), "work_items");
        assert_eq!(
            gone,
            Resolved {
                id: NONE.into(),
                chosen_by: ChosenBy::Fallback,
                wanted: Some("issues".into())
            }
        );
        let disabled = registry(false);
        let none = config(&[], &[]);
        assert_eq!(disabled.active(&none, "work_items"), NONE);
        assert_eq!(disabled.active(&none, "effort_policy"), NONE);
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
    }

    /// A provider instance is available while it runs.
    #[test]
    fn a_running_instance_is_available() {
        let r = registry(true);
        let c = config(&[("work_items", "issues")], &[]);
        let issues = Implementation {
            capability: "work_items".into(),
            id: "issues".into(),
            title: "Issues".into(),
            extension: Some("tracker".into()),
            source: Source::External,
            features: Value::Null,
            fields: serde_json::Value::Array(Vec::new()),
            id_pattern: None,
        };
        r.set_external(issues.clone(), true);
        assert_eq!(r.active(&c, "work_items"), "issues");
        r.set_external(issues, false);
        assert_eq!(r.active(&c, "work_items"), NONE);
    }

    /// `oxplow:tasks`'s features in core's table are what oxplow's
    /// work-items provider does: the two can't drift.
    #[test]
    fn the_tasks_built_in_declares_what_the_provider_does() {
        let provider = serde_json::to_value(crate::work_items::oxplow_provider().features).unwrap();
        let on: Vec<&str> = provider
            .as_object()
            .unwrap()
            .iter()
            .filter(|(_, v)| v.as_bool() == Some(true))
            .map(|(k, _)| k.as_str())
            .collect();
        let mut declared = built_in("oxplow:tasks").unwrap().features.to_vec();
        declared.sort();
        let mut on = on;
        on.sort();
        assert_eq!(declared, on);
    }

    /// With `oxplow-bundled` disabled, nothing declares the optional
    /// defaults: the work list and the effort policy fall to none, while
    /// snapshots — required — keep core's own default, published as the
    /// active row; filing says what it needs.
    #[tokio::test]
    async fn disabling_the_bundled_extension_leaves_core_usable() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        svc.config.write().unwrap().extensions_disabled = vec!["oxplow-bundled".into()];
        std::fs::create_dir_all(svc.layout.project_dir.join(".oxplow")).unwrap();
        std::fs::write(
            oxplow_config::config_path(&svc.layout.project_dir),
            "extensions:\n  disabled: [oxplow-bundled]\n",
        )
        .unwrap();
        svc.extension_catalog.changed();
        refresh(svc).await.unwrap();
        let config = crate::config_service::read_config(&svc.config);
        assert_eq!(svc.capabilities.active(&config, "work_items"), NONE);
        assert_eq!(svc.capabilities.active(&config, "effort_policy"), NONE);
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
            serde_json::json!([["oxplow", 1, 1, "builtin"]])
        );
        let out = svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::commands::work_item::CREATE,
                serde_json::json!({ "title": "x" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["tracked"], serde_json::json!(false));
    }

    /// The rows say which is active and why, and a choice that isn't
    /// available is listed as such.
    #[tokio::test]
    async fn the_rows_say_what_is_active_and_why() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let c = config(&[("work_items", "issues")], &[("effort_policy", "none")]);
        fx.svc.capabilities.publish(&c, &fx.svc.db).await.unwrap();
        let out = fx
            .svc
            .sql
            .query_sql(
                "SELECT capability, provider, active, chosen_by, available FROM v_capability_provider
                  WHERE capability IN ('work_items', 'effort_policy') ORDER BY capability, provider",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            serde_json::json!([
                ["effort_policy", "none", 1, "personal", 1],
                ["effort_policy", "oxplow", 0, null, 1],
                ["work_items", "issues", 0, null, 0],
                ["work_items", "none", 1, "fallback", 1],
                ["work_items", "oxplow", 0, null, 1],
            ])
        );
    }

    /// A change of what's active is logged once, as
    /// `capability.switched`: not the first statement, nor an unchanged
    /// one.
    #[tokio::test]
    async fn a_change_of_the_active_one_is_logged() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let default = config(&[], &[]);
        let off = config(&[("effort_policy", "none")], &[]);
        for c in [&default, &default, &off, &off] {
            svc.capabilities.publish(c, &svc.db).await.unwrap();
        }
        let switches: Vec<String> = svc
            .db
            .read(|tx| {
                let mut stmt = tx
                    .prepare(
                        "SELECT payload FROM event_log WHERE type = 'capability.switched'
                          ORDER BY seq",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = stmt
                    .query_map([], |r| r.get(0))
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
                "capability": "effort_policy",
                "from": "oxplow",
                "to": "none",
                "chosen_by": "project",
            })]
        );
    }

    /// With no work list, the interface is a sink: every verb is offered
    /// and succeeds, keeping nothing — a create files nowhere, a change to an
    /// item (one of another list's, a loose id) changes nothing — and the
    /// interface reads empty.
    #[tokio::test]
    async fn with_no_work_list_the_interface_is_a_sink() {
        use serde_json::json;
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let svc = &fx.svc;
        let agent = oxplow_domain::Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        svc.config
            .write()
            .unwrap()
            .personal_active_providers
            .insert("work_items".into(), NONE.into());
        refresh(svc).await.unwrap();
        let offered: Vec<String> = svc
            .commands
            .list(&agent)
            .into_iter()
            .map(|c| c.id)
            .collect();
        for name in [
            "oxplow.work_item.create",
            "oxplow.work_item.transition",
            "oxplow.work_item.comment",
            "oxplow.work_item.link",
            "oxplow.work_item.reorder",
            "oxplow.work_item.move",
        ] {
            assert!(offered.iter().any(|n| n == name), "{name} offered");
        }
        let run = |name: &'static str, input: serde_json::Value| {
            let svc = svc.clone();
            let agent = agent.clone();
            async move { svc.commands.run(&agent, name, input, false).await.unwrap() }
        };
        let task = oxplow_domain::refs::build::work_item_ref(fx.task);
        for (name, input) in [
            (
                crate::commands::work_item::CREATE,
                json!({ "title": "filed nowhere" }),
            ),
            (
                crate::commands::work_item::NAME,
                json!({ "ref": task, "to": "done" }),
            ),
            (
                crate::commands::work_item::NAME,
                json!({ "ref": fx.task.to_string(), "to": "done" }),
            ),
            (
                crate::commands::work_item::COMMENT,
                json!({ "ref": task, "body": "kept nowhere" }),
            ),
            (crate::commands::work_item::REORDER, json!({ "ref": task })),
            (
                crate::commands::work_item::MOVE,
                json!({ "ref": task, "to": "backlog" }),
            ),
        ] {
            assert_eq!(
                run(name, input).await.result["tracked"],
                json!(false),
                "{name}"
            );
        }
        use oxplow_domain::stores::TaskStore as _;
        let kept = svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(
            kept.status,
            oxplow_domain::TaskStatus::InProgress,
            "oxplow's task untouched"
        );
        let count = svc
            .sql
            .query_sql("SELECT count(*) FROM v_work_item", vec![], None)
            .await
            .unwrap();
        assert_eq!(serde_json::to_value(count.rows).unwrap(), json!([[0]]));
    }

    /// A feature the active work list doesn't declare hides what needs it.
    #[tokio::test]
    async fn a_missing_feature_hides_what_needs_it() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        svc.capabilities.set_external(
            Implementation {
                capability: "work_items".into(),
                id: "plain".into(),
                title: "plain".into(),
                extension: Some("tracker".into()),
                source: Source::External,
                features: serde_json::json!({ "comments": false, "links": true, "ordering": true }),
                fields: serde_json::Value::Array(Vec::new()),
                id_pattern: None,
            },
            true,
        );
        svc.config
            .write()
            .unwrap()
            .active_providers
            .insert("work_items".into(), "plain".into());
        let offered: Vec<String> = svc
            .commands
            .list(&oxplow_domain::Actor::Human)
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert!(offered.iter().any(|n| n == "oxplow.work_item.link"));
        assert!(!offered.iter().any(|n| n == "oxplow.work_item.comment"));
        // Ordering and lists are features like the rest.
        assert!(offered.iter().any(|n| n == "oxplow.work_item.reorder"));
        assert!(!offered.iter().any(|n| n == "oxplow.work_item.move"));
    }

    /// The tools oxplow's tasks own are offered only while they're the
    /// work list.
    #[test]
    fn owned_tools_follow_the_active_list() {
        let r = registry(true);
        assert!(r.snapshot(&config(&[], &[])).offers_tool("list_tasks"));
        assert!(r.snapshot(&config(&[], &[])).offers_tool("query_sql"));
        let none = r.snapshot(&config(&[("work_items", "none")], &[]));
        assert!(!none.offers_tool("list_tasks"));
        assert!(none.offers_tool("query_sql"));
        // Undeclared (the bundled extension off), it isn't offered either.
        let bare = registry(false).snapshot(&config(&[], &[]));
        assert!(!bare.offers_tool("get_task"));
    }

    /// A provider instance's own commands are offered only while it's the
    /// active implementation.
    #[test]
    fn an_instance_namespace_follows_the_active_list() {
        let r = registry(true);
        r.set_external(
            Implementation {
                capability: "work_items".into(),
                id: "fake".into(),
                title: "Fake".into(),
                extension: Some("tracker".into()),
                source: Source::External,
                features: Value::Null,
                fields: serde_json::Value::Array(Vec::new()),
                id_pattern: None,
            },
            true,
        );
        let default = r.snapshot(&config(&[], &[]));
        assert!(default
            .command_refusal("fake.sync_now", &[])
            .is_some_and(|m| m.contains("Fake's")));
        let fake = r.snapshot(&config(&[("work_items", "fake")], &[]));
        assert_eq!(fake.command_refusal("fake.sync_now", &[]), None);
    }

    /// A runtime already on disk gets what's offered (boot refreshes it
    /// for an agent that outlived an upgrade, tsk376): core's skills, the
    /// work-items skill (any list, none included), and oxplow's tasks' own
    /// `/work-next` only while they're the list.
    #[tokio::test]
    async fn the_runtimes_skills_follow_what_is_offered() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let plugin = svc.layout.project_dir.join(".oxplow/runtime/claude-plugin");
        let (skills, commands) = (plugin.join("skills"), plugin.join("commands"));
        std::fs::create_dir_all(skills.join("oxplow-extension")).unwrap();
        std::fs::create_dir_all(&commands).unwrap();
        std::fs::write(skills.join("oxplow-extension/SKILL.md"), "stale").unwrap();
        refresh_agent_text(svc);
        assert_eq!(
            std::fs::read_to_string(skills.join("oxplow-extension/SKILL.md")).unwrap(),
            oxplow_plugin::AgentText::core()
                .skill_body("oxplow-extension")
                .unwrap()
        );
        assert!(skills.join("work-items/SKILL.md").is_file());
        assert!(commands.join("work-next.md").is_file());
        svc.config
            .write()
            .unwrap()
            .personal_active_providers
            .insert("work_items".into(), NONE.into());
        refresh_agent_text(svc);
        assert!(skills.join("work-items/SKILL.md").is_file());
        assert!(!commands.join("work-next.md").exists());
        assert!(commands.join("configure.md").is_file());
    }

    /// An implementation's own fields are declared, and published with its
    /// row: oxplow's tasks have a priority, none has none.
    #[tokio::test]
    async fn an_implementations_fields_are_published() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let out = fx
            .svc
            .sql
            .query_sql(
                "SELECT provider, fields FROM v_capability_provider
                  WHERE capability = 'work_items' ORDER BY provider",
                vec![],
                None,
            )
            .await
            .unwrap();
        let rows = serde_json::to_value(&out.rows).unwrap();
        let fields = |provider: &str| -> Value {
            rows.as_array()
                .unwrap()
                .iter()
                .find(|r| r[0] == provider)
                .map(|r| serde_json::from_str(r[1].as_str().unwrap()).unwrap())
                .unwrap()
        };
        assert_eq!(
            fields("oxplow"),
            serde_json::json!([
                {
                    "name": "priority",
                    "title": "Priority",
                    "kind": "enum",
                    "values": ["urgent", "high", "medium", "low"],
                    "read_only": false
                },
                {
                    "name": "author",
                    "title": "Filed by",
                    "kind": "enum",
                    "values": ["user", "agent"],
                    "read_only": true
                }
            ])
        );
        assert_eq!(fields(NONE), serde_json::json!([]));
    }

    /// Each work list's id pattern is published beside it, so a screen
    /// recognizes its ids in text as core's vocabulary does: oxplow's
    /// `tsk<n>`, nothing for none.
    #[tokio::test]
    async fn a_work_lists_id_pattern_is_published() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let out = fx
            .svc
            .sql
            .query_sql(
                "SELECT provider, id_pattern FROM v_capability_provider
                  WHERE capability = 'work_items' ORDER BY provider",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            serde_json::json!([[NONE, null], ["oxplow", r"tsk\d+"]])
        );
    }

    /// The vocabulary reads the active work list's own ids in text, as it
    /// declares them: `tsk42` with oxplow's tasks, nothing with none.
    #[tokio::test]
    async fn the_vocabulary_reads_the_active_lists_ids() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let found = || {
            oxplow_domain::refs::extract(&svc.vocabulary.current().kinds, "see tsk42 and [[tsk7]]")
                .work_items
        };
        assert_eq!(
            found(),
            vec!["oxplow:tsk42".to_string(), "oxplow:tsk7".to_string()]
        );
        svc.config
            .write()
            .unwrap()
            .personal_active_providers
            .insert("work_items".into(), NONE.into());
        refresh(svc).await.unwrap();
        svc.vocabulary_service.sync().await.unwrap();
        assert!(found().is_empty(), "{:?}", found());
    }
}
