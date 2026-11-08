//! What every extension contribution carries, whatever its kind
//! (`.context/extensions-manifest.md` "Contributions"): one [`Descriptor`], with a
//! stable [`ContributionId`], its owner, how it's invoked
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
/// `_` (event types, ref kinds, dataset views, entity tables and command
/// ids are snake_case). The one place the rule is written.
pub fn extension_namespace(extension: &str) -> String {
    extension.replace('-', "_")
}

/// An extension's name: lowercase letters, digits and single dashes,
/// starting with a letter — safe as a folder, a page-id prefix and (dashes
/// as underscores) a namespace.
pub fn valid_extension_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A namespace: a lowercase letter, then lowercase letters, digits and
/// `_` — what [`extension_namespace`] makes of a valid name, and what a
/// manifest's `namespace:` must be.
pub fn valid_namespace(namespace: &str) -> bool {
    namespace.starts_with(|c: char| c.is_ascii_lowercase())
        && namespace
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
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

/// An implementation's local id: it is chosen by its capability and id,
/// so one extension may implement two capabilities under one id.
pub fn implementation_local(capability: &str, id: &str) -> String {
    format!("{capability}.{id}")
}

/// Every kind of contribution a manifest can declare.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum ContributionKind {
    Dataset,
    Measure,
    Metric,
    Dimension,
    Collector,
    EventType,
    Resource,
    Command,
    Job,
    AiProfile,
    Effect,
    Advisory,
    Skill,
    Implementation,
    Provider,
    Page,
    Panel,
    Slot,
    Decorator,
    Replacement,
    CustomComponent,
}

/// What a kind is, as one table: what the reference doc, the loader and
/// discovery read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindSpec {
    pub kind: ContributionKind,
    /// Its key in `extension.yaml` (`ui.slots` for the nested ones;
    /// `pages` are files, not a key).
    pub key: &'static str,
    /// How a person names it.
    pub title: &'static str,
    /// What it adds, in a line (the reference's table).
    pub summary: &'static str,
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
    dotted: bool,
    shares_names: bool,
) -> KindSpec {
    KindSpec {
        kind,
        key,
        title,
        summary,
        dotted,
        shares_names,
    }
}

/// Every kind, in the order the reference lists them.
pub const KINDS: &[KindSpec] = &[
    spec(
        ContributionKind::Dataset,
        "datasets",
        "Dataset",
        "SQL views over oxplow's datasets, published as `v_<namespace>_<name>`",
        false,
        false,
    ),
    spec(
        ContributionKind::Measure,
        "measures",
        "Measure",
        "what a metric's facts measure",
        true,
        false,
    ),
    spec(
        ContributionKind::Metric,
        "metrics",
        "Metric",
        "a number tracked over a measure",
        true,
        false,
    ),
    spec(
        ContributionKind::Dimension,
        "dimensions",
        "Dimension",
        "a way to slice metrics",
        true,
        false,
    ),
    spec(
        ContributionKind::Collector,
        "collectors",
        "Collector",
        "outside data brought in as entities, or facts for metrics",
        true,
        false,
    ),
    spec(
        ContributionKind::EventType,
        "event_types",
        "Event type",
        "its own namespace's event types",
        true,
        false,
    ),
    spec(
        ContributionKind::Resource,
        "resources",
        "Resource",
        "kinds of thing a URI can name",
        false,
        false,
    ),
    spec(
        ContributionKind::Command,
        "commands",
        "Command",
        "commands on oxplow's command bus",
        true,
        false,
    ),
    spec(
        ContributionKind::Job,
        "jobs",
        "Job",
        "durable host operations scheduled by an extension",
        false,
        false,
    ),
    spec(
        ContributionKind::AiProfile,
        "ai_profiles",
        "AI profile",
        "a named model need an extension declares: bound by the person, or falling back to a built-in role",
        false,
        false,
    ),
    spec(
        ContributionKind::Effect,
        "effects",
        "Effect",
        "scripts that react to logged events",
        false,
        false,
    ),
    spec(
        ContributionKind::Advisory,
        "advisories",
        "Advisory",
        "guidance queries for the coding agent",
        false,
        false,
    ),
    spec(
        ContributionKind::Skill,
        "skills",
        "Skill",
        "skills and slash commands for the coding agent",
        false,
        false,
    ),
    spec(
        ContributionKind::Implementation,
        "implementations",
        "Implementation",
        "an implementation of a capability: a built-in, or an AI provider or effort policy written as a script",
        true,
        false,
    ),
    spec(
        ContributionKind::Provider,
        "providers",
        "Provider",
        "an external program implementing a capability, such as the work list",
        false,
        false,
    ),
    spec(
        ContributionKind::Page,
        "pages",
        "Page",
        "a page: its data loaders and the view that shows them",
        false,
        false,
    ),
    spec(
        ContributionKind::Panel,
        "panels",
        "Panel",
        "a rail panel showing its pages compact",
        false,
        false,
    ),
    spec(
        ContributionKind::Slot,
        "ui.mounts",
        "Slot mount",
        "a page placed at another page's slot (`at: <namespace>/<page>.<slot>`)",
        true,
        true,
    ),
    spec(
        ContributionKind::Decorator,
        "ui.decorators",
        "Decorator",
        "labels on core refs",
        false,
        true,
    ),
    spec(
        ContributionKind::Replacement,
        "ui.mounts",
        "Replacement",
        "a page placed in place of a core component (`at: <target>`, `v_replaceable`)",
        true,
        false,
    ),
    spec(
        ContributionKind::CustomComponent,
        "custom_components",
        "Custom component",
        "sandboxed HTML components a page draws, fed its loaders",
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

    /// Its id segment (`metric`, `custom_component`).
    /// Whether its contributions are named globally — by a name unique
    /// across extensions that callers already use (a command's bus id, a
    /// metric or measure or dimension key, an event type, a ref kind) —
    /// rather than within their extension. Its ref is then
    /// `<kind>:<name>`; every other kind's is `<kind>:<extension>/<id>`.
    pub fn global_name(self) -> bool {
        matches!(
            self,
            ContributionKind::Command
                | ContributionKind::Measure
                | ContributionKind::Metric
                | ContributionKind::Dimension
                | ContributionKind::EventType
        )
    }

    /// The resource a URI of one names (`oxplow/job`), oxplow-foundation's.
    pub fn resource(self) -> String {
        format!("oxplow/{}", self.segment())
    }

    /// Its segment (`job`, `ai_profile`, `command`): its resource's name and
    /// key param.
    pub fn segment(self) -> &'static str {
        match self {
            ContributionKind::Dataset => "dataset",
            ContributionKind::Measure => "measure",
            ContributionKind::Metric => "metric",
            ContributionKind::Dimension => "dimension",
            ContributionKind::Collector => "collector",
            ContributionKind::EventType => "event_type",
            ContributionKind::Resource => "resource",
            ContributionKind::Command => "command",
            ContributionKind::Job => "job",
            ContributionKind::AiProfile => "ai_profile",
            ContributionKind::Effect => "effect",
            ContributionKind::Advisory => "advisory",
            ContributionKind::Skill => "skill",
            ContributionKind::Implementation => "implementation",
            ContributionKind::Provider => "provider",
            ContributionKind::Page => "page",
            ContributionKind::Panel => "panel",
            ContributionKind::Slot => "slot",
            ContributionKind::Decorator => "decorator",
            ContributionKind::Replacement => "replacement",
            ContributionKind::CustomComponent => "custom_component",
        }
    }
}

/// A contribution's identity within the catalog: who declared it, what
/// kind it is, and its id there (for a [`ContributionKind::global_name`]
/// kind, its global name). Its only public name is its ref
/// ([`ContributionId::reference`], also its `Display`):
/// `oxplow:oxplow/job?job=effort_review/review`, `oxplow:oxplow/command?command=oxplow.review.accept`. Storage,
/// events, manifests and the desktop all carry that ref
/// (`.context/resources.md` "Using refs").
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, specta::Type,
)]
pub struct ContributionId {
    /// The namespace it is named in (its extension's `namespace:`), not
    /// the extension that declares it ([`Descriptor::owner`] says that):
    /// moving it to a sibling in the namespace keeps its id.
    pub namespace: String,
    pub kind: ContributionKind,
    pub local: String,
}

impl std::fmt::Display for ContributionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.reference().fmt(f)
    }
}

/// A contribution named by its ref, as text carries it: `oxplow:oxplow/job?job=acme/review`
/// or `oxplow:oxplow/command?command=acme.pr.merge`. A local ref names its owner
/// ([`ContributionRef::id`]); a global one is resolved by the catalog,
/// which knows who declares each global name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, specta::Type)]
#[specta(transparent)]
pub struct ContributionRef(#[specta(type = String)] RefParts);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct RefParts {
    kind: ContributionKind,
    /// `None` for a global name.
    namespace: Option<String>,
    name: String,
}

impl ContributionRef {
    pub fn kind(&self) -> ContributionKind {
        self.0.kind
    }

    /// The namespace it is named in, when the ref says it (a local kind's).
    pub fn namespace(&self) -> Option<&str> {
        self.0.namespace.as_deref()
    }

    /// The id within its namespace, or the global name.
    pub fn name(&self) -> &str {
        &self.0.name
    }

    /// The contribution it names, when the ref says its namespace.
    pub fn id(&self) -> Option<ContributionId> {
        self.0
            .namespace
            .as_ref()
            .map(|ns| ContributionId::new(ns, self.0.kind, &self.0.name))
    }
}

impl std::fmt::Display for ContributionRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let id = match &self.0.namespace {
            Some(ns) => format!("{ns}/{}", self.0.name),
            None => self.0.name.clone(),
        };
        let segment = self.0.kind.segment();
        f.write_str(&crate::uri::build::of(
            &format!("oxplow/{segment}"),
            &[(segment, &id)],
        ))
    }
}

impl std::str::FromStr for ContributionRef {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let parsed =
            crate::uri::OxplowUri::parse(s).map_err(|e| format!("`{s}` isn't a URI: {e}"))?;
        let resource = parsed.resource();
        let kind = KINDS
            .iter()
            .map(|k| k.kind)
            .find(|k| resource.namespace() == "oxplow" && k.segment() == resource.name())
            .ok_or_else(|| format!("`{resource}` isn't a kind of contribution"))?;
        let segment = kind.segment();
        let id = parsed
            .param(segment)
            .filter(|_| parsed.params().len() == 1 && parsed.view().is_empty())
            .ok_or_else(|| {
                format!("`{s}` names a contribution by its `{segment}` param and nothing else")
            })?
            .to_string();
        let parts = if kind.global_name() {
            RefParts {
                kind,
                namespace: None,
                name: id,
            }
        } else {
            let (ns, name) = id
                .split_once('/')
                .ok_or_else(|| format!("`{s}`'s `{segment}` isn't `<namespace>/<id>`"))?;
            let id = ContributionId::new(ns, kind, name);
            if let Some(problem) = id.problem() {
                return Err(problem);
            }
            RefParts {
                kind,
                namespace: Some(ns.to_string()),
                name: name.to_string(),
            }
        };
        if parts.namespace.is_none() && !valid_local(kind, &parts.name) {
            return Err(format!(
                "`{}` isn't a {} name",
                parts.name,
                kind.spec().title
            ));
        }
        Ok(ContributionRef(parts))
    }
}

impl Serialize for ContributionRef {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ContributionRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl ContributionId {
    /// Its ref: `<kind>:<namespace>/<id>`, or `<kind>:<name>` for a kind
    /// whose names are global.
    pub fn reference(&self) -> ContributionRef {
        ContributionRef(RefParts {
            kind: self.kind,
            namespace: (!self.kind.global_name()).then(|| self.namespace.clone()),
            name: self.local.clone(),
        })
    }

    pub fn new(namespace: &str, kind: ContributionKind, local: &str) -> Self {
        ContributionId {
            namespace: namespace.into(),
            kind,
            local: local.into(),
        }
    }

    /// `<namespace>/<local>`, the form a local kind's id takes in storage
    /// and in the `?` of its ref.
    pub fn qualified(&self) -> String {
        format!("{}/{}", self.namespace, self.local)
    }

    /// Who a report names as its producer: the namespace, or for an
    /// import of a global name nothing declares, the name itself.
    pub fn producer_label(&self) -> &str {
        if self.namespace.is_empty() {
            &self.local
        } else {
            &self.namespace
        }
    }

    /// What's wrong with it, if anything.
    pub fn problem(&self) -> Option<String> {
        // An import of a global name nothing declares has no owner to name.
        let ownerless = self.namespace.is_empty() && self.kind.global_name();
        if !ownerless && !valid_namespace(&self.namespace) {
            return Some(format!(
                "`{}` isn't a namespace (lowercase letters, digits and `_`, starting with a \
                 letter)",
                self.namespace
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
    /// Read by a query: a dataset, a measure, a metric, a dimension.
    Query,
    /// Shown where it's mounted: a page, a slot.
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
    /// The contract's own version (a dataset's, an event type's), when it
    /// has one.
    pub contract_version: Option<u32>,
    pub trigger: RunsOn,
    /// What it requires: capabilities, features or scopes
    /// (`capability::check_need`'s forms). Never what it was granted.
    pub needs: Vec<String>,
    /// What it runs after when an event reaches it and them (an effect's
    /// or a collector's `after:`), as refs once the catalog resolved them
    /// (`oxplow_domain::ordering`): `effect:<ext>/<id>`,
    /// `oxplow:oxplow/collector?collector=<ext>/<id>` or `oxplow:oxplow/consumer?name=<name>`.
    pub after: Vec<String>,
    /// Where it runs among the effects and collectors one event reaches:
    /// 0 first, each after every contribution it names. `None` for a kind
    /// that isn't ordered.
    pub order_level: Option<u32>,
    /// The name it takes in a space extensions share (a metric key, an
    /// event type, a skill name, `<capability>/<id>`); two contributions
    /// of a kind that doesn't share names can't hold the same one.
    pub global: Option<String>,
    /// One sentence saying what it is, for the kinds whose declaration
    /// carries one (a collector's `doc:`); `None` otherwise.
    pub description: Option<String>,
    pub source: DeclaredAt,
}

impl Descriptor {
    /// A descriptor with nothing known beyond its identity, owner, trigger
    /// and source.
    pub fn new(id: ContributionId, owner: Owner, trigger: RunsOn, source: DeclaredAt) -> Self {
        Descriptor {
            id,
            owner,
            contract_version: None,
            trigger,
            needs: Vec::new(),
            after: Vec::new(),
            order_level: None,
            global: None,
            description: None,
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
    /// The extension that declares the one kept.
    pub kept_by: String,
    pub refused: ContributionId,
    /// The extension whose declaration is refused.
    pub refused_by: String,
}

impl std::fmt::Display for Collision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} `{}` is already declared by extension `{}`; extension `{}`'s is not used (rename one)",
            self.kind.spec().title.to_lowercase(),
            self.global,
            self.kept_by, self.refused_by
        )
    }
}

/// A catalog's contributions, in the order they were added.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    descriptors: Vec<Descriptor>,
    by_global: BTreeMap<(ContributionKind, String), (ContributionId, String)>,
    /// Who declares each local id: extensions share a namespace (`oxplow`),
    /// and with it the ids named in it.
    by_id: BTreeMap<ContributionId, String>,
}

impl Registry {
    /// Add `d`; a global name its kind doesn't share that another already
    /// holds is refused, the holder kept — and so is an id another
    /// extension in its namespace already declares.
    pub fn add(&mut self, d: Descriptor) -> Result<(), Box<Collision>> {
        let spec = d.id.kind.spec();
        if !spec.shares_names && !d.id.kind.global_name() {
            match self.by_id.get(&d.id) {
                Some(kept_by) if *kept_by != d.owner.extension => {
                    return Err(Box::new(Collision {
                        kind: d.id.kind,
                        global: d.id.qualified(),
                        kept: d.id.clone(),
                        kept_by: kept_by.clone(),
                        refused: d.id,
                        refused_by: d.owner.extension,
                    }));
                }
                Some(_) => {}
                None => {
                    self.by_id.insert(d.id.clone(), d.owner.extension.clone());
                }
            }
        }
        if let Some(global) = &d.global {
            if !d.id.kind.spec().shares_names {
                let key = (d.id.kind, global.clone());
                if let Some((kept, kept_by)) = self.by_global.get(&key) {
                    return Err(Box::new(Collision {
                        kind: d.id.kind,
                        global: global.clone(),
                        kept: kept.clone(),
                        kept_by: kept_by.clone(),
                        refused: d.id,
                        refused_by: d.owner.extension,
                    }));
                }
                self.by_global
                    .insert(key, (d.id.clone(), d.owner.extension.clone()));
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

/// Which own-reference form a token is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefPart {
    /// `v_<namespace>_<dataset>`: the prefix is `v_<namespace>_`.
    Dataset,
    /// `<namespace>/<id>`, a contribution's id (a page, an effect, a
    /// resource): the prefix is `<namespace>/`.
    Page,
    /// `<namespace>.<…>`, a command, an event type or a view node: the
    /// prefix is `<namespace>.`.
    Command,
}

/// One place a text names an extension: `prefix` is the part that holds the
/// extension's namespace (what a rename rewrites), `token` the whole
/// dotted/slashed word it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnReference {
    pub part: RefPart,
    pub prefix: std::ops::Range<usize>,
    pub token: std::ops::Range<usize>,
}

/// Every place `text` names the namespace `namespace`: its dataset views,
/// its contributions' ids and its commands, by the one rule
/// (`check_dependencies` and `extension new --from` share it). A token
/// starts at a word boundary (not after a letter, digit, `_`, `-`, `.` or
/// `/`: `api.triage.com` and `https://triage.com` name a host) and
/// something follows the prefix, so prose ending in the name and a slug
/// such as `triage-summary` are no reference. In order of position.
pub fn own_references(text: &str, namespace: &str) -> Vec<OwnReference> {
    let forms = [
        (RefPart::Dataset, format!("v_{namespace}_")),
        (RefPart::Page, format!("{namespace}/")),
        (RefPart::Command, format!("{namespace}.")),
    ];
    let mut found = Vec::new();
    for (part, prefix) in &forms {
        for (start, _) in text.match_indices(prefix.as_str()) {
            let before = text[..start].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || "_-./".contains(c)) {
                continue;
            }
            let len = text[start..]
                .find(|c: char| !(c.is_alphanumeric() || "_-./".contains(c)))
                .unwrap_or(text.len() - start);
            let token = start..start + text[start..start + len].trim_end_matches('.').len();
            let prefix_end = start + prefix.len();
            if token.end <= prefix_end {
                continue;
            }
            found.push(OwnReference {
                part: *part,
                prefix: start..prefix_end,
                token,
            });
        }
    }
    found.sort_by_key(|r| r.prefix.start);
    found
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
                file: format!(".oxplow/extensions/{ext}/extension.yaml"),
                line: None,
            },
        );
        d.global = Some(key.into());
        d
    }

    /// A contribution's only name is its ref: `<kind>:<namespace>/<id>`,
    /// or `<kind>:<global name>` for a kind whose names are global (a
    /// command's bus id, a metric key, an event type). Parsing one from
    /// text gives a `ContributionRef`; only a local ref says its owner.
    #[test]
    fn a_contribution_is_named_by_its_ref() {
        let job = ContributionId::new("effort_review", ContributionKind::Job, "review");
        assert_eq!(
            job.to_string(),
            "oxplow:oxplow/job?job=effort_review/review"
        );
        let command = ContributionId::new(
            "acme-tools",
            ContributionKind::Command,
            "acme_tools.pr.merge",
        );
        assert_eq!(
            command.to_string(),
            "oxplow:oxplow/command?command=acme_tools.pr.merge"
        );

        let parsed: ContributionRef = "oxplow:oxplow/job?job=effort_review/review"
            .parse()
            .unwrap();
        assert_eq!(parsed, job.reference());
        assert_eq!(
            parsed.to_string(),
            "oxplow:oxplow/job?job=effort_review/review"
        );
        assert_eq!(parsed.id(), Some(job.clone()));
        let global: ContributionRef = "oxplow:oxplow/command?command=acme_tools.pr.merge"
            .parse()
            .unwrap();
        assert_eq!(global, command.reference());
        assert_eq!(global.id(), None, "a global name doesn't say its owner");
        for bad in [
            "effort_review/job/review",
            // An extension's name isn't its namespace.
            "oxplow:oxplow/job?job=effort-review/review",
            "nope:acme/x",
            "oxplow:oxplow/job?job=review",
            "oxplow:oxplow/job?job=Acme/x",
            "effect:acme/a.b",
            "oxplow:oxplow/extension?name=acme",
        ] {
            assert!(bad.parse::<ContributionRef>().is_err(), "{bad}");
        }

        // Every kind's URI names a resource foundation declares, its id fitting.
        let resources = crate::uri::resources::foundation();
        for spec in KINDS {
            let local = match (spec.kind.global_name(), spec.dotted) {
                (true, true) => "acme.x",
                (true, false) => "acme_x",
                (false, _) => "x",
            };
            let r = ContributionId::new("acme", spec.kind, local).to_string();
            let c = crate::uri::OxplowUri::parse(&r).unwrap();
            resources
                .validate(&c)
                .unwrap_or_else(|e| panic!("{r}: {e}"));
        }
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
        assert!(valid_namespace("acme_pr2") && valid_namespace(&extension_namespace("acme-pr")));
        for bad in ["acme-pr", "Acme", "2acme", "_acme"] {
            assert!(!valid_namespace(bad), "{bad}");
        }
    }

    /// Two extensions declaring one metric key: the first keeps it, the
    /// second is refused, naming both.
    #[test]
    fn a_global_name_is_held_by_the_first() {
        let mut r = Registry::default();
        r.add(metric("alpha", "loc", "acme.loc")).unwrap();
        let err = r.add(metric("beta", "loc", "acme.loc")).unwrap_err();
        assert_eq!(err.kept_by, "alpha");
        assert_eq!(err.refused_by, "beta");
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

    /// Ids are named in a namespace, which extensions may share (`oxplow`):
    /// two of them declaring one id collide, the first keeping it, as a
    /// global name does. One extension restating its own id doesn't.
    #[test]
    fn an_id_two_extensions_declare_in_one_namespace_is_held_by_the_first() {
        let effect = |ext: &str| {
            Descriptor::new(
                ContributionId::new("oxplow", ContributionKind::Effect, "notify"),
                owner(ext),
                RunsOn::Events { types: Vec::new() },
                DeclaredAt {
                    file: "extension.yaml".into(),
                    line: None,
                },
            )
        };
        let mut r = Registry::default();
        r.add(effect("oxplow-foundation")).unwrap();
        r.add(effect("oxplow-foundation")).unwrap();
        let err = r.add(effect("oxplow-bundled")).unwrap_err();
        assert_eq!(
            (err.kept_by.as_str(), err.refused_by.as_str()),
            ("oxplow-foundation", "oxplow-bundled")
        );
        assert!(err.to_string().contains("`oxplow/notify`"), "{err}");
    }

    #[test]
    fn every_kind_is_in_the_table_once() {
        let mut kinds: Vec<_> = KINDS.iter().map(|k| k.kind).collect();
        kinds.dedup();
        assert_eq!(kinds.len(), KINDS.len());
        for k in KINDS {
            assert_eq!(k.kind.spec().key, k.key);
        }
        // One manifest list may place several kinds: `ui.mounts` holds slot
        // mounts and replacements, told apart by what `at` names. No other
        // key is shared.
        let mut shared: Vec<&str> = KINDS
            .iter()
            .map(|k| k.key)
            .filter(|key| KINDS.iter().filter(|k| k.key == *key).count() > 1)
            .collect();
        shared.dedup();
        assert_eq!(shared, ["ui.mounts"]);
    }

    #[test]
    fn own_references_find_datasets_pages_and_commands_at_word_boundaries() {
        let text = "FROM v_triage_x t; use triage/summarize and triage.todos.decide, not triage-summary or other-triage/x or v_other_triage_x or api.triage.com or https://triage.com/x. The end of triage.";
        let found: Vec<(RefPart, &str, &str)> = own_references(text, "triage")
            .into_iter()
            .map(|r| (r.part, &text[r.prefix], &text[r.token]))
            .collect();
        assert_eq!(
            found,
            [
                (RefPart::Dataset, "v_triage_", "v_triage_x"),
                (RefPart::Page, "triage/", "triage/summarize"),
                (RefPart::Command, "triage.", "triage.todos.decide"),
            ]
        );
    }

    #[test]
    fn own_references_read_only_the_namespace() {
        // Ids are the namespace's, so the dashed name names nothing.
        let text = "v_task_views_a task_views/board task_views.x.y task-views/old task-views.node";
        let found: Vec<RefPart> = own_references(text, "task_views")
            .into_iter()
            .map(|r| r.part)
            .collect();
        assert_eq!(found, [RefPart::Dataset, RefPart::Page, RefPart::Command]);
    }
}
