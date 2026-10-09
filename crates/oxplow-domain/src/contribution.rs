//! What every extension contribution carries, whatever its kind
//! (`.context/extensions.md` "Contributions"): one [`Descriptor`], with a
//! stable [`ContributionId`], its owner, its stability, how it's invoked
//! and what it needs. The [`Registry`] of a catalog's descriptors is what
//! loading checks against (names, collisions, needs) and what discovery
//! lists, so each kind doesn't restate those rules.
//!
//! The names here are the one vocabulary: a **contribution** is anything
//! a manifest declares; a **capability** is a slot a project fills; an
//! **implementation** fills it (a built-in, a script, a provider's
//! instance); a **provider** is a process implementing one; a **need** is
//! a service or scope a contribution requires; a **grant** is what a
//! person approved. A need is never a grant.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The namespace an extension's names live under: its name, `-` read as
/// `_` (event types, ref kinds, model views, entity tables and command
/// ids are snake_case). The one place the rule is written.
pub fn extension_namespace(extension: &str) -> String {
    extension.replace('-', "_")
}

/// An extension's name: lowercase letters, digits and single dashes,
/// starting with a letter — safe as a folder, a lens-id prefix and (dashes
/// as underscores) a namespace.
pub fn valid_extension_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Names only oxplow uses: its own namespace, `none`, and core's event
/// namespaces.
pub fn reserved_name(name: &str) -> bool {
    let ns = extension_namespace(name);
    ns == crate::OXPLOW_NAMESPACE
        || ns == crate::capability::NONE
        || crate::events::schema::CORE_NAMESPACES.contains(&ns.as_str())
}

/// A contribution's local id: a lowercase letter, then lowercase letters,
/// digits, `-` and `_`; dots too for the kinds whose ids are dotted
/// (commands' `<area>.<verb>`, collectors' `repo.scan_clone`).
pub fn valid_local(kind: ContributionKind, local: &str) -> bool {
    let dotted = kind.spec().dotted;
    local.starts_with(|c: char| c.is_ascii_lowercase())
        && !local.ends_with(['-', '_', '.'])
        && local.chars().all(|c| {
            c.is_ascii_lowercase()
                || c.is_ascii_digit()
                || c == '-'
                || c == '_'
                || (dotted && c == '.')
        })
}

/// Every kind of contribution a manifest can declare.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum ContributionKind {
    Model,
    Measure,
    Metric,
    Dimension,
    Collector,
    EventType,
    RefKind,
    Command,
    Effect,
    Advisory,
    Skill,
    Implementation,
    Provider,
    Lens,
    Page,
    Panel,
    Slot,
    Decorator,
    Replacement,
    CustomComponent,
}

/// Whether a contract is permanent API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Stability {
    Stable,
    Experimental,
}

/// What a kind is, as one table: what the reference doc, the loader and
/// discovery read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindSpec {
    pub kind: ContributionKind,
    /// Its key in `extension.yaml` (`ui.slots` for the nested ones;
    /// `lenses` are files, not a key).
    pub key: &'static str,
    /// How a person names it.
    pub title: &'static str,
    /// What it adds, in a line (the reference's table).
    pub summary: &'static str,
    pub stability: Stability,
    /// Whether its local id may be dotted.
    pub dotted: bool,
    /// Whether two extensions may each declare the same global name of it
    /// (a metric key, an event type): `false` makes the second a load
    /// error, the first kept.
    pub shares_names: bool,
}

const fn spec(
    kind: ContributionKind,
    key: &'static str,
    title: &'static str,
    summary: &'static str,
    stability: Stability,
    dotted: bool,
    shares_names: bool,
) -> KindSpec {
    KindSpec {
        kind,
        key,
        title,
        summary,
        stability,
        dotted,
        shares_names,
    }
}

use Stability::{Experimental, Stable};

/// Every kind, in the order the reference lists them.
pub const KINDS: &[KindSpec] = &[
    spec(
        ContributionKind::Model,
        "models",
        "Model",
        "SQL views over oxplow's models, published as `v_<ext>_<name>`",
        Stable,
        false,
        false,
    ),
    spec(
        ContributionKind::Measure,
        "measures",
        "Measure",
        "what a metric's facts measure",
        Stable,
        true,
        false,
    ),
    spec(
        ContributionKind::Metric,
        "metrics",
        "Metric",
        "a number tracked over a measure",
        Stable,
        true,
        false,
    ),
    spec(
        ContributionKind::Dimension,
        "dimensions",
        "Dimension",
        "a way to slice metrics",
        Stable,
        true,
        false,
    ),
    spec(
        ContributionKind::Collector,
        "collectors",
        "Collector",
        "outside data brought in as entities, or facts for metrics",
        Stable,
        true,
        false,
    ),
    spec(
        ContributionKind::EventType,
        "event_types",
        "Event type",
        "its own namespace's event types",
        Stable,
        true,
        false,
    ),
    spec(
        ContributionKind::RefKind,
        "ref_kinds",
        "Ref kind",
        "kinds of thing a ref can name",
        Stable,
        false,
        false,
    ),
    spec(
        ContributionKind::Command,
        "commands",
        "Command",
        "commands on oxplow's command bus",
        Stable,
        true,
        false,
    ),
    spec(
        ContributionKind::Effect,
        "effects",
        "Effect",
        "scripts that react to logged events",
        Stable,
        false,
        false,
    ),
    spec(
        ContributionKind::Advisory,
        "advisories",
        "Advisory",
        "guidance queries for the coding agent",
        Stable,
        false,
        false,
    ),
    spec(
        ContributionKind::Skill,
        "skills",
        "Skill",
        "skills and slash commands for the coding agent",
        Stable,
        false,
        false,
    ),
    spec(
        ContributionKind::Implementation,
        "implementations",
        "Implementation",
        "an implementation of a capability: a built-in, or an AI provider or effort policy written as a script",
        Stable,
        false,
        false,
    ),
    spec(
        ContributionKind::Provider,
        "providers",
        "Provider",
        "an external program implementing a capability, such as the work list",
        Stable,
        false,
        false,
    ),
    spec(
        ContributionKind::Lens,
        "lenses",
        "Lens",
        "a page or panel over a SQL query",
        Stable,
        false,
        false,
    ),
    spec(
        ContributionKind::Page,
        "pages",
        "Page",
        "where its lenses show as pages",
        Stable,
        false,
        false,
    ),
    spec(
        ContributionKind::Panel,
        "panels",
        "Panel",
        "where its lenses show as panels",
        Stable,
        false,
        false,
    ),
    spec(
        ContributionKind::Slot,
        "ui.slots",
        "Slot mount",
        "a lens mounted into a core page",
        Stable,
        true,
        true,
    ),
    spec(
        ContributionKind::Decorator,
        "ui.decorators",
        "Decorator",
        "labels on core refs",
        Stable,
        false,
        true,
    ),
    spec(
        ContributionKind::Replacement,
        "ui.replacements",
        "Replacement",
        "a lens in place of a core component (private extensions only)",
        Experimental,
        true,
        false,
    ),
    spec(
        ContributionKind::CustomComponent,
        "custom_components",
        "Custom component",
        "sandboxed HTML components for `viz: custom` lenses",
        Stable,
        false,
        false,
    ),
];

impl ContributionKind {
    pub fn spec(self) -> &'static KindSpec {
        KINDS
            .iter()
            .find(|s| s.kind == self)
            .expect("every kind is in KINDS")
    }

    /// The kind whose manifest key is `key`.
    pub fn of_key(key: &str) -> Option<ContributionKind> {
        KINDS.iter().find(|s| s.key == key).map(|s| s.kind)
    }

    /// Its id segment (`metric`, `custom_component`).
    pub fn segment(self) -> &'static str {
        match self {
            ContributionKind::Model => "model",
            ContributionKind::Measure => "measure",
            ContributionKind::Metric => "metric",
            ContributionKind::Dimension => "dimension",
            ContributionKind::Collector => "collector",
            ContributionKind::EventType => "event_type",
            ContributionKind::RefKind => "ref_kind",
            ContributionKind::Command => "command",
            ContributionKind::Effect => "effect",
            ContributionKind::Advisory => "advisory",
            ContributionKind::Skill => "skill",
            ContributionKind::Implementation => "implementation",
            ContributionKind::Provider => "provider",
            ContributionKind::Lens => "lens",
            ContributionKind::Page => "page",
            ContributionKind::Panel => "panel",
            ContributionKind::Slot => "slot",
            ContributionKind::Decorator => "decorator",
            ContributionKind::Replacement => "replacement",
            ContributionKind::CustomComponent => "custom_component",
        }
    }
}

/// A contribution's identity: `<extension>/<kind>/<local>`. Stable across
/// loads; what ordering, imports and the views key on. The public forms a
/// kind already had (`lens:<ext>/<slug>`, a command id, a metric key) are
/// its [`Descriptor::aliases`].
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, specta::Type,
)]
pub struct ContributionId {
    pub extension: String,
    pub kind: ContributionKind,
    pub local: String,
}

impl std::fmt::Display for ContributionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{}/{}",
            self.extension,
            self.kind.segment(),
            self.local
        )
    }
}

impl std::str::FromStr for ContributionId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let mut parts = s.splitn(3, '/');
        let (Some(extension), Some(kind), Some(local)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(format!("`{s}` isn't `<extension>/<kind>/<id>`"));
        };
        let kind = KINDS
            .iter()
            .map(|k| k.kind)
            .find(|k| k.segment() == kind)
            .ok_or_else(|| format!("`{kind}` isn't a kind of contribution"))?;
        let id = ContributionId {
            extension: extension.into(),
            kind,
            local: local.into(),
        };
        id.problem().map_or(Ok(id), Err)
    }
}

impl ContributionId {
    pub fn new(extension: &str, kind: ContributionKind, local: &str) -> Self {
        ContributionId {
            extension: extension.into(),
            kind,
            local: local.into(),
        }
    }

    /// What's wrong with it, if anything.
    pub fn problem(&self) -> Option<String> {
        if !valid_extension_name(&self.extension) {
            return Some(format!(
                "`{}` isn't an extension name (lowercase letters, digits and single dashes, \
                 starting with a letter)",
                self.extension
            ));
        }
        if !valid_local(self.kind, &self.local) {
            return Some(format!(
                "{} id `{}` must be lowercase letters, digits, `-` and `_`{}, starting with a \
                 letter",
                self.kind.spec().title,
                self.local,
                if self.kind.spec().dotted {
                    " (and `.`)"
                } else {
                    ""
                }
            ));
        }
        None
    }
}

/// Who contributed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Owner {
    pub extension: String,
    /// `project` or `bundled`.
    pub origin: String,
    /// The extension's `version:`, when it declares one.
    pub version: Option<String>,
}

/// How a contribution comes to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case", tag = "on")]
pub enum RunsOn {
    /// Logged events of these types.
    Events { types: Vec<String> },
    /// A schedule (`every`, a cron).
    Schedule { every: String },
    /// A run's reports (a report collector).
    Run,
    /// Called by name: a command, an implementation's verbs.
    Call,
    /// Read by a query: a model, a measure, a metric, a dimension.
    Query,
    /// Shown where it's mounted: a lens, a page, a slot.
    Mount,
    /// Read by an agent: a skill, an advisory.
    Agent,
    /// Nothing invokes it: a vocabulary entry (an event type, a ref kind).
    Declared,
}

/// Where in the manifest it was declared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DeclaredAt {
    pub file: String,
    /// 1-based; `None` when the kind doesn't track lines.
    pub line: Option<u32>,
}

/// Everything a contribution is, as core knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Descriptor {
    pub id: ContributionId,
    pub owner: Owner,
    pub stability: Stability,
    /// The contract's own version (a model's, an event type's), when it
    /// has one.
    pub contract_version: Option<u32>,
    pub trigger: RunsOn,
    /// What it requires: capabilities, features or scopes
    /// (`capability::check_need`'s forms). Never what it was granted.
    pub needs: Vec<String>,
    /// Contributions it runs after (by alias, as the manifest names them).
    pub after: Vec<String>,
    /// The name it takes in a space extensions share (a metric key, an
    /// event type, a skill name, `<capability>/<id>`); two contributions
    /// of a kind that doesn't share names can't hold the same one.
    pub global: Option<String>,
    /// Its other public forms (`lens:<ext>/<slug>`, a command id).
    pub aliases: Vec<String>,
    pub source: DeclaredAt,
}

impl Descriptor {
    /// A descriptor with the kind's stability and nothing else known.
    pub fn new(id: ContributionId, owner: Owner, trigger: RunsOn, source: DeclaredAt) -> Self {
        Descriptor {
            stability: id.kind.spec().stability,
            id,
            owner,
            contract_version: None,
            trigger,
            needs: Vec::new(),
            after: Vec::new(),
            global: None,
            aliases: Vec::new(),
            source,
        }
    }
}

/// Two contributions claimed one global name; the first keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    pub kind: ContributionKind,
    pub global: String,
    pub kept: ContributionId,
    pub refused: ContributionId,
}

impl std::fmt::Display for Collision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} `{}` is already declared by extension `{}`; extension `{}`'s is not used (rename one)",
            self.kind.spec().title.to_lowercase(),
            self.global,
            self.kept.extension,
            self.refused.extension
        )
    }
}

/// A catalog's contributions, in the order they were added.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    descriptors: Vec<Descriptor>,
    by_global: BTreeMap<(ContributionKind, String), ContributionId>,
}

impl Registry {
    /// Add `d`; a global name its kind doesn't share that another already
    /// holds is refused, the holder kept.
    pub fn add(&mut self, d: Descriptor) -> Result<(), Box<Collision>> {
        if let Some(global) = &d.global {
            if !d.id.kind.spec().shares_names {
                let key = (d.id.kind, global.clone());
                if let Some(kept) = self.by_global.get(&key) {
                    return Err(Box::new(Collision {
                        kind: d.id.kind,
                        global: global.clone(),
                        kept: kept.clone(),
                        refused: d.id,
                    }));
                }
                self.by_global.insert(key, d.id.clone());
            }
        }
        self.descriptors.push(d);
        Ok(())
    }

    pub fn all(&self) -> &[Descriptor] {
        &self.descriptors
    }

    pub fn of_kind(&self, kind: ContributionKind) -> impl Iterator<Item = &Descriptor> {
        self.descriptors.iter().filter(move |d| d.id.kind == kind)
    }

    pub fn get(&self, id: &ContributionId) -> Option<&Descriptor> {
        self.descriptors.iter().find(|d| &d.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(ext: &str) -> Owner {
        Owner {
            extension: ext.into(),
            origin: "project".into(),
            version: None,
        }
    }

    fn metric(ext: &str, local: &str, key: &str) -> Descriptor {
        let mut d = Descriptor::new(
            ContributionId::new(ext, ContributionKind::Metric, local),
            owner(ext),
            RunsOn::Query,
            DeclaredAt {
                file: format!("oxplow/extensions/{ext}/extension.yaml"),
                line: None,
            },
        );
        d.global = Some(key.into());
        d
    }

    #[test]
    fn an_id_prints_parses_and_checks_its_grammar() {
        let id = ContributionId::new("acme-tools", ContributionKind::Command, "pr.merge");
        assert_eq!(id.to_string(), "acme-tools/command/pr.merge");
        assert_eq!(
            "acme-tools/command/pr.merge".parse::<ContributionId>(),
            Ok(id)
        );
        assert!("acme-tools/effect/a.b".parse::<ContributionId>().is_err());
        assert!("acme-tools/nope/x".parse::<ContributionId>().is_err());
        assert!("Acme/effect/x".parse::<ContributionId>().is_err());
        assert!(
            ContributionId::new("acme", ContributionKind::Effect, "9lives")
                .problem()
                .is_some()
        );
        assert!(
            ContributionId::new("acme", ContributionKind::Provider, "fake_tracker")
                .problem()
                .is_none()
        );
        assert!(
            ContributionId::new("acme", ContributionKind::Skill, "work-next")
                .problem()
                .is_none()
        );
    }

    #[test]
    fn names_and_namespaces_have_one_rule() {
        assert_eq!(extension_namespace("acme-pr"), "acme_pr");
        assert!(valid_extension_name("acme-pr2"));
        for bad in ["Acme", "acme-", "acme--pr", "2acme", "acme_pr"] {
            assert!(!valid_extension_name(bad), "{bad}");
        }
        assert!(reserved_name("oxplow") && reserved_name("none"));
        assert!(!reserved_name("acme"));
    }

    /// Two extensions declaring one metric key: the first keeps it, the
    /// second is refused, naming both.
    #[test]
    fn a_global_name_is_held_by_the_first() {
        let mut r = Registry::default();
        r.add(metric("alpha", "loc", "acme.loc")).unwrap();
        let err = r.add(metric("beta", "loc", "acme.loc")).unwrap_err();
        assert_eq!(err.kept.extension, "alpha");
        assert_eq!(err.refused.extension, "beta");
        let msg = err.to_string();
        assert!(
            msg.contains("`acme.loc`") && msg.contains("`alpha`") && msg.contains("`beta`"),
            "{msg}"
        );
        assert_eq!(r.of_kind(ContributionKind::Metric).count(), 1);
        // Kinds that share names (two mounts of one slot) don't collide.
        let mut slot = metric("alpha", "a", "effort.review.details");
        slot.id.kind = ContributionKind::Slot;
        let mut again = metric("beta", "b", "effort.review.details");
        again.id.kind = ContributionKind::Slot;
        r.add(slot).unwrap();
        r.add(again).unwrap();
    }

    #[test]
    fn every_kind_is_in_the_table_once() {
        let mut keys: Vec<_> = KINDS.iter().map(|k| k.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), KINDS.len());
        for k in KINDS {
            assert_eq!(k.kind.spec().key, k.key);
            assert_eq!(ContributionKind::of_key(k.key), Some(k.kind));
        }
    }
}
