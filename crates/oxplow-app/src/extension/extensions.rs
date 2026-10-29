//! Project extensions and their pages.
//!
//! An extension is a folder `.oxplow/extensions/<name>/` in a stream's
//! worktree with an `extension.yaml` and `pages/<slug>.yaml` files. A
//! viz node is a query over the semantic layer (`v_*` views) plus how to
//! show it. See `.context/extensions.md`.
//!
//! Loading never fails as a whole: a broken extension or page is
//! reported in that extension's `errors` and everything else still
//! loads.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use oxplow_db::SqlCell;
use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

pub mod ai_profiles;
pub(crate) mod candidates;
pub mod config;
pub mod config_schema;
pub mod custom_components;
pub mod decorators;
pub mod descriptors;
pub mod implementations;
pub mod imports;
pub mod jobs;
pub mod manifest_v2;
pub mod public_contracts;
pub mod replacements;
pub mod schema;
pub mod skills;
use crate::pages::spec::{
    action_templates, edit_column, label_refs, param_problem, FormField, FormFieldSpec, FormMode,
    FormSpec, LabelFrom, PageParam, PageProvenance, ParamRole, ViewContext, Viz, VizConfig,
    VizTree,
};
#[cfg(test)]
use crate::pages::spec::{
    parse_actions, resolve_params, view_context, ActionThen, ColumnFormat, ColumnLink, FieldValue,
    FieldValues, LauncherCategory, LinkKind, ParamType, VizCode, VizEditor, VizEmbed, VizGroup,
    VizTerminal,
};
use manifest_v2::{at, entry_line, item_lines, key_line, line_under, ManifestV2};
pub use manifest_v2::{Intent, IntentExample};
pub use oxplow_domain::permissions::Permissions;

/// Where an extension's source is by default, relative to a checkout
/// (`extensionSources` may point a name elsewhere).
pub const EXTENSIONS_DIR: &str = oxplow_config::EXTENSION_SOURCES_DIR;

/// Where a loaded extension's files came from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Deployment {
    /// Read from its source folder: an authoring check, the CLI, a
    /// standalone load.
    #[default]
    Source,
    /// The deployment that ships inside oxplow (a bundled extension with no
    /// deployment of its own), written to the store once per hash.
    Shipped { hash: String },
    /// A deployment in the store.
    Deployed { hash: String },
    /// Its source is here but it has no deployment yet: listed, not run.
    Undeployed,
    /// Listed as on, but neither its source nor a deployment is on this
    /// machine: listed, not run, and not an error.
    NotHere,
}

impl Deployment {
    /// The hash of the files that run: a shipped or a deployed copy's.
    pub fn hash(&self) -> Option<&str> {
        match self {
            Deployment::Shipped { hash } | Deployment::Deployed { hash } => Some(hash),
            Deployment::Source | Deployment::Undeployed | Deployment::NotHere => None,
        }
    }
}

/// When core runs an advisory.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AdvisoryOn {
    /// After each agent tool call; results go to the agent as that call's
    /// context and are recorded as nudges.
    PostToolUse,
    /// On each prompt the human sends; results join the prompt's context.
    Prompt,
    /// When a turn ends (its `thread.checkpoint`): what the turn left
    /// behind, delivered when the agent next hears from oxplow (the next
    /// prompt, else the next tool call).
    TurnEnd,
}

/// How often the same advisory may reach the agent.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AdvisoryOncePer {
    /// Once per effort, the first time the query returns rows.
    Effort,
    /// Once per thread per `key` value: each row's `key` column fires
    /// once (put `:effort_id` in the key for once per effort per row).
    Row,
    /// Every time, whenever the query returns rows.
    Turn,
    /// Once per thread, the first time the query returns rows.
    Thread,
    /// Once per agent session on the thread (the harness's session id).
    Session,
    /// Once per thread per day (UTC).
    Day,
}

/// Who a hint is for.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    specta::Type,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AdvisoryAudience {
    /// The coding agent: delivered on its next prompt or tool call.
    #[default]
    Agent,
    /// A person: raised in Alerts until they dismiss it.
    Person,
}

fn default_once_per() -> AdvisoryOncePer {
    AdvisoryOncePer::Effort
}

/// Guidance an extension gives the coding agent: a query over the semantic
/// layer, run by core at `on`, with `:thread_id`, `:stream_id`, `:turn_id`
/// and `:effort_id` bound (`:effort_id` NULL when the thread has no
/// effort, `:turn_id` NULL at a prompt). Each result row's `message`
/// column is a line of guidance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Advisory {
    pub id: String,
    pub on: AdvisoryOn,
    pub query: String,
    pub once_per: AdvisoryOncePer,
    /// Line put above the messages (e.g. `# Metric deltas (this effort)`).
    pub heading: Option<String>,
    pub audience: AdvisoryAudience,
    /// The capabilities (or features) it needs; without them it doesn't
    /// run.
    pub needs: Vec<String>,
}

/// An advisory as written in `extension.yaml`.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AdvisoryFile {
    id: String,
    on: AdvisoryOn,
    query: String,
    #[serde(default = "default_once_per")]
    once_per: AdvisoryOncePer,
    #[serde(default)]
    heading: Option<String>,
    #[serde(default)]
    audience: AdvisoryAudience,
    #[serde(default)]
    needs: Vec<String>,
}

/// Where a mount goes among a page's parts: `start`, `end` (the default),
/// or `{ after: <id> }` / `{ before: <id> }` beside one of them. An id the
/// page no longer has means the end (and a load warning), so a page that
/// changes never loses what an extension adds.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(untagged)]
pub enum SlotAt {
    Edge(SlotEdge),
    After { after: String },
    Before { before: String },
}

/// The ends of a page's parts.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum SlotEdge {
    Start,
    End,
}

impl Default for SlotAt {
    fn default() -> Self {
        SlotAt::Edge(SlotEdge::End)
    }
}

impl SlotAt {
    /// The part it's placed beside, if any.
    pub fn anchor(&self) -> Option<&str> {
        match self {
            SlotAt::After { after: id } | SlotAt::Before { before: id } => Some(id),
            SlotAt::Edge(_) => None,
        }
    }
}

/// How much of a dashboard a tile takes: one cell, two across, two down,
/// or the whole row.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    specta::Type,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum TileSize {
    #[default]
    Small,
    Wide,
    Tall,
    Full,
}

/// What a left-nav panel is bound to: the project, the current stream, or
/// the current thread — which of `stream_id` / `thread_id` its viz nodes get.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum PanelScope {
    Project,
    Stream,
    Thread,
}

/// A left-nav panel an extension contributes (P6.G1, target §11.3): its
/// `body` viz renders compact in the nav; its `badge` viz's alert gives
/// the count shown on the panel and in the Alerts panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPanel {
    /// `<extension>/<panel>`.
    pub id: String,
    pub extension: String,
    pub title: String,
    pub icon: Option<String>,
    pub scope: PanelScope,
    /// The body viz (`<extension>/<slug>`).
    pub body: String,
    /// The badge viz, which declares an `alert`.
    pub badge: Option<String>,
    /// The page its header opens (`oxplow:oxplow/page?page=<kind>`, a canonical ref): a core
    /// page the panel summarizes, as Comments opens the inbox.
    /// Absent, the body viz's own page.
    pub open: Option<String>,
    /// The viz shown, compact, while the panel is collapsed: its one-line
    /// summary. Absent, a collapsed panel shows only its header.
    pub collapsed: Option<String>,
    /// The viz whose row count (a `number` viz: its value) is the
    /// header's count, without raising an alert. It wins over
    /// the badge's count; the badge still feeds Alerts.
    pub count: Option<String>,
    /// The panel lives in the bottom dock (an icon with a badge that opens
    /// the body above the bar), not in the rail.
    #[serde(default)]
    pub dock: bool,
}

/// The schemes a page may render besides oxplow's resources: a web page.
pub const RENDERED_SCHEMES: &[&str] = &["https", "http"];

/// The namespace oxplow's own extension declares (`namespace: oxplow`): its
/// commands are `oxplow.<area>.<verb>` and its pages `oxplow:oxplow/page?page=<id>`.
pub const CORE_NAMESPACE: &str = "oxplow";

/// A page's `for`: one resource or several.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub(crate) enum Renders {
    One(String),
    Many(Vec<String>),
}

impl Renders {
    pub(crate) fn into_vec(self) -> Vec<String> {
        match self {
            Renders::One(r) => vec![r],
            Renders::Many(rs) => rs,
        }
    }
}

/// A `panels:` entry as the manifest holds it.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct PanelFile {
    id: String,
    title: String,
    #[serde(default)]
    icon: Option<String>,
    scope: PanelScope,
    body: String,
    #[serde(default)]
    badge: Option<String>,
    #[serde(default)]
    open: Option<String>,
    #[serde(default)]
    collapsed: Option<String>,
    #[serde(default)]
    count: Option<String>,
    /// Draw the panel in the bottom dock, not in the rail.
    #[serde(default)]
    dock: bool,
}

/// A loaded extension and anything wrong with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    pub name: String,
    /// The namespace its commands' ids are under: the manifest's
    /// `namespace:`, else the name with `-` → `_`.
    pub namespace: String,
    pub description: String,
    /// Repo-relative path of the extension folder.
    pub path: String,
    /// Problems found while loading; empty when healthy. A page that
    /// failed to load is listed here and missing from `page_files`.
    pub errors: Vec<String>,
    /// Things worth fixing that don't stop it loading: an intent with no
    /// examples.
    pub warnings: Vec<String>,
    /// The manifest version it was read as (`manifest:`).
    pub manifest_version: u32,
    /// Why it exists (required; `None` only when the manifest didn't load).
    pub intent: Option<Intent>,
    /// Its page files (`pages/*.yaml`; valid ones — invalid ones are in
    /// `errors`).
    #[serde(default)]
    pub page_files: Vec<crate::pages::schema::Page>,
    /// Declared collectors (valid ones; invalid ones are in `errors`).
    pub collectors: Vec<oxplow_config::collectors::CollectorSpec>,
    /// Declared providers (valid ones — invalid ones are in `errors`).
    pub providers: Vec<crate::providers::ProviderSpec>,
    /// Web components its `custom` viz nodes render, sandboxed (valid ones).
    pub custom_components: Vec<custom_components::CustomComponent>,
    /// Event types it declares (valid ones — the vocabulary registers
    /// them, `vocabulary_reactor`).
    pub event_types: crate::extension::extension_event_types::EventTypes,
    /// Resources it declares (valid ones — the vocabulary registers
    /// them, `vocabulary_reactor`).
    pub resources: Vec<crate::extension::extension_resources::ExtensionResource>,
    /// Effects it declares (valid ones; each reacts once the extension is
    /// deployed, `effects`).
    pub effects: Vec<crate::effects::EffectDecl>,
    /// The capability implementations it declares (`implementations:`,
    /// each a built-in of core's).
    pub implementations: Vec<implementations::ImplementationDecl>,
    /// Skills and slash commands it gives the coding agent (`skills:`;
    /// valid ones — invalid ones are in `errors`).
    pub skills: Vec<skills::SkillDecl>,
    /// Another extension's event types its effects and collectors react
    /// to (P9.D1): each resolves when an enabled extension registers the
    /// type (`vocabulary_reactor`).
    pub subscriptions: Vec<ForeignSubscription>,
    /// `project` (in `.oxplow/extensions/`) or `bundled` (ships with oxplow,
    /// read-only).
    pub origin: String,
    /// What it adds to the core UI (`ui:`).
    pub ui: ExtensionUi,
    /// False when configuration disables it or public contracts block it;
    /// an inactive extension contributes nothing to the runtime catalog.
    pub enabled: bool,
    /// Guidance for the coding agent (valid ones; invalid ones are in `errors`).
    pub advisories: Vec<Advisory>,
    /// Measures and metrics it contributes to the metric catalog (the
    /// `project.yaml` schema). Metrics are `key:` definitions and are on
    /// while the extension is enabled; the facts they read come from its
    /// collectors (`collectors:` with `facts:`).
    pub measures: Vec<oxplow_config::MeasureEntry>,
    pub metrics: Vec<oxplow_config::MetricEntry>,
    /// Dimensions it contributes (fact or entity), never promoted: an
    /// extension toggling would rebuild the metric cube each time.
    pub dimensions: Vec<oxplow_config::DimensionEntry>,
    /// Its datasets (`datasets:` plus `datasets/<name>.sql`), published as
    /// `v_<extension>_<name>` (`extension_datasets`). Empty when any
    /// declaration is broken (see `errors`).
    pub datasets: Vec<oxplow_db::datasets::DatasetSource>,
    /// Left-nav panels it contributes (valid ones; invalid ones are in
    /// `errors`).
    pub panels: Vec<ExtensionPanel>,
    /// Commands it registers on the bus, each a Starlark script composing
    /// core commands (P6b; valid ones — invalid ones are in `errors`).
    pub commands: Vec<crate::extension::extension_commands::ExtensionCommand>,
    /// Durable host operations declared by this extension.
    pub jobs: Vec<jobs::JobDecl>,
    /// Named dataset needs this extension declares.
    pub ai_profiles: Vec<ai_profiles::AiProfileDecl>,
    /// Its `version:` (semver), when it declares one; required to share.
    pub version: Option<String>,
    /// Its contributions as descriptors (`extensions::descriptors`): the
    /// kinds that have moved onto them so far.
    pub contributions: Vec<oxplow_domain::contribution::Descriptor>,
    /// Validated non-secret settings declaration, not resolved values or grants.
    #[serde(default)]
    pub configuration: Option<config_schema::ConfigurationFile>,
    /// The sources of its configuration migration steps, by `entry`: read
    /// and parsed at load so activation runs them without touching files.
    #[serde(default, skip_serializing)]
    pub config_scripts: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub public_contracts: public_contracts::PublicContracts,
    /// Where its files came from.
    #[serde(default)]
    pub deployment: Deployment,
    /// What its manifest declares it may reach beyond the baseline
    /// (`permissions:`).
    #[serde(default)]
    pub permissions: Permissions,
    /// The other extensions it needs on (`requires:`).
    #[serde(default)]
    pub requires: Vec<String>,
    /// What a deploy builds before it copies (`build:`).
    #[serde(default)]
    pub build: Option<manifest_v2::Build>,
    /// What it waits on a person for when it is on but undeployed: the
    /// permissions its source would add, one line each
    /// ([`Permissions::added_over`]), and its build command when it has one
    /// (`extension_deploy::build_line`): what its deploy will ask.
    #[serde(default)]
    pub pending: Vec<String>,
    /// It is the team's: the project's own list names it
    /// (`oxplow.extension.share`).
    #[serde(default)]
    pub shared: bool,
}

impl Extension {
    /// Its declared configuration with no project or personal override: what a
    /// check, which reads no stored settings, compiles `config()` reads against.
    pub fn default_configuration(&self) -> Option<serde_json::Value> {
        self.configuration
            .as_ref()
            .and_then(|declared| declared.resolve(&self.name, None, None).ok())
            .map(|resolved| resolved.values)
    }

    /// The project's own: not one that ships with oxplow. Only it may run
    /// its own programs to record facts and parse reports, under the
    /// permissions a person approved when it was deployed.
    pub fn is_own(&self) -> bool {
        self.origin == "project"
    }
}

/// A page another extension's page shows at one of its slots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PageSlot {
    /// The page whose slot it is (`<extension>/<slug>`).
    pub host: String,
    /// The slot's name in that page.
    pub slot: String,
    /// The page mounted (`<extension>/<slug>`).
    pub page: String,
    /// Where among the slot's siblings; none: at the slot.
    pub at: Option<SlotAt>,
    pub title: Option<String>,
    pub size: Option<TileSize>,
}

impl PageSlot {
    /// Its contribution id among the extension's slot mounts: the `n`th
    /// (`<host>.<slot>-<n>`, `/` → `.`).
    pub fn local_id(&self, n: usize) -> String {
        format!("{}.{}-{n}", self.host.replace('/', "."), self.slot)
    }
}

/// What an extension adds to the core UI (`ui:` in its manifest).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionUi {
    /// Pages mounted at other pages' slots (valid ones).
    #[serde(default)]
    pub page_slots: Vec<PageSlot>,
    /// Labels from its datasets on core refs (valid ones).
    pub decorators: Vec<decorators::UiDecorator>,
    /// Viz nodes that take the place of a core sub-component while its
    /// provider is the capability's active one (valid ones).
    pub replacements: Vec<replacements::UiReplacement>,
}

/// Load every extension of the project at `root` from its source:
/// bundled ones from their source when `root` has one (`extensionSources`,
/// or `.oxplow/extensions/<name>/`), else as they ship; every other one found
/// in `.oxplow/extensions/` or named by the settings. A source load is what
/// an author's sources would do ([`sources_on`]); what runs is the deployed
/// load ([`load_deployed`]).
pub fn load_extensions(root: &Path) -> Vec<Extension> {
    load_extensions_in(root, root)
}

/// [`load_extensions`] of `root`'s sources, with `project`'s settings
/// saying which are on and where sources are — a stream's working copy
/// read under the main checkout's config.
pub fn load_extensions_in(root: &Path, project: &Path) -> Vec<Extension> {
    let settings = oxplow_config::extension_settings(project);
    let sources = load_sources(root, &settings);
    let settings = sources_on(settings, &sources);
    resolve_catalog(sources, &settings)
}

/// `settings` for a source load: every project source loaded is on unless
/// the person turned it off (`disableExtensions`); for the extensions that
/// ship with oxplow, the lists decide as always.
pub(crate) fn sources_on(
    mut settings: oxplow_config::ExtensionSettings,
    sources: &[Extension],
) -> oxplow_config::ExtensionSettings {
    for ext in sources {
        let present = ext.origin == "project" && ext.deployment == Deployment::Source;
        if present
            && !settings.personal.disable.contains(&ext.name)
            && !settings.is_enabled(&ext.name)
        {
            settings.personal.enable.push(ext.name.clone());
        }
    }
    settings
}

/// How a folder is shown: relative to `root` when inside it, else whole.
pub(crate) fn shown(root: &Path, dir: &Path) -> String {
    dir.strip_prefix(root)
        .map(|rel| rel.to_string_lossy().into_owned())
        .unwrap_or_else(|_| dir.to_string_lossy().into_owned())
}

/// Every extension name the project at `root` knows: the bundled ones, the
/// folders in `.oxplow/extensions/`, the names `extensionSources` points
/// at and the names listed as on. Bundled first, then the rest by name.
pub fn known_names(root: &Path, settings: &oxplow_config::ExtensionSettings) -> Vec<String> {
    let mut names: BTreeSet<String> = std::fs::read_dir(root.join(EXTENSIONS_DIR))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.extend(settings.team_sources.keys().cloned());
    names.extend(settings.personal_sources.keys().cloned());
    names.extend(settings.enabled().into_iter().map(|(name, _)| name));
    let bundled = crate::bundled_extensions::BUNDLED
        .iter()
        .map(|b| b.name.to_string());
    let rest: Vec<String> = names
        .into_iter()
        .filter(|n| crate::bundled_extensions::find(n).is_none())
        .collect();
    bundled.chain(rest).collect()
}

/// A bundled extension as it ships, from its shipped deployment in
/// `project`'s store (written there the first time).
fn shipped(project: &Path, b: &crate::bundled_extensions::BundledExtension) -> Extension {
    match crate::extension::extension_store::extract_shipped(project, b) {
        Ok(hash) => {
            let dir = crate::extension::extension_store::deployment_dir(project, b.name, &hash);
            Extension {
                deployment: Deployment::Shipped { hash },
                ..load_one(
                    &Disk(dir.clone()),
                    b.name,
                    &dir.to_string_lossy(),
                    "bundled",
                )
            }
        }
        Err(e) => Extension {
            errors: vec![format!(
                "{}: the copy that ships with oxplow couldn't be written to {}: {e}",
                b.name,
                crate::extension::extension_store::STORE_DIR
            )],
            ..empty_extension(b.name, b.name, "bundled")
        },
    }
}

/// The warning on a shipped extension whose name a project folder also has.
fn ignored_folder(folder: &str, name: &str) -> String {
    format!(
        "{folder}: ignored — `{name}` ships with oxplow, which runs the copy built into it; \
         give the folder another name"
    )
}

/// The extension named `name` in `extensions`, a load's list. A load holds
/// one entry per name (a project folder never stands in for an extension
/// that ships with oxplow), so this is the one lookup by name: nothing
/// tie-breaks on where an extension came from.
pub fn owner<'a>(extensions: &'a [Extension], name: &str) -> Option<&'a Extension> {
    extensions.iter().find(|ext| ext.name == name)
}

/// A listed extension that has nothing to run here.
fn listed_only(name: &str, path: &str, deployment: Deployment) -> Extension {
    Extension {
        enabled: false,
        deployment,
        ..empty_extension(name, path, "project")
    }
}

pub(crate) fn origin_of(name: &str) -> &'static str {
    if crate::bundled_extensions::find(name).is_some() {
        "bundled"
    } else {
        "project"
    }
}

pub(super) fn load_sources(
    root: &Path,
    settings: &oxplow_config::ExtensionSettings,
) -> Vec<Extension> {
    let mut out = Vec::new();
    for name in known_names(root, settings) {
        let dir = settings.source(root, &name);
        // A shipped oxplow runs the copy of its own extensions built into
        // it: a folder with that name stands in for nothing.
        if let Some(b) = crate::bundled_extensions::shipped(&name, settings) {
            let mut ext = shipped(root, b);
            if dir.is_dir() {
                ext.warnings.push(ignored_folder(&shown(root, &dir), &name));
            }
            out.push(ext);
        } else if dir.is_dir() {
            out.push(load_one(
                &Disk(dir.clone()),
                &name,
                &shown(root, &dir),
                origin_of(&name),
            ));
        } else if settings.is_enabled(&name) {
            out.push(listed_only(&name, &shown(root, &dir), Deployment::NotHere));
        }
    }
    for ext in &mut out {
        locate(ext, root, settings);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.origin.cmp(&b.origin)));
    out
}

/// Load the extensions the project at `project` runs: each one's
/// deployment in `live` (`name → hash`), a bundled one without a
/// deployment as it ships, and the rest it knows listed but not run
/// (undeployed when its source is here, not here otherwise).
pub fn load_deployed(
    project: &Path,
    settings: &oxplow_config::ExtensionSettings,
    live: &BTreeMap<String, String>,
) -> Vec<Extension> {
    let mut names = known_names(project, settings);
    for name in live.keys() {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    let mut out: Vec<Extension> = names
        .iter()
        .filter_map(|name| {
            if let Some(hash) = live.get(name) {
                let dir = crate::extension::extension_store::deployment_dir(project, name, hash);
                return Some(Extension {
                    deployment: Deployment::Deployed { hash: hash.clone() },
                    ..load_one(
                        &Disk(dir.clone()),
                        name,
                        &dir.to_string_lossy(),
                        origin_of(name),
                    )
                });
            }
            if let Some(b) = crate::bundled_extensions::shipped(name, settings) {
                return Some(shipped(project, b));
            }
            let source = settings.source(project, name);
            if source.is_dir() {
                let loaded = load_extension_at(&source, name);
                let mut pending = loaded.permissions.added_over(&Permissions::default());
                if let Some(build) = &loaded.build {
                    pending.push(crate::extension::extension_deploy::build_line(
                        &build.run,
                        &shown(project, &source),
                    ));
                }
                Some(Extension {
                    pending,
                    permissions: loaded.permissions,
                    ..listed_only(name, &shown(project, &source), Deployment::Undeployed)
                })
            } else if settings.is_enabled(name) || crate::bundled_extensions::is_required(name) {
                Some(listed_only(
                    name,
                    &shown(project, &source),
                    Deployment::NotHere,
                ))
            } else {
                None
            }
        })
        .collect();
    for ext in &mut out {
        ext.shared = settings.team.contains(&ext.name);
        locate(ext, project, settings);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.origin.cmp(&b.origin)));
    resolve_catalog(out, settings)
}

/// Fill in where each of `ext`'s pages comes from: the deployment it
/// runs, and its source file when the source is here.
fn locate(ext: &mut Extension, project: &Path, settings: &oxplow_config::ExtensionSettings) {
    let deployment = ext.deployment.hash().map(str::to_string);
    let dir = settings.source(project, &ext.name);
    // A shipped copy has no source here: a folder with its name is ignored.
    let shipped = matches!(ext.deployment, Deployment::Shipped { .. });
    let source = (!shipped && dir.is_dir()).then(|| shown(project, &dir));
    // What it reports names the source files, the ones to edit, not the
    // copy that runs.
    if let Some(source) = source.as_ref().filter(|s| **s != ext.path) {
        let (from, to) = (format!("{}/", ext.path), format!("{source}/"));
        for message in ext.errors.iter_mut().chain(ext.warnings.iter_mut()) {
            if let Some(rest) = message.strip_prefix(&from) {
                *message = format!("{to}{rest}");
            }
        }
    }
    for page in &mut ext.page_files {
        let file = page.origin.running.rsplit('/').next().unwrap_or_default();
        let at = source.as_ref().map(|dir| format!("{dir}/pages/{file}"));
        let running_uri = deployment
            .as_deref()
            .map(|hash| crate::filesystems::extensions::page_running_uri(&ext.name, file, hash));
        let mut place = |o: &mut PageProvenance| {
            o.source = at.clone();
            o.deployment = deployment.clone();
            o.running_uri = running_uri.clone();
        };
        place(&mut page.origin);
        page.view.for_each_origin(&mut place);
        if let Some(rail) = page.rail.as_mut() {
            rail.for_each_origin(&mut place);
        }
    }
}

/// `rel` is `dir` or inside it, by `/` segments.
fn under(dir: &str, rel: &str) -> bool {
    let dir: Vec<&str> = dir.trim_end_matches('/').split('/').collect();
    let rel: Vec<&str> = rel.trim_end_matches('/').split('/').collect();
    rel.starts_with(&dir)
}

/// Extensions whose `requires` lead back to themselves could never be
/// turned on: each member of the cycle is an error naming the next.
fn refuse_requires_cycles(out: &mut [Extension]) {
    let requires: BTreeMap<String, Vec<String>> = out
        .iter()
        .map(|e| (e.name.clone(), e.requires.clone()))
        .collect();
    for ext in out.iter_mut() {
        for next in &ext.requires {
            // Walk on from `next`; reaching `ext` again is a cycle.
            let mut seen = vec![next.as_str()];
            let mut i = 0;
            let mut back = false;
            while i < seen.len() {
                for r in requires.get(seen[i]).into_iter().flatten() {
                    if *r == ext.name {
                        back = true;
                    } else if !seen.contains(&r.as_str()) {
                        seen.push(r.as_str());
                    }
                }
                i += 1;
            }
            if back {
                ext.errors.push(format!(
                    "{}/extension.yaml: it requires `{next}`, which requires it back",
                    ext.path
                ));
            }
        }
    }
}

/// Whether the settings leave `name` off. A required extension is never off.
pub(crate) fn is_off(settings: &oxplow_config::ExtensionSettings, name: &str) -> bool {
    !crate::bundled_extensions::is_required(name) && !settings.is_enabled(name)
}

pub(super) fn resolve_catalog(
    mut out: Vec<Extension>,
    settings: &oxplow_config::ExtensionSettings,
) -> Vec<Extension> {
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.origin.cmp(&b.origin)));
    refuse_requires_cycles(&mut out);
    let mut out: Vec<Extension> = out
        .into_iter()
        .map(|mut e| {
            if is_off(settings, &e.name) {
                e.enabled = false;
            }
            e
        })
        .collect();
    public_contracts::stage(&mut out);
    crate::extension::extension_commands::refuse_shared_namespaces(&mut out);
    crate::pages::schema::refuse_shared_core_pages(&mut out);
    crate::extension::extension_commands::refuse_duplicate_tools(&mut out);
    for ext in &mut out {
        ext.contributions.retain(|descriptor| {
            descriptor.id.kind != oxplow_domain::contribution::ContributionKind::Command
                || ext
                    .commands
                    .iter()
                    .any(|command| descriptor.global.as_deref() == Some(&command.name))
        });
    }
    descriptors::check_catalog(&mut out);
    warn_unplaced_mounts(&mut out);
    public_contracts::activate(&mut out);
    let out: Vec<Extension> = out
        .into_iter()
        .map(|mut ext| {
            if ext.public_contracts.blocked {
                clear_contributions(&mut ext);
            }
            apply_disabled(ext, settings)
        })
        .collect();
    out
}

/// A slot mount whose host page isn't loaded, or has no slot by that name,
/// shows nowhere: a warning on the mounting extension (the host may only
/// be off).
fn warn_unplaced_mounts(out: &mut [Extension]) {
    let slots: BTreeMap<String, Vec<String>> = out
        .iter()
        .filter(|e| e.enabled)
        .flat_map(|e| e.page_files.iter())
        .map(|p| {
            let names = p
                .nodes()
                .into_iter()
                .filter_map(|n| match &n.kind {
                    crate::pages::schema::NodeKind::Slot { name } => Some(name.clone()),
                    _ => None,
                })
                .collect();
            (p.id.clone(), names)
        })
        .collect();
    for ext in out.iter_mut().filter(|e| e.enabled) {
        let warnings: Vec<String> = ext
            .ui
            .page_slots
            .iter()
            .filter_map(|m| match slots.get(&m.host) {
                None => Some(format!(
                    "{}/extension.yaml: slot `{}.{}` mounts `{}` on page `{}`, which isn't loaded; it shows nowhere",
                    ext.path, m.host, m.slot, m.page, m.host
                )),
                Some(names) if !names.contains(&m.slot) => Some(format!(
                    "{}/extension.yaml: page `{}` has no slot `{}` (its slots: {}); `{}` shows nowhere",
                    ext.path,
                    m.host,
                    m.slot,
                    if names.is_empty() { "none".to_string() } else { names.join(", ") },
                    m.page
                )),
                Some(_) => None,
            })
            .collect();
        ext.warnings.extend(warnings);
    }
}

/// A catalog block's error as the manifest's, not project.yaml's.
fn manifest_message(e: oxplow_config::ConfigError) -> String {
    match e {
        oxplow_config::ConfigError::Invalid(m) => m,
        other => other.to_string(),
    }
}

/// A `measures:` / `metrics:` block as typed entries.
fn parse_block<T: serde::de::DeserializeOwned>(
    v: Option<serde_yaml::Value>,
) -> Result<Option<Vec<T>>, String> {
    v.map(|v| serde_yaml::from_value(v).map_err(|e| e.to_string()))
        .transpose()
}

/// Where `name`'s files are for a preview in the checkout at `root`: its
/// source when the checkout has one, else, for an extension that ships with
/// oxplow, its shipped deployment in `project`'s store.
pub fn source_or_shipped(project: &Path, root: &Path, name: &str) -> String {
    let settings = oxplow_config::extension_settings(project);
    if let Some(b) = crate::bundled_extensions::shipped(name, &settings) {
        if let Ok(hash) = crate::extension::extension_store::extract_shipped(project, b) {
            return crate::extension::extension_store::deployment_dir(project, name, &hash)
                .to_string_lossy()
                .into_owned();
        }
    }
    shown(root, &settings.source(root, name))
}

/// Extension `name` alone, from its default source folder under `root`
/// (`.oxplow/extensions/<name>/`).
pub fn load_project_extension(root: &Path, name: &str) -> Extension {
    let rel = format!("{EXTENSIONS_DIR}/{name}");
    load_one(&Disk(root.join(&rel)), name, &rel, origin_of(name))
}

/// Extension `name` alone, from the folder `dir` (its source, or a copy
/// laid out like one).
pub fn load_extension_at(dir: &Path, name: &str) -> Extension {
    load_one(
        &Disk(dir.to_path_buf()),
        name,
        &dir.to_string_lossy(),
        origin_of(name),
    )
}

/// A file inside the extension whose listed `path` is `path` (its
/// folder), e.g. a collector's script. `None` when there's no such file.
pub fn read_extension_file(root: &Path, path: &str, rel: &str) -> Option<String> {
    if !oxplow_domain::service::valid_path(rel) {
        return None;
    }
    files_at(root, path).ok()?.read(rel)
}

/// The most an extension file read from disk may hold.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// The files at an extension's `path` as a listing gives it: the folder
/// `project_dir/path` on disk (an absolute `path`, a deployment's, is
/// itself) — one resolver, so what an approval covers is read the same way
/// wherever the extension lives.
pub(crate) fn files_at(project_dir: &Path, path: &str) -> std::io::Result<Box<dyn ExtensionFiles>> {
    Ok(Box::new(Disk(project_dir.join(path))))
}

/// Where an extension's files come from: a folder on disk, or one
/// revision's in memory.
pub(crate) trait ExtensionFiles {
    /// Contents of a file, by path inside the extension folder.
    fn read(&self, rel: &str) -> Option<String>;
    /// Every file's path inside the extension folder (`/`-separated), what
    /// an approval covers. On disk, dot-files included and macOS's
    /// `.DS_Store` left out; a symlink is an error — its target isn't what
    /// was approved, and can change freely.
    fn paths(&self) -> std::io::Result<Vec<String>>;
    /// A file's bytes, by path inside the extension folder.
    fn bytes(&self, rel: &str) -> std::io::Result<Vec<u8>>;
    /// File names directly inside `dir` (e.g. `pages`).
    fn list(&self, dir: &str) -> Vec<String>;
    /// What the folder `rel` holds, for a custom component's bundle;
    /// `None` when it isn't a folder (or the files aren't on disk).
    fn bundle_stat(&self, rel: &str) -> custom_components::BundleLook;
}

pub(crate) struct Disk(pub(crate) std::path::PathBuf);

impl Disk {
    fn checked_path(&self, rel: &str) -> std::io::Result<std::path::PathBuf> {
        if !rel.is_empty() && !oxplow_domain::service::valid_path(rel) {
            return Err(std::io::Error::other(
                "extension path must be relative without traversal",
            ));
        }
        let mut path = self.0.clone();
        if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(std::io::Error::other("extension folder is a symlink"));
        }
        for segment in rel
            .trim_end_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
        {
            path.push(segment);
            if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
                return Err(std::io::Error::other("extension path contains a symlink"));
            }
        }
        Ok(path)
    }
}

impl ExtensionFiles for Disk {
    fn read(&self, rel: &str) -> Option<String> {
        String::from_utf8(self.bytes(rel).ok()?).ok()
    }
    fn paths(&self) -> std::io::Result<Vec<String>> {
        let mut out = Vec::new();
        for entry in walkdir::WalkDir::new(&self.0).follow_links(false) {
            let entry = entry.map_err(std::io::Error::other)?;
            if entry.path_is_symlink() {
                return Err(std::io::Error::other(format!(
                    "{} is a symlink; a program's folder can't contain symlinks to be approved \
                     (copy the file in instead)",
                    entry.path().display()
                )));
            }
            if !entry.file_type().is_file() && !entry.file_type().is_dir() {
                return Err(std::io::Error::other(format!(
                    "{} is not a regular file or directory",
                    entry.path().display()
                )));
            }
            if entry.file_type().is_file() && entry.file_name() != ".DS_Store" {
                let rel = entry.path().strip_prefix(&self.0).unwrap_or(entry.path());
                out.push(rel.to_string_lossy().into_owned());
            }
        }
        Ok(out)
    }
    fn bytes(&self, rel: &str) -> std::io::Result<Vec<u8>> {
        use std::io::Read;

        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(self.checked_path(rel)?)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other(
                "extension path is not a regular file",
            ));
        }
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(std::io::Error::other(
                "extension file exceeds the 16 MiB limit",
            ));
        }
        Ok(bytes)
    }
    fn list(&self, dir: &str) -> Vec<String> {
        let Ok(path) = self.checked_path(dir) else {
            return Vec::new();
        };
        std::fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default()
    }
    fn bundle_stat(&self, rel: &str) -> custom_components::BundleLook {
        // `stat_bundle` never follows a symlink and reports one (the folder
        // itself included), so a symlinked bundle says so rather than
        // looking absent; only the path's shape is checked here.
        if !oxplow_domain::service::valid_path(rel) {
            return custom_components::BundleLook::Absent;
        }
        match custom_components::stat_bundle(&self.0.join(rel)) {
            Some(stat) => custom_components::BundleLook::Found(stat),
            None => custom_components::BundleLook::Absent,
        }
    }
}

/// An extension's files held in memory — one revision's (P8.C1): paths
/// inside the extension folder → their text.
pub struct Tree(std::collections::BTreeMap<String, String>);

impl Tree {
    pub fn new(files: impl IntoIterator<Item = (String, String)>) -> Self {
        Self(files.into_iter().collect())
    }

    /// A file's text, by path inside the extension folder.
    pub fn file(&self, rel: &str) -> Option<String> {
        self.0.get(rel).cloned()
    }

    /// The extension these files load as, its paths shown under `rel`.
    pub fn load(&self, name: &str, rel: &str) -> Extension {
        load_one(self, name, rel, "project")
    }
}

impl ExtensionFiles for Tree {
    fn read(&self, rel: &str) -> Option<String> {
        self.0.get(rel).cloned()
    }
    fn paths(&self) -> std::io::Result<Vec<String>> {
        Ok(self.0.keys().cloned().collect())
    }
    fn bytes(&self, rel: &str) -> std::io::Result<Vec<u8>> {
        self.0
            .get(rel)
            .map(|t| t.clone().into_bytes())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, rel.to_string()))
    }
    /// What `dir` holds directly: its files, and the folders on the way to
    /// deeper ones, as a directory listing would show them.
    fn list(&self, dir: &str) -> Vec<String> {
        let prefix = format!("{}/", dir.trim_end_matches('/'));
        self.0
            .keys()
            .filter_map(|p| p.strip_prefix(&prefix))
            .map(|rest| rest.split('/').next().unwrap_or(rest).to_string())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    /// A bundle checked from the tree's own paths, when it holds them; a
    /// built bundle usually isn't committed, so none is `Unknown`, not an
    /// error (tsk784).
    fn bundle_stat(&self, rel: &str) -> custom_components::BundleLook {
        custom_components::look_in_paths(rel, self.0.keys().map(String::as_str))
    }
}

/// Project extension `name` as it is at `rev` of the workspace `ws` — a
/// commit, a snapshot, the working tree — through `Trees` (P8.C1). `None`
/// when that revision has no `.oxplow/extensions/<name>/extension.yaml`.
pub async fn extension_at(
    trees: &crate::trees::Trees,
    ws: &Path,
    rev: &oxplow_domain::vcs::Revision,
    name: &str,
) -> Result<Option<Extension>, DomainError> {
    let rel = format!("{EXTENSIONS_DIR}/{name}");
    Ok(extension_tree_at(trees, ws, rev, name)
        .await?
        .map(|tree| tree.load(name, &rel)))
}

/// Project extension `name`'s files at `rev` (P8.C1): `None` when that
/// revision has no `extension.yaml` for it.
pub async fn extension_tree_at(
    trees: &crate::trees::Trees,
    ws: &Path,
    rev: &oxplow_domain::vcs::Revision,
    name: &str,
) -> Result<Option<Tree>, DomainError> {
    let prefix = format!("{EXTENSIONS_DIR}/{name}/");
    let files = trees
        .corpus(ws, rev, |p| p.starts_with(&prefix))
        .await?
        .into_iter()
        .filter_map(|(path, text)| Some((path.strip_prefix(&prefix)?.to_string(), text)));
    let tree = Tree::new(files);
    Ok(tree.read("extension.yaml").is_some().then_some(tree))
}

/// One extension that changed between two revisions of a workspace:
/// what an effort's review shows for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionChange {
    pub name: String,
    /// `added`, `removed` or `changed`.
    pub change: crate::extension::extension_impact::Change,
    /// What the change does; `None` for a removed extension.
    pub impact: Option<crate::extension::extension_impact::ImpactReport>,
    /// What's wrong with the later version.
    pub errors: Vec<String>,
}

/// The extensions under `.oxplow/extensions/` whose files differ between
/// `start` (none: nothing before) and `end` of workspace `ws`, each with
/// what the change does — both versions loaded as their revision holds
/// them and reviewed on their own overlays.
pub async fn extension_changes_between(
    svc: &crate::Services,
    ws: &Path,
    start: Option<&oxplow_domain::vcs::Revision>,
    end: &oxplow_domain::vcs::Revision,
) -> Result<Vec<ExtensionChange>, DomainError> {
    let prefix = format!("{EXTENSIONS_DIR}/");
    let names: BTreeSet<String> = svc
        .trees
        .diff(ws, start, end)
        .await?
        .into_iter()
        .filter_map(|e| {
            let rest = e.path.strip_prefix(&prefix)?;
            let (name, _) = rest.split_once('/')?;
            Some(name.to_string())
        })
        .collect();
    let mut out = Vec::new();
    for name in names {
        let rel = format!("{EXTENSIONS_DIR}/{name}");
        let before = match start {
            Some(rev) => extension_tree_at(&svc.trees, ws, rev, &name).await?,
            None => None,
        };
        let after = extension_tree_at(&svc.trees, ws, end, &name).await?;
        let Some(after) = after else {
            if before.is_some() {
                out.push(ExtensionChange {
                    name,
                    change: crate::extension::extension_impact::Change::Removed,
                    impact: None,
                    errors: Vec::new(),
                });
            }
            continue;
        };
        let read_before = |file: &str| before.as_ref().and_then(|t| t.file(file));
        let read_after = |file: &str| after.file(file);
        let mut side = ReviewSide {
            extension: after.load(&name, &rel),
            read: &read_after,
        };
        let impact = impact_between(
            &svc.sql,
            &svc.extension_catalog,
            ws,
            before.as_ref().map(|t| ReviewSide {
                extension: t.load(&name, &rel),
                read: &read_before,
            }),
            &mut side,
            svc.commands.as_ref(),
        )
        .await;
        out.push(ExtensionChange {
            change: if before.is_some() {
                crate::extension::extension_impact::Change::Changed
            } else {
                crate::extension::extension_impact::Change::Added
            },
            name,
            impact: Some(impact),
            errors: side.extension.errors,
        });
    }
    Ok(out)
}

/// One version of an extension under review: as loaded, and how to read
/// its files.
pub struct ReviewSide<'a> {
    pub extension: Extension,
    pub read: &'a (dyn Fn(&str) -> Option<String> + Sync),
}

/// What going from `before` (none: a first install) to `after` would
/// change: each side on its own overlay — `after` checked,
/// `before` only read — then compared: viz nodes, datasets and their rows,
/// collectors and their outputs, providers, config. `after.extension`
/// gets its check's errors.
pub async fn impact_between(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension::extension_catalog::ExtensionCatalog,
    root: &Path,
    mut before: Option<ReviewSide<'_>>,
    after: &mut ReviewSide<'_>,
    commands: CommandSchemas<'_>,
) -> crate::extension::extension_impact::ImpactReport {
    let prepared_after = prepare(
        layer,
        catalog,
        root,
        &mut after.extension,
        after.read,
        Some(commands),
    )
    .await;
    let prepared_before = match &mut before {
        Some(b) => Some(read_side(layer, catalog, root, &mut b.extension).await),
        None => None,
    };
    let no_runs = crate::pages::checks::PageChecks::new();
    crate::extension::extension_impact::impact(
        layer,
        before
            .as_ref()
            .map(|b| crate::extension::extension_impact::Version {
                extension: &b.extension,
                read: b.read,
                pages: prepared_before.as_ref().map_or(&no_runs, |p| &p.pages),
                overlay: prepared_before
                    .as_ref()
                    .map_or(&[], |p| p.overlay.as_slice()),
            }),
        crate::extension::extension_impact::Version {
            extension: &after.extension,
            read: after.read,
            pages: &prepared_after.pages,
            overlay: &prepared_after.overlay,
        },
    )
    .await
}

/// An effect's or collector's `on:` naming an event type of another
/// extension's namespace (P9.D1). The type's name is the dependency —
/// there is no `depends:` key, as a dataset's `ref('<ext>/<name>')` has
/// none: it resolves when an enabled extension registers the type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ForeignSubscription {
    /// What reacts: "effect `note`", "collector `pull`".
    pub by: String,
    pub event_type: String,
    /// `file:line` of the declaration.
    pub declared_at: String,
}

impl ForeignSubscription {
    /// What loading says: not an error — the owner may not be here yet.
    fn warning(&self) -> String {
        format!(
            "{}: {} reacts to `{}`, another extension's event type; it runs once an enabled \
             extension registers that type (`v_event_type`)",
            self.declared_at, self.by, self.event_type
        )
    }
}

/// How an `on:` type stands for the extension declaring the reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subscribes {
    /// A core type, or one it declares.
    Known,
    /// One in another extension's namespace: known when that one is here.
    Foreign,
    /// No type at all: a core namespace's that doesn't exist, its own
    /// that it doesn't declare, a name that isn't a type's.
    Unknown,
}

fn subscribes(
    extension: &str,
    declared: &[crate::extension::extension_event_types::EventTypeDecl],
    event_type: &str,
) -> Subscribes {
    use oxplow_domain::events::schema::{
        extension_namespace, extension_type_namespace, is_core_type,
    };
    if is_core_type(event_type) || declared.iter().any(|d| d.event_type == event_type) {
        return Subscribes::Known;
    }
    match extension_type_namespace(event_type) {
        Some(ns) if ns != extension_namespace(extension) => Subscribes::Foreign,
        _ => Subscribes::Unknown,
    }
}

pub(crate) fn empty_extension(name: &str, path: &str, origin: &str) -> Extension {
    Extension {
        name: name.to_string(),
        namespace: oxplow_domain::contribution::extension_namespace(name),
        description: String::new(),
        path: path.to_string(),
        errors: Vec::new(),
        warnings: Vec::new(),
        manifest_version: manifest_v2::CURRENT,
        intent: None,
        page_files: Vec::new(),
        collectors: Vec::new(),
        providers: Vec::new(),
        custom_components: Vec::new(),
        event_types: Default::default(),
        resources: Vec::new(),
        effects: Vec::new(),
        implementations: Vec::new(),
        skills: Vec::new(),
        subscriptions: Vec::new(),
        origin: origin.to_string(),
        ui: ExtensionUi::default(),
        enabled: true,
        advisories: Vec::new(),
        measures: Vec::new(),
        dimensions: Vec::new(),
        datasets: Vec::new(),
        metrics: Vec::new(),
        panels: Vec::new(),
        commands: Vec::new(),
        jobs: Vec::new(),
        ai_profiles: Vec::new(),
        version: None,
        contributions: Vec::new(),
        configuration: None,
        config_scripts: Default::default(),
        public_contracts: Default::default(),
        deployment: Deployment::Source,
        permissions: Permissions::default(),
        requires: Vec::new(),
        build: None,
        pending: Vec::new(),
        shared: false,
    }
}

/// Load one extension. Always returns an `Extension`; problems go in
/// `errors`. `rel` is how its paths are shown (a repo-relative folder,
/// or `bundled:<name>`).
fn load_one(files: &dyn ExtensionFiles, name: &str, rel: &str, origin: &str) -> Extension {
    let mut ext = empty_extension(name, rel, origin);

    let Some(manifest) = files.read("extension.yaml") else {
        ext.errors.push(format!("{rel}: missing extension.yaml"));
        return ext;
    };
    let file = format!("{rel}/extension.yaml");
    // `manifest:` says which shape to read; this oxplow reads one.
    if key_line(&manifest, "manifest").is_none() {
        ext.errors.push(at(
            &file,
            Some(1),
            format!(
                "`manifest: {}` is required: it says which shape the file is in",
                manifest_v2::CURRENT
            ),
        ));
        return ext;
    }
    let parsed: Result<ManifestV2, String> = serde_yaml::from_str::<ManifestV2>(&manifest)
        .map_err(|e| oxplow_domain::manifest_keys::hint_unknown(&e.to_string()));
    let m = match parsed {
        Ok(m) if m.name != name => {
            ext.errors.push(format!(
                "{file}: name `{}` must match its folder `{name}`",
                m.name
            ));
            return ext;
        }
        Ok(m) => m,
        Err(e) => {
            ext.errors.push(format!("{file}: {e}"));
            return ext;
        }
    };
    ext.manifest_version = m.manifest;
    // A bundled extension ships with oxplow: its version is oxplow's.
    ext.version = if origin == "bundled" {
        Some(env!("CARGO_PKG_VERSION").to_string())
    } else {
        m.version.clone()
    };
    if origin != "bundled" && !oxplow_domain::contribution::valid_extension_name(name) {
        ext.errors.push(format!(
            "{file}: the name `{name}` must be lowercase letters, digits and single dashes, \
             starting with a letter (rename the folder)"
        ));
    }
    if let Some(ns) = &m.namespace {
        match crate::extension::extension_commands::check_namespace(ns, origin == "bundled") {
            Ok(()) => ext.namespace = ns.clone(),
            Err(e) => ext
                .errors
                .push(at(&file, key_line(&manifest, "namespace"), e)),
        }
    }
    for required in &m.requires {
        let problem = if required == name {
            Some("an extension can't require itself".to_string())
        } else if !oxplow_domain::contribution::valid_extension_name(required) {
            Some(format!("`{required}` isn't an extension name"))
        } else {
            None
        };
        match problem {
            Some(p) => ext.errors.push(at(
                &file,
                key_line(&manifest, "requires"),
                format!("requires: {p}"),
            )),
            None => ext.requires.push(required.clone()),
        }
    }
    let (build, errors) = manifest_v2::build_of(&m, &file, &manifest);
    ext.build = build;
    ext.errors.extend(errors);
    ext.intent = manifest_v2::intent_of(&m, &file, &manifest).0;
    ext.description = m.description.clone();
    ext.configuration = manifest_v2::configuration_of(&m, &file, &manifest).0;
    if let Some(configuration) = &ext.configuration {
        let line = key_line(&manifest, "configuration");
        for step in &configuration.migrations {
            match files.read(&step.entry) {
                None => ext.errors.push(at(
                    &file,
                    line,
                    format!(
                        "configuration migration {} -> {}: `{}` isn't in the extension",
                        step.from, step.to, step.entry
                    ),
                )),
                Some(script) => {
                    match oxplow_script::runtime::check_starlark(&step.entry, &script) {
                        Ok(()) => {
                            ext.config_scripts.insert(step.entry.clone(), script);
                        }
                        Err(e) => ext.errors.push(at(
                            &file,
                            line,
                            format!(
                                "configuration migration {} -> {}: `{}` {e}",
                                step.from, step.to, step.entry
                            ),
                        )),
                    }
                }
            }
        }
    }
    public_contracts::load(&mut ext, &m, &manifest, &file);
    let (errors, warnings) = manifest_v2::check(&m, &file, &manifest);
    if !errors.is_empty()
        && (!m.exports.is_empty() || !m.imports.is_empty() || !m.import_guards.is_empty())
    {
        ext.public_contracts.invalid = true;
    }
    ext.errors.extend(errors);
    ext.warnings.extend(warnings);
    let panel_files = m.panels.clone();
    let decorator_files = m.ui.decorators.clone();
    let resource_files = m.resources.clone();
    let slot_files = {
        if let Some(v) = m.event_types.as_ref() {
            let (declared, errors) = crate::extension::extension_event_types::parse_event_types(
                name,
                v,
                &file,
                &manifest,
                &|rel| files.read(rel),
            );
            ext.event_types = declared;
            ext.errors.extend(errors);
        }
        // After `event_types`: an effect may react to its own types — and
        // to another extension's (P9.D1), which resolves when that one is
        // here (`subscriptions`).
        let declared = ext.event_types.types.clone();
        let subscribable = |t: &str| subscribes(name, &declared, t) != Subscribes::Unknown;
        if let Some(v) = m.effects.as_ref() {
            let (effects, errors) = crate::effects::parse_effects(
                name,
                v,
                &file,
                &manifest,
                &|rel| files.read(rel),
                &subscribable,
            );
            for e in &effects {
                for t in &e.on {
                    if subscribes(name, &declared, t) == Subscribes::Foreign {
                        ext.subscriptions.push(ForeignSubscription {
                            by: format!("effect `{}`", e.id),
                            event_type: t.clone(),
                            declared_at: e.declared_at.clone(),
                        });
                    }
                }
            }
            ext.effects = effects;
            ext.errors.extend(errors);
        }
        if let Some(v) = m.skills.as_ref() {
            let (declared, errors) = skills::parse_skills(v, &file, &manifest, files);
            ext.skills = declared;
            ext.errors.extend(errors);
        }
        if let Some(v) = m.implementations.as_ref() {
            let (declared, errors) =
                implementations::parse_implementations(v, &file, &manifest, &|rel| files.read(rel));
            // An implementation owns the skills it lists: they must be its
            // extension's.
            for d in &declared {
                for skill in d
                    .skills
                    .iter()
                    .filter(|s| !ext.skills.iter().any(|k| &k.name == *s))
                {
                    ext.errors.push(at(
                        &file,
                        entry_line(&manifest, "implementations", "id", &d.id),
                        format!("implementation `{}` lists skill `{skill}`, which `skills:` doesn't declare", d.id),
                    ));
                }
            }
            ext.implementations = declared;
            ext.errors.extend(errors);
        }
        // The project's own: not one that ships with oxplow.
        let own = origin == "project";
        if let Some(v) = &m.collectors {
            // A collector may follow core types, its own extension's, and
            // another's.
            let (collectors, errors) = oxplow_config::collectors::parse_collectors_indexed(
                &oxplow_config::collectors::Declarer {
                    extension: name.to_string(),
                    namespace: ext.namespace.clone(),
                    own,
                },
                v,
                &subscribable,
            );
            let line = key_line(&manifest, "collectors");
            let lines = item_lines(&manifest, "collectors");
            ext.errors.extend(errors.into_iter().map(|(i, e)| {
                let at_entry = i.and_then(|i| lines.get(i).copied());
                at(&file, at_entry.or(line), format!("collector: {e}"))
            }));
            // A script or program the collector runs must be in the folder,
            // and a Starlark script must parse and define `transform`; a
            // report read by one of oxplow's own parsers (`oxplow:<name>`)
            // runs nothing of the extension's.
            for c in collectors {
                let c_line = entry_line(&manifest, "collectors", "id", &c.id).or(line);
                if c.bundled_parser().is_some() {
                    ext.collectors.push(c);
                    continue;
                }
                match c.entry.as_deref().map(|entry| (entry, files.read(entry))) {
                    Some((entry, None)) => ext.errors.push(at(
                        &file,
                        c_line,
                        format!(
                            "collector `{}`: entry `{entry}` isn't in the extension",
                            c.id
                        ),
                    )),
                    Some((entry, Some(script)))
                        if c.runtime == oxplow_config::collectors::CollectorRuntime::Starlark =>
                    {
                        let needs = crate::services::collector_needs(&c, Some(&script));
                        let checked = oxplow_script::runtime::check_starlark(entry, &script)
                            .and_then(|()| {
                                crate::services::check_uses(&script, &needs, |_| Ok(()))
                            });
                        match checked {
                            Ok(()) => ext.collectors.push(c),
                            Err(e) => ext.errors.push(at(
                                &file,
                                c_line,
                                format!("collector `{}`: `{entry}` {e}", c.id),
                            )),
                        }
                    }
                    _ => ext.collectors.push(c),
                }
            }
            for c in &ext.collectors {
                let oxplow_config::collectors::Trigger::On { events, .. } = &c.trigger else {
                    continue;
                };
                for t in events {
                    if subscribes(name, &declared, t) == Subscribes::Foreign {
                        let c_line = entry_line(&manifest, "collectors", "id", &c.id).or(line);
                        ext.subscriptions.push(ForeignSubscription {
                            by: format!("collector `{}`", c.id),
                            event_type: t.clone(),
                            declared_at: at(&file, c_line, "").trim_end_matches(": ").to_string(),
                        });
                    }
                }
            }
        }
        let foreign: Vec<String> = ext
            .subscriptions
            .iter()
            .map(ForeignSubscription::warning)
            .collect();
        ext.warnings.extend(foreign);
        if let Some(v) = &m.providers {
            let (providers, errors) = crate::providers::parse_providers(v, &|rel| files.read(rel));
            ext.providers = providers;
            ext.errors.extend(
                errors
                    .into_iter()
                    .map(|e| at(&file, key_line(&manifest, "providers"), e)),
            );
        }
        if let Some(v) = &m.commands {
            let providers = ext.providers.clone();
            let provider_ops = |provider: &str, op: &str| {
                let spec = providers
                    .iter()
                    .find(|p| p.id == provider)
                    .ok_or_else(|| format!("no provider `{provider}` in this extension"))?;
                let contract = crate::providers::spec::contract_of(spec);
                if contract.verb(op).is_some() {
                    return Err(match contract.dispatch {
                        Some(family) => format!(
                            "`{op}` is a {} verb: it runs as `{family}.{op}`",
                            spec.capability
                        ),
                        None => format!("`{op}` is a {} verb: core calls it", spec.capability),
                    });
                }
                crate::providers::spec::read_declarations(spec, &|rel| files.read(rel))?
                    .commands
                    .into_iter()
                    .find(|c| c.name == op)
                    .ok_or_else(|| format!("its declarations have no command `{op}`"))
            };
            let (commands, errors) = crate::extension::extension_commands::parse_commands(
                &ext.namespace,
                origin == "bundled",
                v,
                &file,
                &manifest,
                &|rel| files.read(rel),
                &provider_ops,
            );
            ext.commands = commands;
            ext.errors.extend(errors);
        }
        if let Some(v) = &m.jobs {
            let (jobs, errors) =
                jobs::parse_jobs(v, &file, &manifest, &ext.namespace, &ext.commands);
            ext.jobs = jobs;
            ext.errors.extend(errors);
        }
        if let Some(v) = &m.ai_profiles {
            let (profiles, errors) = ai_profiles::parse_ai_profiles(v, &file, &manifest);
            ext.ai_profiles = profiles;
            ext.errors.extend(errors);
        }
        if let Some(v) = &m.custom_components {
            let (components, errors) =
                custom_components::parse_custom_components(name, v, &file, &manifest, &|rel| {
                    match (files.bundle_stat(rel), &ext.build) {
                        // Not built yet (no output at all): the deployment's
                        // load checks it.
                        (custom_components::BundleLook::Absent, Some(build))
                            if under(&build.output, rel)
                                && matches!(
                                    files.bundle_stat(&build.output),
                                    custom_components::BundleLook::Absent
                                ) =>
                        {
                            custom_components::BundleLook::Unknown
                        }
                        (look, _) => look,
                    }
                });
            ext.custom_components = components;
            ext.errors.extend(errors);
        }
        for v in m.advisories.clone() {
            match serde_yaml::from_value::<AdvisoryFile>(v).map(|a| Advisory {
                id: a.id,
                on: a.on,
                query: a.query,
                once_per: a.once_per,
                heading: a.heading,
                audience: a.audience,
                needs: a.needs,
            }) {
                Ok(a)
                    if a.needs
                        .iter()
                        .any(|n| oxplow_domain::capability::check_need(n).is_err()) =>
                {
                    for need in &a.needs {
                        if let Err(e) = oxplow_domain::capability::check_need(need) {
                            ext.errors.push(at(
                                &file,
                                line_under(&manifest, "advisories", &a.id),
                                format!("advisory {}: {e}", a.id),
                            ));
                        }
                    }
                }
                Ok(a)
                    if !oxplow_domain::contribution::valid_local(
                        oxplow_domain::contribution::ContributionKind::Advisory,
                        &a.id,
                    ) =>
                {
                    ext.errors.push(at(
                        &file,
                        line_under(&manifest, "advisories", &a.id),
                        format!(
                            "advisory id `{}` must be lowercase letters, digits and dashes",
                            a.id
                        ),
                    ))
                }
                Ok(a) => ext.advisories.push(a),
                Err(e) => ext.errors.push(at(
                    &file,
                    key_line(&manifest, "advisories"),
                    format!("advisory: {e}"),
                )),
            }
        }
        let err_at = |key: &str, e: String| at(&file, key_line(&manifest, key), e);
        match parse_block(m.measures.clone()).and_then(|v| {
            oxplow_config::validate_measures(
                v,
                oxplow_config::CatalogLayer::Extension(&ext.namespace),
            )
            .map_err(manifest_message)
        }) {
            Ok(v) => ext.measures = v,
            Err(e) => ext.errors.push(err_at("measures", e)),
        }
        match parse_block(m.dimensions.clone()).and_then(|v| {
            oxplow_config::validate_dimensions(
                v,
                oxplow_config::CatalogLayer::Extension(&ext.namespace),
            )
            .map_err(manifest_message)
        }) {
            Ok(v) => {
                for d in v {
                    // Promoting rebuilds the metric cube: the project's call,
                    // or oxplow's own (the foundation's dimensions are the
                    // cube's grain from the first boot).
                    if d.promote && !own && origin != "bundled" {
                        ext.errors.push(err_at(
                            "dimensions",
                            format!(
                                "dimension `{}`: `promote` is the project's own extension's (it rebuilds the metric cube)",
                                d.key.clone().unwrap_or_default()
                            ),
                        ));
                    } else {
                        ext.dimensions.push(d);
                    }
                }
            }
            Err(e) => ext.errors.push(err_at("dimensions", e)),
        }
        match parse_block(m.metrics.clone()).and_then(|v| {
            oxplow_config::validate_metrics(
                v,
                oxplow_config::CatalogLayer::Extension(&ext.namespace),
            )
            .map_err(manifest_message)
        }) {
            Ok(v) => {
                for e in v {
                    let unknown_collector = e
                        .collector
                        .as_deref()
                        .filter(|c| !ext.collectors.iter().any(|x| x.id == *c));
                    if let Some(c) = unknown_collector {
                        ext.errors.push(err_at(
                            "metrics",
                            format!(
                                "metric `{}`: collector `{c}` is not a collector this extension declares",
                                e.key.clone().unwrap_or_default()
                            ),
                        ));
                    } else if e.key.is_some() {
                        ext.metrics.push(e);
                    } else {
                        ext.errors.push(err_at(
                            "metrics",
                            format!(
                                "metrics: `use: {}` belongs in .oxplow/project.yaml; an extension defines metrics with `key:`",
                                e.use_key.unwrap_or_default()
                            ),
                        ));
                    }
                }
            }
            Err(e) => ext.errors.push(err_at("metrics", e)),
        }
        match parse_block::<oxplow_db::datasets::DatasetDecl>(m.datasets.clone()) {
            Ok(decls) => {
                let dir = format!("{rel}/datasets");
                let joined = oxplow_db::datasets::join_sources(
                    decls.unwrap_or_default(),
                    &dir,
                    &file,
                    |sql_file| files.read(&format!("datasets/{sql_file}")),
                    || {
                        files
                            .list("datasets")
                            .into_iter()
                            .filter_map(|f| f.strip_suffix(".sql").map(str::to_string))
                            .collect()
                    },
                );
                match joined {
                    Ok(datasets) => ext.datasets = datasets,
                    Err(e) => ext.errors.push(err_at("datasets", e.to_string())),
                }
            }
            Err(e) => ext.errors.push(err_at(
                "datasets",
                oxplow_domain::manifest_keys::hint_unknown(&format!("datasets: {e}")),
            )),
        }
        // `models/` is the old name of `datasets/`: not read, so say so.
        if files.list("models").iter().any(|f| f.ends_with(".sql")) {
            ext.errors.push(err_at(
                "datasets",
                "`models/` is now `datasets/`: move `models/<name>.sql` to `datasets/<name>.sql`"
                    .to_string(),
            ));
        }
        // Cross-references inside the catalog: a metric's source measure
        // and a collector's fact measures are normally ones this extension
        // declares. A measure from another scope
        // (the project's or another extension's `measures:`) resolves
        // when the catalog is assembled, so that is a warning, not an
        // error — but a typo would be silent otherwise.
        // The `oxplow` namespace is shared by the extensions that ship with
        // oxplow, so one of them may name a measure a sibling declares.
        let shares = ext.namespace == oxplow_config::OXPLOW_NAMESPACE;
        let measure_known = |key: &str| {
            ext.measures.iter().any(|mm| mm.key.as_deref() == Some(key))
                || (shares && key.starts_with("oxplow."))
        };
        for spec in &ext.metrics {
            if let Some(sm) = spec.source_measure.as_deref() {
                if !measure_known(sm) {
                    ext.warnings.push(err_at(
                        "metrics",
                        format!(
                            "metric `{}`: measure `{sm}` is not a measure this extension declares; it must come from the project's or another extension's `measures:`",
                            spec.key.clone().unwrap_or_default()
                        ),
                    ));
                }
            }
        }
        for c in &ext.collectors {
            for fact in &c.facts {
                if !measure_known(fact) {
                    ext.warnings.push(err_at(
                        "collectors",
                        format!(
                            "collector `{}`: facts `{fact}`, which is not a measure this extension declares; it must come from the project's or another extension's `measures:`",
                            c.id
                        ),
                    ));
                }
            }
        }
        manifest_v2::mounts_of(&m, &file, &manifest)
    };
    let manifest_v2::Mounts {
        slots: slot_files,
        replacements: replacement_files,
        errors: mount_errors,
    } = slot_files;
    ext.errors.extend(mount_errors);
    // Lenses are pages now: an old `lenses/` folder is named, never read.
    if files
        .list("lenses")
        .iter()
        .any(|f| f.ends_with(".yaml") || f.ends_with(".yml"))
    {
        ext.errors.push(format!(
            "{rel}/lenses/: lenses are pages now — rewrite each as `pages/<slug>.yaml` \
             (metadata, `loaders:` loaders and a `view:`; the oxplow-extension skill's \
             page format) and remove the folder"
        ));
    }

    let mut page_names: Vec<String> = files
        .list("pages")
        .into_iter()
        .filter(|f| f.ends_with(".yaml") || f.ends_with(".yml"))
        .collect();
    page_names.sort();
    for file in page_names {
        let slug = file
            .trim_end_matches(".yaml")
            .trim_end_matches(".yml")
            .to_string();
        let page_rel = format!("{rel}/pages/{file}");
        let text = files.read(&format!("pages/{file}"));
        let parsed = text
            .as_deref()
            .ok_or_else(|| "unreadable".to_string())
            .and_then(|t| {
                crate::pages::schema::parse_page(name, &ext.namespace, &slug, &page_rel, t)
            });
        match parsed {
            Ok(mut page) => {
                page.origin.bundled = origin == "bundled";
                qualify_page_refs(&mut page);
                for param in crate::pages::checks::launcher_unfilled_params(&page) {
                    ext.warnings.push(at(
                        &page_rel,
                        text.as_deref()
                            .and_then(|t| entry_line(t, "params", "name", &param)),
                        format!(
                            "param `{param}` opens empty from the launcher — fix: give it a `default`, \
                             a `filter` or `selection` role, or options from a loader (`options_from`); \
                             or hide the page (`hidden: true`) and open it from a slot that fills it"
                        ),
                    ));
                }
                ext.warnings.extend(
                    crate::pages::checks::layout_warnings(&page)
                        .into_iter()
                        .map(|w| format!("{page_rel}: {w}")),
                );
                ext.page_files.push(page);
            }
            Err(e) => ext.errors.push(format!("{page_rel}: {e}")),
        }
    }
    let slugs: Vec<String> = ext.page_files.iter().map(|p| p.slug.clone()).collect();
    ext.page_files.retain(|page| {
        match page_icon_problem(page).or_else(|| page_ref_problem(page, &slugs)) {
            Some(e) => {
                ext.errors.push(format!("{}: {e}", page.origin.running));
                false
            }
            None => true,
        }
    });

    for (mount_line, s) in slot_files {
        let page = s.page.clone();
        if !ext.page_files.iter().any(|p| p.slug == page) {
            ext.errors.push(at(
                &file,
                mount_line,
                format!(
                    "slot `{}` mounts page `{page}`, which isn't in pages/",
                    s.slot
                ),
            ));
            continue;
        }
        let Some((host, slot_name)) = s.slot.rsplit_once('.') else {
            ext.errors.push(at(
                &file,
                mount_line,
                format!(
                    "slot `{}`: a page mounts at a page's slot, `<extension>/<page>.<slot>`",
                    s.slot
                ),
            ));
            continue;
        };
        if !host.contains('/') {
            ext.errors.push(at(
                &file,
                mount_line,
                format!(
                    "unknown slot `{}`: a page mounts at a page's slot, `<extension>/<page>.<slot>`",
                    s.slot
                ),
            ));
            continue;
        }
        ext.ui.page_slots.push(PageSlot {
            host: host.to_string(),
            slot: slot_name.to_string(),
            page: format!("{name}/{page}"),
            at: s.at,
            title: s.title,
            size: s.size,
        });
    }
    if let Some(v) = decorator_files {
        let (decorators, errors) =
            decorators::parse_decorators(name, &ext.datasets, &v, &file, &manifest);
        ext.ui.decorators = decorators;
        ext.errors.extend(errors);
    }
    if !replacement_files.is_empty() {
        let capabilities: Vec<String> =
            ext.providers.iter().map(|p| p.capability.clone()).collect();
        let (replaced, errors) = replacements::parse_replacements(
            name,
            &capabilities,
            &ext.page_files,
            replacement_files,
            &file,
        );
        ext.ui.replacements = replaced;
        ext.errors.extend(errors);
    }
    // After datasets: a resource is titled by one.
    if let Some(v) = resource_files {
        let (resources, errors) = crate::extension::extension_resources::parse_resources(
            name,
            &ext.namespace,
            &ext.datasets,
            &v,
            &file,
            &manifest,
        );
        let mut errors = errors;
        ext.resources =
            crate::extension::extension_resources::inline_svg_icons(resources, &mut errors, files);
        ext.errors.extend(errors);
    }
    // After resources: a page that renders one is checked against it.
    let mut checked: Vec<crate::pages::schema::Page> = Vec::new();
    for page in std::mem::take(&mut ext.page_files) {
        let checks = renderer_problem(name, &ext.namespace, &ext.resources, &page, &checked)
            .and_then(|()| component_problem(&page, &ext.custom_components));
        match checks {
            Ok(()) => checked.push(page),
            Err(e) => ext.errors.push(format!("{}: {e}", page.origin.running)),
        }
    }
    ext.page_files = checked;
    if let Some(v) = panel_files {
        let (panels, errors) = parse_panels(name, &ext.page_files, v);
        ext.panels = panels;
        let lines = item_lines(&manifest, "panels");
        ext.errors.extend(errors.into_iter().map(|(i, e)| {
            let line = i.and_then(|i| lines.get(i).copied());
            at(&file, line.or(key_line(&manifest, "panels")), e)
        }));
    }
    check_config_reads(&mut ext);
    ext.permissions = m.permissions.clone();
    check_permissions(&mut ext, files, &file, &manifest);
    ext.contributions = descriptors::describe(&ext);
    descriptors::check_ids(&mut ext);
    public_contracts::prepare(&mut ext);
    ext
}

/// What a contribution's declared `needs` ask beyond the baseline: a
/// model (`ai.compute`, `ai.discover`), its own SQL (`sql.read`), the
/// project's files (`tree.read`).
fn needs_permissions(needs: &[String]) -> Permissions {
    let mut p = Permissions::default();
    for need in needs {
        match oxplow_domain::service::need_scope(need) {
            "ai.compute" | "ai.discover" => p.ai = true,
            "sql.read" => p.sql = Some(oxplow_domain::permissions::SqlAccess::Read),
            "tree.read" => p.files = Some(oxplow_domain::permissions::FilesAccess::Read),
            _ => {}
        }
    }
    p
}

/// Each of `ext`'s contributions that needs a permission, with the
/// manifest line it is declared at and what it is.
fn needed_permissions(
    ext: &Extension,
    files: &dyn ExtensionFiles,
    manifest: &str,
) -> Vec<(Option<usize>, String, Permissions)> {
    let mut out = Vec::new();
    let line = |kind: &str, i: usize| item_lines(manifest, kind).get(i).copied();
    for (i, p) in ext.providers.iter().enumerate() {
        let runs = p.entry.is_some()
            || matches!(
                p.adapter.as_ref().map(|a| &a.mcp),
                Some(crate::providers::spec::McpServer::Command { .. })
            );
        let needed = Permissions {
            process: runs,
            network: p.network.clone(),
            secrets: p.credentials.iter().map(|c| c.name.clone()).collect(),
            ..Permissions::default()
        };
        out.push((line("providers", i), format!("provider `{}`", p.id), needed));
    }
    for (i, c) in ext.collectors.iter().enumerate() {
        // Its declared needs and what its kind implies: a Starlark fact
        // collector reads the project's code (`tree.read`).
        let mut needed = needs_permissions(&c.service_needs());
        if c.runtime == oxplow_config::collectors::CollectorRuntime::Exec
            && c.bundled_parser().is_none()
        {
            needed.process = true;
            needed.network = c.network.clone();
            needed.secrets = c.credentials.clone();
        }
        if c.runtime == oxplow_config::collectors::CollectorRuntime::Starlark
            && c.entry
                .as_deref()
                .and_then(|entry| files.read(entry))
                .is_some_and(|script| oxplow_script::ai::calls_ai(&script))
        {
            needed.ai = true;
        }
        out.push((
            line("collectors", i),
            format!("collector `{}`", c.id),
            needed,
        ));
    }
    for (i, e) in ext.effects.iter().enumerate() {
        out.push((
            line("effects", i),
            format!("effect `{}`", e.id),
            needs_permissions(&e.needs),
        ));
    }
    for (i, j) in ext.jobs.iter().enumerate() {
        out.push((
            line("jobs", i),
            format!("job `{}`", j.id),
            needs_permissions(&j.needs),
        ));
    }
    for (i, c) in ext.commands.iter().enumerate() {
        out.push((
            line("commands", i),
            format!("command `{}`", c.name),
            needs_permissions(&c.needs),
        ));
    }
    for (i, imp) in ext.implementations.iter().enumerate() {
        let mut needed = needs_permissions(&imp.needs);
        needed.ai |= imp.capability == "ai_provider";
        needed.agent |= !imp.skills.is_empty();
        out.push((
            line("implementations", i),
            format!("implementation `{}`", imp.id),
            needed,
        ));
    }
    for (i, a) in ext.advisories.iter().enumerate() {
        let needed = Permissions {
            agent: true,
            ..needs_permissions(&a.needs)
        };
        out.push((
            line("advisories", i),
            format!("advisory `{}`", a.id),
            needed,
        ));
    }
    for (i, skill) in ext.skills.iter().enumerate() {
        let needed = Permissions {
            agent: true,
            ..needs_permissions(&skill.needs)
        };
        out.push((line("skills", i), format!("skill `{}`", skill.name), needed));
    }
    for (i, c) in ext.custom_components.iter().enumerate() {
        let needed = Permissions {
            commands: c
                .commands
                .iter()
                .filter(|cmd| !Permissions::default().command_allowed(&ext.namespace, cmd))
                .cloned()
                .collect(),
            sql: c.sql,
            ..Permissions::default()
        };
        out.push((
            line("custom_components", i),
            format!("component `{}`", c.id),
            needed,
        ));
    }
    out
}

/// Each contribution that needs a permission `ext` doesn't declare is an
/// error at its line, naming what to declare: the extension doesn't load
/// until its manifest says what it reaches.
fn check_permissions(ext: &mut Extension, files: &dyn ExtensionFiles, file: &str, manifest: &str) {
    let fallback = key_line(manifest, "permissions").or(Some(1));
    let errors: Vec<String> = needed_permissions(ext, files, manifest)
        .into_iter()
        .filter_map(|(line, what, needed)| {
            let missing = ext.permissions.missing(&needed);
            (!missing.is_empty()).then(|| {
                at(
                    file,
                    line.or(fallback),
                    format!("{what} needs {} in `permissions:`", missing.join(", ")),
                )
            })
        })
        .collect();
    ext.errors.extend(errors);
}

/// The SQL of `ext` that may read its own configuration — a dataset, a page's
/// loader, an advisory — with where each is declared.
fn config_sql(ext: &Extension) -> Vec<(String, &str)> {
    let datasets = ext
        .datasets
        .iter()
        .map(|m| (m.file.clone(), m.sql.as_str()));
    let pages = ext.page_files.iter().flat_map(|p| {
        p.loaders.iter().filter_map(|d| match &d.loader {
            crate::pages::schema::Loader::Sql { sql, .. } => {
                Some((p.origin.running.clone(), sql.as_str()))
            }
            _ => None,
        })
    });
    let advisories = ext
        .advisories
        .iter()
        .map(|a| (format!("advisory `{}`", a.id), a.query.as_str()));
    datasets.chain(pages).chain(advisories).collect()
}

/// A `config('/pointer')` read names a scalar key the extension's own
/// `configuration.schema` declares, or the extension fails to load saying
/// which: an unknown key, a key that is an object or array (no SQL literal), or
/// a read in an extension that declares no configuration.
fn check_config_reads(ext: &mut Extension) {
    let mut problems = Vec::new();
    for (place, sql) in config_sql(ext) {
        let reads = match oxplow_db::datasets::config_reads(sql) {
            Ok(reads) => reads,
            Err(e) => {
                problems.push(format!("{place}: {e}"));
                continue;
            }
        };
        for read in reads {
            let Some(declaration) = &ext.configuration else {
                problems.push(format!(
                    "{place}: config('{}') reads the extension's own configuration, which it \
                     doesn't declare (`configuration:` in extension.yaml)",
                    read.pointer
                ));
                continue;
            };
            match config_key_type(&declaration.schema, &read.pointer) {
                Some("integer" | "number" | "string" | "boolean") => {}
                Some(other) => problems.push(format!(
                    "{place}: config('{}') is a `{other}` in the configuration schema; \
                     SQL reads a number, string or boolean",
                    read.pointer
                )),
                None => problems.push(format!(
                    "{place}: config('{}') names no key of `configuration.schema` (declare it \
                     under `properties` with a scalar `type`)",
                    read.pointer
                )),
            }
        }
    }
    ext.errors.extend(problems);
}

/// The `type` of the schema property at JSON pointer `pointer`, following
/// `properties`; `None` when the schema declares no such property or type.
fn config_key_type<'a>(schema: &'a serde_json::Value, pointer: &str) -> Option<&'a str> {
    let mut node = schema;
    for token in pointer.split('/').skip(1) {
        let token = token.replace("~1", "/").replace("~0", "~");
        node = node.get("properties")?.get(&token)?;
    }
    node.get("type")?.as_str()
}

/// Name each page `page` refers to by slug — a param's `options_from`, a
/// column's `expand`, a form field's `from` — whole, as one of its own
/// extension's: `<extension>/<slug>`, what every reader looks it up by.
pub(crate) fn qualify_page_refs(page: &mut crate::pages::schema::Page) {
    use crate::pages::schema::{Node, NodeKind};
    fn whole(extension: &str, id: &mut String) {
        if !id.contains('/') {
            *id = format!("{extension}/{id}");
        }
    }
    fn params(extension: &str, params: &mut [PageParam]) {
        for p in params {
            if let Some(from) = &mut p.options_from {
                whole(extension, from);
            }
        }
    }
    fn walk(extension: &str, node: &mut Node) {
        match &mut node.kind {
            NodeKind::Stack { children }
            | NodeKind::Grid { children }
            | NodeKind::Switch { children, .. } => {
                for c in children {
                    walk(extension, c);
                }
            }
            NodeKind::Viz { config, .. } => {
                params(extension, &mut config.params);
                for c in &mut config.columns {
                    if let Some(to) = &mut c.expand {
                        whole(extension, to);
                    }
                }
                let fields = config
                    .form
                    .iter_mut()
                    .flat_map(|f| f.fields.iter_mut().flatten());
                for f in fields {
                    if let FormField::Spec(FormFieldSpec {
                        from: Some(from), ..
                    }) = f
                    {
                        whole(extension, from);
                    }
                }
            }
            _ => {}
        }
    }
    let extension = page.extension.clone();
    params(&extension, &mut page.params);
    walk(&extension, &mut page.view);
    if let Some(rail) = &mut page.rail {
        walk(&extension, rail);
    }
}

/// A page `icon` oxplow doesn't draw.
fn page_icon_problem(page: &crate::pages::schema::Page) -> Option<String> {
    let icon = page.icon.as_deref()?;
    (!crate::extension::extension_resources::ICONS.contains(&icon))
        .then(|| format!("`icon: {icon}` isn't one oxplow draws"))
}

/// What `page` names by id that isn't there: a param's `options_from`, a
/// column's `expand` and a form field's `from` name a page (by slug for
/// one of its own extension's); one of this extension's (`slugs`) must be in
/// pages/, and only a form field's may be the page itself.
fn page_ref_problem(page: &crate::pages::schema::Page, slugs: &[String]) -> Option<String> {
    // Each one: what names it, the page it names, and whether that may be
    // the page itself (a form's own loader may list a field's choices).
    let mut refs: Vec<(String, &str, bool)> = page
        .params
        .iter()
        .filter_map(|p| {
            Some((
                format!("param `{}`: `options_from`", p.name),
                p.options_from.as_deref()?,
                false,
            ))
        })
        .collect();
    for n in page.nodes() {
        let crate::pages::schema::NodeKind::Viz { config, .. } = &n.kind else {
            continue;
        };
        refs.extend(config.columns.iter().filter_map(|c| {
            Some((
                format!("node `{}`: column `{}`: `expand`", n.id, c.key),
                c.expand.as_deref()?,
                false,
            ))
        }));
        refs.extend(
            config
                .form
                .iter()
                .flat_map(|f| f.fields.iter().flatten())
                .filter_map(|f| {
                    Some((
                        format!("node `{}`: form field `{}`: `from`", n.id, f.name()),
                        f.from()?,
                        true,
                    ))
                }),
        );
    }
    refs.into_iter().find_map(|(what, target, self_ok)| {
        // Qualified at load (`qualify_page_refs`): `<extension>/<slug>`.
        let (extension, slug) = target.split_once('/')?;
        let slug = slug.split('#').next().unwrap_or(slug);
        if extension == page.extension && slug == page.slug && !self_ok {
            Some(format!("{what} names the page itself"))
        } else if extension == page.extension && !slugs.iter().any(|s| s == slug) {
            Some(format!(
                "{what} page `{target}` isn't in this extension's pages/"
            ))
        } else {
            None
        }
    })
}

/// A page's `component` node or `viz: custom` naming a component its
/// extension doesn't declare (`custom_components:`). One of another
/// extension's (`<extension>/<id>`) is that one's to have.
fn component_problem(
    page: &crate::pages::schema::Page,
    components: &[custom_components::CustomComponent],
) -> Result<(), String> {
    use crate::pages::schema::NodeKind;
    for node in page.nodes() {
        let named = match &node.kind {
            NodeKind::Component { component, .. } => Some(component.clone()),
            NodeKind::Viz { config, .. } if config.viz == Viz::Custom => {
                match config.custom.as_ref().and_then(|c| c.component.clone()) {
                    Some(c) => Some(c),
                    None => {
                        return Err(format!(
                            "node `{}`: viz `custom` needs `custom: {{ component: <id> }}`",
                            node.id
                        ))
                    }
                }
            }
            _ => None,
        };
        if let Some(id) = named.filter(|id| !id.contains('/')) {
            if !components.iter().any(|c| c.id == id) {
                return Err(format!(
                    "node `{}`: `{id}` isn't one of this extension's `custom_components`",
                    node.id
                ));
            }
        }
    }
    Ok(())
}

/// What's wrong with a page's `for`, `accepts`, `default` and `pinned`:
/// each resource it renders is a scheme oxplow renders (`https`), one this
/// extension declares, one of foundation's (`oxplow/*`) or another
/// extension's (`<extension>/<name>`); `accepts` names a param of each
/// (when they're known) with `present`, `absent` or an id pattern; it
/// takes no resource param the resource hasn't (it is run with the URI's
/// params);
/// `default` is for a resource of this extension's, one page per resource;
/// `pinned` and `accepts` are a renderer's.
fn renderer_problem(
    extension: &str,
    namespace: &str,
    resources: &[crate::extension::extension_resources::ExtensionResource],
    p: &crate::pages::schema::Page,
    pages: &[crate::pages::schema::Page],
) -> Result<(), String> {
    let renders = p.renders.clone();
    // What it takes from the URI it is run for: its resource params but
    // the URI itself and the context's.
    let takes: Vec<&str> = p
        .params
        .iter()
        .filter(|q| !q.role.is_some_and(ParamRole::rides_view))
        .map(|q| q.name.as_str())
        .filter(|n| !matches!(*n, "uri" | "stream_id" | "thread_id"))
        .collect();
    let refs = label_refs(p.label.as_deref())?;
    let named: Vec<String> = refs
        .iter()
        .filter(|(from, _)| *from == LabelFrom::Param)
        .map(|(_, n)| n.clone())
        .collect();
    if renders.is_empty() {
        if p.default || p.pinned || !p.accepts.is_empty() || p.accepts_content.is_some() {
            return Err(
                "`default`, `pinned` and `accepts` are for a renderer: give it `for`".into(),
            );
        }
        if let Some(name) = named
            .iter()
            .find(|n| !p.params.iter().any(|q| &&q.name == n))
        {
            return Err(format!(
                "`label` names `{name}`, which isn't one of its `params`"
            ));
        }
        return Ok(());
    }
    let segment = crate::extension::extension_resources::resource_extension(extension, namespace);
    let foundation = oxplow_domain::uri::resources::foundation();
    for r in &renders {
        let scheme = RENDERED_SCHEMES.contains(&r.as_str());
        let own = |name: &str| resources.iter().find(|x| x.decl.name.to_string() == name);
        let decl = if scheme {
            None
        } else {
            let Ok(name) = r.parse::<oxplow_domain::uri::oxplow_uri::ResourceName>() else {
                return Err(format!(
                    "`for: {r}` isn't a resource (`<extension>/<name>`) or a scheme oxplow renders ({})",
                    RENDERED_SCHEMES.join(", ")
                ));
            };
            match own(r) {
                Some(x) => Some(&x.decl),
                None if name.extension() == CORE_NAMESPACE => match foundation.get(&name) {
                    Some(d) => Some(d),
                    None => {
                        return Err(format!(
                            "`for: {r}` isn't a resource oxplow-foundation declares"
                        ))
                    }
                },
                None if name.extension() == segment => {
                    return Err(format!(
                        "`for: {r}` isn't one of this extension's `resources:`"
                    ))
                }
                // Another extension's: checked when the vocabulary is.
                None => None,
            }
        };
        let has = |name: &str| match decl {
            Some(d) => d.additional || d.params.contains_key(name),
            // A web page's viz takes the page as its `uri`.
            None => !scheme || name == "uri",
        };
        if let Some(name) = named.iter().find(|n| !has(n)) {
            return Err(format!(
                "`label` names `{name}`, which isn't a param of `{r}`"
            ));
        }
        if let Some(name) = takes.iter().find(|n| !has(n)) {
            return Err(format!(
                "it takes `{name}`, which isn't a param of `{r}`: a renderer is run with its URI's params"
            ));
        }
        for (key, rule) in &p.accepts {
            if let Some(d) = decl {
                if !d.additional && !d.params.contains_key(key) {
                    return Err(format!("`accepts: {key}` isn't a param of `{r}`"));
                }
            }
            if !matches!(rule.as_str(), "present" | "absent") {
                crate::param_pattern::param_pattern(rule).map_err(|e| {
                    format!("`accepts: {key}: {rule}` is `present`, `absent` or an id pattern: {e}")
                })?;
            }
        }
        if p.default {
            let ours = if scheme {
                namespace == CORE_NAMESPACE
            } else {
                r.split('/').next() == Some(segment.as_str())
            };
            if !ours {
                return Err(format!(
                    "`default` is for a resource this extension declares; `{r}` is another's (a page of yours is one of its renderers to pick)"
                ));
            }
            if pages.iter().any(|q| q.default && q.renders.contains(r)) {
                return Err(format!("`{r}` has another default renderer"));
            }
        }
    }
    Ok(())
}

/// Check the manifest's `panels:` against the extension's viz nodes: a panel
/// id is kebab-case and unique, its viz nodes exist, a badge declares an
/// `alert`, and a `stream` / `thread` scope's viz nodes declare `stream_id` /
/// `thread_id` (what the nav binds). Each error names the entry's index.
fn parse_panels(
    extension: &str,
    pages: &[crate::pages::schema::Page],
    raw: serde_yaml::Value,
) -> (Vec<ExtensionPanel>, Vec<(Option<usize>, String)>) {
    let mut panels: Vec<ExtensionPanel> = Vec::new();
    let mut errors = Vec::new();
    let Some(list) = raw.as_sequence() else {
        return (panels, vec![(None, "`panels` must be a list".into())]);
    };
    for (i, v) in list.iter().enumerate() {
        let p = match serde_yaml::from_value::<PanelFile>(v.clone()) {
            Ok(p) => p,
            Err(e) => {
                errors.push((Some(i), format!("panels: {e} (a panel is `{{ id, title, icon?, scope: project | stream | thread, body, badge?, open?, collapsed?, count?, dock? }}`)")));
                continue;
            }
        };
        let err = |e: String| (Some(i), format!("panel `{}`: {e}", p.id));
        if !oxplow_domain::contribution::valid_local(
            oxplow_domain::contribution::ContributionKind::Panel,
            &p.id,
        ) {
            errors.push(err("a panel id is lowercase letters, digits and `-`".into()));
            continue;
        }
        if panels
            .iter()
            .any(|q| q.id == format!("{extension}/{}", p.id))
        {
            errors.push(err("declared twice".into()));
            continue;
        }
        let needs = match p.scope {
            PanelScope::Project => None,
            PanelScope::Stream => Some("stream_id"),
            PanelScope::Thread => Some("thread_id"),
        };
        let mut ok = true;
        if let Some(open) = &p.open {
            let page = oxplow_domain::uri::build::parse_resource(
                open,
                oxplow_domain::uri::resources::PAGE,
            )
            .is_ok();
            if !page {
                errors.push(err(format!(
                    "`open` must be a page URI (`oxplow:oxplow/page?page=<page>`), not `{open}`"
                )));
                ok = false;
            }
        }
        for (role, slug) in [
            ("body", Some(&p.body)),
            ("badge", p.badge.as_ref()),
            ("collapsed", p.collapsed.as_ref()),
            ("count", p.count.as_ref()),
        ] {
            let Some(slug) = slug else { continue };
            let Some(page) = pages.iter().find(|pg| &pg.slug == slug) else {
                errors.push(err(format!("its {role} page `{slug}` isn't in pages/")));
                ok = false;
                continue;
            };
            if role == "badge" && page.badge.is_none() && page.alert.is_none() {
                errors.push(err(format!(
                    "its badge page `{slug}` declares no `badge` or `alert` (the badge is its count)"
                )));
                ok = false;
            }
            if let Some(param) = needs {
                if !page.params.iter().any(|pp| pp.name == param) {
                    errors.push(err(format!(
                        "a panel scoped to a stream or thread gives its pages `{param}`; page `{slug}` must declare it in `params`"
                    )));
                    ok = false;
                }
            }
        }
        if ok {
            panels.push(ExtensionPanel {
                id: format!("{extension}/{}", p.id),
                extension: extension.to_string(),
                title: p.title,
                icon: p.icon,
                scope: p.scope,
                body: format!("{extension}/{}", p.body),
                badge: p.badge.map(|b| format!("{extension}/{b}")),
                open: p.open,
                collapsed: p.collapsed.map(|c| format!("{extension}/{c}")),
                count: p.count.map(|c| format!("{extension}/{c}")),
                dock: p.dock,
            });
        }
    }
    (panels, errors)
}

/// What's wrong with a form's `fields`, `mode` and their conditions.
fn form_fields_problem(f: &FormSpec) -> Option<String> {
    let Some(fields) = &f.fields else {
        return (f.mode == FormMode::Edit)
            .then(|| "form: `mode: edit` needs `fields`: the ones it edits".to_string());
    };
    let names: Vec<&str> = fields.iter().map(|x| x.name()).collect();
    if let Some(missing) = f
        .again
        .iter()
        .flat_map(|a| &a.keep)
        .find(|k| !names.contains(&k.as_str()))
    {
        return Some(format!(
            "form: `again.keep` names `{missing}`, which isn't one of its `fields`"
        ));
    }
    fields.iter().find_map(|field| {
        let name = field.name();
        if field.secret() && f.fixed.iter().any(|k| k == name) {
            return Some(format!("form: `{name}` can't be both fixed and secret"));
        }
        field.visible_if().keys().find_map(|on| {
            if on == name {
                Some(format!("form: `{name}` can't show by its own value"))
            } else if !names.contains(&on.as_str()) {
                Some(format!(
                    "form: `{name}`'s `visible_if` names `{on}`, which isn't one of its `fields`"
                ))
            } else {
                None
            }
        })
    })
}

/// What's wrong with a lazy tree (`tree.lazy` / `tree.branch`): the param
/// must be the viz's own, with a `default` (the top level) and not one
/// the context supplies; `branch` says which rows open; a column that
/// `select`s the lazy param would re-root the page instead of opening.
fn lazy_tree_problem(cfg: &VizConfig, t: &VizTree) -> Option<String> {
    let Some(lazy) = &t.lazy else {
        return t
            .branch
            .is_some()
            .then(|| "`tree.branch` is for a lazy tree (`tree.lazy: <param>`)".to_string());
    };
    if ViewContext::supplies(lazy) {
        return Some(format!(
            "`tree.lazy` can't be `{lazy}`: the context supplies it"
        ));
    }
    let Some(param) = cfg.params.iter().find(|p| p.name == *lazy) else {
        return Some(format!(
            "`tree.lazy` names param `{lazy}`, which the viz doesn't declare"
        ));
    };
    if param.default.is_none() {
        return Some(format!(
            "`tree.lazy` param `{lazy}` needs a `default`: the top level"
        ));
    }
    if t.branch.is_none() {
        return Some("`tree.lazy` needs `tree.branch: <column>` (the rows that open)".to_string());
    }
    cfg.columns
        .iter()
        .find(|c| c.select.as_deref() == Some(lazy.as_str()))
        .map(|c| {
            format!(
                "column `{}`: `select` names the tree's `lazy` param; a click opens the branch instead",
                c.key
            )
        })
}

pub(crate) fn shape_problem(cfg: &VizConfig, components: &[String]) -> Option<String> {
    // `block` names the viz key the missing columns go under.
    let need = |block: &str, fields: &[(&str, &Option<String>)]| -> Option<String> {
        let missing: Vec<&str> = fields
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(n, _)| *n)
            .collect();
        (!missing.is_empty()).then(|| {
            format!(
                "viz `{}` needs `{block}: {{ {} }}`",
                format!("{:?}", cfg.viz).to_lowercase(),
                missing
                    .iter()
                    .map(|m| format!("{m}: <column>"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
    };
    if let Some(problem) = row_style_problem(cfg)
        .or_else(|| param_problem(&cfg.params))
        .or_else(|| highlight_problem(cfg))
        .or_else(|| markdown_problem(cfg))
    {
        return Some(problem);
    }
    let chart = cfg.chart.clone().unwrap_or_default();
    if chart.targets.is_some() && cfg.viz != Viz::Line {
        return Some("chart: `targets` draws target lines on viz `line` only".to_string());
    }
    match cfg.viz {
        Viz::Bar | Viz::Line => need("chart", &[("x", &chart.x), ("y", &chart.y)]),
        Viz::Treemap => need("chart", &[("label", &chart.label), ("size", &chart.size)]),
        Viz::Tree => {
            let t = cfg.tree.clone().unwrap_or_default();
            need(
                "tree",
                &[("id", &t.id), ("parent", &t.parent), ("label", &t.label)],
            )
            .or_else(|| lazy_tree_problem(cfg, &t))
        }
        Viz::Timeline => {
            let t = cfg.timeline.clone().unwrap_or_default();
            need("timeline", &[("at", &t.at), ("label", &t.label)])
        }
        Viz::Steps => {
            let s = cfg.steps.clone().unwrap_or_default();
            need("steps", &[("label", &s.label)])
        }
        Viz::Code => {
            let c = cfg.code.clone().unwrap_or_default();
            need("code", &[("path", &c.path), ("revision", &c.revision)])
        }
        Viz::Editor => {
            let e = cfg.editor.clone().unwrap_or_default();
            need("editor", &[("path", &e.path)])
        }
        Viz::Terminal => {
            let t = cfg.terminal.clone().unwrap_or_default();
            need("terminal", &[("of", &t.of)])
        }
        Viz::Embed => {
            let e = cfg.embed.clone().unwrap_or_default();
            need("embed", &[("url", &e.url)])
        }
        Viz::Diff => {
            let d = cfg.diff.clone().unwrap_or_default();
            need(
                "diff",
                &[("path", &d.path), ("from", &d.from), ("to", &d.to)],
            )
        }
        Viz::Hunks => {
            let h = cfg.hunks.clone().unwrap_or_default();
            need(
                "hunks",
                &[("path", &h.path), ("from", &h.from), ("to", &h.to)],
            )
        }
        Viz::Form => {
            let f = cfg.form.clone().unwrap_or_default();
            need("form", &[("command", &f.command)])
                .or_else(|| {
                    let command = f.command.as_deref().unwrap_or_default();
                    oxplow_domain::CommandSpec::validate_id(command)
                        .err()
                        .map(|e| format!("form: {e}"))
                })
                .or_else(|| {
                    f.fixed
                        .iter()
                        .find(|k| {
                            f.defaults
                                .as_ref()
                                .and_then(|d| d.get(k.as_str()))
                                .is_none()
                        })
                        .map(|k| format!("form: fixed field `{k}` needs a value in `defaults`"))
                })
                .or_else(|| form_fields_problem(&f))
        }
        Viz::Custom => match cfg.custom.as_ref().and_then(|c| c.component.as_ref()) {
            None => Some("viz `custom` needs `custom: { component: <id> }`".into()),
            Some(c) if !components.contains(c) => Some(format!(
                "viz `custom`: `{c}` isn't one of this extension's `custom_components`"
            )),
            Some(_) => None,
        },
        Viz::Board if cfg.group.is_none() => {
            Some("`board` needs `group: { by: <column> }` (its columns)".to_string())
        }
        Viz::Table
        | Viz::List
        | Viz::Number
        | Viz::Markdown
        | Viz::Detail
        | Viz::Board
        | Viz::Approvals => None,
    }
}

/// A `markdown` block off its viz, or an edit action that doesn't save the
/// text the viz shows: a markdown viz's text, or a row-tile dashboard's
/// text tiles (`tile.text`, edited in place).
fn markdown_problem(cfg: &VizConfig) -> Option<String> {
    let viz = format!("{:?}", cfg.viz).to_lowercase();
    if cfg.markdown.is_some() && cfg.viz != Viz::Markdown {
        return Some(format!("`markdown` is for viz `markdown`, not `{viz}`"));
    }
    let edit = cfg.actions.iter().find(|a| a.edit)?;
    if cfg.viz != Viz::Markdown {
        return Some(format!(
            "an edit action (`{}`) is a `markdown` viz's, not `{viz}`'s",
            edit.id
        ));
    }
    let saved = edit_column(edit)?;
    match cfg.markdown.as_ref().and_then(|m| m.text.as_deref()) {
        None => Some(format!(
            "`{}` saves the text: give the viz `markdown: {{ text: {saved} }}`",
            edit.id
        )),
        Some(shown) if shown != saved => Some(format!(
            "`{}` saves `{saved}`, but the viz shows `{shown}`",
            edit.id
        )),
        Some(_) => None,
    }
}

/// A column that highlights a param the viz doesn't declare.
fn highlight_problem(cfg: &VizConfig) -> Option<String> {
    cfg.columns.iter().find_map(|c| {
        [("highlight", &c.highlight), ("select", &c.select)]
            .into_iter()
            .find_map(|(field, name)| {
                let name = name.as_deref()?;
                (!cfg.params.iter().any(|p| p.name == name)).then(|| {
                    format!(
                        "column `{}`: `{field}: {name}` names a param the viz doesn't declare",
                        c.key
                    )
                })
            })
    })
}

/// Row styling on a viz that doesn't draw rows that way: `group` and
/// `depth` are for `list` / `table`, `emphasis` also for `tree`.
fn row_style_problem(cfg: &VizConfig) -> Option<String> {
    let rows = matches!(cfg.viz, Viz::List | Viz::Table);
    let viz = format!("{:?}", cfg.viz).to_lowercase();
    if cfg.group.is_some() && !rows && cfg.viz != Viz::Board {
        return Some(format!(
            "`group` is only for `list`, `table` and `board`, not `{viz}`"
        ));
    }
    if cfg.depth.is_some() && !rows {
        return Some(format!(
            "`depth` is only for `list` and `table`, not `{viz}`"
        ));
    }
    if cfg.emphasis.is_some() && !rows && cfg.viz != Viz::Tree {
        return Some(format!(
            "`emphasis` is only for `list`, `table` and `tree`, not `{viz}`"
        ));
    }
    None
}

/// Clear what a disabled extension contributes; it stays listed so
/// Settings can turn it back on.
fn apply_disabled(mut ext: Extension, settings: &oxplow_config::ExtensionSettings) -> Extension {
    // A listed-only one runs nothing; one oxplow relies on is never off.
    let runs = !matches!(ext.deployment, Deployment::Undeployed | Deployment::NotHere);
    if !runs || is_off(settings, &ext.name) {
        ext.enabled = false;
        clear_contributions(&mut ext);
    }
    ext
}

pub(crate) fn clear_contributions(ext: &mut Extension) {
    ext.ui = ExtensionUi::default();
    ext.collectors.clear();
    ext.providers.clear();
    ext.custom_components.clear();
    ext.event_types = Default::default();
    ext.resources.clear();
    ext.effects.clear();
    ext.implementations.clear();
    ext.skills.clear();
    ext.subscriptions.clear();
    ext.advisories.clear();
    ext.measures.clear();
    ext.dimensions.clear();
    ext.metrics.clear();
    ext.datasets.clear();
    ext.panels.clear();
    ext.commands.clear();
    ext.jobs.clear();
    ext.ai_profiles.clear();
    ext.contributions.clear();
}

pub(crate) fn disabled_error(name: &str) -> DomainError {
    DomainError::Invalid(format!(
        "extension `{name}` is disabled (`disableExtensions` in .oxplow/personal.yaml, or left off the project's `extensions` list); enable it in Settings → Extensions"
    ))
}

/// The HEAD commit of stream `stream_id`, when it is known.
pub(crate) async fn stream_head(
    layer: &crate::sql_gateway::SqlGateway,
    stream_id: i64,
) -> Result<Option<String>, DomainError> {
    let rows = layer
        .run(
            oxplow_db::SqlQuery::new("SELECT head_sha FROM v_stream WHERE id = :id")
                .named(vec![("id".into(), SqlCell::Int(stream_id))])
                .limit(Some(1)),
        )
        .await?;
    Ok(match rows.rows.first().and_then(|r| r.first()) {
        Some(SqlCell::Text(sha)) => Some(sha.clone()),
        _ => None,
    })
}

/// Load one extension and dry-run every page with default params,
/// reporting query failures and `columns` keys the query doesn't
/// return as errors.
pub async fn validate_extension(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension::extension_catalog::ExtensionCatalog,
    root: &Path,
    name: &str,
    commands: Option<CommandSchemas<'_>>,
) -> Result<Extension, DomainError> {
    // A check reads the source whether or not the extension is on: it is
    // how an extension is made ready to deploy.
    let settings = oxplow_config::extension_settings(catalog.project_of(root));
    let mut declared = load_sources(root, &settings);
    let index = declared
        .iter()
        .position(|ext| ext.name == name)
        .ok_or(DomainError::NotFound)?;
    let mut ext = declared.swap_remove(index);
    let path = ext.path.clone();
    let read = |rel: &str| files_at(root, &path).ok().and_then(|f| f.read(rel));
    prepare(layer, catalog, root, &mut ext, &read, commands).await;
    if let Ok(files) = files_at(root, &path) {
        check_fixture_pairing(&mut ext, files.as_ref());
        check_credentials(&mut ext, files.as_ref());
        check_dependencies(&mut ext, &declared, files.as_ref());
    }
    Ok(ext)
}

/// Another extension `ext` uses but doesn't declare — reads its dataset
/// (`v_<namespace>_…` in a page or a dataset), mounts at or names its page
/// (`<name>/…`), runs its command (`<namespace>.…`) — is an error at the
/// first place it does: a test runs `ext` with only what it declares
/// (`requires:`, or the extension an import names), and a person may turn
/// the other off. The required foundation, and a command in a namespace it
/// shares, need no declaration.
fn check_dependencies(ext: &mut Extension, others: &[Extension], files: &dyn ExtensionFiles) {
    let declared: std::collections::BTreeSet<&str> = ext
        .requires
        .iter()
        .map(String::as_str)
        .chain(
            ext.public_contracts
                .imports
                .iter()
                .map(|i| i.contribution.extension.as_str()),
        )
        .collect();
    let base = ext.path.trim_end_matches('/').to_string();
    // A command namespace the required foundation, or `ext` itself, holds
    // too is no one other extension's to declare.
    let shared: Vec<String> = others
        .iter()
        .filter(|o| crate::bundled_extensions::is_required(&o.name))
        .map(|o| o.namespace.clone())
        .chain([ext.namespace.clone()])
        .collect();
    let others: Vec<&Extension> = others
        .iter()
        .filter(|o| {
            o.name != ext.name
                && !crate::bundled_extensions::is_required(&o.name)
                && !declared.contains(o.name.as_str())
        })
        .collect();
    let mut found: Vec<(String, String, String)> = Vec::new();
    let mut note = |owner: &str, what: String, at: String| {
        if !found.iter().any(|(o, _, _)| o == owner) {
            found.push((owner.to_string(), what, at));
        }
    };
    // A slot it mounts at.
    for mount in &ext.ui.page_slots {
        if let Some(owner) = others
            .iter()
            .find(|o| mount.host.split('/').next() == Some(o.name.as_str()))
        {
            note(
                &owner.name,
                format!("its slot `{}.{}`", mount.host, mount.slot),
                format!("{base}/extension.yaml:1"),
            );
        }
    }
    // What its pages, datasets and scripts name.
    let mut paths = files.paths().unwrap_or_default();
    paths.sort();
    for rel in paths.iter().filter(|p| {
        (p.starts_with("pages/") && p.ends_with(".yaml"))
            || (p.starts_with("datasets/") && p.ends_with(".sql"))
            || p.ends_with(".star")
    }) {
        let Some(text) = files.read(rel) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            let at = format!("{base}/{rel}:{}", n + 1);
            for owner in &others {
                let dataset = format!("v_{}_", owner.namespace);
                let page = format!("{}/", owner.name);
                let command = format!("{}.", owner.namespace);
                let word = |token: &str| {
                    line.match_indices(token).find_map(|(i, _)| {
                        let before = line[..i].chars().next_back();
                        let starts =
                            before.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '-'));
                        starts.then(|| {
                            line[i..]
                                .split(|c: char| !(c.is_alphanumeric() || "_-./".contains(c)))
                                .next()
                                .unwrap_or_default()
                                .trim_end_matches('.')
                                .to_string()
                        })
                    })
                };
                // Datasets are named by namespace, so extensions sharing one
                // (oxplow's own shipped ones) can't be told apart by a name.
                let by_dataset = (owner.namespace != ext.namespace)
                    .then(|| word(&dataset))
                    .flatten();
                if let Some(name) = by_dataset {
                    note(&owner.name, format!("its dataset `{name}`"), at.clone());
                } else if let Some(name) = word(&page) {
                    note(&owner.name, format!("its page `{name}`"), at.clone());
                } else if !shared.contains(&owner.namespace) {
                    if let Some(name) = word(&command) {
                        note(&owner.name, format!("its command `{name}`"), at.clone());
                    }
                }
            }
        }
    }
    for (owner, what, at) in found {
        ext.errors.push(format!(
            "{at}: uses `{owner}` ({what}) without declaring it — a test runs `{}` without \
             `{owner}`, and a person may turn it off — fix: `requires: [{owner}]` in \
             extension.yaml (or import what it reads)",
            ext.name
        ));
    }
}

/// An extension's manifest, scripts and fixtures, read for what looks like
/// a credential (`credential_lint`): warnings, never the value — a source
/// is one `share` away from a commit.
fn check_credentials(ext: &mut Extension, files: &dyn ExtensionFiles) {
    let Ok(mut paths) = files.paths() else { return };
    paths.sort_unstable();
    let base = ext.path.trim_end_matches('/').to_string();
    for rel in paths {
        let scanned =
            rel == "extension.yaml" || rel.ends_with(".star") || rel.starts_with("fixtures/");
        if !scanned {
            continue;
        }
        if let Some(text) = files.read(&rel) {
            ext.warnings.extend(crate::credential_lint::warnings(
                &format!("{base}/{rel}"),
                &text,
            ));
        }
    }
}

/// A fixture runs only when an intent example names it, so a
/// `fixtures/<name>.yaml` no example lists silently runs nothing (the
/// reverse, an example with no fixture, is `extension test`'s warning: a
/// fresh scaffold has such examples). `dataset-*` and `provider-*` fixtures
/// belong to a dataset or a provider, not to an example.
fn check_fixture_pairing(ext: &mut Extension, files: &dyn ExtensionFiles) {
    let Some(intent) = &ext.intent else { return };
    let fixtures = files.list("fixtures");
    let mut orphans: Vec<&str> = fixtures
        .iter()
        .filter_map(|f| f.strip_suffix(".yaml"))
        .filter(|n| !n.starts_with("dataset-") && !n.starts_with("provider-"))
        .filter(|n| !intent.examples.iter().any(|e| e.name == *n))
        .collect();
    orphans.sort_unstable();
    for name in orphans {
        ext.warnings.push(format!(
            "{}/fixtures/{name}.yaml:1: no intent example names this fixture, so it never runs — \
             fix: add `{name}` to intent.examples",
            ext.path.trim_end_matches('/'),
        ));
    }
}

/// What a check asks the running oxplow's command registry (the
/// `CommandBus`): a command's input schema by name — what an example's or
/// a component's command is checked against — and who holds a
/// command namespace. A plain schema closure is one with no namespaces.
/// `None` where there's no running app to ask (the CLI).
pub trait RunningCommands: Sync {
    fn input_schema(&self, name: &str) -> Option<serde_json::Value>;
    /// Who holds `namespace` (`oxplow`, `oxplow:oxplow/extension?name=<name>`,
    /// `provider:<instance>`), `None` when it's free.
    fn namespace_owner(&self, _namespace: &str) -> Option<String> {
        None
    }
}

impl<F: Fn(&str) -> Option<serde_json::Value> + Sync> RunningCommands for F {
    fn input_schema(&self, name: &str) -> Option<serde_json::Value> {
        self(name)
    }
}

/// The registry a check is given.
pub type CommandSchemas<'a> = &'a dyn RunningCommands;

/// A custom component's declared commands must exist; a `custom` viz
/// that also fills a kit role block gets a nudge — the kit may already
/// render it (the honest extent of a "this reimplements the kit" lint).
fn check_components(
    ext: &mut Extension,
    read: &dyn Fn(&str) -> Option<String>,
    commands: Option<CommandSchemas<'_>>,
) {
    let pages = custom_components::bundle_problems(ext, read);
    ext.errors.extend(pages);
    let warnings = custom_components::bundle_warnings(ext, read);
    ext.warnings.extend(warnings);
    if let Some(schema_of) = commands {
        let missing: Vec<(String, String)> = ext
            .custom_components
            .iter()
            .flat_map(|c| {
                c.commands
                    .iter()
                    .filter(|n| schema_of.input_schema(n).is_none())
                    .map(|n| (c.id.clone(), n.clone()))
            })
            .collect();
        for (component, command) in missing {
            ext.errors.push(format!(
                "{}/extension.yaml: custom component `{component}` declares command `{command}`, \
                 which isn't registered",
                ext.path
            ));
        }
    }
    let lookalikes: Vec<(String, &'static str)> = ext
        .page_files
        .iter()
        .flat_map(|p| p.nodes().into_iter().map(move |n| (p, n)))
        .filter_map(|(p, n)| match &n.kind {
            crate::pages::schema::NodeKind::Viz { config: cfg, .. } if cfg.viz == Viz::Custom => {
                Some((p, cfg))
            }
            _ => None,
        })
        .flat_map(|(p, l)| {
            [
                ("chart", l.chart.is_some()),
                ("tree", l.tree.is_some()),
                ("timeline", l.timeline.is_some()),
                ("steps", l.steps.is_some()),
                ("hunks", l.hunks.is_some()),
                ("code", l.code.is_some()),
                ("diff", l.diff.is_some()),
                ("editor", l.editor.is_some()),
                ("embed", l.embed.is_some()),
                ("terminal", l.terminal.is_some()),
            ]
            .into_iter()
            .filter(|(_, set)| *set)
            .map(|(block, _)| (p.origin.running.clone(), block))
        })
        .collect();
    for (path, block) in lookalikes {
        ext.warnings.push(format!(
            "{path}: a custom viz that fills `{block}`: the kit's `{block}` viz may already cover \
             it — prefer the kit where it does"
        ));
    }
}

/// The empty stand-ins a check gives `ext`'s declared entities (P7.C6).
fn entity_stubs(ext: &Extension) -> Vec<oxplow_db::datasets::EntityStub> {
    ext.collectors
        .iter()
        .flat_map(|c| &c.entities)
        .map(|e| oxplow_db::datasets::EntityStub {
            owner: ext.name.clone(),
            name: e.name.clone(),
            view: e.view.clone(),
            columns: e
                .columns
                .iter()
                .map(|c| (c.name.clone(), crate::collector_runner::stored(c.col_type)))
                .collect(),
        })
        .collect()
}

/// Dry-run a loaded extension's datasets, commands, advisories and viz nodes,
/// appending what's wrong to its `errors`. `root` names sources for an
/// unsynced view. What it declares but hasn't published — its collectors'
/// entities (empty), its datasets, the other enabled extensions' — is an
/// overlay of temp views every query reads through (P7.C6), so a fresh
/// extension checks clean before its first sync.
/// An extension version checked against a layer (P8.C2): its viz nodes, run
/// through its own datasets, and that overlay — the temp views its datasets
/// (and the other enabled extensions') compile to, published nowhere —
/// for whatever else reads this version (a review's row counts).
pub struct Prepared {
    pub pages: crate::pages::checks::PageChecks,
    pub overlay: Vec<oxplow_db::TempView>,
}

/// The temp views `ext`'s datasets compile to — beside the other enabled
/// extensions' — and what's wrong with them.
async fn dataset_overlay(
    layer: &crate::sql_gateway::SqlGateway,
    candidate: &candidates::CandidateCatalog,
) -> (Vec<oxplow_db::TempView>, Vec<String>) {
    let datasets = candidate
        .extensions
        .iter()
        .filter(|ext| ext.enabled && !ext.datasets.is_empty())
        .map(|ext| oxplow_db::datasets::ExtensionDatasets {
            extension: ext.name.clone(),
            sources: ext.datasets.clone(),
            config: ext.default_configuration(),
        })
        .collect();
    let stubs = candidate
        .extensions
        .iter()
        .filter(|ext| ext.enabled)
        .flat_map(entity_stubs)
        .collect();
    match layer.check_extension_datasets(datasets, stubs).await {
        Ok(checked) => {
            let target = &candidate.candidate().name;
            let errors = checked
                .errors
                .into_iter()
                .filter(|(owner, _)| candidate.dependents.contains(owner))
                .flat_map(|(owner, errors)| {
                    errors.into_iter().map(move |error| {
                        if &owner == target {
                            error
                        } else {
                            format!("consumer `{owner}`: {error}")
                        }
                    })
                })
                .collect();
            (checked.views, errors)
        }
        Err(e) => (Vec::new(), vec![format!("datasets: {e}")]),
    }
}

/// The temp views `draft`'s datasets (and the other enabled extensions')
/// compile to, read from `root`: what a draft page's queries read through,
/// published nowhere.
pub(crate) async fn draft_overlay(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension::extension_catalog::ExtensionCatalog,
    root: &Path,
    draft: Extension,
) -> Vec<oxplow_db::TempView> {
    let candidate = catalog.candidate(root, draft);
    dataset_overlay(layer, &candidate).await.0
}

async fn read_side(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension::extension_catalog::ExtensionCatalog,
    root: &Path,
    ext: &mut Extension,
) -> Prepared {
    let candidate = catalog.candidate(root, ext.clone());
    *ext = candidate.candidate().clone();
    let (overlay, _) = dataset_overlay(layer, &candidate).await;
    let pages = crate::pages::checks::dry_run_all(&layer.with_overlay(overlay.clone()), ext).await;
    Prepared { pages, overlay }
}

/// Check `ext` — its commands, components, datasets, advisories and viz nodes —
/// reading through its own datasets' overlay, and report what's wrong in
/// `ext.errors`. `read` gives the files of the version under check (a
/// component's page is linted as that version has it). Each side of a review prepares its own, so a viz renders
/// against its version's datasets, whichever are published.
async fn prepare(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension::extension_catalog::ExtensionCatalog,
    root: &Path,
    ext: &mut Extension,
    read: &(dyn Fn(&str) -> Option<String> + Sync),
    commands: Option<CommandSchemas<'_>>,
) -> Prepared {
    let candidate = catalog.candidate(root, ext.clone());
    *ext = candidate.candidate().clone();
    check_components(ext, read, commands);
    let (overlay, errors) = dataset_overlay(layer, &candidate).await;
    ext.errors.extend(errors);
    let layer = &layer.with_overlay(overlay.clone());
    crate::extension::extension_commands::check_extension_commands(layer, ext, commands).await;
    for a in ext.advisories.clone() {
        // A check reads no stored settings: the declared defaults.
        let query = match oxplow_db::datasets::substitute_config(
            &a.query,
            ext.default_configuration().as_ref(),
        ) {
            Ok(query) => query,
            Err(e) => {
                ext.errors.push(format!(
                    "advisory {}: {}",
                    a.id,
                    e.to_string().replacen("invalid value: ", "", 1)
                ));
                continue;
            }
        };
        let run = layer
            .run(
                oxplow_db::SqlQuery::new(&query)
                    .named(
                        crate::advisories::PARAMS
                            .iter()
                            .map(|p| (p.to_string(), SqlCell::Null(())))
                            .collect(),
                    )
                    .limit(None),
            )
            .await;
        match run {
            Err(e) => ext.errors.push(format!(
                "advisory {}: {}",
                a.id,
                e.to_string().replacen("invalid value: ", "", 1)
            )),
            Ok(r) => {
                let mut need = vec!["message"];
                if a.once_per == AdvisoryOncePer::Row {
                    need.push("key");
                }
                for col in need {
                    if !r.columns.iter().any(|c| c == col) {
                        ext.errors.push(format!(
                            "advisory {}: the query must return a `{col}` column (columns: {})",
                            a.id,
                            r.columns.join(", ")
                        ));
                    }
                }
            }
        }
    }
    check_collector_inputs(layer, ext, read).await;
    let pages = crate::pages::checks::dry_run_all(layer, ext).await;
    for page in ext.page_files.clone() {
        let Some(loaders) = pages.get(&page.slug) else {
            continue;
        };
        for (name, result) in loaders {
            if let Err(e) = result {
                ext.errors
                    .push(format!("{}: loader `{name}`: {e}", page.origin.running));
            }
        }
        for (node, viz, result) in crate::pages::checks::viz_nodes(&page, loaders) {
            let Ok(result) = result else { continue };
            for problem in viz_column_problems(viz, &result.columns) {
                ext.errors
                    .push(format!("{}: node `{node}`: {problem}", page.origin.running));
            }
        }
    }
    Prepared { pages, overlay }
}

/// A collector's `input` query reads published views only: a physical table
/// is no part of the contract an extension reads through, and the read refuses
/// it. Named at its collector's line in `extension.yaml`.
async fn check_collector_inputs(
    layer: &crate::sql_gateway::SqlGateway,
    ext: &mut Extension,
    read: &(dyn Fn(&str) -> Option<String> + Sync),
) {
    let file = format!("{}/extension.yaml", ext.path);
    let manifest = read("extension.yaml").unwrap_or_default();
    for c in ext.collectors.clone() {
        let Some(input) = &c.input else { continue };
        let line = manifest_v2::entry_line(&manifest, "collectors", "id", &c.id);
        let problem = match oxplow_db::datasets::substitute_config(
            &input.sql,
            ext.default_configuration().as_ref(),
        ) {
            Err(e) => Some(e.to_string().replacen("invalid value: ", "", 1)),
            Ok(sql) => match layer.check(&sql).await {
                Err(e) => Some(e.to_string().replacen("invalid value: ", "", 1)),
                Ok(reads) => {
                    let tables: Vec<&String> = reads
                        .tables
                        .iter()
                        .filter(|t| !t.starts_with("temp."))
                        .collect();
                    (!tables.is_empty()).then(|| {
                        format!(
                            "reads {tables:?}, which {} not published datasets — fix: read the `v_` views (`oxplow_catalog` lists them)",
                            if tables.len() == 1 { "is" } else { "are" }
                        )
                    })
                }
            },
        };
        if let Some(problem) = problem {
            ext.errors.push(manifest_v2::at(
                &file,
                line,
                format!("collector `{}` input: {problem}", c.id),
            ));
        }
    }
}

/// What a viz names that its loader's columns lack: a role block's
/// column, an action's `when` or row placeholder, a column's key, unit,
/// icon, tone or link column.
fn viz_column_problems(cfg: &VizConfig, cols: &[String]) -> Vec<String> {
    let mut problems: Vec<String> = Vec::new();
    for (block, k) in cfg.role_columns() {
        if !cols.contains(k) {
            problems.push(format!(
                "{block} column `{k}` isn't in the query result (columns: {})",
                cols.join(", ")
            ));
        }
    }
    for a in &cfg.actions {
        if let Some(when) = a.when.as_ref().filter(|w| !cols.contains(w)) {
            problems.push(format!(
                "action `{}` is offered `when: {when}`, which isn't in the \
                     query result (columns: {})",
                a.id,
                cols.join(", ")
            ));
        }
        for (scope, name) in action_templates(&a.input) {
            // A drop's neighbours and its order are rows of the
            // same result.
            let a_row = matches!(
                scope.as_str(),
                "row" | "before" | "after" | "onto" | "order"
            );
            if a_row && !cols.contains(&name) {
                problems.push(format!(
                    "action `{}` uses `{{{{{scope}.{name}}}}}`, which isn't in \
                         the query result (columns: {})",
                    a.id,
                    cols.join(", ")
                ));
            }
        }
    }
    for c in &cfg.columns {
        let mut keys = vec![&c.key];
        keys.extend(c.unit.iter());
        keys.extend(c.icon.iter());
        keys.extend(c.tone.iter());
        if let Some(link) = c.link.as_ref() {
            keys.extend(link.from.iter());
            keys.extend(link.line.iter());
            keys.extend(link.base.iter());
            keys.extend(link.head.iter());
        }
        for k in keys {
            if !cols.contains(k) {
                problems.push(format!(
                    "column `{k}` isn't in the query result (columns: {})",
                    cols.join(", ")
                ));
            }
        }
    }
    problems
}

/// A lowercase letter, then lowercase letters, digits and single dashes —
/// safe as a folder name, a page-id prefix and (dashes as underscores) a
/// command namespace.
pub fn is_valid_name(name: &str) -> bool {
    oxplow_domain::contribution::valid_extension_name(name)
}

/// A viz slug from a title: lowercase letters and digits, runs of
/// anything else one dash.
pub fn slug_of(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "viz".into()
    } else {
        out
    }
}

/// The source folder of extension `name` in the checkout at `root` (where
/// `project`'s settings say it is), if oxplow may write viz files into it:
/// a valid name, not a bundled extension. The one check every writer of an
/// extension's files (a page save, `extension.deploy`) runs first, so a
/// path or a reserved name never reaches the filesystem.
pub fn writable_extension_dir(
    root: &Path,
    project: &Path,
    name: &str,
) -> Result<PathBuf, DomainError> {
    if !is_valid_name(name) {
        return Err(DomainError::Invalid(format!(
            "extension name `{name}` must start with a letter and be lowercase letters, digits and single dashes"
        )));
    }
    if crate::bundled_extensions::is_reserved(name) {
        return Err(DomainError::Invalid(format!(
            "`{name}` is a bundled extension (read-only); save to another extension"
        )));
    }
    Ok(oxplow_config::extension_settings(project).source(root, name))
}

/// What a scaffolded `extension.yaml` says. One template for
/// `oxplow extension new` and `extension.deploy`, so every new extension starts
/// with an intent and passes `check`.
pub struct ManifestScaffold<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub purpose: &'a str,
    /// The thread/effort ref that asked for it, when known.
    pub origin: Option<&'a str>,
    pub example_name: &'a str,
    /// YAML flow text for the example's input (`{ page: demo }`).
    pub example_input: &'a str,
    pub example_expect: &'a str,
}

/// A v2 manifest with an `intent` and one example, naming its schema on the
/// first line.
pub fn scaffold_manifest(m: &ManifestScaffold<'_>) -> String {
    let quote = |s: &str| {
        serde_yaml::to_string(s)
            .expect("string serializes")
            .trim_end()
            .to_string()
    };
    format!(
        "{modeline}manifest: 2\nname: {name}\ndescription: {description}\nintent:\n  purpose: {purpose}\n  origin: {origin}\n  examples:\n    - name: {example_name}\n      input: {input}\n      expect: {expect}\n",
        modeline = schema::modeline(schema::MANIFEST_SCHEMA_URL),
        name = m.name,
        description = quote(m.description),
        purpose = quote(m.purpose),
        origin = m.origin.unwrap_or("null"),
        example_name = m.example_name,
        input = m.example_input,
        expect = quote(m.example_expect),
    )
}

#[cfg(test)]
mod tests {

    /// A malformed list entry is reported on its own line — the shape
    /// error too, which has no name to find it by — in every list block.
    #[test]
    fn a_broken_list_entry_is_an_error_at_its_own_line() {
        for (block, says) in [
            ("commands:\n  - name: a.b\n    summary: S.\n    entry: h.star\n  - name: c.d\n    bogus: 1\n", "command"),
            ("effects:\n  - id: a\n    summary: S.\n    on: [work_item.state_changed]\n    entry: e.star\n  - id: b\n    bogus: 1\n", "effect"),
            ("collectors:\n  - id: a\n    runtime: starlark\n    entry: c.star\n  - id: b\n    bogus: 1\n", "collector"),
            ("skills:\n  - name: a\n    file: a.md\n  - name: b\n    bogus: 1\n", "skill"),
            ("implementations:\n  - id: a\n    capability: work_items\n  - id: b\n    bogus: 1\n", "implementation"),
        ] {
            let d = tempfile::tempdir().unwrap();
            let manifest = format!("manifest: 2\nname: x\nintent:\n  purpose: p\n{block}");
            let second = manifest
                .lines()
                .position(|l| l.contains("bogus"))
                .expect("a broken entry")
                // Its `- ` line is the one before.
                ;
            std::fs::create_dir_all(d.path().join(".oxplow/extensions/x")).unwrap();
            std::fs::write(d.path().join(".oxplow/extensions/x/extension.yaml"), &manifest).unwrap();
            let ext = load_extensions(d.path())
                .into_iter()
                .find(|e| e.name == "x")
                .unwrap();
            let errors = ext.errors.join("\n");
            assert!(
                errors.contains(&format!("extension.yaml:{second}:")) && errors.contains("bogus"),
                "{says}: want line {second}\n{errors}"
            );
        }
    }

    /// A broken entry inside the manifest's own nested lists — a slot, an
    /// intent example — is that entry's error: the rest of the extension
    /// still loads.
    #[test]
    fn a_broken_nested_entry_doesnt_sink_the_manifest() {
        let d = tempfile::tempdir().unwrap();
        let manifest = "manifest: 2\nname: x\nintent:\n  purpose: p\n  examples:\n    - { name: a }\n    - { name: b, bogus: 1 }\nui:\n  mounts:\n    - { page: l, at: diff.file.header, bogus: 1 }\nskills:\n  - name: s\n    file: s.md\n";
        std::fs::create_dir_all(d.path().join(".oxplow/extensions/x")).unwrap();
        std::fs::write(
            d.path().join(".oxplow/extensions/x/extension.yaml"),
            manifest,
        )
        .unwrap();
        std::fs::write(
            d.path().join(".oxplow/extensions/x/s.md"),
            "---\nname: s\ndescription: S.\n---\n# s\n",
        )
        .unwrap();
        let ext = load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap();
        let errors = ext.errors.join("\n");
        assert!(
            errors.contains("extension.yaml:7:") && errors.contains("bogus"),
            "{errors}"
        );
        assert!(errors.contains("extension.yaml:10:"), "{errors}");
        assert_eq!(ext.skills.len(), 1, "{errors}");
        assert_eq!(
            ext.intent.as_ref().map(|i| i.examples.len()),
            Some(1),
            "the good example stays"
        );
    }

    /// The user guide's hand-written extension loads as written, page and
    /// all: following it never makes a dead extension.
    #[test]
    fn the_guides_hand_written_extension_loads() {
        let guide = include_str!("../../../../docs/guide/pages.md");
        let section = &guide[guide.find("## Writing one by hand").expect("the section")..];
        let blocks: Vec<&str> = section
            .split("```yaml\n")
            .skip(1)
            .take(2)
            .map(|b| b.split("```").next().unwrap())
            .collect();
        let d = tempfile::tempdir().unwrap();
        for block in blocks {
            let path = block
                .lines()
                .next()
                .unwrap()
                .trim_start_matches("# ")
                .trim();
            let file = d.path().join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, block).unwrap();
        }
        let ext = load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "review")
            .expect("it loads");
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert!(ext.page_files.iter().any(|p| p.slug == "blocked"));
    }

    /// An action's placeholders are read by the domain's one
    /// tokenizer (`oxplow_domain::template`), a stray opener included.
    #[test]
    fn action_templates_read_the_one_tokenizer() {
        assert_eq!(
            action_templates(&serde_json::json!({ "t": ["{{param.x}}", "{{{{row.a}}"] })),
            vec![
                ("param".to_string(), "x".to_string()),
                ("{{row".to_string(), "a".to_string())
            ]
        );
    }
    use super::*;
    use oxplow_db::Database;
    use std::fs;

    fn cat() -> crate::extension::extension_catalog::ExtensionCatalog {
        crate::extension::extension_catalog::ExtensionCatalog::new()
    }

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    /// A shipped oxplow runs the copy of its own extensions built into it:
    /// a project folder with a shipped name loads nothing in its place —
    /// not as a source, not for a preview — and says it is ignored. Each
    /// name is one entry, so a lookup by name needs no tie-break.
    #[test]
    fn a_project_folder_never_stands_in_for_a_shipped_extension() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/oxplow-bundled/extension.yaml",
            "manifest: 2\nname: oxplow-bundled\nintent:\n  purpose: p\n",
        );
        let all = load_extensions(dir.path());
        let named: Vec<&Extension> = all.iter().filter(|e| e.name == "oxplow-bundled").collect();
        assert_eq!(named.len(), 1, "{named:#?}");
        let ext = named[0];
        assert!(
            matches!(ext.deployment, Deployment::Shipped { .. }),
            "{:?}",
            ext.deployment
        );
        assert!(
            ext.warnings
                .iter()
                .any(|w| w.contains(".oxplow/extensions/oxplow-bundled")
                    && w.contains("ships with oxplow")),
            "{:?}",
            ext.warnings
        );
        let preview = source_or_shipped(dir.path(), dir.path(), "oxplow-bundled");
        assert!(!preview.starts_with(".oxplow/extensions"), "{preview}");
        assert_eq!(
            owner(&all, "oxplow-bundled").map(|e| &e.deployment),
            Some(&ext.deployment)
        );
    }

    /// `when` names a column a row action is offered on; it means nothing
    /// without `row: true`.
    #[test]
    fn when_needs_a_row_action() {
        let parse = |action: &str| {
            parse_actions(
                vec![serde_yaml::from_str(action).unwrap()],
                &[],
                false,
                false,
            )
        };
        assert!(parse("{ id: a, label: A, command: x.y.z, row: true, when: open }").is_ok());
        let err = parse("{ id: a, label: A, command: x.y.z, when: open }").unwrap_err();
        assert!(err.contains("`when` is for a row action"), "{err}");
    }

    /// A viz that holds one row (detail, form, markdown) — and a page's
    /// own actions, over its first loader's row — lets an action that isn't
    /// a row action name `when` and `{{row.*}}`: that one row. Derived at
    /// load as `one_row`, not declared; any other viz refuses both.
    #[test]
    fn an_action_over_one_row_may_name_when_and_the_row() {
        let parse = |action: &str, holds_one_row: bool| {
            parse_actions(
                vec![serde_yaml::from_str(action).unwrap()],
                &[],
                false,
                holds_one_row,
            )
        };
        let named =
            "{ id: a, label: A, command: x.y.z, when: open, input: { t: '{{row.title}}' } }";
        let ok = parse(named, true).unwrap();
        assert!(ok[0].one_row && !ok[0].row);
        let plain = parse("{ id: a, label: A, command: x.y.z }", true).unwrap();
        assert!(!plain[0].one_row, "naming neither is an ordinary button");
        let only_when = parse("{ id: a, label: A, command: x.y.z, when: open }", true).unwrap();
        assert!(only_when[0].one_row);
        let err = parse(named, false).unwrap_err();
        assert!(err.contains("holds one row"), "{err}");
        // A row action stays what it was, on any viz.
        let row = parse(
            "{ id: a, label: A, command: x.y.z, row: true, when: open }",
            false,
        )
        .unwrap();
        assert!(row[0].row && !row[0].one_row);
    }

    /// `primary` marks a row action a click on the row (and Enter) runs: it
    /// is a row action.
    #[test]
    fn primary_is_the_one_row_action_a_click_runs() {
        let parse = |actions: &[&str]| {
            parse_actions(
                actions
                    .iter()
                    .map(|a| serde_yaml::from_str(a).unwrap())
                    .collect(),
                &[],
                false,
                false,
            )
        };
        let ok = parse(&["{ id: a, label: A, command: x.y.z, row: true, primary: true }"]).unwrap();
        assert!(ok[0].primary);
        let err = parse(&["{ id: a, label: A, command: x.y.z, primary: true }"]).unwrap_err();
        assert!(err.contains("`primary` is for a row action"), "{err}");
    }

    /// `prominent` keeps a page action a button in the page's nav bar
    /// instead of its ☰ menu; several may be. A row, group or edit action
    /// isn't in the nav bar, so it can't be.
    #[test]
    fn prominent_keeps_page_actions_in_the_bar() {
        // Over a grouped viz, so a group action is declared as it may be.
        let parse = |actions: &[&str]| {
            parse_actions(
                actions
                    .iter()
                    .map(|a| serde_yaml::from_str(a).unwrap())
                    .collect(),
                &[],
                true,
                false,
            )
        };
        let ok = parse(&[
            "{ id: a, label: A, command: x.y.z, prominent: true }",
            "{ id: b, label: B, command: x.y.z, prominent: true }",
            "{ id: c, label: C, command: x.y.z }",
        ])
        .unwrap();
        assert_eq!(
            ok.iter().map(|a| a.prominent).collect::<Vec<_>>(),
            [true, true, false]
        );
        for (action, said) in [
            (
                "{ id: a, label: A, command: x.y.z, row: true, prominent: true }",
                "`prominent` is for a page action",
            ),
            (
                "{ id: a, label: A, command: x.y.z, group: open, prominent: true }",
                "`prominent` is for a page action",
            ),
            (
                "{ id: a, label: A, command: x.y.z, drag: true, prominent: true }",
                "`prominent` is for a page action",
            ),
        ] {
            let err = parse(&[action]).unwrap_err();
            assert!(err.contains(said), "{action}: {err}");
        }
    }

    /// An action may name an icon oxplow draws (`ICONS`): alone on its
    /// button in the bar, beside its label in a menu.
    #[test]
    fn an_action_names_an_icon_oxplow_draws() {
        let parse = |actions: &[&str]| {
            parse_actions(
                actions
                    .iter()
                    .map(|a| serde_yaml::from_str(a).unwrap())
                    .collect(),
                &[],
                false,
                false,
            )
        };
        let ok = parse(&[
            "{ id: a, label: A, command: x.y.z, icon: plus, prominent: true }",
            "{ id: b, label: B, command: x.y.z, row: true, icon: pencil }",
        ])
        .unwrap();
        assert_eq!(ok[0].icon.as_deref(), Some("plus"));
        assert_eq!(ok[1].icon.as_deref(), Some("pencil"));
        let err = parse(&["{ id: a, label: A, command: x.y.z, icon: unicorn }"]).unwrap_err();
        assert!(
            err.contains("`icon: unicorn` isn't one oxplow draws"),
            "{err}"
        );
    }

    /// `submenu` puts a page action in a submenu of that name in the
    /// page's ☰ menu. A row, group, drag or edit action isn't in the ☰,
    /// and a prominent one is a button in the bar, so neither can be.
    #[test]
    fn submenu_groups_page_actions_in_the_menu() {
        let parse = |actions: &[&str]| {
            parse_actions(
                actions
                    .iter()
                    .map(|a| serde_yaml::from_str(a).unwrap())
                    .collect(),
                &[],
                true,
                false,
            )
        };
        let ok = parse(&[
            "{ id: a, label: A, command: x.y.z, submenu: Export }",
            "{ id: b, label: B, command: x.y.z }",
        ])
        .unwrap();
        assert_eq!(ok[0].submenu.as_deref(), Some("Export"));
        assert_eq!(ok[1].submenu, None);
        for (action, said) in [
            (
                "{ id: a, label: A, command: x.y.z, row: true, submenu: Export }",
                "`submenu` is for a page action",
            ),
            (
                "{ id: a, label: A, command: x.y.z, group: open, submenu: Export }",
                "`submenu` is for a page action",
            ),
            (
                "{ id: a, label: A, command: x.y.z, prominent: true, submenu: Export }",
                "a prominent action is a button in the bar",
            ),
            (
                "{ id: a, label: A, command: x.y.z, submenu: '  ' }",
                "`submenu` needs a name",
            ),
        ] {
            let err = parse(&[action]).unwrap_err();
            assert!(err.contains(said), "{action}: {err}");
        }
    }

    /// A drag action is a row action a drop runs, binding where the row
    /// landed (`{{before.*}}`, `{{after.*}}`, `{{onto.*}}`, `{{group.value}}`);
    /// a bulk action runs per marked row; a key action from the keyboard.
    /// Each refuses what it can't mean.
    #[test]
    fn drag_bulk_and_key_actions_say_what_they_mean() {
        let parse = |actions: &[&str], grouped: bool| {
            parse_actions(
                actions
                    .iter()
                    .map(|a| serde_yaml::from_str(a).unwrap())
                    .collect(),
                &[],
                grouped,
                false,
            )
        };
        let ok = parse(
            &[
                "{ id: move, label: Move, command: x.y.z, drag: true, input: { uri: '{{row.uri}}', to: '{{group.value}}' } }",
                "{ id: put, label: Put, command: x.y.z, drag: true, input: { uri: '{{row.uri}}', before: '{{before.uri}}', after: '{{after.uri}}', parent: '{{onto.uri}}' } }",
                "{ id: done, label: Done, command: x.y.z, row: true, bulk: true, key: f, input: { uri: '{{row.uri}}' } }",
                "{ id: del, label: Delete, command: x.y.z, row: true, key: Delete }",
                "{ id: order, label: Reorder, command: x.y.z, drag: true, input: { order: '{{order.uri}}' } }",
            ],
            true,
        )
        .unwrap();
        assert!(ok[0].drag && ok[0].row, "a drag action is a row action");
        assert!(ok[2].bulk && ok[2].key.as_deref() == Some("f"));
        assert_eq!(ok[3].key.as_deref(), Some("Delete"));

        for (actions, grouped, says) in [
            (vec!["{ id: a, label: A, command: x.y.z, drag: true, form: true }"], true, "a drag action can't be a form"),
            (vec!["{ id: a, label: A, command: x.y.z, drag: true, primary: true }"], true, "a drag action can't be `primary`"),
            (vec!["{ id: a, label: A, command: x.y.z, row: true, input: { b: '{{before.uri}}' } }"], true, "`{{before.uri}}` needs `drag: true`"),
            (vec!["{ id: a, label: A, command: x.y.z, row: true, input: { o: '{{order.uri}}' } }"], true, "`{{order.uri}}` needs `drag: true`"),
            (vec!["{ id: a, label: A, command: x.y.z, drag: true, input: { g: '{{group.value}}' } }"], false, "`{{group.value}}` needs the viz to have a `group`"),
            (vec!["{ id: a, label: A, command: x.y.z, drag: true, input: { g: '{{group.by}}' } }"], true, "`{{group.by}}` isn't `{{group.value}}`"),
            (vec!["{ id: a, label: A, command: x.y.z, bulk: true }"], true, "`bulk` is for a row action"),
            (vec!["{ id: a, label: A, command: x.y.z, key: f }"], true, "`key` is for a row action"),
            (vec!["{ id: a, label: A, command: x.y.z, row: true, key: Enter }"], true, "`Enter` is the kit's"),
            (vec!["{ id: a, label: A, command: x.y.z, row: true, key: ff }"], true, "`ff` isn't one key"),
            (
                vec![
                    "{ id: a, label: A, command: x.y.z, row: true, key: f }",
                    "{ id: b, label: B, command: x.y.z, row: true, key: f }",
                ],
                true,
                "key `f` is already `a`'s",
            ),
        ] {
            let err = parse(&actions, grouped).unwrap_err();
            assert!(err.contains(says), "{actions:?}: {err}");
        }
    }

    /// An edit action is the markdown viz's Save: a form over the first
    /// row, its edited field the one bound exactly to a row column and
    /// not fixed. It refuses what can't be an editor's save.
    #[test]
    fn an_edit_action_is_the_save_of_a_markdown_editor() {
        let parse = |actions: &[&str]| {
            parse_actions(
                actions
                    .iter()
                    .map(|a| serde_yaml::from_str(a).unwrap())
                    .collect(),
                &[],
                true,
                false,
            )
        };
        let ok = parse(&["{ id: save, label: Save, command: x.y.z, edit: true, fixed: [uri], input: { uri: '{{row.uri}}', body: '{{row.body}}' } }"]).unwrap();
        assert!(
            ok[0].edit && ok[0].row && ok[0].form,
            "an edit action is a form over a row"
        );
        assert_eq!(ok[0].edit_field.as_deref(), Some("body"));

        for (actions, says) in [
            (vec!["{ id: a, label: A, command: x.y.z, edit: true, fixed: [uri], input: { uri: '{{row.uri}}' } }"], "`a` edits nothing: bind one input field to `{{row.<the text column>}}`"),
            (vec!["{ id: a, label: A, command: x.y.z, edit: true, input: { uri: '{{row.uri}}', body: '{{row.body}}' } }"], "`a` edits `uri` and `body`: fix all but one"),
            (vec!["{ id: a, label: A, command: x.y.z, edit: true, drag: true, input: { body: '{{row.body}}' } }"], "an edit action is the editor's Save"),
            (vec!["{ id: a, label: A, command: x.y.z, edit: true, bulk: true, input: { body: '{{row.body}}' } }"], "an edit action is the editor's Save"),
            (vec!["{ id: a, label: A, command: x.y.z, edit: true, primary: true, input: { body: '{{row.body}}' } }"], "an edit action is the editor's Save"),
            (
                vec![
                    "{ id: a, label: A, command: x.y.z, edit: true, input: { body: '{{row.body}}' } }",
                    "{ id: b, label: B, command: x.y.z, edit: true, input: { body: '{{row.body}}' } }",
                ],
                "`b`: the viz already saves with `a`",
            ),
        ] {
            let err = parse(&actions).unwrap_err();
            assert!(err.contains(says), "{actions:?}: {err}");
        }
    }

    /// A markdown viz names the column it shows; an edit action saves that
    /// column, on a markdown viz only.
    #[test]
    fn a_markdown_editor_saves_the_column_it_shows() {
        let save = "actions: [{ id: save, label: Save, command: x.y.z, edit: true, fixed: [slug], input: { slug: '{{row.slug}}', body: '{{row.body}}' } }]\n";
        let rows = "SELECT 'a' AS slug, 'hi' AS body";
        let (_d, ext) = load_x(
            &[
                (
                    "ok",
                    &viz_page(
                        "",
                        rows,
                        &format!("viz: markdown\nmarkdown: {{ text: body }}\n{save}"),
                    ),
                ),
                (
                    "view",
                    &viz_page(
                        "",
                        "SELECT 'hi' AS body",
                        "viz: markdown\nmarkdown: { text: body }\n",
                    ),
                ),
                (
                    "notext",
                    &viz_page("", rows, &format!("viz: markdown\n{save}")),
                ),
                (
                    "other",
                    &viz_page(
                        "",
                        "SELECT 'a' AS slug, 'hi' AS slug2",
                        &format!("viz: markdown\nmarkdown: {{ text: slug2 }}\n{save}"),
                    ),
                ),
                ("table", &viz_page("", rows, &format!("viz: table\n{save}"))),
                (
                    "block",
                    &viz_page(
                        "",
                        "SELECT 'hi' AS body",
                        "viz: table\nmarkdown: { text: body }\n",
                    ),
                ),
            ],
            "",
        );
        let viz = |slug: &str| node(&ext, slug);
        let ok = viz("ok").expect("ok loads");
        assert_eq!(ok.markdown.as_ref().unwrap().text.as_deref(), Some("body"));
        assert!(viz("view").is_some());
        for slug in ["notext", "other", "table", "block"] {
            assert!(viz(slug).is_none(), "{slug} is refused");
        }
        let errs = ext.errors.join("\n");
        for says in [
            "`save` saves the text: give the viz `markdown: { text: body }`",
            "`save` saves `body`, but the viz shows `slug2`",
            "an edit action (`save`) is a `markdown` viz's",
            "`markdown` is for viz `markdown`, not `table`",
        ] {
            assert!(errs.contains(says), "{says}\n{errs}");
        }
    }

    /// A board draws its groups as columns, so it needs a `group`; its
    /// `values` fix their order.
    #[test]
    fn a_board_needs_a_group_and_its_values_fix_the_order() {
        let (_d, ext) = load_x(
            &[
                (
                    "b",
                    &viz_page(
                        "",
                        "SELECT 'a' AS uri, 'todo' AS state",
                        "viz: board\ngroup: { by: state, values: [todo, done] }\n",
                    ),
                ),
                ("nob", &viz_page("", "SELECT 'a' AS uri", "viz: board\n")),
            ],
            "",
        );
        let viz = |slug: &str| node(&ext, slug);
        let b = viz("b").expect("b loads");
        assert_eq!(b.group.as_ref().unwrap().values, vec!["todo", "done"]);
        assert!(viz("nob").is_none());
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("`board` needs `group: { by: <column> }`"),
            "{errs}"
        );
    }

    /// `acme` at `dir` with `manifest` (and the files it names): loaded as
    /// a deployment is, its permission errors alone.
    fn permission_errors(dir: &Path, manifest: &str) -> Vec<String> {
        write(dir, "extension.yaml", manifest);
        load_extension_at(dir, "acme")
            .errors
            .into_iter()
            .filter(|e| e.contains("in `permissions:`"))
            .collect()
    }

    fn load_named(root: &Path, exts: &[(&str, &str)]) -> Vec<Extension> {
        for (name, body) in exts {
            let dir = root.join(".oxplow/extensions").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("extension.yaml"),
                format!("manifest: 2\nname: {name}\nintent:\n  purpose: p\n{body}"),
            )
            .unwrap();
        }
        load_extensions(root)
    }

    fn errors_of(exts: &[Extension], name: &str) -> String {
        exts.iter()
            .find(|e| e.name == name)
            .unwrap()
            .errors
            .join("\n")
    }

    /// `requires` names other extensions: a bad name or the extension's
    /// own is an error at its line.
    #[test]
    fn a_requires_entry_must_name_another_valid_extension() {
        let d = tempfile::tempdir().unwrap();
        let exts = load_named(
            d.path(),
            &[
                ("good", "requires: [other]\n"),
                ("bad", "requires: [Not_A_Name]\n"),
                ("selfish", "requires: [selfish]\n"),
            ],
        );
        assert_eq!(errors_of(&exts, "good"), "");
        let good = exts.iter().find(|e| e.name == "good").unwrap();
        assert_eq!(good.requires, vec!["other".to_string()]);
        let bad = errors_of(&exts, "bad");
        assert!(
            bad.contains("extension.yaml:5:") && bad.contains("`Not_A_Name`"),
            "{bad}"
        );
        let selfish = errors_of(&exts, "selfish");
        assert!(
            selfish.contains("extension.yaml:5:") && selfish.contains("itself"),
            "{selfish}"
        );
    }

    /// A `build:` needs a command, and its output and excluded folders are
    /// relative folders inside the extension that leave its manifest alone.
    #[test]
    fn a_build_block_is_checked_for_shape() {
        let d = tempfile::tempdir().unwrap();
        let exts = load_named(
            d.path(),
            &[
                (
                    "good",
                    "build: { run: make, output: dist, exclude: [node_modules] }\n",
                ),
                ("norun", "build: { run: \"\", output: dist }\n"),
                ("escape", "build: { run: make, output: ../dist }\n"),
                ("dot", "build: { run: make, output: . }\n"),
                (
                    "same",
                    "build: { run: make, output: dist, exclude: [dist] }\n",
                ),
                ("unknown", "build: { run: make, output: dist, bogus: 1 }\n"),
            ],
        );
        assert_eq!(errors_of(&exts, "good"), "");
        let good = exts.iter().find(|e| e.name == "good").unwrap();
        assert_eq!(good.build.as_ref().map(|b| b.run.as_str()), Some("make"));
        for (name, says) in [
            ("norun", "build: `run` is empty"),
            (
                "escape",
                "build: `../dist` isn't a folder inside the extension",
            ),
            ("dot", "build: `.` isn't a folder inside the extension"),
            (
                "same",
                "build: `dist` is the output; it can't be excluded too",
            ),
            ("unknown", "bogus"),
        ] {
            let errors = errors_of(&exts, name);
            assert!(
                errors.contains("extension.yaml:5:") && errors.contains(says),
                "{name}: {errors}"
            );
        }
    }

    /// A component bundle under the build's output isn't there until it is
    /// built: the source loads with it taken as declared.
    #[test]
    fn a_bundle_under_the_build_output_loads_as_declared_before_it_is_built() {
        let d = tempfile::tempdir().unwrap();
        let exts = load_named(
            d.path(),
            &[
                ("built", "build: { run: make, output: dist }\ncustom_components:\n  - { id: w, bundle: dist/w }\n"),
                ("unbuilt", "custom_components:\n  - { id: w, bundle: dist/w }\n"),
            ],
        );
        assert_eq!(errors_of(&exts, "built"), "");
        assert!(!errors_of(&exts, "unbuilt").is_empty());
    }

    /// Two extensions that require each other could never be turned on:
    /// each is an error naming the other.
    #[test]
    fn a_requires_cycle_is_an_error_on_each_member() {
        let d = tempfile::tempdir().unwrap();
        let exts = load_named(
            d.path(),
            &[
                ("a", "requires: [b]\n"),
                ("b", "requires: [a]\n"),
                ("c", "requires: [a]\n"),
            ],
        );
        let a = errors_of(&exts, "a");
        assert!(a.contains("it requires `b`, which requires it back"), "{a}");
        let b = errors_of(&exts, "b");
        assert!(b.contains("it requires `a`, which requires it back"), "{b}");
        assert_eq!(errors_of(&exts, "c"), "");
    }

    /// A provider runs a program with the person's authority and reaches
    /// its hosts: without `process` and those hosts declared, the
    /// extension doesn't load, and the error is at the provider's line.
    #[test]
    fn a_provider_without_process_permission_is_a_load_error_at_its_line() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "provider.json",
            &serde_json::to_string(&oxplow_provider_fake::declarations()).unwrap(),
        );
        let body = "providers:\n  - id: tracker\n    capability: work_items\n    entry: bin/provider\n    declarations: provider.json\n    network: [api.example.com]\n";
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\nintent:\n  purpose: x\n{body}"),
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].contains("extension.yaml:6:")
                && errors[0].contains("`process.run`")
                && errors[0].contains("`network:api.example.com`")
                && errors[0].contains("provider `tracker`"),
            "{errors:?}"
        );
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\npermissions: [process.run, network:api.example.com]\nintent:\n  purpose: x\n{body}"),
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// A component may invoke its own extension's commands; any other
    /// command a `commands:<pattern>` permission must name.
    #[test]
    fn a_component_may_invoke_only_its_own_or_a_declared_command() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "components/board/index.html", "<!doctype html>");
        let body = "custom_components:\n  - { id: board, commands: [oxplow.work_item.transition, acme.item.tidy] }\n";
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\nintent:\n  purpose: x\n{body}"),
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].contains("`commands:oxplow.work_item.transition`")
                && errors[0].contains("component `board`"),
            "{errors:?}"
        );
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\npermissions: [commands:oxplow.work_item.*]\nintent:\n  purpose: x\n{body}"),
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// A component that runs its own SQL (`sql: read`) needs
    /// `sql.read`, as a script's `sql.read` does.
    #[test]
    fn a_component_that_runs_sql_needs_the_sql_permission() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "components/board/index.html", "<!doctype html>");
        let body = "custom_components:\n  - { id: board, sql: read }\n";
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\nintent:\n  purpose: x\n{body}"),
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].contains("`sql.read`") && errors[0].contains("component `board`"),
            "{errors:?}"
        );
        let errors = permission_errors(
            dir.path(),
            &format!(
                "manifest: 2\nname: acme\npermissions: [sql.read]\nintent:\n  purpose: x\n{body}"
            ),
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// A Starlark collector that asks a model spends the person's key.
    #[test]
    fn an_ai_calling_starlark_collector_needs_ai() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "sum.star",
            "def transform(x):\n    return {\"notes\": [{\"n\": 1, \"s\": ai_summarize(\"x\")}]}\n",
        );
        let body = "collectors:\n  - id: sum\n    runtime: starlark\n    entry: sum.star\n    needs: [ai.compute]\n    entities:\n      - { name: notes, key: n, columns: { n: int, s: text } }\n";
        let errors = permission_errors(
            dir.path(),
            &format!("manifest: 2\nname: acme\nintent:\n  purpose: x\n{body}"),
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].contains("`ai.compute`") && errors[0].contains("collector `sum`"),
            "{errors:?}"
        );
        let errors = permission_errors(
            dir.path(),
            &format!(
                "manifest: 2\nname: acme\npermissions: [ai.compute]\nintent:\n  purpose: x\n{body}"
            ),
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// A collector's `input:` query is declared and reviewable: baseline.
    /// A script's own SQL (`needs: [sql.read]`) is `sql: read`.
    #[test]
    fn a_collector_input_query_is_baseline_but_needs_sql_read_is_not() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "sum.star",
            "def transform(x):\n    return {\"notes\": []}\n",
        );
        let collector = |needs: &str| {
            format!("manifest: 2\nname: acme\nintent:\n  purpose: x\ncollectors:\n  - id: sum\n    runtime: starlark\n    entry: sum.star\n    input: SELECT 1 AS n\n{needs}    entities:\n      - {{ name: notes, key: n, columns: {{ n: int }} }}\n")
        };
        let errors = permission_errors(dir.path(), &collector(""));
        assert!(errors.is_empty(), "{errors:?}");
        let errors = permission_errors(dir.path(), &collector("    needs: [sql.read]\n"));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("`sql.read`"), "{errors:?}");
    }

    const EXT: &str =
        "manifest: 2\nname: review\nintent:\n  purpose: test\ndescription: Review helpers\n";
    const PAGE: &str = r#"
title: Items by state
description: Every work item in a given state.
params:
  - { name: state, label: State, default: in_progress }
loaders:
  items:
    sql: |
      SELECT uri, title FROM v_work_item WHERE state = :state ORDER BY title
view:
  viz: table
  from: items
  columns:
    - { key: title, label: Item, link: { kind: page, from: uri } }
  empty: No items.
"#;

    async fn layer() -> crate::sql_gateway::SqlGateway {
        crate::sql_gateway::SqlGateway::new(Database::in_memory())
    }

    /// Project extensions only (bundled ones are always present).
    fn project_extensions(root: &Path) -> Vec<Extension> {
        load_extensions(root)
            .into_iter()
            .filter(|e| e.origin == "project")
            .collect()
    }

    /// A page whose view is one viz node over the `sql` loader `q`: `head`
    /// is its metadata (`title: T` unless it names one), `node` the node's
    /// keys beside `from: q`, a line each (`viz: table` unless it names one).
    fn viz_page(head: &str, sql: &str, node: &str) -> String {
        let title = if head.contains("title:") {
            ""
        } else {
            "title: T\n"
        };
        let viz = if node.contains("viz:") {
            ""
        } else {
            "  viz: table\n"
        };
        let node: String = node.lines().map(|l| format!("  {l}\n")).collect();
        format!("{title}{head}loaders:\n  q:\n    sql: {sql:?}\nview:\n  from: q\n{viz}{node}")
    }

    /// Page `slug` of `ext`, when it loaded.
    fn page_of<'a>(ext: &'a Extension, slug: &str) -> Option<&'a crate::pages::schema::Page> {
        ext.page_files.iter().find(|p| p.slug == slug)
    }

    /// The config of page `slug`'s view, when it loaded and its view is a
    /// viz node.
    fn node<'a>(ext: &'a Extension, slug: &str) -> Option<&'a VizConfig> {
        match &page_of(ext, slug)?.view.kind {
            crate::pages::schema::NodeKind::Viz { config, .. } => Some(config),
            _ => None,
        }
    }

    /// Services over a git project at `dir`, its extensions active.
    async fn services(dir: &Path) -> std::sync::Arc<crate::Services> {
        crate::test_fixtures::init_git_repo(dir);
        let svc = crate::Services::in_memory(dir).await.unwrap();
        crate::test_fixtures::activate(&svc).await;
        svc
    }

    /// Run page `id` with `params` as the person, from `ctx`.
    async fn run_page(
        svc: &crate::Services,
        id: &str,
        params: BTreeMap<String, SqlCell>,
        ctx: ViewContext,
    ) -> Result<crate::pages::runner::PageRun, DomainError> {
        use crate::pages::runner::{PageRunner, RunRequest, RunTarget};
        PageRunner::of(svc)
            .run(RunRequest {
                target: RunTarget::Page(id.into()),
                params,
                ctx,
                viewer: oxplow_domain::Actor::Human,
            })
            .await
    }

    /// Loader `loader`'s rows in `run`, as JSON.
    fn rows_of(run: &crate::pages::runner::PageRun, loader: &str) -> serde_json::Value {
        serde_json::to_value(&run.loaders[loader].result.rows).unwrap()
    }

    #[test]
    fn no_extensions_dir_means_no_project_extensions() {
        let dir = tempfile::tempdir().unwrap();
        assert!(project_extensions(dir.path()).is_empty());
    }

    #[test]
    fn loads_an_extension_and_its_pages() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/by-status.yaml",
            PAGE,
        );

        let exts = project_extensions(dir.path());
        assert_eq!(exts.len(), 1);
        let e = &exts[0];
        assert_eq!(e.name, "review");
        assert_eq!(e.description, "Review helpers");
        assert_eq!(e.path, ".oxplow/extensions/review");
        assert!(e.errors.is_empty(), "{:?}", e.errors);
        assert_eq!(e.page_files.len(), 1);
        let p = &e.page_files[0];
        assert_eq!(p.id, "review/by-status");
        assert_eq!(p.slug, "by-status");
        assert_eq!(p.title, "Items by state");
        assert_eq!(
            p.origin.running,
            ".oxplow/extensions/review/pages/by-status.yaml"
        );
        assert_eq!(
            p.params[0].default,
            Some(SqlCell::Text("in_progress".into()))
        );
        let l = node(e, "by-status").unwrap();
        assert_eq!(l.viz, Viz::Table);
        assert_eq!(
            l.columns[0].link,
            Some(ColumnLink {
                kind: LinkKind::Page,
                from: Some("uri".into()),
                line: None,
                base: None,
                head: None,
            })
        );
    }

    /// An extension still holding a `lenses/` folder is told to rewrite it
    /// as pages; nothing in it loads.
    #[test]
    fn a_lenses_folder_is_an_error_naming_pages() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/lenses/old.yaml",
            "title: Old\nquery: SELECT 1\n",
        );
        let e = &project_extensions(dir.path())[0];
        let errs = e.errors.join("\n");
        assert!(
            errs.contains("review/lenses/") && errs.contains("pages/<slug>.yaml"),
            "{errs}"
        );
        assert!(e.page_files.is_empty());
    }

    #[test]
    fn a_broken_extension_reports_errors_without_hiding_others() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/good.yaml",
            PAGE,
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/bad.yaml",
            "title: x\nveiw: { markdown: hi }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/nomanifest/pages/a.yaml",
            PAGE,
        );
        write(
            dir.path(),
            ".oxplow/extensions/misnamed/extension.yaml",
            "manifest: 2\nname: other\nintent:\n  purpose: test\n",
        );

        let exts = project_extensions(dir.path());
        let by = |n: &str| {
            exts.iter()
                .find(|e| e.name == n)
                .unwrap_or_else(|| panic!("{n} missing"))
        };

        let review = by("review");
        assert_eq!(
            review
                .page_files
                .iter()
                .map(|p| p.slug.as_str())
                .collect::<Vec<_>>(),
            vec!["good"]
        );
        assert_eq!(review.errors.len(), 1);
        assert!(
            review.errors[0].contains("pages/bad.yaml"),
            "{:?}",
            review.errors
        );

        let nm = by("nomanifest");
        assert!(nm.errors[0].contains("extension.yaml"), "{:?}", nm.errors);
        assert!(nm.page_files.is_empty());

        let mis = by("misnamed");
        assert!(
            mis.errors[0].contains("must match its folder"),
            "{:?}",
            mis.errors
        );
    }

    #[test]
    fn find_page_by_id() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/by-status.yaml",
            PAGE,
        );
        assert_eq!(
            cat()
                .find_page(dir.path(), "review/by-status")
                .unwrap()
                .title,
            "Items by state"
        );
        assert!(matches!(
            cat().find_page(dir.path(), "review/nope"),
            Err(DomainError::NotFound)
        ));
        assert!(matches!(
            cat().find_page(dir.path(), "nope"),
            Err(DomainError::NotFound)
        ));
    }

    #[tokio::test]
    async fn a_page_run_binds_defaults_and_overrides() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/echo.yaml",
            &viz_page(
                "title: Echo\nparams:\n  - { name: a, default: 1 }\n  - { name: b, default: two }\n",
                "SELECT :a AS a, :b AS b",
                "",
            ),
        );
        let svc = services(dir.path()).await;

        let run = run_page(&svc, "review/echo", BTreeMap::new(), ViewContext::default())
            .await
            .unwrap();
        assert_eq!(run.loaders["q"].result.columns, vec!["a", "b"]);
        assert_eq!(rows_of(&run, "q"), serde_json::json!([[1, "two"]]));
        assert_eq!(run.params.get("b"), Some(&SqlCell::Text("two".into())));

        let over = BTreeMap::from([("b".to_string(), SqlCell::Text("three".into()))]);
        let run = run_page(&svc, "review/echo", over, ViewContext::default())
            .await
            .unwrap();
        assert_eq!(rows_of(&run, "q"), serde_json::json!([[1, "three"]]));
    }

    /// A page declaring `stream_id` / `thread_id` gets the caller's
    /// current ones unless it's given others; a default applies only with
    /// no current value.
    #[tokio::test]
    async fn a_page_run_binds_the_current_stream_and_thread() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/mine.yaml",
            &viz_page(
                "title: Mine\nparams:\n  - { name: stream_id }\n  - { name: thread_id, default: 9 }\n",
                "SELECT :stream_id AS s, :thread_id AS t",
                "",
            ),
        );
        let svc = services(dir.path()).await;
        let here = || ViewContext {
            stream_id: Some(2),
            thread_id: Some(5),
            active: None,
        };
        let run = run_page(&svc, "review/mine", BTreeMap::new(), here())
            .await
            .unwrap();
        assert_eq!(rows_of(&run, "q"), serde_json::json!([[2, 5]]));

        let over = BTreeMap::from([("thread_id".to_string(), SqlCell::Int(7))]);
        let run = run_page(&svc, "review/mine", over, here()).await.unwrap();
        assert_eq!(
            rows_of(&run, "q"),
            serde_json::json!([[2, 7]]),
            "an explicit value wins"
        );

        let run = run_page(&svc, "review/mine", BTreeMap::new(), ViewContext::default())
            .await
            .unwrap();
        assert_eq!(
            rows_of(&run, "q"),
            serde_json::json!([[null, 9]]),
            "no context: defaults"
        );
    }

    /// The viewer's context: the given stream (else the primary) and the
    /// given thread (else the stream's selected one).
    #[tokio::test]
    async fn view_context_defaults_to_the_primary_stream_and_its_thread() {
        let f = crate::test_fixtures::services_with_effort().await;
        use oxplow_domain::stores::ThreadStore as _;
        let thread = f.svc.thread_store.get(&f.thread).await.unwrap().unwrap();
        let expect = ViewContext {
            stream_id: Some(thread.stream_id.value()),
            thread_id: Some(f.thread.value()),
            active: Some(
                f.svc
                    .capabilities
                    .snapshot(&crate::config_service::read_config(&f.svc.config)),
            ),
        };
        assert_eq!(view_context(&f.svc, None, None).await, expect);
        assert_eq!(view_context(&f.svc, None, Some(f.thread)).await, expect);
        assert_eq!(
            view_context(&f.svc, Some(thread.stream_id), None).await,
            expect
        );
    }

    /// A misspelt param is refused, naming it; a loader whose query fails
    /// carries its error and the page still runs.
    #[tokio::test]
    async fn a_page_run_refuses_unknown_params_and_carries_query_errors() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/by-status.yaml",
            PAGE,
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/broken.yaml",
            &viz_page("title: Broken\n", "SELECT nope FROM v_nothing", ""),
        );
        let svc = services(dir.path()).await;

        let bad = BTreeMap::from([("stauts".to_string(), SqlCell::Text("done".into()))]);
        let err = run_page(&svc, "review/by-status", bad, ViewContext::default())
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("stauts")),
            "{err:?}"
        );

        let run = run_page(
            &svc,
            "review/broken",
            BTreeMap::new(),
            ViewContext::default(),
        )
        .await
        .unwrap();
        let err = run.loaders["q"].error.as_deref().unwrap_or_default();
        assert!(err.contains("v_nothing"), "{err}");
    }

    #[tokio::test]
    async fn validate_dry_runs_every_page() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/by-status.yaml",
            PAGE,
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/broken.yaml",
            &viz_page("title: Broken\n", "SELECT nope FROM v_nothing", ""),
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/badcol.yaml",
            &viz_page(
                "title: Bad column\n",
                "SELECT 1 AS n",
                "columns:\n  - { key: missing }\n",
            ),
        );
        let sl = layer().await;

        let e = validate_extension(&sl, &cat(), dir.path(), "review", None)
            .await
            .unwrap();
        assert_eq!(e.errors.len(), 2, "{:?}", e.errors);
        assert!(e
            .errors
            .iter()
            .any(|m| m.contains("pages/broken.yaml") && m.contains("loader `q`")));
        assert!(e
            .errors
            .iter()
            .any(|m| m.contains("pages/badcol.yaml") && m.contains("missing")));

        assert!(matches!(
            validate_extension(&sl, &cat(), dir.path(), "nope", None).await,
            Err(DomainError::NotFound)
        ));
    }

    /// What a scaffold writes points an editor at its schema on the first
    /// line.
    #[test]
    fn a_scaffolded_manifest_names_its_schema() {
        let manifest = scaffold_manifest(&ManifestScaffold {
            name: "mine",
            description: "d",
            purpose: "p",
            origin: None,
            example_name: "e",
            example_input: "{}",
            example_expect: "x",
        });
        assert!(
            manifest.starts_with(&schema::modeline(schema::MANIFEST_SCHEMA_URL)),
            "{manifest}"
        );
    }

    #[test]
    fn loads_declared_sources_and_reports_bad_ones_without_dropping_pages() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/review/extension.yaml",
            "manifest: 2\nname: review\nintent:\n  purpose: test\npermissions: [process.run]\ncollectors:\n  - id: gh\n    runtime: exec\n    entry: bin/sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int, title: text } }\n  - id: bad\n    runtime: python\n    entry: x\n    entities: []\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/by-status.yaml",
            PAGE,
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/bin/sync.sh",
            "#!/bin/sh\n",
        );
        let e = &project_extensions(dir.path())[0];
        assert_eq!(e.collectors.len(), 1, "{:?}", e.errors);
        assert_eq!(e.collectors[0].entities[0].view, "v_review_pr");
        assert_eq!(e.errors.len(), 1, "{:?}", e.errors);
        assert!(
            e.errors[0].contains("extension.yaml") && e.errors[0].contains("runtime"),
            "{:?}",
            e.errors
        );
        assert_eq!(e.page_files.len(), 1);
    }

    /// A check stands an empty view in for an entity no sync has filled
    /// yet: a page over it checks clean before its first sync.
    #[tokio::test]
    async fn a_check_stands_an_empty_view_in_for_an_uncollected_entity() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/gh/extension.yaml",
            "manifest: 2\nname: gh\nintent:\n  purpose: test\npermissions: [process.run]\ncollectors:\n  - id: prs\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int } }\n",
        );
        write(dir.path(), ".oxplow/extensions/gh/sync.sh", "#!/bin/sh\n");
        write(
            dir.path(),
            ".oxplow/extensions/gh/pages/all.yaml",
            &viz_page("title: All\n", "SELECT number FROM v_gh_pr", ""),
        );
        let e = validate_extension(&layer().await, &cat(), dir.path(), "gh", None)
            .await
            .unwrap();
        assert!(e.errors.is_empty(), "{:?}", e.errors);
    }
    const EXT_V2: &str = "manifest: 2\nname: review\ndescription: Review helpers\nintent:\n  purpose: Review agent work\n  examples:\n    - { name: one, input: {}, expect: {} }\n";

    fn only(root: &Path, name: &str) -> Extension {
        load_extensions(root)
            .into_iter()
            .find(|e| e.name == name && e.origin == "project")
            .unwrap()
    }

    #[test]
    fn a_v2_manifest_needs_an_intent_and_says_where() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/review/extension.yaml",
            "manifest: 2\nname: review\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/tasks.yaml",
            PAGE,
        );
        let e = only(dir.path(), "review");
        assert_eq!(e.manifest_version, 2);
        assert!(
            e.errors.iter().any(
                |m| m.starts_with(".oxplow/extensions/review/extension.yaml:1:")
                    && m.contains("`intent` is required")
            ),
            "{:?}",
            e.errors
        );
        // The pages still load: an intent problem is the manifest's.
        assert_eq!(e.page_files.len(), 1);
        // A clean v2 manifest has no errors and no warnings.
        write(
            dir.path(),
            ".oxplow/extensions/review/extension.yaml",
            EXT_V2,
        );
        let e = only(dir.path(), "review");
        assert!(
            e.errors.is_empty() && e.warnings.is_empty(),
            "{:?} {:?}",
            e.errors,
            e.warnings
        );
        assert_eq!(e.intent.as_ref().unwrap().purpose, "Review agent work");
    }

    #[test]
    fn a_slot_mount_to_a_missing_page_names_its_line() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = format!("{EXT_V2}ui:\n  mounts:\n    - {{ page: tasks, at: oxplow-foundation/diff.file-header }}\n    - {{ page: nope, at: oxplow-foundation/diff.file-header }}\n");
        write(
            dir.path(),
            ".oxplow/extensions/review/extension.yaml",
            &manifest,
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/tasks.yaml",
            PAGE,
        );
        let e = only(dir.path(), "review");
        assert_eq!(e.ui.page_slots.len(), 1, "{:?}", e.errors);
        let err = e
            .errors
            .iter()
            .find(|m| m.contains("mounts page `nope`, which isn't in pages/"))
            .unwrap_or_else(|| panic!("{:?}", e.errors));
        assert!(
            err.starts_with(".oxplow/extensions/review/extension.yaml:11:"),
            "{err}"
        );
    }

    #[test]
    fn a_metric_naming_a_measure_from_elsewhere_is_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = format!(
            "{EXT_V2}measures:\n  - {{ key: review.todo, title: TODOs, unit: count, subjectKind: file, fold: last }}\nmetrics:\n  - {{ key: review.todo_total, title: TODO total, measure: review.todo, expression: \"SUM(value)\" }}\n  - {{ key: review.bad, title: Bad, measure: review.missing, expression: \"SUM(value)\" }}\n"
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/extension.yaml",
            &manifest,
        );
        let e = only(dir.path(), "review");
        assert!(e.errors.is_empty(), "{:?}", e.errors);
        let warn = e
            .warnings
            .iter()
            .find(|m| m.contains("measure `review.missing`"))
            .unwrap_or_else(|| panic!("{:?}", e.warnings));
        assert!(warn.contains("extension.yaml:"), "{warn}");
        assert_eq!(e.metrics.len(), 2, "both specs still load");
    }

    /// A manifest says its version, or it doesn't load — there is
    /// no v1 reader any more.
    #[test]
    fn a_manifest_without_a_version_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/review/extension.yaml",
            "name: review\ndescription: Review helpers\n",
        );
        let e = only(dir.path(), "review");
        assert!(
            e.errors
                .iter()
                .any(|m| m.contains("extension.yaml:1") && m.contains("`manifest: 2` is required")),
            "{:?}",
            e.errors
        );
        assert!(e.intent.is_none());
    }

    /// The tally extension: a starlark collector declaring entity `thing`,
    /// a dataset over it, pages over both, and a command reading the dataset.
    fn write_tally(root: &Path, script: &str) {
        write(
            root,
            ".oxplow/extensions/tally/extension.yaml",
            "manifest: 2
name: tally
intent: { purpose: x, examples: [{ name: a }] }
collectors:
  - id: things
    runtime: starlark
    entry: collectors/things.star
    input: \"SELECT uri FROM v_work_item\"
    entities:
      - { name: thing, key: id, columns: { id: int, label: text } }
datasets:
  - name: labelled
    version: 1
    description: Labelled things.
    fields:
      - { name: id, type: INTEGER, doc: The thing. }
      - { name: label, type: TEXT, doc: Its label. }
commands:
  - name: thing.note
    summary: Note a thing.
    input_schema: { type: object, required: [id], properties: { id: { type: integer } } }
    entry: handlers/note.star
",
        );
        write(
            root,
            ".oxplow/extensions/tally/collectors/things.star",
            script,
        );
        write(
            root,
            ".oxplow/extensions/tally/datasets/labelled.sql",
            "SELECT id, label FROM ref('thing') WHERE label IS NOT NULL\n",
        );
        write(
            root,
            ".oxplow/extensions/tally/handlers/note.star",
            "def transform(x):\n    return {\"commands\": []}\n",
        );
        write(
            root,
            ".oxplow/extensions/tally/pages/labelled.yaml",
            &viz_page(
                "title: Labelled\n",
                "SELECT id, label FROM v_tally_labelled",
                "",
            ),
        );
        write(
            root,
            ".oxplow/extensions/tally/pages/things.yaml",
            &viz_page("title: Things\n", "SELECT id, label FROM v_tally_thing", ""),
        );
    }

    /// P7.C6: a check sees what the extension declares before it ever ran
    /// or published — its collectors' entities (empty stand-ins) and its
    /// own datasets (compiled for the check) — from its pages (and its
    /// commands' examples' reads), on a database that has neither.
    #[tokio::test]
    async fn a_check_sees_the_extensions_own_datasets_and_unsynced_entities() {
        let dir = tempfile::tempdir().unwrap();
        write_tally(
            dir.path(),
            "def transform(x):\n    return {\"entities\": {\"thing\": []}}\n",
        );
        let v = validate_extension(&layer().await, &cat(), dir.path(), "tally", None)
            .await
            .unwrap();
        assert!(v.errors.is_empty(), "{:?}", v.errors);

        // Still a real check: a column the dataset doesn't have is an error.
        write(
            dir.path(),
            ".oxplow/extensions/tally/pages/labelled.yaml",
            &viz_page(
                "title: Labelled\n",
                "SELECT id, colour FROM v_tally_labelled",
                "",
            ),
        );
        let v = validate_extension(&layer().await, &cat(), dir.path(), "tally", None)
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        assert!(errs.contains("colour"), "{errs}");
    }

    /// P7.C6: a starlark collector's script is parsed at load — one that
    /// doesn't define `transform` is an error at the collector's line.
    #[test]
    fn a_collector_script_without_transform_is_an_error_at_its_line() {
        let dir = tempfile::tempdir().unwrap();
        write_tally(dir.path(), "def other(x):\n    return {}\n");
        let ext = only(dir.path(), "tally");
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("extension.yaml:5:")
                && errs.contains("collector `things`")
                && errs.contains("must define `transform`"),
            "{errs}"
        );
    }

    /// An effect or a command may declare `needs: [ai.discover]`: it loads
    /// and its script can ask whether a model serves a role.
    #[test]
    fn a_script_can_declare_ai_discover() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(".oxplow/extensions/x");
        std::fs::create_dir_all(dir.join("effects")).unwrap();
        std::fs::create_dir_all(dir.join("commands")).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            "manifest: 2\nname: x\nintent:\n  purpose: p\npermissions: [ai.compute]\neffects:\n  - { id: e, summary: s, on: [effort.finished], entry: effects/e.star, needs: [ai.discover] }\ncommands:\n  - { name: c.go, summary: s, entry: commands/c.star, needs: [ai.discover], input_schema: { type: object, additionalProperties: false } }\n",
        )
        .unwrap();
        let script = "def transform(x):\n    r = scope(\"ai.discover\", {\"role\": \"summarize\"})\n    if not r[\"dataset\"]:\n        return {\"skip\": r[\"problem\"]}\n    return {\"commands\": []}\n";
        std::fs::write(dir.join("effects/e.star"), script).unwrap();
        std::fs::write(dir.join("commands/c.star"), script).unwrap();
        let ext = load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap();
        assert_eq!(ext.errors, Vec::<String>::new());
        assert_eq!(ext.effects.len(), 1);
        assert_eq!(ext.commands.len(), 1);
    }

    /// `ai_profiles:` load into the extension, each as a contribution
    /// `<ext>/ai_profile/<id>`, and a mistake is an error at its line.
    #[test]
    fn ai_profiles_load_and_are_described() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".oxplow/extensions/x")).unwrap();
        let manifest = "manifest: 2\nname: x\nintent:\n  purpose: p\nai_profiles:\n  - { id: review, description: Reviews., fallback: summarize, features: [tool_calling] }\n  - { id: bad, description: x, fallback: huge }\n";
        std::fs::write(
            d.path().join(".oxplow/extensions/x/extension.yaml"),
            manifest,
        )
        .unwrap();
        let ext = load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap();
        assert_eq!(ext.ai_profiles.len(), 1, "{:?}", ext.errors);
        assert_eq!(ext.ai_profiles[0].id, "review");
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("extension.yaml:7:") && e.contains("`fallback` is `main`")),
            "{:?}",
            ext.errors
        );
        let described: Vec<String> = ext
            .contributions
            .iter()
            .map(|c| c.id.to_string())
            .filter(|id| id.contains("ai_profile"))
            .collect();
        assert_eq!(described, ["oxplow:oxplow/ai_profile?ai_profile=x/review"]);
    }

    /// Every command an agent is offered — core's, oxplow-foundation's and
    /// oxplow-bundled's — declares an example input beside its schema, and the
    /// example fits: an agent reads a whole right input instead of guessing a
    /// shape field by field (`.context/commands.md` "Exposure to agents").
    #[tokio::test(flavor = "multi_thread")]
    async fn every_command_an_agent_is_offered_has_an_example_that_fits() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        let services = crate::Services::in_memory(dir.path()).await.unwrap();
        let offered = services.commands.list(&oxplow_domain::Actor::Agent {
            session_id: None,
            thread_id: Some(oxplow_domain::ThreadId::new(7)),
            stream_id: None,
        });
        assert!(offered.len() > 40, "{}", offered.len());
        let mut missing = Vec::new();
        let mut misfit = Vec::new();
        for spec in &offered {
            match oxplow_domain::commands::schema_example(&spec.input_schema) {
                None => missing.push(spec.id.clone()),
                Some(example) => {
                    let validator =
                        oxplow_domain::InputValidator::compile(&spec.input_schema).unwrap();
                    if let Err(err) = validator.check(example) {
                        misfit.push(format!("{}: {err}", spec.id));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "no example: {missing:#?}");
        assert!(misfit.is_empty(), "example doesn't fit: {misfit:#?}");
    }

    /// Oxplow ships two extensions: `oxplow-foundation`, its own commands
    /// over the scopes' operations; and `oxplow-bundled`: the review packet,
    /// the analytics pages, the agent's advisories and the code metrics. They load clean and every page's SQL runs.
    #[tokio::test]
    async fn the_bundled_extensions_ship_and_load_clean() {
        let dir = tempfile::tempdir().unwrap();
        let exts = load_extensions(dir.path());
        let bundled: Vec<&Extension> = exts.iter().filter(|e| e.origin == "bundled").collect();
        assert_eq!(
            bundled.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["oxplow-bundled", "oxplow-foundation"]
        );
        let foundation = bundled[1];
        assert!(foundation.errors.is_empty(), "{:?}", foundation.errors);
        assert!(foundation.warnings.is_empty(), "{:?}", foundation.warnings);
        assert_eq!(foundation.namespace, "oxplow");
        assert!(foundation.commands.iter().all(|c| matches!(
            c.handler,
            crate::extension::extension_commands::CommandHandler::Scope { .. }
        )));
        // The agent harnesses and AI providers core runs are
        // foundation's declarations — many of each, all active.
        let declared = |capability: &str| {
            foundation
                .implementations
                .iter()
                .filter(|d| d.capability == capability)
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(declared("agent_harness"), ["claude", "codex", "opencode"]);
        assert_eq!(
            declared("ai_provider"),
            [
                "anthropic",
                "openai",
                "openai_compatible",
                "openrouter",
                "gemini",
                "mistral",
                "groq",
                "deepseek",
                "xai",
                "ollama",
                "llamacpp",
                "typesafe"
            ]
        );
        let b = bundled[0];
        assert!(b.errors.is_empty(), "{:?}", b.errors);
        // The code metrics: 16 collectors beside its own churn one.
        assert!(b.collectors.len() >= 17, "{}", b.collectors.len());
        // Its manifest is v2, with an intent, and clean.
        assert!(b.warnings.is_empty(), "{:?}", b.warnings);
        assert_eq!(b.manifest_version, 2);
        assert!(!b.intent.as_ref().unwrap().examples.is_empty());
        // The review packet and the analytics pages, mounted together.
        for (host, slot, page) in [
            ("oxplow-foundation/diff-view", "sections", "decisions"),
            (
                "oxplow-foundation/diff-view",
                "sections",
                "inferred-decisions",
            ),
            ("oxplow-foundation/diff-view", "sections", "change-review"),
            ("oxplow-foundation/diff-view", "sections", "effort-tests"),
            ("oxplow-foundation/commit", "sections", "tests-weakened"),
            ("oxplow-foundation/work-item", "sections", "task-tokens"),
        ] {
            assert!(
                b.ui.page_slots.iter().any(|s| s.host == host
                    && s.slot == slot
                    && s.page == format!("oxplow-bundled/{page}")),
                "{host}.{slot} {page}"
            );
        }
        assert!(b.panels.iter().any(|p| p.id == "oxplow-bundled/waiting"));
        let commands: Vec<&str> = b.commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            commands,
            [
                "oxplow.review_policy.read",
                "oxplow.review_policy.write",
                "oxplow.reasoning.ask",
                "oxplow.reasoning.status",
                "oxplow.reasoning.cancel",
                "oxplow.effort.infer_reasoning",
                "oxplow.review.accept",
                "oxplow.review.request_changes",
                "oxplow.work.clear_finished"
            ]
        );
        let advisories: Vec<&str> = b.advisories.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            advisories,
            [
                "large-uncommitted",
                "landed-in-progress",
                "metric-deltas",
                "threshold-crossed",
                "hint-never-fires",
                "shared-files",
                "shared-files-agent"
            ]
        );
        let mut datasets: Vec<&str> = b.datasets.iter().map(|m| m.decl.name.as_str()).collect();
        datasets.sort();
        assert_eq!(
            datasets,
            [
                "change_co_change",
                "change_interest",
                "claim_status",
                "co_change_pair",
                "deviation",
                "finished_cleared",
                "struggle",
                "thread_work",
                "verdict",
                "verdicts"
            ]
        );
        let effects: Vec<&str> = b.effects.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            effects,
            [
                "verify-unchecked",
                "repair-disabled",
                "infer-reasoning",
                "store-reasoning"
            ]
        );
        assert!(b
            .collectors
            .iter()
            .any(|c| c.id == "oxplow_bundled.effort_churn"));
        // Every shipped page's SQL and advisory runs against a real schema
        // (a running oxplow's, whose live views read its services) — its
        // own datasets compiled for the check, nothing published — and every
        // viz node's columns are its loader's.
        let svc = services(dir.path()).await;
        for name in ["oxplow-bundled", "oxplow-foundation"] {
            let v = validate_extension(&svc.sql, &svc.extension_catalog, dir.path(), name, None)
                .await
                .unwrap();
            assert!(v.errors.is_empty(), "{name}: {:#?}", v.errors);
        }
        assert_eq!(
            cat()
                .find_page(dir.path(), "oxplow-bundled/decisions")
                .unwrap()
                .title,
            "Decisions Made"
        );
    }

    #[test]
    fn a_name_starts_with_a_letter() {
        for ok in ["review", "review-notes", "a1", "x"] {
            assert!(is_valid_name(ok), "{ok}");
        }
        // `1x` would be the command namespace `1x`, which no command
        // name can have.
        for bad in ["1x", "9-lives", "", "-a", "a-", "a--b", "A", "a_b"] {
            assert!(!is_valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn bundled_names_are_reserved() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/oxplow-bundled/extension.yaml",
            "manifest: 2\nname: oxplow-bundled\nintent:\n  purpose: test\n",
        );
        let exts = load_extensions(dir.path());
        let named: Vec<&Extension> = exts.iter().filter(|e| e.name == "oxplow-bundled").collect();
        // A source under a bundled name is never loaded: the bundled one
        // is the only one.
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].origin, "bundled");
    }

    /// A mount's slot is `<extension>/<page>.<slot>`, and its page one of
    /// the extension's own.
    #[test]
    fn slots_must_name_a_known_slot_and_an_existing_page() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/mine/extension.yaml",
            "manifest: 2\nname: mine\nintent:\n  purpose: test\nui:\n  mounts:\n    - { page: nope, at: acme/host.sections }\n    - { page: a, at: sidebar }\n    - { page: a, at: acme/host.sections }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/mine/pages/a.yaml",
            &viz_page("params: [{ name: path }]\n", "SELECT :path AS p", ""),
        );
        let e = load_extensions(dir.path())
            .into_iter()
            .find(|e| e.name == "mine")
            .unwrap();
        let mounted: Vec<(String, &str)> =
            e.ui.page_slots
                .iter()
                .map(|s| (format!("{}.{}", s.host, s.slot), s.page.as_str()))
                .collect();
        assert_eq!(mounted, [("acme/host.sections".to_string(), "mine/a")]);
        assert_eq!(e.errors.len(), 2, "{:?}", e.errors);
        assert!(e.errors.iter().any(|m| m.contains("nope")));
        assert!(e.errors.iter().any(|m| m.contains("sidebar")));
    }

    /// Another extension mounts its page at a page's slot
    /// (`<extension>/<page>.<slot>`), placed by its `at`; a slot named
    /// without its page is an unknown slot and goes.
    #[test]
    fn an_extension_mounts_a_page_at_another_extensions_page_slot() {
        let dir = tempfile::tempdir().unwrap();
        let ext_yaml = |name: &str, tail: &str| {
            format!("manifest: 2\nname: {name}\nintent:\n  purpose: x\n  examples: [{{ name: a }}]\n{tail}")
        };
        write(
            dir.path(),
            ".oxplow/extensions/base/extension.yaml",
            &ext_yaml("base", ""),
        );
        write(
            dir.path(),
            ".oxplow/extensions/base/pages/page.yaml",
            "title: Page\nparams: [{ name: stream_id }]\nloaders:\n  a: { sql: \"SELECT :stream_id AS n\" }\n  b: { sql: \"SELECT 2 AS n\" }\nview:\n  stack:\n    - { title: A, viz: number, from: a }\n    - { slot: sections }\n    - { title: B, viz: number, from: b }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/addon/extension.yaml",
            &ext_yaml(
                "addon",
                "ui:\n  mounts:\n    - { page: extra, at: base/page.sections, position: { after: a }, title: More }\n    - { page: extra, at: page.sections }\n",
            ),
        );
        write(
            dir.path(),
            ".oxplow/extensions/addon/pages/extra.yaml",
            &viz_page(
                "title: Extra\nparams: [{ name: stream_id }]\n",
                "SELECT :stream_id AS n",
                "",
            ),
        );
        let base = only(dir.path(), "base");
        assert!(base.errors.is_empty(), "{:?}", base.errors);
        let addon = only(dir.path(), "addon");
        assert_eq!(
            addon.ui.page_slots,
            vec![PageSlot {
                host: "base/page".into(),
                slot: "sections".into(),
                page: "addon/extra".into(),
                at: Some(SlotAt::After { after: "a".into() }),
                title: Some("More".into()),
                size: None,
            }]
        );
        let errs = addon.errors.join("\n");
        assert!(
            errs.contains("`at: page.sections` is neither a page's slot"),
            "{errs}"
        );
    }

    /// The documented examples in `examples/extensions/` load cleanly, so
    /// a format change can't silently break what the guide tells people to
    /// copy.
    #[test]
    fn documented_examples_load_without_errors() {
        let examples =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions");
        let names: Vec<String> = std::fs::read_dir(&examples)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        assert!(!names.is_empty());
        for name in names {
            let ext = load_one(&Disk(examples.join(&name)), &name, &name, "project");
            assert!(ext.errors.is_empty(), "{name}: {:?}", ext.errors);
            // The github example's pull requests are a resource of its own.
            if name == "github" {
                let resources: Vec<String> = ext
                    .resources
                    .iter()
                    .map(|r| r.decl.name.to_string())
                    .collect();
                assert_eq!(resources, vec!["github/pr"]);
            }
        }
    }

    /// One project extension `x` with the given page files and
    /// extension.yaml tail; returns its load result.
    fn load_x(files: &[(&str, &str)], manifest_tail: &str) -> (tempfile::TempDir, Extension) {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/x/extension.yaml",
            &if manifest_tail.starts_with("manifest:") {
                format!("name: x\n{manifest_tail}")
            } else {
                format!("manifest: 2\nname: x\nintent:\n  purpose: test\n{manifest_tail}")
            },
        );
        for (slug, body) in files {
            write(
                dir.path(),
                &format!(".oxplow/extensions/x/pages/{slug}.yaml"),
                body,
            );
        }
        let ext = project_extensions(dir.path()).remove(0);
        (dir, ext)
    }

    /// A viz node shows its loader's rows, so it names one (`from`) that
    /// the page has; a form shows none, so it needs none.
    #[test]
    fn a_viz_node_names_its_loader_unless_it_shows_no_rows() {
        let (_d, ext) = load_x(
            &[
                (
                    "form",
                    "title: F\nview: { viz: form, form: { command: a.b.c } }\n",
                ),
                ("bare", "title: B\nview: { viz: table }\n"),
                ("stray", "title: S\nview: { viz: table, from: nope }\n"),
            ],
            "",
        );
        assert!(node(&ext, "form").is_some(), "{:?}", ext.errors);
        let errs = ext.errors.join("\n");
        for (slug, says) in [
            ("bare", "needs `from`: the loader whose rows it shows"),
            (
                "stray",
                "shows `nope`, but there is no loader `nope` in `loaders:`",
            ),
        ] {
            assert!(page_of(&ext, slug).is_none(), "{slug} loaded");
            assert!(
                errs.lines()
                    .any(|l| l.contains(&format!("{slug}.yaml")) && l.contains(says)),
                "{slug}: {errs}"
            );
        }
    }

    #[test]
    fn chart_nodes_parse_and_missing_fields_are_errors() {
        let (_d, ext) = load_x(
            &[
                (
                    "visits",
                    &viz_page(
                        "",
                        "SELECT 'a' AS day, 1 AS n",
                        "viz: bar\nchart: { x: day, y: n }\n",
                    ),
                ),
                (
                    "trend",
                    &viz_page(
                        "",
                        "SELECT 1 AS at, 2 AS v, 'm' AS s",
                        "viz: line\nchart: { x: at, y: v, series: s }\n",
                    ),
                ),
                (
                    "map",
                    &viz_page(
                        "",
                        "SELECT 'p' AS path, 3 AS churn, 'core' AS zone",
                        "viz: treemap\nchart: { label: path, size: churn, group: zone }\n",
                    ),
                ),
                ("nobar", &viz_page("", "SELECT 1", "viz: bar\n")),
            ],
            "",
        );
        let viz = |slug: &str| node(&ext, slug);
        assert_eq!(viz("visits").unwrap().viz, Viz::Bar);
        assert_eq!(
            viz("visits").unwrap().chart.as_ref().unwrap().y.as_deref(),
            Some("n")
        );
        assert_eq!(
            viz("trend")
                .unwrap()
                .chart
                .as_ref()
                .unwrap()
                .series
                .as_deref(),
            Some("s")
        );
        assert_eq!(
            viz("map").unwrap().chart.as_ref().unwrap().group.as_deref(),
            Some("zone")
        );
        assert!(viz("nobar").is_none());
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("nobar.yaml") && errs.contains("`chart: { x: <column>, y: <column> }`"),
            "{errs}"
        );
    }

    /// The structure components parse their blocks, and a missing column
    /// role is refused naming the block it goes under.
    #[test]
    fn structure_nodes_parse_and_missing_roles_are_errors() {
        let (_d, ext) = load_x(
            &[
                (
                    "t",
                    &viz_page(
                        "",
                        "SELECT 1 AS id, NULL AS p, 'a' AS n",
                        "viz: tree\ntree: { id: id, parent: p, label: n }\n",
                    ),
                ),
                (
                    "tl",
                    &viz_page(
                        "",
                        "SELECT 'x' AS at, 'y' AS w",
                        "viz: timeline\ntimeline: { at: at, label: w }\n",
                    ),
                ),
                ("d", &viz_page("", "SELECT 1 AS a", "viz: detail\n")),
                (
                    "s",
                    &viz_page("", "SELECT 'a' AS l", "viz: steps\nsteps: { label: l }\n"),
                ),
                (
                    "h",
                    &viz_page(
                        "",
                        "SELECT 'f' AS p, 'working' AS a, 'working' AS b",
                        "viz: hunks\nhunks: { path: p, from: a, to: b }\n",
                    ),
                ),
                (
                    "notree",
                    &viz_page("", "SELECT 1", "viz: tree\ntree: { id: id, label: n }\n"),
                ),
                ("nohunks", &viz_page("", "SELECT 1", "viz: hunks\n")),
            ],
            "",
        );
        let viz = |slug: &str| node(&ext, slug);
        for slug in ["t", "tl", "d", "s", "h"] {
            assert!(viz(slug).is_some(), "{slug}: {:?}", ext.errors);
        }
        assert_eq!(
            viz("t").unwrap().tree.as_ref().unwrap().parent.as_deref(),
            Some("p")
        );
        assert!(viz("notree").is_none() && viz("nohunks").is_none());
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("notree") && errs.contains("`tree: { parent: <column> }`"),
            "{errs}"
        );
        assert!(
            errs.contains("nohunks")
                && errs.contains("`hunks: { path: <column>, from: <column>, to: <column> }`"),
            "{errs}"
        );
    }

    /// A viz node fills its page when its viz takes the page's whole
    /// height (a file, a diff).
    #[test]
    fn a_viz_node_says_whether_it_fills() {
        let (_d, ext) = load_x(
            &[
                (
                    "c",
                    &viz_page(
                        "",
                        "SELECT 'a.rs' AS path, 'HEAD' AS rev",
                        "viz: code\ncode: { path: path, revision: rev }\n",
                    ),
                ),
                ("t", &viz_page("", "SELECT 1 AS n", "")),
            ],
            "",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert!(node(&ext, "c").unwrap().fills);
        assert!(!node(&ext, "t").unwrap().fills);
    }

    /// `section` is the page's view key: no param may take it.
    #[test]
    fn section_is_the_pages_view_key_not_a_param() {
        let (_d, ext) = load_x(
            &[(
                "a",
                &viz_page(
                    "params: [{ name: section, default: x }]\n",
                    "SELECT :section AS n",
                    "",
                ),
            )],
            "",
        );
        assert!(ext.page_files.is_empty());
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("`section` is the page's view key (`#section=`): rename the param"),
            "{errs}"
        );
    }

    /// A lazy tree (`tree.lazy: <param>`, `tree.branch: <column>`) runs one
    /// level at a time: its param needs a default (the top level) and can't
    /// be one the context supplies; `branch` says which rows open; a
    /// column that `select`s the lazy param would re-root the page instead.
    #[test]
    fn a_lazy_tree_names_its_param_and_its_branch_column() {
        let tree = |params: &str, node: &str| {
            viz_page(
                params,
                "SELECT 'a' AS id, '' AS p, 'a' AS n, 1 AS d",
                &format!("viz: tree\n{node}"),
            )
        };
        let folder = "params:\n  - { name: folder, default: \"\" }\n";
        let (_d, ext) = load_x(
            &[
                ("lazy", &tree(folder, "tree: { id: id, parent: p, label: n, lazy: folder, branch: d }\n")),
                ("undeclared", &tree("", "tree: { id: id, parent: p, label: n, lazy: nope, branch: d }\n")),
                ("nodefault", &tree("params:\n  - { name: folder }\n", "tree: { id: id, parent: p, label: n, lazy: folder, branch: d }\n")),
                ("context", &tree("params:\n  - { name: stream_id, default: 1 }\n", "tree: { id: id, parent: p, label: n, lazy: stream_id, branch: d }\n")),
                ("nobranch", &tree(folder, "tree: { id: id, parent: p, label: n, lazy: folder }\n")),
                ("branchonly", &tree("", "tree: { id: id, parent: p, label: n, branch: d }\n")),
                ("selectlazy", &tree(folder, "tree: { id: id, parent: p, label: n, lazy: folder, branch: d }\ncolumns: [{ key: n, select: folder }]\n")),
            ],
            "",
        );
        let viz = |slug: &str| node(&ext, slug);
        let lazy = viz("lazy").unwrap_or_else(|| panic!("{:?}", ext.errors));
        let t = lazy.tree.as_ref().unwrap();
        assert_eq!(
            (t.lazy.as_deref(), t.branch.as_deref()),
            (Some("folder"), Some("d"))
        );
        let errs = ext.errors.join("\n");
        for (slug, says) in [
            ("undeclared", "`tree.lazy` names param `nope`, which the viz doesn't declare"),
            ("nodefault", "`tree.lazy` param `folder` needs a `default`: the top level"),
            ("context", "`tree.lazy` can't be `stream_id`: the context supplies it"),
            ("nobranch", "`tree.lazy` needs `tree.branch: <column>` (the rows that open)"),
            ("branchonly", "`tree.branch` is for a lazy tree (`tree.lazy: <param>`)"),
            ("selectlazy", "column `n`: `select` names the tree's `lazy` param; a click opens the branch instead"),
        ] {
            assert!(viz(slug).is_none(), "{slug} loaded");
            assert!(
                errs.lines().any(|l| l.contains(slug) && l.contains(says)),
                "{slug}: {errs}"
            );
        }
    }

    /// `code` shows each row's file at a revision, `diff` the first row's
    /// file between two: each needs its block's columns, and the rest of
    /// the block is optional.
    #[test]
    fn code_and_diff_nodes_name_their_columns() {
        let (_d, ext) = load_x(
            &[
                (
                    "c",
                    &viz_page(
                        "",
                        "SELECT 'f' AS p, 'working' AS r, '3-8' AS l",
                        "viz: code\ncode: { path: p, revision: r, lines: l, sync: true }\n",
                    ),
                ),
                (
                    "c2",
                    &viz_page(
                        "",
                        "SELECT 'f' AS p, 'working' AS r",
                        "viz: code\ncode: { path: p, revision: r }\n",
                    ),
                ),
                (
                    "df",
                    &viz_page(
                        "",
                        "SELECT 'f' AS p, 'git:HEAD' AS a, 'working' AS b",
                        "viz: diff\ndiff: { path: p, from: a, to: b }\n",
                    ),
                ),
                (
                    "nocode",
                    &viz_page("", "SELECT 'f' AS p", "viz: code\ncode: { path: p }\n"),
                ),
                ("nodiff", &viz_page("", "SELECT 1", "viz: diff\n")),
            ],
            "",
        );
        let viz = |slug: &str| node(&ext, slug);
        let c = viz("c").expect("c loads");
        assert_eq!(
            c.code,
            Some(VizCode {
                path: Some("p".into()),
                filesystem: None,
                revision: Some("r".into()),
                lines: Some("l".into()),
                sync: true,
            })
        );
        assert!(!viz("c2").unwrap().code.as_ref().unwrap().sync);
        assert_eq!(
            viz("df").unwrap().diff.as_ref().unwrap().to.as_deref(),
            Some("b")
        );
        assert!(viz("nocode").is_none() && viz("nodiff").is_none());
        let errs = ext.errors.join("\n");
        assert!(errs.contains("`code: { revision: <column> }`"), "{errs}");
        assert!(
            errs.contains("`diff: { path: <column>, from: <column>, to: <column> }`"),
            "{errs}"
        );
    }

    /// `embed` opens the web page its first row names, filling the page;
    /// it needs the URL's column.
    #[test]
    fn an_embed_node_names_its_url_and_fills() {
        let (_d, ext) = load_x(
            &[
                (
                    "e",
                    &viz_page(
                        "",
                        "SELECT 'https://a.test/x' AS u",
                        "viz: embed\nembed: { url: u }\n",
                    ),
                ),
                ("noembed", &viz_page("", "SELECT 1", "viz: embed\n")),
            ],
            "",
        );
        let e = node(&ext, "e").expect("e loads");
        assert_eq!(
            e.embed,
            Some(VizEmbed {
                url: Some("u".into())
            })
        );
        assert!(e.fills);
        assert!(node(&ext, "noembed").is_none());
        let errs = ext.errors.join("\n");
        assert!(errs.contains("`embed: { url: <column> }`"), "{errs}");
    }

    /// `editor` edits the working tree's file its first row names (its
    /// `lines` revealed), filling the page; it needs the path's column,
    /// and an inline page never holds one (it would save as the person).
    #[test]
    fn an_editor_node_names_its_file_and_fills() {
        let (_d, ext) = load_x(
            &[
                (
                    "e",
                    &viz_page(
                        "",
                        "SELECT 'src/a.rs' AS p, '3' AS l",
                        "viz: editor\neditor: { path: p, lines: l }\n",
                    ),
                ),
                ("noeditor", &viz_page("", "SELECT 1", "viz: editor\n")),
            ],
            "",
        );
        let e = node(&ext, "e").expect("e loads");
        assert_eq!(
            e.editor,
            Some(VizEditor {
                path: Some("p".into()),
                filesystem: None,
                lines: Some("l".into()),
            })
        );
        assert!(e.fills);
        assert!(node(&ext, "noeditor").is_none());
        let errs = ext.errors.join("\n");
        assert!(errs.contains("`editor: { path: <column> }`"), "{errs}");
        let inline = crate::pages::schema::parse_inline_page(&viz_page(
            "",
            "SELECT 'a' AS p",
            "viz: editor\neditor: { path: p }\n",
        ))
        .unwrap_err();
        assert!(inline.contains("can't hold an editor"), "{inline}");
    }

    /// `terminal` draws a terminal per row (a shell's URI, or an agent
    /// session's), filling the page; it needs the URI's column, and an
    /// inline page never holds one.
    #[test]
    fn a_terminal_node_names_its_rows() {
        let (_d, ext) = load_x(
            &[
                (
                    "t",
                    &viz_page(
                        "",
                        "SELECT 'oxplow:oxplow/terminal?id=trm1' AS u, 'One' AS t, NULL AS c",
                        "viz: terminal\nterminal: { of: u, title: t, conversation: c }\n",
                    ),
                ),
                ("noterminal", &viz_page("", "SELECT 1", "viz: terminal\n")),
            ],
            "",
        );
        let t = node(&ext, "t").expect("t loads");
        assert_eq!(
            t.terminal,
            Some(VizTerminal {
                of: Some("u".into()),
                title: Some("t".into()),
                conversation: Some("c".into()),
                thread: None,
            })
        );
        assert!(t.fills);
        assert!(node(&ext, "noterminal").is_none());
        assert!(ext
            .errors
            .join("\n")
            .contains("`terminal: { of: <column> }`"));
        let inline = crate::pages::schema::parse_inline_page(&viz_page(
            "",
            "SELECT 'u' AS u",
            "viz: terminal\nterminal: { of: u }\n",
        ))
        .unwrap_err();
        assert!(inline.contains("can't hold a terminal"), "{inline}");
    }

    /// A param's `options` are the values it takes: its default
    /// must be one, and a run supplying another is refused.
    #[test]
    fn param_options_bound_their_values() {
        let (_d, ext) = load_x(
            &[
                ("ok", &viz_page("params:\n  - { name: mode, default: recent, options: [{ value: recent, label: Recent }, { value: top, label: Most visited }] }\n", "SELECT :mode AS m", "viz: list\n")),
                ("bad", &viz_page("params:\n  - { name: mode, default: all, options: [{ value: recent, label: Recent }] }\n", "SELECT :mode AS m", "viz: list\n")),
            ],
            "",
        );
        let ok = page_of(&ext, "ok").expect("ok loads");
        assert_eq!(ok.params[0].options[1].label, "Most visited");
        assert!(page_of(&ext, "bad").is_none());
        let errs = ext.errors.join("\n");
        assert!(errs.contains("`mode`") && errs.contains("`all`"), "{errs}");
        let ok = node(&ext, "ok").unwrap();
        let supplied = BTreeMap::from([("mode".to_string(), SqlCell::Text("top".into()))]);
        assert!(resolve_params(ok, &supplied, &ViewContext::default()).is_ok());
        let supplied = BTreeMap::from([("mode".to_string(), SqlCell::Text("all".into()))]);
        assert!(resolve_params(ok, &supplied, &ViewContext::default()).is_err());
    }

    /// A param's `type` says what it holds and how a page draws it; with
    /// none it's a choice when it has options, else text. What doesn't fit
    /// its type is refused at load.
    #[test]
    fn typed_params_are_checked_at_load() {
        let one = |params: &str| viz_page(params, "SELECT 1 AS a", "");
        let (_d, ext) = load_x(
            &[
                ("ok", &one("params:\n  - { name: range, type: date_range, presets: [7d, all], default: 7d }\n  - { name: flag, type: bool, default: true }\n  - { name: q, type: search }\n  - { name: br, type: branch }\n  - { name: mode, default: a, options: [{ value: a, label: A }] }\n  - { name: free }\n  - { name: end, options_from: ends }\n  - { name: start, type: choice, options_from: other/ends, default: anything }\n")),
                ("ends", &viz_page("title: Ends\n", "SELECT 'snap:1' AS value, 'One' AS label", "")),
                ("choice_without_options", &one("params: [{ name: m, type: choice }]\n")),
                ("both_option_sources", &one("params: [{ name: m, options_from: x/ends, options: [{ value: a, label: A }] }]\n")),
                ("options_from_on_text", &one("params: [{ name: m, type: text, options_from: x/ends }]\n")),
                ("options_from_missing", &one("params: [{ name: m, options_from: nowhere }]\n")),
                ("options_from_itself", &one("params: [{ name: m, options_from: x/options_from_itself }]\n")),
                ("options_on_text", &one("params: [{ name: m, type: text, options: [{ value: a, label: A }] }]\n")),
                ("presets_on_text", &one("params: [{ name: m, presets: [7d] }]\n")),
                ("unknown_preset", &one("params: [{ name: m, type: date_range, presets: [5y] }]\n")),
                ("bad_range", &one("params: [{ name: m, type: date_range, default: soon }]\n")),
                ("bad_bool", &one("params: [{ name: m, type: bool, default: maybe }]\n")),
                ("placed", &one("params: [{ name: m, place: toolbar }]\n")),
                ("roled", &one("params: [{ name: m, role: selection }]\n")),
            ],
            "",
        );
        let ok = page_of(&ext, "ok").expect("ok loads");
        let kinds: Vec<_> = ok.params.iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            [
                ParamType::DateRange,
                ParamType::Bool,
                ParamType::Search,
                ParamType::Branch,
                ParamType::Choice,
                ParamType::Text,
                ParamType::Choice,
                ParamType::Choice,
            ]
            .map(Some)
        );
        // A dynamic choice's page is named whole, as `expand`'s is; its
        // options aren't known at load, so its default (if any) isn't
        // checked, and another extension's page isn't either.
        let from: Vec<_> = ok
            .params
            .iter()
            .skip(6)
            .map(|p| p.options_from.as_deref())
            .collect();
        assert_eq!(from, [Some("x/ends"), Some("other/ends")]);
        let errs = ext.errors.join("\n");
        for (slug, says) in [
            (
                "choice_without_options",
                "needs `options` or `options_from`",
            ),
            (
                "both_option_sources",
                "`options` or `options_from`, not both",
            ),
            (
                "options_from_on_text",
                "`options_from` is only for a choice",
            ),
            (
                "options_from_missing",
                "`options_from` page `x/nowhere` isn't in this extension's pages/",
            ),
            (
                "options_from_itself",
                "`options_from` names the page itself",
            ),
            ("options_on_text", "`options` are only for a choice"),
            ("presets_on_text", "`presets` are only for a date_range"),
            ("unknown_preset", "`5y`"),
            ("bad_range", "`soon`"),
            ("bad_bool", "`maybe`"),
            // A page draws every param in its rail; where is not the
            // author's to say, and a selection follows from the columns.
            ("placed", "unknown field `place`"),
            ("roled", "`role: selection` follows from the columns"),
        ] {
            assert!(page_of(&ext, slug).is_none(), "{slug} loaded");
            assert!(
                errs.lines()
                    .any(|e| e.contains(&format!("{slug}.yaml")) && e.contains(says)),
                "{slug}: {errs}"
            );
        }
    }
    /// A choice whose options come from a page takes any text (the query
    /// says what an unknown value means) and NULL, as every param.
    #[test]
    fn a_dynamic_choice_takes_any_text() {
        let p = PageParam {
            name: "end".into(),
            label: None,
            default: None,
            options: Vec::new(),
            options_from: Some("x/ends".into()),
            kind: None,
            role: None,
            presets: Vec::new(),
        };
        assert_eq!(p.kind(), ParamType::Choice);
        assert_eq!(
            p.take(&SqlCell::Text("snap:9".into())),
            Ok(SqlCell::Text("snap:9".into()))
        );
        assert_eq!(p.take(&SqlCell::Null(())), Ok(SqlCell::Null(())));
        assert!(p.take(&SqlCell::Int(3)).unwrap_err().contains("text"));
    }

    /// A bool binds 1 or 0, a search its trimmed text or NULL, and a
    /// date_range its own text plus `:<name>_from` / `:<name>_to`: a
    /// preset back from now, a custom range's days with the last one
    /// included, `all` neither end. What doesn't fit is refused.
    #[tokio::test]
    async fn typed_params_bind_by_type() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/typed.yaml",
            &viz_page(
                "title: Typed\nparams:\n  - { name: flag, type: bool, default: false }\n  - { name: q, type: search }\n  - { name: range, type: date_range, default: all }\n",
                "SELECT :flag AS flag, :q AS q, :range AS range, :range_from AS f, :range_to AS t",
                "",
            ),
        );
        let svc = services(dir.path()).await;
        let run = |over: Vec<(&str, &str)>| {
            let supplied: BTreeMap<String, SqlCell> = over
                .into_iter()
                .map(|(k, v)| (k.to_string(), SqlCell::Text(v.to_string())))
                .collect();
            let svc = svc.clone();
            async move { run_page(&svc, "review/typed", supplied, ViewContext::default()).await }
        };
        let cell =
            |r: &crate::pages::runner::PageRun, i: usize| r.loaders["q"].result.rows[0][i].clone();

        let first = run(vec![]).await.unwrap();
        assert_eq!(
            rows_of(&first, "q"),
            serde_json::json!([[0, null, "all", null, null]])
        );
        assert!(!first.params.contains_key("range_from"));

        let custom = run(vec![
            ("flag", "true"),
            ("q", "  fix "),
            ("range", "2026-10-01..2026-10-20"),
        ])
        .await
        .unwrap();
        assert_eq!(
            rows_of(&custom, "q"),
            serde_json::json!([[
                1,
                "fix",
                "2026-10-01..2026-10-20",
                "2026-10-01",
                "2026-10-21"
            ]])
        );
        assert_eq!(
            cell(&run(vec![("q", "  ")]).await.unwrap(), 1),
            SqlCell::Null(())
        );

        let week = run(vec![("range", "7d")]).await.unwrap();
        let SqlCell::Text(from) = cell(&week, 3) else {
            panic!("7d binds a from: {:?}", week.loaders["q"].result.rows)
        };
        let from =
            time::OffsetDateTime::parse(&from, &time::format_description::well_known::Rfc3339)
                .expect("an RFC 3339 time");
        let back = time::OffsetDateTime::now_utc() - from;
        assert!(
            back > time::Duration::days(7) - time::Duration::minutes(1)
                && back < time::Duration::days(7) + time::Duration::minutes(1),
            "{back}"
        );
        assert_eq!(cell(&week, 4), SqlCell::Null(()));

        for (k, v) in [
            ("flag", "maybe"),
            ("range", "soon"),
            ("range", "2026-10-20..2026-10-01"),
        ] {
            assert!(run(vec![(k, v)]).await.is_err(), "{k}={v} was taken");
        }
    }

    /// A column's `select` names a param the page declares.
    #[test]
    fn a_select_names_a_param_its_page_holds() {
        let (_d, ext) = load_x(
            &[
                (
                    "rows",
                    &viz_page(
                        "params: [{ name: commit }]\n",
                        "SELECT 'a' AS sha",
                        "columns: [{ key: sha, select: commit }]\n",
                    ),
                ),
                (
                    "nope",
                    &viz_page(
                        "",
                        "SELECT 'a' AS sha",
                        "columns: [{ key: sha, select: commit }]\n",
                    ),
                ),
            ],
            "",
        );
        let rows = node(&ext, "rows").expect("rows loads");
        assert_eq!(rows.columns[0].select.as_deref(), Some("commit"));
        assert!(page_of(&ext, "nope").is_none());
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("nope.yaml") && errs.contains("`select: commit`"),
            "{errs}"
        );
    }

    /// A placeholder's `|json` filter loads; any other filter is refused.
    #[test]
    fn an_action_placeholder_may_only_filter_with_json() {
        let (_d, ext) = load_x(
            &[
                ("ok", &viz_page("", "SELECT '[1]' AS order_json", "actions:\n  - { id: a, label: A, command: oxplow.agent.set_agents, row: true, input: { agents: \"{{row.order_json|json}}\" } }\n")),
                ("bad", &viz_page("", "SELECT 1 AS n", "actions:\n  - { id: a, label: A, command: oxplow.agent.set_agents, row: true, input: { agents: \"{{row.n|upper}}\" } }\n")),
            ],
            "",
        );
        assert!(page_of(&ext, "ok").is_some(), "{:?}", ext.errors);
        assert!(page_of(&ext, "bad").is_none());
        assert!(
            ext.errors
                .join("\n")
                .contains("unknown placeholder filter `upper`"),
            "{:?}",
            ext.errors
        );
    }

    /// A column's `expand` names a page (by slug or whole id); one of this
    /// extension's must be in pages/ and not be the page itself (another
    /// extension's is found when it runs).
    #[test]
    fn an_expand_names_a_page() {
        let expands = |to: &str| {
            viz_page(
                "",
                "SELECT 1 AS id",
                &format!("columns: [{{ key: id, expand: {to} }}]\n"),
            )
        };
        let (_d, ext) = load_x(
            &[
                (
                    "detail",
                    &viz_page("params: [{ name: id }]\n", "SELECT :id AS n", ""),
                ),
                ("by-id", &expands("x/detail")),
                ("elsewhere", &expands("other/detail")),
                ("by-slug", &expands("detail")),
                ("missing", &expands("gone")),
                ("itself", &expands("itself")),
            ],
            "",
        );
        for slug in ["by-id", "by-slug", "elsewhere"] {
            assert!(
                node(&ext, slug).is_some(),
                "{slug} didn't load: {:?}",
                ext.errors
            );
        }
        // Named whole, whichever way it was written.
        for slug in ["by-id", "by-slug"] {
            assert_eq!(
                node(&ext, slug).unwrap().columns[0].expand.as_deref(),
                Some("x/detail")
            );
        }
        let errs = ext.errors.join("\n");
        for (slug, says) in [
            (
                "missing",
                "`expand` page `x/gone` isn't in this extension's pages/",
            ),
            ("itself", "`expand` names the page itself"),
        ] {
            assert!(page_of(&ext, slug).is_none(), "{slug} loaded");
            assert!(
                errs.lines()
                    .any(|e| e.contains(&format!("{slug}.yaml")) && e.contains(says)),
                "{slug}: {errs}"
            );
        }
    }

    /// `targets` draws target lines on a `line` chart; on another viz it is
    /// refused. The formats `sparkline` and `badge` load.
    #[test]
    fn chart_targets_and_the_new_formats_load() {
        let (_d, ext) = load_x(
            &[
                (
                    "trend",
                    &viz_page(
                        "",
                        "SELECT 1 AS at, 2 AS v, 3 AS goal, '[1,2]' AS s, 'ok' AS level",
                        "viz: line\nchart: { x: at, y: v, targets: goal }\ncolumns: [{ key: s, format: sparkline }, { key: level, format: badge }]\n",
                    ),
                ),
                (
                    "bars",
                    &viz_page(
                        "",
                        "SELECT 1 AS at, 2 AS v, 3 AS goal",
                        "viz: bar\nchart: { x: at, y: v, targets: goal }\n",
                    ),
                ),
            ],
            "",
        );
        let trend =
            node(&ext, "trend").unwrap_or_else(|| panic!("trend didn't load: {:?}", ext.errors));
        assert_eq!(
            trend.chart.as_ref().unwrap().targets.as_deref(),
            Some("goal")
        );
        assert_eq!(trend.columns[0].format, Some(ColumnFormat::Sparkline));
        assert_eq!(trend.columns[1].format, Some(ColumnFormat::Badge));
        assert!(page_of(&ext, "bars").is_none());
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("bars.yaml") && errs.contains("`targets`"),
            "{errs}"
        );
    }

    /// A param a column selects is a selection, on the page and on the
    /// node whose rows set it; one that says `role: filter` is a filter;
    /// every other param names the resource, and says no role.
    #[test]
    fn a_params_role_is_resource_unless_it_filters_or_a_column_selects_it() {
        let (_d, ext) = load_x(
            &[(
                "rows",
                &viz_page(
                    "params: [{ name: commit }, { name: q, type: search, role: filter }, { name: stream_id }]\n",
                    "SELECT 'a' AS sha, :q AS q, :stream_id AS s",
                    "columns: [{ key: sha, select: commit }]\n",
                ),
            )],
            "",
        );
        let page = page_of(&ext, "rows").unwrap_or_else(|| panic!("{:?}", ext.errors));
        let role = |name: &str| page.params.iter().find(|p| p.name == name).unwrap().role;
        assert_eq!(role("commit"), Some(ParamRole::Selection));
        assert_eq!(role("q"), Some(ParamRole::Filter));
        // No role is the resource's (a renderer takes it from its URI).
        assert_eq!(role("stream_id"), None);
    }

    /// A selection follows from the columns: a page that writes one is
    /// refused.
    #[test]
    fn a_written_selection_role_is_refused() {
        let (_d, ext) = load_x(
            &[(
                "rows",
                &viz_page(
                    "params: [{ name: commit, role: selection }]\n",
                    "SELECT 1 AS n",
                    "",
                ),
            )],
            "",
        );
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("`role: selection` follows from the columns"),
            "{errs}"
        );
    }

    /// A page says where it comes from: the source file to edit, relative
    /// to the project, and the copy that runs. Loaded from its source,
    /// they are the same file.
    #[test]
    fn a_page_from_its_source_names_that_file() {
        let (_d, ext) = load_x(&[("ok", &viz_page("", "SELECT 1 AS a", ""))], "");
        let origin = &ext.page_files[0].origin;
        assert_eq!(origin.extension, "x");
        assert_eq!(
            origin.source.as_deref(),
            Some(".oxplow/extensions/x/pages/ok.yaml")
        );
        assert_eq!(origin.running, ".oxplow/extensions/x/pages/ok.yaml");
        assert_eq!(origin.deployment, None);
        assert!(!origin.bundled);
    }

    /// A deployed page runs from its copy in the store, but its origin
    /// names the source to edit and the deployment it came from.
    #[test]
    fn a_deployed_page_names_its_source_and_its_deployment() {
        let (d, _) = load_x(&[("ok", &viz_page("", "SELECT 1 AS a", ""))], "");
        let source = d.path().join(".oxplow/extensions/x");
        let hash = crate::extension::extension_store::source_hash(&source).unwrap();
        crate::extension::extension_store::copy_into_store(d.path(), "x", &source, &hash).unwrap();
        let mut settings = oxplow_config::extension_settings(d.path());
        settings.personal.enable.push("x".into());
        let live = BTreeMap::from([("x".to_string(), hash.clone())]);
        let ext = load_deployed(d.path(), &settings, &live)
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap();
        let origin = &ext.page_files[0].origin;
        assert_eq!(
            origin.source.as_deref(),
            Some(".oxplow/extensions/x/pages/ok.yaml")
        );
        assert!(
            origin.running.ends_with(&format!("x/{hash}/pages/ok.yaml")),
            "{}",
            origin.running
        );
        assert_eq!(origin.deployment.as_deref(), Some(hash.as_str()));
        assert!(!origin.bundled);
        // A node's config names the same origin as its page.
        assert_eq!(&node(&ext, "ok").unwrap().origin, origin);
        // An edit to the source shows until it is deployed.
        assert!(!origin.out_of_date(d.path()));
        write(
            d.path(),
            ".oxplow/extensions/x/pages/ok.yaml",
            &viz_page("title: Edited\n", "SELECT 1 AS a", ""),
        );
        assert!(origin.out_of_date(d.path()));
        assert_eq!(origin.to_json(d.path())["outOfDate"], true);
    }

    /// The layout warnings of one page file: its loaded extension's
    /// warnings, and whether the page still loaded.
    fn layout_of(page: &str) -> (Vec<String>, bool) {
        let (_d, ext) = load_x(&[("p", page)], "");
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        (layout_only(&ext.warnings), page_of(&ext, "p").is_some())
    }

    /// The layout warnings among `warnings`.
    fn layout_only(warnings: &[String]) -> Vec<String> {
        const MARKS: [&str; 6] = [
            " filters; ",
            "isn't read by any loader",
            " options — ",
            "the description is",
            " sections — ",
            " columns — ",
        ];
        warnings
            .iter()
            .filter(|w| MARKS.iter().any(|m| w.contains(m)))
            .cloned()
            .collect()
    }

    fn filters(n: usize) -> String {
        let params: Vec<String> = (0..n)
            .map(|i| format!("{{ name: f{i}, role: filter }}"))
            .collect();
        let reads: Vec<String> = (0..n).map(|i| format!(":f{i}")).collect();
        viz_page(
            &format!("params: [{}]\n", params.join(", ")),
            &format!("SELECT {} AS a", reads.join(" || ")),
            "",
        )
    }

    #[test]
    fn more_than_six_filters_warn_and_six_do_not() {
        let (warns, loaded) = layout_of(&filters(7));
        assert!(loaded);
        assert!(
            warns.iter().any(|w| w.starts_with(
                ".oxplow/extensions/x/pages/p.yaml: 7 filters; a page shows 6 at most — fold"
            )),
            "{warns:?}"
        );
        assert!(layout_of(&filters(6)).0.is_empty());
    }

    fn choice(n: usize) -> String {
        let options: Vec<String> = (0..n)
            .map(|i| format!("{{ value: v{i}, label: V{i} }}"))
            .collect();
        viz_page(
            &format!(
                "params: [{{ name: c, default: v0, options: [{}] }}]\n",
                options.join(", ")
            ),
            "SELECT :c AS a",
            "",
        )
    }

    #[test]
    fn a_choice_of_more_than_twelve_options_warns() {
        let (warns, loaded) = layout_of(&choice(13));
        assert!(loaded);
        assert!(
            warns.iter().any(|w| w.ends_with(
                "param `c` has 13 options — use a `search`, or a page loader that lists them (`options_from`)"
            )),
            "{warns:?}"
        );
        assert!(layout_of(&choice(12)).0.is_empty());
    }

    #[test]
    fn a_param_no_loader_reads_warns_and_a_token_is_whole() {
        let (warns, loaded) = layout_of(&viz_page(
            "params: [{ name: q }, { name: r }]\n",
            "SELECT :q AS a, :r_x AS b",
            "",
        ));
        assert!(loaded);
        assert_eq!(warns.len(), 1, "{warns:?}");
        assert!(
            warns[0].ends_with(
                "param `r` isn't read by any loader of the page — remove it, or read it as `:r`"
            ),
            "{warns:?}"
        );
        // `:q` at the end of the text counts.
        assert!(layout_of(&viz_page(
            "params: [{ name: q }]\n",
            "SELECT 1 AS a WHERE 1 = :q",
            ""
        ))
        .0
        .is_empty());
    }

    /// A param a column `select`s or `highlight`s is in use: a click sets
    /// it, or the cells mark it, whether or not a loader reads it.
    #[test]
    fn a_param_a_column_selects_or_highlights_is_read() {
        let (warns, loaded) = layout_of(&viz_page(
            "params: [{ name: commit }, { name: q }]\n",
            "SELECT 'a' AS sha, 'b' AS s",
            "columns: [{ key: sha, select: commit }, { key: s, highlight: q }]\n",
        ));
        assert!(loaded);
        assert!(warns.is_empty(), "{warns:?}");
    }

    /// What a deployed extension reports names its source files, the
    /// ones to edit, not the copy in the store.
    #[test]
    fn a_deployed_extension_s_warnings_name_its_source() {
        let (d, _) = load_x(
            &[(
                "ok",
                &viz_page("params: [{ name: z }]\n", "SELECT 1 AS a", ""),
            )],
            "",
        );
        let source = d.path().join(".oxplow/extensions/x");
        let hash = crate::extension::extension_store::source_hash(&source).unwrap();
        crate::extension::extension_store::copy_into_store(d.path(), "x", &source, &hash).unwrap();
        let mut settings = oxplow_config::extension_settings(d.path());
        settings.personal.enable.push("x".into());
        let live = BTreeMap::from([("x".to_string(), hash.clone())]);
        let ext = load_deployed(d.path(), &settings, &live)
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap();
        let unread: Vec<&String> = ext
            .warnings
            .iter()
            .filter(|w| w.contains("param `z`"))
            .collect();
        assert_eq!(unread.len(), 1, "{:?}", ext.warnings);
        assert!(
            unread[0].starts_with(".oxplow/extensions/x/pages/ok.yaml: "),
            "{unread:?}"
        );
        assert!(
            ext.warnings.iter().all(|w| !w.contains(&hash)),
            "{:?}",
            ext.warnings
        );
    }

    #[test]
    fn a_range_is_read_through_its_from_or_to() {
        let page = |read: &str| {
            viz_page(
                "params: [{ name: range, type: date_range, default: all }]\n",
                &format!("SELECT 1 AS a WHERE 1 {read}"),
                "",
            )
        };
        assert!(layout_of(&page("OR :range_from IS NULL")).0.is_empty());
        assert!(layout_of(&page("OR :range_to IS NULL")).0.is_empty());
        assert!(layout_of(&page("OR :range = 'all'")).0.is_empty());
    }

    /// A param a node's or the page's action input names is read.
    #[test]
    fn an_action_input_reads_a_param() {
        let (_d, ext) = load_x(
            &[
                (
                    "node",
                    &viz_page(
                        "params: [{ name: k }]\n",
                        "SELECT 1 AS a",
                        "actions:\n  - { id: go, label: Go, command: oxplow.work_item.transition, input: { uri: '{{param.k}}' } }\n",
                    ),
                ),
                (
                    "page",
                    &viz_page(
                        "params: [{ name: k }]\nactions:\n  - { id: go, label: Go, command: oxplow.work_item.transition, input: { uri: '{{param.k}}' } }\n",
                        "SELECT 1 AS a",
                        "",
                    ),
                ),
            ],
            "",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert!(
            ext.warnings.iter().all(|w| !w.contains("isn't read")),
            "{:?}",
            ext.warnings
        );
    }

    #[test]
    fn a_long_description_warns() {
        let page = |n: usize| {
            viz_page(
                &format!("description: {}\n", "d".repeat(n)),
                "SELECT 1 AS a",
                "",
            )
        };
        let (warns, loaded) = layout_of(&page(201));
        assert!(loaded);
        assert!(
            warns.iter().any(|w| w.ends_with("the description is 201 characters — keep it to a sentence or two; put the rest in a markdown node")),
            "{warns:?}"
        );
        assert!(layout_of(&page(200)).0.is_empty());
    }

    #[test]
    fn a_stack_of_more_than_eight_sections_warns() {
        let stack = |n: usize| {
            let nodes: Vec<String> = (0..n)
                .map(|i| format!("    - {{ id: s{i}, title: S{i}, markdown: hi }}\n"))
                .collect();
            format!("title: P\nview:\n  stack:\n{}", nodes.concat())
        };
        let (warns, loaded) = layout_of(&stack(9));
        assert!(loaded);
        assert!(
            warns
                .iter()
                .any(|w| w.ends_with("9 sections — a page reads best with 8 at most; split it")),
            "{warns:?}"
        );
        assert!(layout_of(&stack(8)).0.is_empty());
    }

    #[test]
    fn a_table_of_more_than_eight_columns_warns() {
        let table = |n: usize| {
            let cols: Vec<String> = (0..n).map(|i| format!("  - {{ key: c{i} }}")).collect();
            let sel: Vec<String> = (0..n).map(|i| format!("1 AS c{i}")).collect();
            viz_page(
                "",
                &format!("SELECT {}", sel.join(", ")),
                &format!("columns:\n{}\n", cols.join("\n")),
            )
        };
        let (warns, loaded) = layout_of(&table(9));
        assert!(loaded);
        assert!(
            warns.iter().any(|w| w.ends_with(
                "node `q`: 9 columns — show 8 at most; put the rest in a node a row `select`s"
            )),
            "{warns:?}"
        );
        assert!(layout_of(&table(8)).0.is_empty());
    }

    #[tokio::test]
    async fn validate_returns_the_layout_warning() {
        let (d, _) = load_x(&[("p", &filters(7))], "");
        let v = validate_extension(&layer().await, &cat(), d.path(), "x", None)
            .await
            .unwrap();
        assert!(
            v.warnings.iter().any(|w| w.contains("7 filters")),
            "{:?}",
            v.warnings
        );
        assert!(page_of(&v, "p").is_some());
    }

    /// A page in the launcher (a `category`, not hidden) opens with no
    /// params given, so one without a default, a filter or selection role,
    /// options from a loader, or a viewer's `stream_id` / `thread_id` /
    /// `head_sha` opens empty: the check names it at its line.
    #[test]
    fn a_launcher_page_with_a_param_nobody_fills_gets_a_warning() {
        let page = |head: &str, params: &str| {
            viz_page(
                &format!("category: Work\nparams:\n{params}{head}"),
                "SELECT :effort_id || :state || :q || :thread_id AS a",
                "",
            )
        };
        let params = "  - { name: effort_id, label: Effort }\n  - { name: state, default: open }\n  - { name: q, role: filter }\n  - { name: thread_id }\n";
        let (_d, ext) = load_x(&[("p", &page("", params))], "");
        let warned: Vec<&String> = ext
            .warnings
            .iter()
            .filter(|w| w.contains("opens empty"))
            .collect();
        assert_eq!(warned.len(), 1, "{:?}", ext.warnings);
        assert!(
            warned[0].contains("/pages/p.yaml:4: param `effort_id` opens empty from the launcher"),
            "{}",
            warned[0]
        );
        assert!(warned[0].contains(" — fix: "), "{}", warned[0]);
        // Hidden, or in no category, it isn't opened from the launcher.
        let (_d, ext) = load_x(&[("p", &page("hidden: true\n", params))], "");
        assert!(
            ext.warnings.iter().all(|w| !w.contains("opens empty")),
            "{:?}",
            ext.warnings
        );
        let uncategorized = viz_page(
            &format!("params:\n{params}"),
            "SELECT :effort_id || :state || :q || :thread_id AS a",
            "",
        );
        let (_d, ext) = load_x(&[("p", &uncategorized)], "");
        assert!(ext.warnings.iter().all(|w| !w.contains("opens empty")));
    }

    /// What the app ships follows its own page-layout rules.
    #[test]
    fn the_bundled_pages_have_no_layout_warning() {
        let dir = tempfile::tempdir().unwrap();
        let exts = load_extensions(dir.path());
        let bundled: Vec<&Extension> = exts.iter().filter(|e| e.origin == "bundled").collect();
        assert_eq!(bundled.len(), 2);
        for e in bundled {
            let layout = layout_only(&e.warnings);
            assert!(layout.is_empty(), "{}: {layout:#?}", e.name);
        }
    }
    /// A check names each query that reads a physical table (a page's
    /// loader, an advisory, a collector's `input`): only the published
    /// views are the extension's to read, and a table is not one of them.
    #[tokio::test]
    async fn a_check_names_a_query_that_reads_a_table() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/tbl/extension.yaml",
            "manifest: 2\nname: tbl\nintent:\n  purpose: test\nadvisories:\n  - id: peek\n    on: turn-end\n    query: SELECT title AS message FROM work_item\ncollectors:\n  - id: sum\n    runtime: starlark\n    entry: sum.star\n    input: SELECT title FROM work_item\n    entities:\n      - { name: notes, key: n, columns: { n: int } }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/tbl/sum.star",
            "def transform(x):\n    return {\"notes\": []}\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/tbl/pages/all.yaml",
            &viz_page("title: All\n", "SELECT title FROM work_item", ""),
        );
        let e = validate_extension(&layer().await, &cat(), dir.path(), "tbl", None)
            .await
            .unwrap();
        for what in ["advisory peek", "loader `q`", "collector `sum` input"] {
            let found = e
                .errors
                .iter()
                .find(|m| m.contains(what) && m.contains("`work_item` is a physical table"));
            let found = found.unwrap_or_else(|| panic!("{what}: {:?}", e.errors));
            assert!(found.contains("read v_work_item"), "{found}");
        }
    }

    /// Validation checks every block's columns, not only the chart's.
    #[tokio::test]
    async fn validate_checks_structure_columns_exist() {
        let (d, _) = load_x(
            &[(
                "a",
                &viz_page(
                    "",
                    "SELECT 'x' AS l",
                    "viz: steps\nsteps: { label: l, status: gone }\n",
                ),
            )],
            "",
        );
        let v = validate_extension(&layer().await, &cat(), d.path(), "x", None)
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        assert!(
            errs.contains("a.yaml: node `q`: steps column `gone`"),
            "{errs}"
        );
    }

    #[tokio::test]
    async fn validate_checks_chart_and_link_columns_exist() {
        let (d, _) = load_x(
            &[
                (
                    "a",
                    &viz_page(
                        "",
                        "SELECT 'x' AS day, 1 AS n",
                        "viz: bar\nchart: { x: day, y: missing }\n",
                    ),
                ),
                (
                    "b",
                    &viz_page(
                        "",
                        "SELECT 'f.rs' AS path",
                        "columns:\n  - { key: path, link: { kind: file, line: ln } }\n",
                    ),
                ),
            ],
            "",
        );
        let v = validate_extension(&layer().await, &cat(), d.path(), "x", None)
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        assert!(
            errs.contains("`missing`") && errs.contains("`ln`"),
            "{errs}"
        );
    }

    #[test]
    fn new_link_kinds_parse() {
        let (_d, ext) = load_x(
            &[(
                "l",
                &viz_page(
                    "",
                    "SELECT 1",
                    "columns:\n  - { key: a, link: { kind: commit } }\n  - { key: b, link: { kind: metric } }\n  - { key: p, link: { kind: page } }\n  - { key: d, link: { kind: file, line: n } }\n",
                ),
            )],
            "",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let l = node(&ext, "l").unwrap();
        let kinds: Vec<LinkKind> = l
            .columns
            .iter()
            .map(|c| c.link.as_ref().unwrap().kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                LinkKind::Commit,
                LinkKind::Metric,
                LinkKind::Page,
                LinkKind::File
            ]
        );
        assert_eq!(
            l.columns[3].link.as_ref().unwrap().line.as_deref(),
            Some("n")
        );
    }

    #[test]
    fn this_repos_own_extension_loads_clean() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let ext = project_extensions(&repo)
            .into_iter()
            .find(|e| e.name == "project")
            .expect("the repo's own extension");
        assert_eq!(ext.errors, Vec::<String>::new());
        assert!(ext.is_own());
        assert!(ext
            .collectors
            .iter()
            .any(|c| c.id == "tests.rust_junit" && c.records.is_some()));
        assert!(ext.collectors.iter().any(|c| c.id == "repo.scan_clone"));
    }

    #[test]
    fn an_extension_metric_names_its_own_collector_and_keeps_out_of_oxplow_keys() {
        let (_d, ext) = load_x(
            &[],
            "measures:\n  - { key: oxplow.loc, title: Lines }\n\
             metrics:\n  - { key: acme.loc, title: Lines, measure: acme.x, expression: \"SUM(value)\", collector: acme.nope }\n",
        );
        assert!(ext.metrics.is_empty(), "{:?}", ext.metrics);
        for needle in [
            "uses the reserved",
            "collector `acme.nope` is not a collector",
        ] {
            assert!(
                ext.errors.iter().any(|e| e.contains(needle)),
                "{needle}: {:?}",
                ext.errors
            );
        }
    }

    #[test]
    fn extensions_declare_measures_metrics_and_fact_collectors() {
        let manifest = [
            "measures:",
            "  - { key: acme.todo, title: TODOs }",
            "metrics:",
            "  - { key: acme.todos, title: TODOs, measure: acme.todo, expression: \"SUM(value)\" }",
            "  - { use: oxplow.rust.unsafe_blocks }",
            "collectors:",
            "  - { id: acme.missing, runtime: starlark, entry: collectors/nope.star, facts: [acme.todo] }",
            "  - { id: acme.shell, runtime: exec, entry: collectors/todo.sh, facts: [acme.todo] }",
            "  - { id: acme.stray, runtime: starlark, entry: collectors/todo.star, facts: [acme.other] }",
            "",
        ]
        .join("\n");
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/x/extension.yaml",
            &format!("manifest: 2\nname: x\nintent:\n  purpose: test\n{manifest}"),
        );
        write(
            dir.path(),
            ".oxplow/extensions/x/collectors/todo.star",
            "def transform(input):\n    return {\"facts\": []}\n",
        );
        let ext = project_extensions(dir.path()).remove(0);
        assert_eq!(ext.measures.len(), 1, "{:?}", ext.errors);
        assert_eq!(ext.metrics.len(), 1, "{:?}", ext.errors);
        // A missing script or program is refused; so is a `use:`, which
        // only the project's config can write.
        assert!(ext.collectors.iter().all(|c| c.id == "acme.stray"));
        for (needle, also) in [
            ("acme.missing", "collectors/nope.star"),
            ("acme.shell", "collectors/todo.sh"),
            ("use: oxplow.rust.unsafe_blocks", "project.yaml"),
        ] {
            assert!(
                ext.errors
                    .iter()
                    .any(|e| e.contains(needle) && e.contains(also)),
                "{needle}: {:?}",
                ext.errors
            );
        }
        // A measure it doesn't declare is a warning, not an error.
        assert!(
            ext.warnings.iter().any(|w| w.contains("acme.other")),
            "{:?}",
            ext.warnings
        );
        // `gauges:` is a key the manifest doesn't have.
        let (_d, ext) = load_x(&[], "gauges:\n  - { key: acme.x }\n");
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("unknown field `gauges`")),
            "{:?}",
            ext.errors
        );
    }

    /// `datasets:` entries with their `datasets/<name>.sql`
    /// load as the extension's datasets; a declaration without its file, a
    /// file without its declaration, and a malformed entry are errors.
    #[test]
    fn extensions_declare_datasets_with_their_sql() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = [
            "manifest: 2",
            "name: x",
            "intent:",
            "  purpose: test",
            "datasets:",
            "  - name: late",
            "    version: 1",
            "    description: Blocked work items.",
            "    fields:",
            "      - { name: uri, type: TEXT, doc: Work item ref. }",
            "  - { name: gone, version: 1, description: No file., fields: [] }",
            "",
        ]
        .join("\n");
        write(dir.path(), ".oxplow/extensions/x/extension.yaml", &manifest);
        write(
            dir.path(),
            ".oxplow/extensions/x/datasets/late.sql",
            "SELECT uri FROM ref('work_item') WHERE state = 'blocked'",
        );
        write(
            dir.path(),
            ".oxplow/extensions/x/datasets/stray.sql",
            "SELECT 1",
        );
        let ext = project_extensions(dir.path()).remove(0);
        assert_eq!(
            ext.datasets
                .iter()
                .map(|m| m.decl.name.as_str())
                .collect::<Vec<_>>(),
            Vec::<&str>::new(),
            "a broken declaration set loads no datasets"
        );
        let errors = ext.errors.join("\n");
        assert!(
            errors.contains(".oxplow/extensions/x/datasets/gone.sql is missing"),
            "{errors}"
        );

        write(
            dir.path(),
            ".oxplow/extensions/x/extension.yaml",
            &manifest.replace(
                "  - { name: gone, version: 1, description: No file., fields: [] }\n",
                "",
            ),
        );
        std::fs::remove_file(dir.path().join(".oxplow/extensions/x/datasets/stray.sql")).unwrap();
        let ext = project_extensions(dir.path()).remove(0);
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.datasets.len(), 1);
        assert_eq!(
            ext.datasets[0].file,
            ".oxplow/extensions/x/datasets/late.sql"
        );
        assert!(ext.datasets[0].sql.contains("ref('work_item')"));

        write(
            dir.path(),
            ".oxplow/extensions/x/datasets/stray.sql",
            "SELECT 1",
        );
        let ext = project_extensions(dir.path()).remove(0);
        assert!(
            ext.errors.join("\n").contains("stray.sql has no entry"),
            "{:?}",
            ext.errors
        );

        let (_d, ext) = load_x(&[], "datasets:\n  - { name: late, colour: red }\n");
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("datasets") && e.contains("colour")),
            "{:?}",
            ext.errors
        );
    }

    /// A `models/` folder is the old name of `datasets/`: the load error
    /// says to move each file, and the file is not read.
    #[test]
    fn an_old_models_folder_says_to_move_its_files() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/x/extension.yaml",
            "manifest: 2\nname: x\nintent:\n  purpose: test\ndatasets:\n  - { name: late, version: 1, description: d, fields: [] }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/x/models/late.sql",
            "SELECT 1 AS n",
        );
        let ext = project_extensions(dir.path()).remove(0);
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("move `models/<name>.sql` to `datasets/<name>.sql`")),
            "{:?}",
            ext.errors
        );
    }

    /// The old spellings are unknown keys whose error says what to write:
    /// `models:` is `datasets:`, a dataset's `columns:` is `fields:`.
    #[test]
    fn old_models_and_columns_keys_say_what_to_write() {
        let (_d, ext) = load_x(
            &[],
            "models:\n  - { name: late, version: 1, description: d, fields: [] }\n",
        );
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("unknown key `models` — write `datasets:`")),
            "{:?}",
            ext.errors
        );
        let (_d, ext) = load_x(
            &[],
            "datasets:\n  - { name: late, version: 1, description: d, columns: [] }\n",
        );
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("unknown key `columns` — write `fields:`")),
            "{:?}",
            ext.errors
        );
    }

    #[test]
    fn extensions_declare_dimensions() {
        let manifest = [
            "dimensions:",
            "  - { key: acme.team, label: Team }",
            "  - { key: acme.prio, entity: v_work_item, expr: \"json_extract(e.native, '$.priority')\" }",
            "  - { key: acme.hot, promote: true }",
            "",
        ]
        .join("\n");
        let (d, _) = load_x(&[], &manifest);
        let ext = project_extensions(d.path()).remove(0);
        let keys: Vec<_> = ext
            .dimensions
            .iter()
            .filter_map(|d| d.key.clone())
            .collect();
        assert_eq!(
            keys,
            vec!["acme.team", "acme.prio", "acme.hot"],
            "{:?}",
            ext.errors
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
    }

    #[test]
    fn a_project_extension_declares_a_program_collector() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/x/extension.yaml",
            "manifest: 2\nname: x\nintent:\n  purpose: test\nmeasures:\n  - { key: acme.todo }\ncollectors:\n  - { id: acme.shell, runtime: exec, entry: collectors/todo.sh, facts: [acme.todo] }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/x/collectors/todo.sh",
            "echo",
        );
        let own = project_extensions(dir.path()).remove(0);
        assert!(own.is_own());
        assert!(
            own.collectors.iter().any(|c| c.id == "acme.shell"),
            "{:?}",
            own.errors
        );
    }

    #[test]
    fn an_extension_fact_collector_reads_its_own_script() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/x/extension.yaml",
            "manifest: 2\nname: x\nintent:\n  purpose: test\npermissions: [files.read]\nmeasures:\n  - { key: acme.todo }\ncollectors:\n  - { id: acme.todo_scan, runtime: starlark, entry: collectors/todo.star, on: [snapshot.taken], facts: [acme.todo] }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/x/collectors/todo.star",
            "def transform(input):\n    return {\"facts\": []}\n",
        );
        let ext = project_extensions(dir.path()).remove(0);
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.collectors.len(), 1);
        assert_eq!(
            read_extension_file(dir.path(), &ext.path, "collectors/todo.star").as_deref(),
            Some("def transform(input):\n    return {\"facts\": []}\n")
        );
        assert_eq!(
            read_extension_file(dir.path(), &ext.path, "collectors/none.star"),
            None
        );
    }

    #[test]
    fn alerts_take_a_row_count_or_a_value_threshold() {
        let (_d, ext) = load_x(
            &[
                (
                    "rows",
                    &viz_page("alert: { min_rows: 2, label: Too many }\n", "SELECT 1", ""),
                ),
                (
                    "value",
                    &viz_page("alert: { column: n, above: 5 }\n", "SELECT 1 AS n", ""),
                ),
                (
                    "both",
                    &viz_page(
                        "alert: { min_rows: 1, column: n, above: 5 }\n",
                        "SELECT 1 AS n",
                        "",
                    ),
                ),
                (
                    "nothing",
                    &viz_page("alert: { column: n }\n", "SELECT 1 AS n", ""),
                ),
            ],
            "",
        );
        let alert = |slug: &str| page_of(&ext, slug).map(|p| p.alert.clone().unwrap());
        let rows = alert("rows").unwrap();
        assert_eq!(
            (rows.min_rows, rows.label.as_deref()),
            (Some(2), Some("Too many"))
        );
        let value = alert("value").unwrap();
        assert_eq!(
            (value.column.as_deref(), value.above),
            (Some("n"), Some(5.0))
        );
        assert!(alert("both").is_none() && alert("nothing").is_none());
        assert_eq!(
            ext.errors.iter().filter(|e| e.contains("alert")).count(),
            2,
            "{:?}",
            ext.errors
        );
    }

    #[tokio::test]
    async fn a_run_carries_its_alert_state() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/rows.yaml",
            &viz_page(
                "title: Rows\nparams: [{ name: n, default: 0 }]\nalert: { min_rows: 2 }\n",
                "SELECT value FROM json_each('[1,2,3]') WHERE value <= :n",
                "",
            ),
        );
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/cov.yaml",
            &viz_page(
                "title: Cov\nparams: [{ name: v, default: 90 }]\nalert: { column: pct, below: 80, label: Coverage low }\n",
                "SELECT :v AS pct",
                "",
            ),
        );
        let svc = services(dir.path()).await;
        let run = |id: &'static str, k: &'static str, v: i64| {
            let svc = svc.clone();
            async move {
                let p = BTreeMap::from([(k.to_string(), SqlCell::Int(v))]);
                run_page(&svc, id, p, ViewContext::default())
                    .await
                    .unwrap()
                    .alert
                    .unwrap()
            }
        };
        let a = run("review/rows", "n", 1).await;
        assert!(!a.firing);
        let a = run("review/rows", "n", 3).await;
        assert!(a.firing);
        assert_eq!(a.message, "3 rows");
        let a = run("review/cov", "v", 72).await;
        assert!(a.firing);
        assert_eq!(a.message, "Coverage low: 72");
        assert!(!run("review/cov", "v", 91).await.firing);
    }

    /// A slot is a core slot or a page's (`<extension>/<page>.<slot>`):
    /// anything else is refused at its line; `ui:` keys are unknown at the
    /// top.
    #[test]
    fn an_unknown_slot_is_an_error_and_ui_keys_stay_under_ui() {
        let (_d, ext) = load_x(
            &[(
                "c",
                &viz_page("params: [{ name: change_id }]\n", "SELECT :change_id AS c", ""),
            )],
            "manifest: 2\nintent:\n  purpose: p\nui:\n  mounts:\n    - { page: c, at: commit }\n    - { page: c, at: commit.sections }\n",
        );
        assert!(ext.ui.page_slots.is_empty());
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("`at: commit` is neither a page's slot")
                && errs.contains("`at: commit.sections` is neither a page's slot")
                && errs.contains("extension.yaml:"),
            "{errs}"
        );
        // Keys that live under `ui:` are unknown at the top.
        for key in ["slot_mounts", "decorators", "mounts"] {
            let (_d, ext) = load_x(
                &[],
                &format!("manifest: 2\nintent:\n  purpose: p\n{key}: []\n"),
            );
            let errs = ext.errors.join("\n");
            assert!(
                errs.contains(&format!("unknown field `{key}`")),
                "{key}: {errs}"
            );
        }
    }

    /// A panel is a page in the left nav, bound to a scope, with an
    /// optional badge page whose alert gives its count.
    #[test]
    fn panels_are_scoped_pages_with_a_badge() {
        let open = viz_page(
            "params: [{ name: stream_id, default: '' }]\n",
            "SELECT :stream_id AS s",
            "",
        );
        let count = viz_page(
            "params: [{ name: stream_id, default: '' }]\nalert: { min_rows: 1 }\n",
            "SELECT :stream_id AS s",
            "",
        );
        let quiet = viz_page("", "SELECT 1", "");
        let pages = [
            ("open", open.as_str()),
            ("count", &count),
            ("quiet", &quiet),
        ];
        let (_d, ext) = load_x(
            &pages,
            "manifest: 2\nintent:\n  purpose: p\npanels:\n  - { id: prs, title: PRs, icon: git-pull-request, scope: stream, body: open, badge: count }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(
            ext.panels,
            vec![ExtensionPanel {
                id: "x/prs".into(),
                extension: "x".into(),
                title: "PRs".into(),
                icon: Some("git-pull-request".into()),
                scope: PanelScope::Stream,
                body: "x/open".into(),
                badge: Some("x/count".into()),
                open: None,
                collapsed: None,
                count: None,
                dock: false,
            }]
        );
        // A panel may name the page its header opens (a core page,
        // as the Comments panel opens the inbox); without one, its body page.
        let (_d, ext) = load_x(
            &pages,
            "manifest: 2\nintent:\n  purpose: p\npanels:\n  - { id: c, title: C, scope: project, body: quiet, open: 'oxplow:oxplow/page?page=comments' }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(
            ext.panels[0].open.as_deref(),
            Some("oxplow:oxplow/page?page=comments")
        );
        for (panel, says) in [
            (
                "{ id: a, title: A, scope: project, body: quiet, open: comments }",
                "`open` must be a page URI",
            ),
            (
                "{ id: a, title: A, scope: project, body: quiet, open: 'task:1' }",
                "`open` must be a page URI",
            ),
            (
                "{ id: a, title: A, scope: project, body: nope }",
                "its body page `nope` isn't in pages/",
            ),
            (
                "{ id: a, title: A, scope: project, body: quiet, badge: quiet }",
                "declares no `badge` or `alert`",
            ),
            (
                "{ id: a, title: A, scope: thread, body: open }",
                "page `open` must declare it in `params`",
            ),
            ("{ id: a, title: A, scope: galaxy, body: open }", "scope"),
            (
                "{ id: Bad Id, title: A, scope: project, body: quiet }",
                "panel id",
            ),
        ] {
            let (_d, ext) = load_x(
                &pages,
                &format!("manifest: 2\nintent:\n  purpose: p\npanels:\n  - {panel}\n"),
            );
            let errs = ext.errors.join("\n");
            assert!(
                errs.contains(says) && errs.contains("extension.yaml"),
                "{panel}: {errs}"
            );
            assert!(ext.panels.is_empty(), "{panel}");
        }
    }

    /// `dock: true` puts the panel in the bottom dock; absent, the rail.
    #[test]
    fn a_panel_can_live_in_the_dock() {
        let quiet = viz_page("", "SELECT 1", "");
        let (_d, ext) = load_x(
            &[("quiet", &quiet)],
            "manifest: 2\nintent:\n  purpose: p\npanels:\n  - { id: d, title: D, scope: project, body: quiet, dock: true }\n  - { id: r, title: R, scope: project, body: quiet }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert!(ext.panels[0].dock);
        assert!(!ext.panels[1].dock);
    }

    /// A panel may name a page its collapsed header shows (one line,
    /// compact) and a page whose row count (or `number`) is its header
    /// count — checked like `body` and `badge`.
    #[test]
    fn panels_summarize_with_collapsed_and_count_pages() {
        let scoped = |node: &str| {
            viz_page(
                "params: [{ name: stream_id, default: '' }]\n",
                "SELECT :stream_id AS s",
                node,
            )
        };
        let (open, line, n) = (scoped(""), scoped(""), scoped("viz: number\n"));
        let quiet = viz_page("", "SELECT 1", "");
        let pages = [
            ("open", open.as_str()),
            ("line", &line),
            ("n", &n),
            ("quiet", &quiet),
        ];
        let (_d, ext) = load_x(
            &pages,
            "manifest: 2\nintent:\n  purpose: p\npanels:\n  - { id: w, title: W, scope: stream, body: open, collapsed: line, count: n }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.panels[0].collapsed.as_deref(), Some("x/line"));
        assert_eq!(ext.panels[0].count.as_deref(), Some("x/n"));
        for (panel, says) in [
            (
                "{ id: a, title: A, scope: project, body: quiet, collapsed: nope }",
                "its collapsed page `nope` isn't in pages/",
            ),
            (
                "{ id: a, title: A, scope: project, body: quiet, count: nope }",
                "its count page `nope` isn't in pages/",
            ),
            (
                "{ id: a, title: A, scope: stream, body: open, collapsed: quiet }",
                "page `quiet` must declare it in `params`",
            ),
            (
                "{ id: a, title: A, scope: stream, body: open, count: quiet }",
                "page `quiet` must declare it in `params`",
            ),
        ] {
            let (_d, ext) = load_x(
                &pages,
                &format!("manifest: 2\nintent:\n  purpose: p\npanels:\n  - {panel}\n"),
            );
            let errs = ext.errors.join("\n");
            assert!(errs.contains(says), "{panel}: {errs}");
            assert!(ext.panels.is_empty(), "{panel}");
        }
    }

    /// A list or table can group its rows under headings (`group`),
    /// highlight one (`emphasis`), indent some (`depth`), and draw a
    /// cell's icon and tone from other columns; an action can sit in a
    /// group's heading. Shapes are checked at load; the columns they name,
    /// against the loader's result by `oxplow_validate_extension`.
    #[test]
    fn nodes_group_and_style_their_rows() {
        let (_d, ext) = load_x(
            &[
                (
                    "a",
                    &viz_page(
                        "params: [{ name: stream_id, default: '' }]\n",
                        "SELECT 1 AS id, 'Ready' AS bucket, 'oxplow:oxplow/page?page=tasks' AS bucket_ref, 1 AS current, 0 AS indent, 'done' AS glyph, 'success' AS hue",
                        "viz: list\ngroup: { by: bucket, link: { kind: page, from: bucket_ref } }\nemphasis: current\ndepth: indent\ncolumns:\n  - { key: id, icon: glyph, tone: hue }\nactions:\n  - { id: add, label: '+', command: oxplow.work_item.create, group: Ready, input: { stream: '{{param.stream_id}}' } }\n",
                    ),
                ),
                ("t", &viz_page("", "SELECT 1 AS id, NULL AS p, 'x' AS l, 1 AS on_", "viz: tree\ntree: { id: id, parent: p, label: l }\nemphasis: on_\n")),
                ("b", &viz_page("", "SELECT 1 AS id, NULL AS p, 'x' AS l", "viz: tree\ntree: { id: id, parent: p, label: l }\ngroup: { by: l }\n")),
                ("c", &viz_page("", "SELECT 1", "viz: list\ngroup: { link: { kind: page } }\n")),
                ("d", &viz_page("", "SELECT 1", "viz: number\ndepth: n\n")),
                ("e", &viz_page("", "SELECT 1", "viz: list\nactions: [{ id: x, label: X, command: a.b.c, group: Ready }]\n")),
                ("f", &viz_page("", "SELECT 1", "viz: list\ngroup: { by: g }\nactions: [{ id: x, label: X, command: a.b.c, group: Ready, row: true }]\n")),
                ("g", &viz_page("", "SELECT 1 AS x, 2 AS y", "viz: bar\nchart: { x: x, y: y }\nemphasis: x\n")),
            ],
            "",
        );
        let a = node(&ext, "a").expect("a grouped, styled list loads");
        assert_eq!(
            a.group,
            Some(VizGroup {
                by: "bucket".into(),
                link: Some(ColumnLink {
                    kind: LinkKind::Page,
                    from: Some("bucket_ref".into()),
                    line: None,
                    base: None,
                    head: None,
                }),
                values: Vec::new(),
            })
        );
        assert_eq!(a.emphasis.as_deref(), Some("current"));
        assert_eq!(a.depth.as_deref(), Some("indent"));
        assert_eq!(a.columns[0].icon.as_deref(), Some("glyph"));
        assert_eq!(a.columns[0].tone.as_deref(), Some("hue"));
        assert_eq!(a.actions[0].group.as_deref(), Some("Ready"));
        assert!(node(&ext, "t").is_some(), "a tree may emphasize");
        for (slug, needle) in [
            ("b", "`group` is only for `list`, `table` and `board`"),
            ("c", "missing field `by`"),
            ("d", "`depth` is only for `list` and `table`"),
            ("e", "`group: Ready` needs the viz to have a `group`"),
            ("f", "a group action isn't a row action"),
            ("g", "`emphasis` is only for `list`, `table` and `tree`"),
        ] {
            assert!(page_of(&ext, slug).is_none(), "{slug} should fail");
            assert!(
                ext.errors
                    .iter()
                    .any(|e| e.contains(&format!("{slug}.yaml")) && e.contains(needle)),
                "{slug}: {:?}",
                ext.errors
            );
        }
    }

    /// The columns `group`, `emphasis`, `depth` and a column's `icon` /
    /// `tone` name must be in the loader's result (`oxplow_validate_extension`).
    #[tokio::test]
    async fn validate_checks_row_style_columns() {
        let (d, _) = load_x(
            &[(
                "a",
                &viz_page(
                    "",
                    "SELECT 1 AS id",
                    "viz: table\ngroup: { by: g1, link: { kind: page, from: g2 } }\nemphasis: e1\ndepth: d1\ncolumns:\n  - { key: id, icon: i1, tone: t1 }\n",
                ),
            )],
            "",
        );
        let v = validate_extension(&layer().await, &cat(), d.path(), "x", None)
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        for k in ["g1", "g2", "e1", "d1", "i1", "t1"] {
            assert!(errs.contains(&format!("`{k}`")), "{k}: {errs}");
        }
    }

    /// A column's `highlight` names a param the page declares: the text
    /// of the param's value is marked wherever the cell has it.
    #[test]
    fn a_columns_highlight_names_a_declared_param() {
        let (_d, ext) = load_x(
            &[
                (
                    "a",
                    &viz_page(
                        "params: [{ name: filter, default: '' }]\n",
                        "SELECT 'x' AS t",
                        "columns:\n  - { key: t, highlight: filter }\n",
                    ),
                ),
                (
                    "b",
                    &viz_page(
                        "",
                        "SELECT 'x' AS t",
                        "columns:\n  - { key: t, highlight: nope }\n",
                    ),
                ),
            ],
            "",
        );
        let a = node(&ext, "a").expect("loads");
        assert_eq!(a.columns[0].highlight.as_deref(), Some("filter"));
        assert!(page_of(&ext, "b").is_none());
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("b.yaml") && e.contains("`highlight: nope`")),
            "{:?}",
            ext.errors
        );
    }

    /// An action is a command. Its placeholders must name a param (or, in
    /// a row action, a column); anything else is refused.
    #[test]
    fn actions_are_commands() {
        let (_d, ext) = load_x(
            &[
                (
                    "a",
                    &viz_page(
                        "params: [{ name: item, default: '' }]\n",
                        "SELECT 1 AS id",
                        "actions:\n  - { id: finish, label: Finish, command: oxplow.work_item.transition, input: { uri: '{{param.item}}', to: done } }\n  - { id: row, label: Row, command: oxplow.work_item.transition, row: true, input: { uri: 'work_item:oxplow:tsk{{row.id}}', to: done } }\n",
                    ),
                ),
                ("b", &viz_page("", "SELECT 1", "actions: [copy]\n")),
                ("c", &viz_page("", "SELECT 1", "actions: [{ action: run-source, source: a/b }]\n")),
                ("d", &viz_page("", "SELECT 1", "actions: [{ id: x, label: X, command: oxplow.work_item.create, input: { title: '{{param.nope}}' } }]\n")),
                ("e", &viz_page("", "SELECT 1", "actions: [{ id: x, label: X, command: oxplow.work_item.create, input: { title: '{{row.id}}' } }]\n")),
                ("f", &viz_page("", "SELECT 1", "actions: [{ id: x, label: X, command: Not-A-Name }]\n")),
                ("g", &viz_page("", "SELECT 1", "actions: [{ id: x, label: X, command: a.b.c }, { id: x, label: Y, command: a.b.d }]\n")),
                // A whole-page action is never a row's: the row is a node's.
                ("h", &viz_page("actions: [{ id: x, label: X, command: a.b.c, row: true }]\n", "SELECT 1", "")),
            ],
            "",
        );
        let a = node(&ext, "a").expect("valid actions load");
        assert_eq!(
            a.actions
                .iter()
                .map(|x| (x.id.as_str(), x.command.as_str(), x.row))
                .collect::<Vec<_>>(),
            vec![
                ("finish", "oxplow.work_item.transition", false),
                ("row", "oxplow.work_item.transition", true)
            ]
        );
        for (slug, needle) in [
            ("b", "actions:"),
            ("c", "unknown field `action`"),
            ("d", "names no param"),
            ("e", "needs `row: true`"),
            ("f", "command id"),
            ("g", "twice"),
            ("h", "a row action goes on the viz node that shows the row"),
        ] {
            assert!(page_of(&ext, slug).is_none(), "{slug} should fail");
            assert!(
                ext.errors
                    .iter()
                    .any(|e| e.contains(&format!("{slug}.yaml")) && e.contains(needle)),
                "{slug}: {:?}",
                ext.errors
            );
        }
    }

    /// What becomes of the page after an action, or a form's submit:
    /// `then: open` (default) | `replace` | `stay` | `close`.
    #[test]
    fn an_action_says_what_becomes_of_its_page() {
        let (_d, ext) = load_x(
            &[
                (
                    "a",
                    &viz_page(
                        "",
                        "SELECT 1",
                        "actions:\n  - { id: plain, label: P, command: a.b.c }\n  - { id: gone, label: G, command: a.b.c, then: replace }\n  - { id: again, label: A, command: a.b.c, then: stay }\n",
                    ),
                ),
                (
                    "f",
                    "title: F\nview: { viz: form, form: { command: a.b.c, then: replace } }\n",
                ),
                ("b", &viz_page("", "SELECT 1", "actions: [{ id: x, label: X, command: a.b.c, then: away }]\n")),
            ],
            "",
        );
        let a = node(&ext, "a").expect("a loads");
        assert_eq!(
            a.actions.iter().map(|x| x.then).collect::<Vec<_>>(),
            vec![ActionThen::Open, ActionThen::Replace, ActionThen::Stay]
        );
        let f = node(&ext, "f").expect("f loads");
        assert_eq!(f.form.as_ref().map(|x| x.then), Some(ActionThen::Replace));
        assert!(page_of(&ext, "b").is_none());
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("b.yaml") && e.contains("away")),
            "{:?}",
            ext.errors
        );
    }

    /// An edit form says so; a field may show only for some values of
    /// another and may be secret; a field's choices come from a page of
    /// the extension; each refusal names the form's problem.
    #[test]
    fn a_form_edits_shows_fields_by_value_and_keeps_secrets() {
        let form = |spec: &str| format!("title: F\nview:\n  viz: form\n  form: {spec}\n");
        let (_d, ext) = load_x(
            &[
                (
                    "e",
                    "title: E\nview:\n  viz: form\n  form:\n    command: a.b.c\n    mode: edit\n    fields:\n      - kind\n      - { name: url, visible_if: { kind: [http, https] } }\n      - { name: token, secret: true, visible_if: { kind: https } }\n",
                ),
                ("p", &form("{ command: a.b.c, fields: [title] }")),
                ("self", &form("{ command: a.b.c, fields: [{ name: a, visible_if: { a: x } }] }")),
                ("unknown", &form("{ command: a.b.c, fields: [a, { name: b, visible_if: { gone: x } }] }")),
                ("fixedsecret", &form("{ command: a.b.c, defaults: { a: 1 }, fixed: [a], fields: [{ name: a, secret: true }] }")),
                ("nofields", &form("{ command: a.b.c, mode: edit }")),
                ("again", &form("{ command: a.b.c, fields: [a, { name: n, from: again }, { name: m, from: x/e }], again: { label: Again, keep: [a, n] } }")),
                ("keepgone", &form("{ command: a.b.c, fields: [a], again: { label: Again, keep: [gone] } }")),
                ("fromgone", &form("{ command: a.b.c, fields: [{ name: n, from: x/nothing }] }")),
            ],
            "",
        );
        let e = node(&ext, "e").unwrap_or_else(|| panic!("e: {:?}", ext.errors));
        let form = e.form.as_ref().expect("a form");
        assert_eq!(form.mode, FormMode::Edit);
        let fields = form.fields.as_ref().expect("fields");
        assert_eq!(
            fields
                .iter()
                .map(|f| (f.name(), f.secret()))
                .collect::<Vec<_>>(),
            vec![("kind", false), ("url", false), ("token", true)]
        );
        assert_eq!(
            fields[1].visible_if().get("kind"),
            Some(&FieldValues::Any(vec![
                FieldValue::Text("http".into()),
                FieldValue::Text("https".into())
            ]))
        );
        let p = node(&ext, "p").expect("p loads");
        assert_eq!(p.form.as_ref().map(|f| f.mode), Some(FormMode::Create));
        let said = |slug: &str| {
            ext.errors
                .iter()
                .find(|e| e.contains(&format!("{slug}.yaml")))
                .cloned()
                .unwrap_or_default()
        };
        for (slug, says) in [
            ("self", "`a` can't show by its own value"),
            (
                "unknown",
                "`visible_if` names `gone`, which isn't one of its `fields`",
            ),
            ("fixedsecret", "`a` can't be both fixed and secret"),
            ("nofields", "`mode: edit` needs `fields`"),
            ("keepgone", "`again.keep` names `gone`"),
            (
                "fromgone",
                "form field `n`: `from` page `x/nothing` isn't in this extension's pages/",
            ),
        ] {
            assert!(said(slug).contains(says), "{slug}: {:?}", ext.errors);
            assert!(page_of(&ext, slug).is_none(), "{slug} loaded");
        }
        // A field's choices may come from the form's own page.
        let again = node(&ext, "again");
        let form = again
            .and_then(|l| l.form.as_ref())
            .unwrap_or_else(|| panic!("again: {:?}", ext.errors));
        assert_eq!(form.again.as_ref().unwrap().keep, ["a", "n"]);
        assert_eq!(form.fields.as_ref().unwrap()[1].from(), Some("x/again"));
    }

    /// A row action's `{{row.x}}` must be a column the loader returns.
    #[tokio::test]
    async fn validate_checks_row_action_columns() {
        let (d, _) = load_x(
            &[(
                "a",
                &viz_page(
                    "",
                    "SELECT 1 AS id",
                    "actions: [{ id: x, label: X, command: a.b.c, row: true, input: { v: '{{row.gone}}' } }]\n",
                ),
            )],
            "",
        );
        let v = validate_extension(&layer().await, &cat(), d.path(), "x", None)
            .await
            .unwrap();
        assert!(
            v.errors.join("\n").contains("`{{row.gone}}`"),
            "{:?}",
            v.errors
        );
    }

    #[test]
    fn refresh_is_opt_in_on_a_page() {
        let (a, b) = (
            viz_page("", "SELECT 1", ""),
            viz_page("refresh: true\n", "SELECT 1", ""),
        );
        let (_d, ext) = load_x(
            &[("a", &a), ("b", &b)],
            "manifest: 2\nintent:\n  purpose: p\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert!(!page_of(&ext, "a").unwrap().refresh);
        assert!(page_of(&ext, "b").unwrap().refresh);
    }

    /// A page's launcher metadata: its category, keywords and icon, and
    /// whether it is hidden. An unknown category or an icon oxplow doesn't
    /// draw is refused.
    #[test]
    fn launcher_metadata_parses_and_is_checked() {
        let (_d, ext) = load_x(
            &[
                (
                    "a",
                    &viz_page(
                        "category: Activity\nkeywords: pull requests\nicon: git-pull-request\ncomments: true\n",
                        "SELECT 1",
                        "",
                    ),
                ),
                ("b", &viz_page("hidden: true\n", "SELECT 1", "")),
                ("c", &viz_page("category: Nowhere\n", "SELECT 1", "")),
                ("d", &viz_page("icon: unicorn\n", "SELECT 1", "")),
            ],
            "",
        );
        let page = |slug: &str| page_of(&ext, slug);
        let a = page("a").unwrap_or_else(|| panic!("{:?}", ext.errors));
        assert_eq!(a.category, Some(LauncherCategory::Activity));
        assert_eq!(a.keywords.as_deref(), Some("pull requests"));
        assert_eq!(a.icon.as_deref(), Some("git-pull-request"));
        assert!(a.comments && !a.hidden);
        assert!(page("b").unwrap().hidden);
        assert!(page("c").is_none() && page("d").is_none());
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("c.yaml") && errs.contains("Nowhere"),
            "{errs}"
        );
        assert!(
            errs.contains("d.yaml") && errs.contains("`icon: unicorn` isn't one oxplow draws"),
            "{errs}"
        );
    }

    /// A page's id is its file's name: one that isn't kebab-case is an
    /// error.
    #[test]
    fn a_page_file_names_a_valid_id() {
        let (_d, ext) = load_x(&[("Bad_Page", &viz_page("", "SELECT 1", ""))], "");
        let errs = ext.errors.join("\n");
        assert!(errs.contains("Bad_Page"), "{errs}");
    }

    #[tokio::test]
    async fn disabled_extensions_load_empty_and_their_pages_refuse_to_run() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/by-status.yaml",
            PAGE,
        );
        write(
            dir.path(),
            ".oxplow/personal.yaml",
            "disableExtensions: [review, oxplow-bundled]\n",
        );
        let exts = load_extensions(dir.path());
        for name in ["review", "oxplow-bundled"] {
            let e = exts.iter().find(|e| e.name == name).unwrap();
            assert!(!e.enabled, "{name}");
            assert!(
                e.ui.page_slots.is_empty() && e.collectors.is_empty() && e.panels.is_empty(),
                "{name}"
            );
        }
        let err = cat().find_page(dir.path(), "review/by-status").unwrap_err();
        assert!(err.to_string().contains("disabled"), "{err}");
        // What runs holds no page of a disabled extension.
        let svc = services(dir.path()).await;
        let err = run_page(
            &svc,
            "review/by-status",
            BTreeMap::new(),
            ViewContext::default(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("review/by-status"), "{err}");
    }

    #[tokio::test]
    async fn advisories_parse_and_validate_their_columns() {
        let (d, ext) = load_x(
            &[],
            "permissions: [agent.write]\nadvisories:\n  - id: low-coverage\n    on: post-tool-use\n    once_per: effort\n    query: SELECT 'add tests' AS message WHERE :effort_id IS NOT NULL\n  - id: crossings\n    on: prompt\n    once_per: row\n    heading: '# Metric thresholds'\n    query: SELECT 'x' AS message, 'k' AS key\n  - id: nokey\n    on: prompt\n    once_per: row\n    query: SELECT 'x' AS message\n  - id: Bad Id\n    on: prompt\n    query: SELECT 1\n",
        );
        let ids: Vec<&str> = ext.advisories.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["low-coverage", "crossings", "nokey"]);
        assert_eq!(ext.advisories[0].on, AdvisoryOn::PostToolUse);
        assert_eq!(ext.advisories[1].once_per, AdvisoryOncePer::Row);
        assert_eq!(
            ext.advisories[1].heading.as_deref(),
            Some("# Metric thresholds")
        );
        assert!(ext.errors.join("\n").contains("Bad Id"), "{:?}", ext.errors);

        let v = validate_extension(&layer().await, &cat(), d.path(), "x", None)
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        assert!(errs.contains("nokey") && errs.contains("`key`"), "{errs}");
        assert!(
            !errs.contains("low-coverage") && !errs.contains("crossings"),
            "{errs}"
        );
    }

    #[test]
    fn change_link_kinds_name_their_revisions() {
        let (_d, ext) = load_x(
            &[
                (
                    "files",
                    &viz_page(
                        "params: [{ name: path }]\n",
                        "SELECT :path AS path, 1 AS dup",
                        "columns:\n  - { key: path, link: { kind: diff-at, line: ln, base: b, head: h } }\n  - { key: dup, link: { kind: compare, head: h } }\n",
                    ),
                ),
            ],
            "",
        );
        let link = node(&ext, "files").unwrap().columns[0]
            .link
            .clone()
            .unwrap();
        assert_eq!(link.kind, LinkKind::DiffAt);
        assert_eq!(
            (link.base.as_deref(), link.head.as_deref()),
            (Some("b"), Some("h"))
        );
    }

    /// A page's `label` is what its tab says, `{{param.<name>}}` filled
    /// from its URI: a renderer's names a param of what it renders, any
    /// other page's one of its `params`; a filter (`short`) may shape it.
    /// Its `title` stays its name (launcher, Open with).
    #[test]
    fn a_pages_label_names_its_params() {
        let resources =
            "resources:\n  - { name: pr, label: PR, params: { id: { required: true } } }\n";
        let renderer = |label: &str| {
            viz_page(
                &format!("title: PR\nlabel: '{label}'\nfor: x/pr\nparams: [{{ name: uri }}, {{ name: id }}]\n"),
                "SELECT :id AS id",
                "",
            )
        };
        let route = |label: &str| {
            viz_page(
                &format!("title: Route\nlabel: '{label}'\nparams: [{{ name: id }}]\n"),
                "SELECT :id AS id",
                "",
            )
        };
        let (_d, ext) = load_x(
            &[
                ("pr", &renderer("PR {{param.id}}")),
                ("route", &route("{{ param.id }}")),
                ("short", &renderer("{{row.title}} {{param.id|short}}")),
            ],
            resources,
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let pr = page_of(&ext, "pr").unwrap();
        assert_eq!(pr.label.as_deref(), Some("PR {{param.id}}"));
        assert_eq!(pr.title, "PR");
        for (page, says) in [
            (
                renderer("{{param.nope}}"),
                "`label` names `nope`, which isn't a param of `x/pr`",
            ),
            (
                route("{{param.nope}}"),
                "`label` names `nope`, which isn't one of its `params`",
            ),
            (
                route("{{nope.id}}"),
                "`label` fills only `{{param.<name>}}`",
            ),
            (renderer("{{param.id|loud}}"), "`label` filter `loud`"),
        ] {
            let (_d, ext) = load_x(&[("a", &page)], resources);
            let errs = ext.errors.join("\n");
            assert!(errs.contains(says), "{page}: {errs}");
            assert!(page_of(&ext, "a").is_none(), "{page}");
        }
    }

    /// A page that renders resources says which (`for`), which of their
    /// URIs it takes (`accepts`), and whether it is what they open on
    /// (`default`) — only for the extension's own resources.
    #[test]
    fn a_page_renders_the_resources_it_is_for() {
        let resources =
            "resources:\n  - { name: pr, label: PR, params: { id: { required: true } } }\n";
        let pr = |meta: &str| {
            viz_page(
                &format!("title: PR\nparams: [{{ name: uri }}]\n{meta}"),
                "SELECT :uri AS u",
                "",
            )
        };
        let wide = |meta: &str| {
            viz_page(
                &format!("title: Wide\nparams: [{{ name: uri }}, {{ name: id }}, {{ name: stream_id }}, {{ name: q, role: filter }}, {{ name: nope }}]\n{meta}"),
                "SELECT :uri, :id, :stream_id, :q, :nope",
                "",
            )
        };
        let fits = |meta: &str| {
            viz_page(
                &format!("title: Fits\nparams: [{{ name: uri }}, {{ name: id }}, {{ name: q, role: filter }}]\n{meta}"),
                "SELECT :uri, :id, :q",
                "",
            )
        };
        let viewed = viz_page(
            "title: Viewed\nparams: [{ name: uri }, { name: id }, { name: mode, role: view }]\nfor: x/pr\n",
            "SELECT :uri, :id, :mode",
            "",
        );
        let (_d, ext) = load_x(
            &[
                ("viewed", &viewed),
                ("pr", &pr("for: x/pr\ndefault: true\n")),
                ("pr-raw", &pr("for: x/pr\naccepts: { id: '^\\d+$' }\n")),
                ("both", &pr("for: [x/pr, oxplow/file]\n")),
                ("web", &pr("for: https\npinned: true\n")),
                ("fits", &fits("for: x/pr\n")),
            ],
            resources,
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let page = |slug: &str| page_of(&ext, slug).unwrap();
        assert_eq!(page("pr").renders, vec!["x/pr".to_string()]);
        assert!(page("pr").default);
        assert_eq!(
            page("both").renders,
            vec!["x/pr".to_string(), "oxplow/file".to_string()]
        );
        assert_eq!(
            page("pr-raw").accepts.get("id").map(String::as_str),
            Some("^\\d+$")
        );
        assert!(page("web").pinned);

        for (page, says) in [
            (pr("for: nope\n"), "isn't a resource"),
            (pr("for: x/missing\n"), "isn't one of this extension's"),
            (
                pr("for: oxplow/missing\n"),
                "isn't a resource oxplow-foundation declares",
            ),
            (
                pr("for: x/pr\naccepts: { nope: present }\n"),
                "`accepts: nope` isn't a param of `x/pr`",
            ),
            (
                pr("for: x/pr\naccepts: { id: '[0-9]+' }\n"),
                "an id pattern",
            ),
            (pr("for: oxplow/file\ndefault: true\n"), "is another's"),
            (pr("default: true\n"), "are for a renderer"),
            // It's run with its URI's params: one the resource hasn't, it
            // would never get (a filter is its view's, not the URI's).
            (
                wide("for: x/pr\n"),
                "takes `nope`, which isn't a param of `x/pr`",
            ),
            (
                fits("for: https\n"),
                "takes `id`, which isn't a param of `https`",
            ),
        ] {
            let (_d, ext) = load_x(&[("a", &page)], resources);
            let errs = ext.errors.join("\n");
            assert!(errs.contains(says), "{page}: {errs}");
            assert!(page_of(&ext, "a").is_none(), "{page}");
        }
        let (_d, ext) = load_x(
            &[
                ("a", &pr("for: x/pr\ndefault: true\n")),
                ("b", &pr("for: x/pr\ndefault: true\n")),
            ],
            resources,
        );
        assert!(
            ext.errors.join("\n").contains("another default renderer"),
            "{:?}",
            ext.errors
        );
    }

    /// A page's view may be one of its extension's custom components: the
    /// component owns the page body (a custom page), fed the page's data.
    #[test]
    fn a_page_may_be_one_of_its_custom_components() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/x/extension.yaml",
            "manifest: 2\nname: x\nintent:\n  purpose: p\ncustom_components:\n  - { id: board }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/x/components/board/index.html",
            "<!doctype html>",
        );
        write(
            dir.path(),
            ".oxplow/extensions/x/pages/board.yaml",
            "title: Board\ncategory: Work\nview: { component: { id: board, props: { compact: true } } }\n",
        );
        let ext = project_extensions(dir.path()).remove(0);
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert!(matches!(
            &page_of(&ext, "board").unwrap().view.kind,
            crate::pages::schema::NodeKind::Component { component, .. } if component == "board"
        ));
    }
    /// P8.C1: an extension loads from any revision — at `git:HEAD` of a
    /// clean worktree exactly as from disk, and at a snapshot as that
    /// snapshot holds it, whatever the disk says now.
    #[tokio::test]
    async fn an_extension_loads_at_any_revision() {
        use oxplow_domain::stores::StreamStore as _;
        use oxplow_domain::vcs::Revision;
        let f = crate::test_fixtures::services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        // Git sees an extension's source once it is shared.
        crate::extension::extension_deploy::share(&f.svc, "acme").unwrap();
        let manifest = |purpose: &str| {
            format!(
                "manifest: 2\nname: acme\ndescription: {purpose}\nintent:\n  purpose: {purpose}\n  origin: thread:thr1\n  examples: []\n"
            )
        };
        let dir = ws.join(".oxplow/extensions/acme");
        std::fs::create_dir_all(dir.join("pages")).unwrap();
        std::fs::write(dir.join("extension.yaml"), manifest("Committed.")).unwrap();
        std::fs::write(
            dir.join("pages/streams.yaml"),
            viz_page("title: Streams\n", "SELECT id FROM v_stream", ""),
        )
        .unwrap();
        crate::test_fixtures::commit_all(&ws, "acme");

        let at_head = extension_at(&f.svc.trees, &ws, &Revision::git("HEAD"), "acme")
            .await
            .unwrap()
            .expect("acme is at HEAD");
        assert_eq!(at_head, load_project_extension(&ws, "acme"));
        assert_eq!(at_head.page_files.len(), 1);
        assert!(
            extension_at(&f.svc.trees, &ws, &Revision::git("HEAD"), "nope")
                .await
                .unwrap()
                .is_none()
        );

        // A snapshot holding an edited manifest loads the edit, though the
        // disk holds the committed one.
        let stream = f.svc.stream_store.list().await.unwrap()[0].id;
        let snap = f.svc.snapshot_store.create_snapshot(stream).await.unwrap();
        let edited = manifest("Edited.");
        let hash = f.svc.blobs.write(edited.as_bytes()).unwrap();
        f.svc
            .snapshot_store
            .capture(oxplow_db::FileSnapshot {
                id: 0,
                stream_id: stream,
                path: ".oxplow/extensions/acme/extension.yaml".into(),
                blob_hash: Some(hash),
                size_bytes: edited.len() as i64,
                captured_at: oxplow_domain::Timestamp::now(),
                storage: oxplow_db::SnapshotStorage::Oxplow,
                snapshot_id: Some(snap),
                mtime_ms: None,
                content_hash: None,
            })
            .await
            .unwrap();
        let at_snapshot = extension_at(&f.svc.trees, &ws, &Revision::Snapshot(snap), "acme")
            .await
            .unwrap()
            .expect("acme is in the snapshot");
        assert_eq!(at_snapshot.description, "Edited.");
    }

    /// An effort's review names the extensions that changed between
    /// its two revisions, each with what the change does; files outside
    /// `.oxplow/extensions/` change nothing here.
    #[tokio::test]
    async fn an_effort_review_names_the_extensions_that_changed() {
        use oxplow_domain::stores::StreamStore as _;
        use oxplow_domain::vcs::Revision;
        let f = crate::test_fixtures::services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        // Git sees an extension's source once it is shared.
        crate::extension::extension_deploy::share(&f.svc, "acme").unwrap();
        let stream = f.svc.stream_store.list().await.unwrap()[0].id;
        let snapshot = |files: Vec<(&'static str, String)>| {
            let svc = f.svc.clone();
            async move {
                let id = svc.snapshot_store.create_snapshot(stream).await.unwrap();
                for (path, body) in files {
                    let hash = svc.blobs.write(body.as_bytes()).unwrap();
                    svc.snapshot_store
                        .capture(oxplow_db::FileSnapshot {
                            id: 0,
                            stream_id: stream,
                            path: path.into(),
                            blob_hash: Some(hash),
                            size_bytes: body.len() as i64,
                            captured_at: oxplow_domain::Timestamp::now(),
                            storage: oxplow_db::SnapshotStorage::Oxplow,
                            snapshot_id: Some(id),
                            mtime_ms: None,
                            content_hash: None,
                        })
                        .await
                        .unwrap();
                }
                id
            }
        };
        let manifest = "manifest: 2\nname: acme\nintent:\n  purpose: Count.\n  origin: thread:thr1\n  examples: []\n".to_string();
        let page = |n: u32| {
            viz_page(
                "title: Count\n",
                &format!("SELECT {n} AS n"),
                "viz: number\n",
            )
        };
        let start = snapshot(vec![
            (".oxplow/extensions/acme/extension.yaml", manifest.clone()),
            (".oxplow/extensions/acme/pages/count.yaml", page(1)),
            ("src/a.rs", "fn a() {}\n".into()),
        ])
        .await;
        let end = snapshot(vec![
            (".oxplow/extensions/acme/pages/count.yaml", page(2)),
            ("src/a.rs", "fn a() { 1 }\n".into()),
        ])
        .await;
        let changes = extension_changes_between(
            &f.svc,
            &ws,
            Some(&Revision::Snapshot(start)),
            &Revision::Snapshot(end),
        )
        .await
        .unwrap();
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].name, "acme");
        assert_eq!(
            changes[0].change,
            crate::extension::extension_impact::Change::Changed
        );
        let report = changes[0].impact.as_ref().unwrap();
        assert!(
            report.lines.iter().any(|l| l == "Page acme/count: changed"),
            "{:?}",
            report.lines
        );
    }

    /// The earlier side of a review is only read — its pages
    /// rendered on its own datasets — never checked: its command examples
    /// don't run (here, a runaway one the later version removed).
    #[tokio::test(flavor = "multi_thread")]
    async fn the_earlier_side_of_a_review_is_not_checked() {
        use oxplow_domain::stores::StreamStore as _;
        use oxplow_domain::vcs::Revision;
        let f = crate::test_fixtures::services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        // Git sees an extension's source once it is shared.
        crate::extension::extension_deploy::share(&f.svc, "acme").unwrap();
        let stream = f.svc.stream_store.list().await.unwrap()[0].id;
        let snapshot = |files: Vec<(&'static str, String)>| {
            let svc = f.svc.clone();
            async move {
                let id = svc.snapshot_store.create_snapshot(stream).await.unwrap();
                for (path, body) in files {
                    let hash = svc.blobs.write(body.as_bytes()).unwrap();
                    svc.snapshot_store
                        .capture(oxplow_db::FileSnapshot {
                            id: 0,
                            stream_id: stream,
                            path: path.into(),
                            blob_hash: Some(hash),
                            size_bytes: body.len() as i64,
                            captured_at: oxplow_domain::Timestamp::now(),
                            storage: oxplow_db::SnapshotStorage::Oxplow,
                            snapshot_id: Some(id),
                            mtime_ms: None,
                            content_hash: None,
                        })
                        .await
                        .unwrap();
                }
                id
            }
        };
        let manifest = "manifest: 2\nname: acme\nintent:\n  purpose: Count.\n  origin: thread:thr1\n  examples: []\n";
        let command = "commands:\n  - name: spin\n    summary: Spin.\n    input_schema: { type: object }\n    entry: handlers/spin.star\n    examples:\n      - { name: once, input: {}, expect_commands: [] }\n";
        let start = snapshot(vec![
            (
                ".oxplow/extensions/acme/extension.yaml",
                format!("{manifest}{command}"),
            ),
            (
                ".oxplow/extensions/acme/handlers/spin.star",
                "def transform(x):\n    n = 0\n    for i in range(100000):\n        for j in range(100000):\n            n += j\n    return {\"commands\": []}\n".into(),
            ),
        ])
        .await;
        let end = snapshot(vec![(
            ".oxplow/extensions/acme/extension.yaml",
            manifest.to_string(),
        )])
        .await;
        let started = std::time::Instant::now();
        let changes = extension_changes_between(
            &f.svc,
            &ws,
            Some(&Revision::Snapshot(start)),
            &Revision::Snapshot(end),
        )
        .await
        .unwrap();
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert!(
            // The runaway handler would take minutes; only a busy machine
            // makes the rest slow.
            started.elapsed() < std::time::Duration::from_secs(30),
            "{:?}",
            started.elapsed()
        );
    }
}
