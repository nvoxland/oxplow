//! Config file load + validation for oxplow.
//!
//! Replaces the TS `src/config/**` module. Schema validation is
//! enforced at deserialization; errors carry typed variants so the
//! UI can surface them precisely.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;
use tracing::info;

pub mod collectors;
pub mod keys;

pub mod recent;
pub use recent::{RecentProject, RecentProjects};

mod atomic;
pub mod session;
pub use session::SessionProjects;

/// Project-relative state directory holding oxplow's per-project config
/// and local data (DB, snapshots, wiki, …).
pub const OXPLOW_STATE_DIR: &str = ".oxplow";

/// Config file name, inside [`OXPLOW_STATE_DIR`]
/// (`<project>/.oxplow/project.yaml`).
pub const OXPLOW_CONFIG_FILE: &str = "project.yaml";

/// A person's own layer over the project's config, inside
/// [`OXPLOW_STATE_DIR`] (`<project>/.oxplow/personal.yaml`): their choice of
/// implementations (`activeProviders`), what a new thread starts with
/// (`newThreadSession`), the extensions they turn on or off over the
/// team's list (`enableExtensions`, `disableExtensions`) and where their
/// extensions' sources are
/// (`extensionSources`), which git ignores (`.oxplow` ignores everything
/// but `project.yaml`).
pub const PERSONAL_CONFIG_FILE: &str = "personal.yaml";

/// The keys a person's layer may set.
pub const PERSONAL_KEYS: &[&str] = &[
    "activeProviders",
    "newThreadSession",
    "enableExtensions",
    "disableExtensions",
    "extensionSources",
    "enableBundledExtensions",
];

/// What a personal key is, for Settings.
pub fn personal_key_doc(key: &str) -> &'static str {
    match key {
        "activeProviders" => {
            "Your own `activeProviders`, over the project's (.oxplow/personal.yaml, which git ignores)."
        }
        "newThreadSession" => {
            "What a new thread starts with: `ask` (the session picker), `none`, or an agent \
             (.oxplow/personal.yaml, which git ignores)."
        }
        "enableExtensions" => {
            "Extensions you turn on beside the project's list \
             (.oxplow/personal.yaml, which git ignores)."
        }
        "disableExtensions" => {
            "Extensions you turn off, whoever listed them \
             (.oxplow/personal.yaml, which git ignores)."
        }
        "extensionSources" => {
            "Where your extensions' sources are, over the project's `extensionSources` \
             (.oxplow/personal.yaml, which git ignores)."
        }
        "enableBundledExtensions" => {
            "Your own `enableBundledExtensions`, over the project's: `false` runs oxplow's own \
             extensions from their sources in a dev build (.oxplow/personal.yaml, which git \
             ignores)."
        }
        _ => "",
    }
}

/// Absolute path to a project's personal layer:
/// `<project_dir>/.oxplow/personal.yaml`.
pub fn personal_path(project_dir: impl AsRef<Path>) -> std::path::PathBuf {
    project_dir
        .as_ref()
        .join(OXPLOW_STATE_DIR)
        .join(PERSONAL_CONFIG_FILE)
}

/// Absolute path to a project's config file:
/// `<project_dir>/.oxplow/project.yaml`.
pub fn config_path(project_dir: impl AsRef<Path>) -> std::path::PathBuf {
    project_dir
        .as_ref()
        .join(OXPLOW_STATE_DIR)
        .join(OXPLOW_CONFIG_FILE)
}

/// Reverse-DNS app identifier. Mirrors `identifier` in
/// `tauri.conf.json`; used to derive the global app-config dir so code
/// without a Tauri handle (e.g. `main.rs` before the app is built) can
/// resolve the same location Tauri's path resolver would.
pub const APP_IDENTIFIER: &str = "net.voxland.oxplow";

/// Env var redirecting [`global_config_dir`] somewhere other than the
/// platform location. Set it to run a dev build alongside an installed
/// one without sharing `session.json`, recents, or global metric
/// manifests. Inherited by spawned project windows, so one export
/// covers every window a dev instance opens.
pub const OXPLOW_HOME_ENV: &str = "OXPLOW_HOME";

/// Global app-config dir (`<platform config dir>/net.voxland.oxplow`),
/// where launcher-level state like `recent-projects.json` and
/// `session.json` live. Matches Tauri's `app_config_dir()` on macOS /
/// Linux / Windows, unless [`OXPLOW_HOME_ENV`] overrides it. `None`
/// only if the platform config dir is undiscoverable.
///
/// Note this moves oxplow's *own* global state only — Tauri's
/// `app_config_dir()` path resolver (webview storage, etc.) still uses
/// the platform location.
pub fn global_config_dir() -> Option<PathBuf> {
    global_config_dir_from(std::env::var_os(OXPLOW_HOME_ENV))
}

/// [`global_config_dir`] with the env read lifted out, so tests exercise
/// the path logic without mutating process-global env state.
fn global_config_dir_from(home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    // Used verbatim: the override names the dir itself, so a dev
    // instance gets a self-contained home rather than one buried under
    // another `net.voxland.oxplow`. An exported-but-empty value is
    // treated as unset — "" would resolve relative to the cwd.
    match home {
        Some(h) if !h.is_empty() => Some(PathBuf::from(h)),
        _ => dirs::config_dir().map(|d| d.join(APP_IDENTIFIER)),
    }
}

const DEFAULT_SNAPSHOT_RETENTION_DAYS: u32 = 7;
/// Metric retention defaults to KEEP EVERYTHING (tsk93): per-test history is
/// what makes the fact substrate worth having, and ~420k facts read fine. The
/// knob exists so growth can be bounded later without a code change.
const DEFAULT_METRIC_RETENTION_DAYS: u32 = 0;
/// Detail COMPACTION, unlike capture pruning, is on by default (tsk211): it
/// drops only the per-run drill-in payload, never a fact, so no metric value or
/// trend point changes. ~0.5 MB per coverage run made `detail_json` 200 MB of a
/// 795 MB database in under three weeks of heavy use.
const DEFAULT_METRIC_DETAIL_MAX_PER_PRODUCER: u32 = 100;
const DEFAULT_METRIC_DETAIL_RETENTION_DAYS: u32 = 30;
const DEFAULT_SNAPSHOT_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// How long a turn-end snapshot may hold up the Stop hook (P2.3).
pub const DEFAULT_SNAPSHOT_TURN_BUDGET_MS: u64 = 2000;
/// The smallest budget accepted: the capture's own predrain wait is 300 ms.
const MIN_SNAPSHOT_TURN_BUDGET_MS: u64 = 100;
/// The most script workers that run at once, by default (capped by the
/// machine's parallelism).
const DEFAULT_SCRIPTS_MAX_CONCURRENT_CAP: usize = 4;
/// A script worker's memory cap, in MiB, by default.
const DEFAULT_SCRIPTS_MEMORY_MB: u64 = 512;
/// The smallest cap accepted: a worker's own runtime and the tree shipped
/// to it need room before any script runs.
const MIN_SCRIPTS_MEMORY_MB: u64 = 64;
const DEFAULT_INJECT_SESSION_CONTEXT: bool = true;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
pub struct LspServerConfig {
    #[serde(rename = "languageId")]
    pub language_id: String,
    pub extensions: Vec<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// A fact predicate on a `metrics:` spec (the `filter:` block) — the config
/// mirror of the engine's `FactFilter`. A conjunctive predicate
/// keeping only the facts that match before aggregation: `minValue` for a
/// count-over-threshold (complexity ≥ N), `severity` for a lint slice, `dimEq`
/// for a conformed-dimension slice (`[oxplow.rule, unsafe_block]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilterConfig {
    /// Keep facts with `value >= minValue`.
    #[serde(rename = "minValue", default)]
    pub min_value: Option<f64>,
    /// Keep facts with `value <= maxValue` (`0`: the untested files).
    #[serde(rename = "maxValue", default)]
    pub max_value: Option<f64>,
    /// Keep facts whose reported severity equals this (e.g. `error`).
    #[serde(default)]
    pub severity: Option<String>,
    /// Keep facts whose dimension `[key]` equals `[value]` — a 2-element list.
    #[serde(rename = "dimEq", default)]
    pub dim_eq: Option<Vec<String>>,
}

/// A derived-metric formula on a `metrics:` spec (the `formula:` block) — a
/// constrained binary op over two OTHER metric keys (no source measure). The
/// engine aligns the two metrics on their shared rollup key and applies `op`
/// (`div` is the ratio primitive: bugs-per-KLOC, cost-per-token).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormulaConfig {
    /// `add` | `sub` | `mul` | `div`.
    pub op: String,
    /// The left operand metric key.
    pub left: String,
    /// The right operand metric key.
    pub right: String,
}

/// One entry in the top-level `metrics:` block — a **pure read-time SPEC** over a
/// measure. A metric no longer *computes* anything: it names a
/// `sourceMeasure` + an `aggregation` (+ optional `filter`), or a `formula` over
/// other metrics, and the engine aggregates the durable facts a `collectors:` entry
/// emitted. Two forms, distinguished by which key is set:
/// - **`use:`** — enable an existing catalog metric by key (built-in/global),
///   optionally overriding `target`/thresholds for this project.
/// - **`key:`** — define a NEW spec (`sourceMeasure` + `aggregation`, or `formula`).
///
/// Resolved across the three scopes into [`ResolvedSpec`]s. All non-discriminant
/// fields are optional so both forms share one struct; validation enforces the
/// per-form rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MetricEntry {
    /// `use:` form — the catalog key to enable.
    #[serde(rename = "use", default)]
    pub use_key: Option<String>,
    /// `key:` form — the new metric's namespaced key.
    #[serde(default)]
    pub key: Option<String>,
    /// Active flag. `None`/`Some(true)` = active (a bare `use:`/`key:` entry is
    /// on); `Some(false)` = an explicit **disable marker** kept in config so a
    /// default-ON metric (producer/extension) or a config-defined metric can be
    /// turned off without deleting its definition. Not a structural field, so a
    /// `use:` entry may carry it (unlike measure/aggregation/filter/formula).
    #[serde(default)]
    pub enabled: Option<bool>,
    /// An extension metric's default: `on` (the default) is active unless
    /// the project turns it off (`enabled: false`); `off` is active only
    /// while the project lists it with `use:`.
    #[serde(default)]
    pub default: Option<MetricDefault>,
    /// An extension metric's collector: the id of the collector in the same
    /// extension that computes its facts. A collector some metric names
    /// runs only while one of those metrics is active.
    #[serde(default)]
    pub collector: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// The measure whose facts this metric aggregates (required for a `key:`
    /// metric unless it sets `formula`). NULL for a pure formula metric.
    #[serde(rename = "sourceMeasure", default)]
    pub source_measure: Option<String>,
    /// `count` | `sum` | `avg` | `min` | `max` | `last` | `ratio` (default `last`).
    /// Combines the facts WITHIN a capture; cross-time collapse is governed by the
    /// source measure's `temporalSemantics`.
    #[serde(default)]
    pub aggregation: Option<String>,
    /// Fact predicate applied before aggregation (`minValue` / `severity` / `dimEq`).
    #[serde(default)]
    pub filter: Option<FilterConfig>,
    /// Derived-metric formula over other metric keys (mutually exclusive with
    /// `sourceMeasure`).
    #[serde(default)]
    pub formula: Option<FormulaConfig>,
    #[serde(default)]
    pub unit: Option<String>,
    /// `higher-better` | `lower-better` | `neutral` (default `neutral`).
    #[serde(default)]
    pub direction: Option<String>,
    /// Read-time presentation: `gauge` | `findings` | `test` | `coverage` |
    /// `event` (default `gauge`).
    #[serde(rename = "displayKind", default)]
    pub display_kind: Option<String>,
    /// Catalog grouping: `operational` | `testing` | `coverage` | `static-quality` | `custom`.
    #[serde(default)]
    pub category: Option<String>,
    /// Language this metric measures (e.g. `rust`), for the catalog filter.
    #[serde(default)]
    pub language: Option<String>,
    /// One-line human description (shown atop the Metric Detail page). Inherent to
    /// the definition — a `use:` can't override.
    #[serde(default)]
    pub description: Option<String>,
    /// Conformed-dimension keys this metric can be sliced by (drill-across).
    #[serde(rename = "sliceableDims", default)]
    pub sliceable_dims: Vec<String>,
    #[serde(default)]
    pub target: Option<f64>,
    #[serde(rename = "warnAt", default)]
    pub warn_at: Option<f64>,
    #[serde(rename = "failAt", default)]
    pub fail_at: Option<f64>,
    /// Entity metric: the `v_*` view it aggregates, instead of a
    /// measure's facts. Fragments below are SQL over that view, aliased `e`.
    #[serde(default)]
    pub entity: Option<String>,
    /// Entity metric: which rows count (a SQL condition).
    #[serde(rename = "where", default)]
    pub where_: Option<String>,
    /// Entity metric: the timestamp column/expression that makes it an EVENT
    /// metric (rows bucketed by when they happened). Without it the metric is
    /// a STATE metric: its current value, captured over time.
    #[serde(default)]
    pub time: Option<String>,
    /// Entity metric: the value expression aggregated (not needed for `count`).
    #[serde(default)]
    pub value: Option<String>,
}

/// Whether an extension's metric is active before the project says
/// anything about it ([`MetricEntry::default`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MetricDefault {
    On,
    Off,
}

/// A fully-resolved metric SPEC — the flat form the runner (oxplow-app) seeds
/// into `metric_spec` (and, until reads flip, `metric_definition`). Produced by
/// [`resolve_metrics`] after merging the three scopes (built-in ∪ global ∪
/// project, precedence project > global > built-in by key). Not serialized.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSpec {
    pub key: String,
    pub title: String,
    pub source_measure: Option<String>,
    pub aggregation: String,
    pub filter: Option<FilterConfig>,
    pub formula: Option<FormulaConfig>,
    pub unit: Option<String>,
    pub direction: String,
    pub display_kind: String,
    pub category: Option<String>,
    pub language: Option<String>,
    pub description: Option<String>,
    pub sliceable_dims: Vec<String>,
    pub target: Option<f64>,
    pub warn_at: Option<f64>,
    pub fail_at: Option<f64>,
    /// `built-in` | `global` | `project` | `extension:<name>`.
    pub scope: String,
    /// The collector, in the defining extension, that computes its facts
    /// ([`MetricEntry::collector`]).
    pub collector: Option<String>,
    /// Whether this metric is active. Derived from the config entry's `enabled`
    /// flag (default `true`). A disabled spec is still resolved (so the Catalog
    /// can list it as an unchecked toggle), but `seed_catalog` prunes it from the
    /// `metric_spec` table so all spec-driven reads + producer collection stop.
    pub enabled: bool,
    /// Set for an entity metric (tsk322); `aggregation` is then one of
    /// [`ENTITY_METRIC_AGGS`].
    pub entity: Option<EntitySpec>,
}

/// The entity half of an entity metric: SQL over one `v_*` view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntitySpec {
    pub view: String,
    #[serde(rename = "where", default, skip_serializing_if = "Option::is_none")]
    pub where_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// One of [`ENTITY_METRIC_AGGS`].
    pub aggregation: String,
}

/// The dimension half of an entity dimension: a SQL expression over one
/// `v_*` view (aliased `e`), with an optional join.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityDimensionSpec {
    pub view: String,
    pub expr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<String>,
}

/// One entry in the top-level `measures:` block — the **measure catalog**
/// authoring surface. A measure is a *type of atomic
/// fact* a collector may emit (`oxplow.complexity`, `acme.api_latency`, …); the
/// `oxplow.*` built-ins are seeded by the DB migration, so config only *adds*
/// global/project measures. Unlike [`MetricEntry`] there is no `use:`/`key:`
/// split — a measure entry is always a definition (you declare the fact type,
/// you don't "enable" one). Resolved across the global+project scopes by
/// [`resolve_measures`] and seeded into the `measure` table at boot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeasureEntry {
    /// The new measure's namespaced key (`<vendor>.<id>`). Required.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub unit: Option<String>,
    /// The grain's subject kind (`symbol` | `file` | `test` | `model` | …).
    #[serde(rename = "subjectKind", default)]
    pub subject_kind: Option<String>,
    /// `additive` | `semi-additive` | `non-additive` — additivity OVER TIME
    /// (default `semi-additive`).
    #[serde(rename = "temporalSemantics", default)]
    pub temporal_semantics: Option<String>,
    /// `complete` | `per-path` — what ONE capture restates (default `complete`).
    /// A SEPARATE AXIS from `temporalSemantics`: `complete` means every capture
    /// restates the whole population (a coverage report, a test run), so the
    /// temporal fold applies directly. `per-path` means a capture restates only the
    /// paths in its snapshot — which is what a **tree collector over a per-commit delta**
    /// does. Such a measure is folded to the latest capture per (producer, path)
    /// before aggregating, so a repo-wide total stays correct while only changed
    /// files are rescanned. Set this on any measure a snapshot-triggered collector
    /// emits per-file facts on.
    #[serde(rename = "captureScope", default)]
    pub capture_scope: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// A fully-resolved measure — the flat form the boot seeder upserts into the
/// `measure` catalog. Produced by [`resolve_measures`] after merging the global
/// and project scopes (precedence project > global). Not serialized.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedMeasure {
    pub key: String,
    pub title: String,
    pub unit: Option<String>,
    pub subject_kind: Option<String>,
    pub temporal_semantics: String,
    /// `complete` | `per-path` — see [`MeasureEntry::capture_scope`].
    pub capture_scope: String,
    /// `global` | `project` (built-ins are the migration seed, not config).
    pub scope: String,
    pub description: Option<String>,
}

/// One entry in the top-level `dimensions:` block — the **conformed-dimension
/// catalog** authoring surface. A dimension is a
/// slice axis that means the same thing to every fact that carries it
/// (`oxplow.severity`, `acme.license`, …), enabling cross-metric drill-across.
/// Like [`MeasureEntry`] it is definition-only; the `oxplow.*` built-ins are the
/// migration seed. Resolved by [`resolve_dimensions`] and seeded into the
/// `dimension` table at boot; `promote` requests a generated column + index
/// (catalog teeth).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DimensionEntry {
    /// The new dimension's namespaced key (`<vendor>.<id>`). Required.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    /// `categorical` | `numeric` | `temporal` | `entity-ref` (default
    /// `categorical`).
    #[serde(rename = "valueType", default)]
    pub value_type: Option<String>,
    /// For `entity-ref` dims — the subject kind the value points at.
    #[serde(rename = "subjectKind", default)]
    pub subject_kind: Option<String>,
    /// Optional controlled vocabulary (the allowed value set).
    #[serde(default)]
    pub vocabulary: Vec<String>,
    /// Request a generated column + expression index on `fact` for this dim
    /// (fast group-by/filter). Off by default — the long tail lives in
    /// `dims_json`, promoted only when hot.
    #[serde(default)]
    pub promote: bool,
    /// Entity dimension: the `v_*` view it slices, for entity
    /// metrics over the same view.
    #[serde(default)]
    pub entity: Option<String>,
    /// Entity dimension: the SQL expression (over the view, aliased `e`).
    #[serde(default)]
    pub expr: Option<String>,
    /// Entity dimension: an optional join, e.g.
    /// `LEFT JOIN v_thread t ON t.id = e.thread_id`.
    #[serde(default)]
    pub join: Option<String>,
}

/// A fully-resolved dimension — the flat form the boot seeder upserts into the
/// `dimension` catalog. Produced by [`resolve_dimensions`] (project > global).
/// Not serialized.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDimension {
    pub key: String,
    pub label: String,
    pub value_type: String,
    pub subject_kind: Option<String>,
    pub vocabulary: Vec<String>,
    /// `global` | `project` (built-ins are the migration seed, not config).
    pub scope: String,
    pub promote: bool,
    pub entity: Option<EntityDimensionSpec>,
}

/// How this project's tests run (the `testing:` block), written by
/// `/oxplow:configure`. The Bash-hook detector reads the commands and
/// patterns to tell a test or analysis run; what such a run's reports hold
/// is read by the report collectors (`collectors:` with `records:`, see
/// [`collectors::Records`]). All optional.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
pub struct TestingConfig {
    /// The command that runs the project's tests and writes their reports
    /// (counts as a test run; named in the report-less nudge).
    pub command: Option<String>,
    /// The coverage-free counterpart to `command`, for the red/green loop
    /// (tsk171). It still writes a test report (JUnit), so the
    /// progression lands in the effort's Tests panel, but skips coverage
    /// and should accept a filter.
    ///
    /// This exists because the alternative is worse. When the only
    /// report-emitting command is a full instrumented run of the whole
    /// suite, "route every invocation through it" is unfollowable in a TDD
    /// loop, so it gets dropped — and then NONE of the red→green runs are
    /// recorded. A weaker rule that is actually followed beats a stricter
    /// one that isn't.
    #[serde(rename = "fastCommand")]
    pub fast_command: Option<String>,
    /// Extra command substrings that count as a test run, on top of the
    /// built-in defaults (pytest, cargo test, jest, …).
    #[serde(rename = "runPatterns")]
    pub run_patterns: Vec<String>,
    /// Extra command substrings that count as a static-analysis run, on
    /// top of the built-in defaults (cargo clippy, eslint, ruff, …).
    #[serde(rename = "analysisPatterns")]
    pub analysis_patterns: Vec<String>,
    /// Globs (repo-relative, `/`-separated) naming test files, on top of the
    /// built-in conventions (`.test.` / `.spec.` files, `test/` and `tests/`
    /// directories, `test_*.py`, `*_test.go`, …): `spec/**`, `**/it/**`. Change
    /// analysis reads them for `is_test` in `v_change_file` and
    /// `v_change_function` on its next run.
    #[serde(rename = "testPaths")]
    pub test_paths: Vec<String>,
    /// Free-form hint injected verbatim into every agent system prompt:
    /// which test command to run, what coverage to meet, and so on.
    #[serde(rename = "agentHint")]
    pub agent_hint: Option<String>,
}

/// The limits on the processes that run extension and project scripts
/// (the `scripts:` block): how many run at once, host-wide, and how much
/// memory one may use before it is stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ScriptsConfig {
    /// Workers running at once; a run past it waits for a slot. Default
    /// `min(4, available parallelism)`, minimum 1.
    #[specta(type = oxplow_domain::JsNumber)]
    pub max_concurrent: usize,
    /// A worker's memory cap in bytes (`memoryMb` in the file). Default
    /// 512 MiB, minimum 64 MiB.
    #[specta(type = oxplow_domain::JsNumber)]
    pub memory_bytes: u64,
}

impl Default for ScriptsConfig {
    fn default() -> Self {
        let parallelism = std::thread::available_parallelism().map_or(1, |n| n.get());
        Self {
            max_concurrent: parallelism.min(DEFAULT_SCRIPTS_MAX_CONCURRENT_CAP),
            memory_bytes: DEFAULT_SCRIPTS_MEMORY_MB * 1024 * 1024,
        }
    }
}

/// What oxplow watches / snapshots / indexes, on top of the always-on
/// `.git`/`.oxplow` ignores and the repo's `.gitignore`.
///
/// - `exclude`: extra paths to ignore even when `.gitignore` doesn't
///   (e.g. a tracked-but-noisy generated file).
/// - `include`: gitignored paths to force back in (override
///   `.gitignore` for something oxplow should still see).
///
/// Each entry is a single segment (matches any path component —
/// `target` matches every `target/`) or a repo-relative path (matches
/// that path exactly or as a prefix — `apps/desktop/dist`).
// No `skip_serializing_if`: specta's unified-mode TS export forbids it
// (the whole `generated` key is only written when non-empty anyway).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct GeneratedConfig {
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub include: Vec<String>,
}

/// One row of the project's `zones:` table: the globs that select files
/// and the zone label they belong to.
///
/// Zones are entirely project-defined (tsk251) — oxplow ships no rule
/// table of its own, because a file's architectural role follows from
/// how *this* project lays out its repo. Order in the table is
/// load-bearing: [`ZoneRules`](../oxplow_code_deps/zones/struct.ZoneRules.html)
/// takes the FIRST matching rule, so specific patterns must precede
/// catch-alls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, schemars::JsonSchema)]
pub struct ZoneRuleConfig {
    /// Globs selecting the files in this zone (any-of). In YAML `match`
    /// accepts a single string or a list; both land here as a list.
    /// Matched against the full repo-relative path with `*` stopping at
    /// `/` — `**` is the only way to span directories.
    #[serde(rename = "match")]
    pub patterns: Vec<String>,
    /// The zone label. Free-form; `other` and `external` are reserved.
    pub zone: String,
    /// Optional display colour (`#rrggbb` or `#rgb`). The first rule to
    /// name a label wins; labels with no colour get a palette entry
    /// assigned by order of first appearance.
    #[serde(default)]
    pub color: Option<String>,
}

/// Zone label for a file no rule matched.
pub const ZONE_OTHER: &str = "other";
/// Zone label for an import target outside the repo (a third-party
/// crate / package), which is never a layer violation.
pub const ZONE_EXTERNAL: &str = "external";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct OxplowConfig {
    /// The agent harnesses enabled for this project, by registry key, in
    /// priority order: the first is a new session's default. Empty (the
    /// default) enables every registered harness, the first declared one
    /// the default.
    pub agents: Vec<String>,
    /// Human-readable project name. Defaults to the basename of the
    /// project dir when not set in .oxplow/project.yaml.
    #[serde(rename = "projectName")]
    pub project_name: String,
    /// Extra language servers registered on top of the built-ins.
    #[serde(rename = "lspServers")]
    pub lsp_servers: Vec<LspServerConfig>,
    /// User-supplied text appended verbatim to every agent's system prompt.
    #[serde(rename = "agentPromptAppend")]
    pub agent_prompt_append: String,
    /// File-snapshot retention window in days. 0 disables pruning.
    #[serde(rename = "snapshotRetentionDays")]
    pub snapshot_retention_days: u32,
    /// Metric-capture retention window in days: captures older than the
    /// cutoff are pruned (facts cascade) — EXCEPT effort-stamped ones
    /// (attribution history), each producer's newest capture, and any capture
    /// still contributing a current value to the fold. **0 (the default)
    /// keeps everything** (tsk93); pruning trades away per-test drill-down /
    /// flakiness horizon for bounded growth, so it is strictly opt-in.
    #[serde(rename = "metricRetentionDays")]
    pub metric_retention_days: u32,
    /// Keep the per-run `detail_json` drill-in payload for only the newest N
    /// captures **per producer**; older ones are compacted (the payload is
    /// nulled, the capture and all its facts stay). `0` disables the cap.
    ///
    /// This is the knob that bounds a BUSY project: the payload is ~0.5 MB per
    /// coverage run, so age alone never catches up with a repo that runs tests
    /// dozens of times a day (tsk211). Compaction never changes a metric value
    /// — only the effort Tests/Coverage panel's detail for old runs is lost.
    #[serde(rename = "metricDetailMaxPerProducer")]
    pub metric_detail_max_per_producer: u32,
    /// Compact `detail_json` older than this many days. `0` disables it.
    /// Complements [`Self::metric_detail_max_per_producer`]: the count cap
    /// bounds a busy repo, this reaches a project that has gone quiet.
    #[serde(rename = "metricDetailRetentionDays")]
    pub metric_detail_retention_days: u32,
    /// Extra `exclude`/`include` paths layered on top of `.gitignore`
    /// for fs-watch / snapshot capture / code-quality scans. `.git`,
    /// `.oxplow`, and everything in `.gitignore` (+ `.git/info/exclude`)
    /// are ignored automatically — this only adds extras or forces
    /// gitignored paths back in. See [`GeneratedConfig`].
    #[serde(rename = "generated")]
    pub generated: GeneratedConfig,
    /// Maximum blob size for content-addressed snapshotting; larger
    /// files get a stat-only entry. Default 5 MiB.
    #[serde(rename = "snapshotMaxFileBytes")]
    #[specta(type = oxplow_domain::JsNumber)]
    pub snapshot_max_file_bytes: u64,
    /// How long the Stop hook waits for the turn-end snapshot, in ms.
    /// A take that runs longer keeps going in the background and is
    /// recorded as over budget. Default 2000.
    #[serde(rename = "snapshotTurnBudgetMs")]
    #[specta(type = oxplow_domain::JsNumber)]
    pub snapshot_turn_budget_ms: u64,
    /// When true, the UserPromptSubmit hook injects a session-context
    /// block into every agent prompt.
    #[serde(rename = "injectSessionContext")]
    pub inject_session_context: bool,
    /// Hex colour composited behind this project's app icon, so concurrent
    /// windows are tellable apart at a glance (`#c2410c`, `#abc`, or
    /// unprefixed). `None` leaves the stock icon alone.
    ///
    /// Per-project rather than a dev-only flag: colour-coding *any* checkout is
    /// the general case, and "this is the dev build" is just one instance of
    /// it. Consumed on macOS only — see the desktop shell's `icon_tint`.
    ///
    /// No `skip_serializing_if`: this type is a command result, and specta
    /// rejects conditional omission in unified mode. Absence from the written
    /// YAML is handled by `render_project_config`, which builds its mapping by
    /// hand rather than through this derive.
    #[serde(rename = "iconTint")]
    pub icon_tint: Option<String>,
    /// How the project's tests run (the `testing:` block).
    pub testing: TestingConfig,
    /// Limits on script workers (the `scripts:` block).
    pub scripts: ScriptsConfig,
    /// Which metrics are on in this project (the `metrics:` block): a `use:`
    /// turns one on, layering its thresholds; an `enabled: false` marker
    /// turns an on-by-default one off. Metrics, measures, dimensions and
    /// collectors are defined by extensions (the project's own in its own
    /// extension); see [`resolve_metrics`].
    #[serde(default)]
    pub metrics: Vec<MetricEntry>,
    /// Project-declared architectural zones (the `zones:` block) — an
    /// ORDERED table, first match wins. Empty (the default) means oxplow
    /// classifies nothing: every file is `other` and the Change-analysis
    /// zone surfaces stay empty until the project declares its own.
    #[serde(default)]
    pub zones: Vec<ZoneRuleConfig>,
    /// Each harness's configuration, by its key: what its `launch` reads,
    /// e.g. `agentConfig: { opencode: { model: "github-copilot/gpt-5-mini" } }`.
    #[serde(rename = "agentConfig")]
    #[specta(type = std::collections::BTreeMap<String, oxplow_domain::Json>)]
    pub agent_config: std::collections::BTreeMap<String, serde_json::Value>,
    /// The project's extension provider instances
    /// (`extensionInstances: { "<ext>/<id>": { enabled, config } }`).
    #[serde(rename = "extensionInstances")]
    pub extension_instances: std::collections::BTreeMap<String, ExtensionInstanceConfig>,
    /// The project's choice of implementation per choosable capability
    /// (`activeProviders: { work_items: issues, effort_policy: none }`); a
    /// capability absent here keeps its default.
    #[serde(rename = "activeProviders")]
    pub active_providers: std::collections::BTreeMap<String, String>,
    /// A person's own choices (`activeProviders` in
    /// `.oxplow/personal.yaml`), over the project's.
    #[serde(rename = "personalActiveProviders")]
    pub personal_active_providers: std::collections::BTreeMap<String, String>,
    /// What a new thread starts with, a person's own (`newThreadSession`
    /// in `.oxplow/personal.yaml`): `ask` (the session picker), `none`, or
    /// an agent (a harness's key). Unset means `ask`.
    #[serde(rename = "personalNewThreadSession")]
    pub personal_new_thread_session: Option<String>,
    /// Whose component shows for a replaceable core sub-component, by
    /// target (`replacements: { work_item.board: task-views,
    /// vcs.history.graph: oxplow }`): an extension replacing it, or
    /// `oxplow` for oxplow's own. A target not listed shows its default:
    /// the only extension replacing it, or a bound target's provider's.
    /// Only `oxplow` may be chosen for a bound target (its provider's
    /// extension decides otherwise).
    #[serde(rename = "replacements")]
    pub replacements: std::collections::BTreeMap<String, String>,
    /// How long this project keeps each event namespace's payloads and
    /// large content (`eventRetention: { agent: { payloadDays,
    /// contentDays } }`), over core's defaults; an extension's namespace is
    /// kept no longer than its own window.
    #[serde(rename = "eventRetention")]
    pub event_retention:
        std::collections::BTreeMap<String, oxplow_domain::events::retention::RetentionWindow>,
    /// This project's AI role assignments (`ai: { roles: … }`), layered
    /// over the user-global `ai.yaml`. Keyed by role name (one of
    /// [`AI_ROLE_NAMES`]). Provider ids refer to each person's `ai.yaml`.
    #[serde(rename = "aiRoles")]
    pub ai_roles: std::collections::BTreeMap<String, AiRoleOverride>,
    /// The extensions on for the whole team (`extensions: [...]`;
    /// [`DEFAULT_EXTENSIONS`] when absent). Committed. oxplow-foundation is
    /// always on and never listed.
    #[serde(rename = "extensions")]
    pub extensions: Vec<String>,
    /// Where an extension's source is when it isn't
    /// `.oxplow/extensions/<name>/` (`extensionSources: { <name>: <folder> }`),
    /// relative to the checkout or absolute.
    #[serde(rename = "extensionSources")]
    pub extension_sources: std::collections::BTreeMap<String, String>,
    /// A person's own turning on and off over the team's list
    /// (`enableExtensions`, `disableExtensions` in `.oxplow/personal.yaml`).
    #[serde(rename = "personalExtensions")]
    pub personal_extensions: PersonalExtensions,
    /// A person's own sources (`extensionSources` in
    /// `.oxplow/personal.yaml`), over the team's.
    #[serde(rename = "personalExtensionSources")]
    pub personal_extension_sources: std::collections::BTreeMap<String, String>,
    /// `enableBundledExtensions`: `false` asks a dev build to run oxplow's
    /// own extensions from their `extensionSources` folders, not the
    /// copies built in ([`bundled_from_sources`]).
    #[serde(rename = "enableBundledExtensions")]
    pub enable_bundled_extensions: Option<bool>,
    /// The person's own `enableBundledExtensions`, over the project's.
    #[serde(rename = "personalEnableBundledExtensions")]
    pub personal_enable_bundled_extensions: Option<bool>,
}

/// A person's `enableExtensions` and `disableExtensions`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct PersonalExtensions {
    /// Turned on for this person, beside the team's list.
    pub enable: Vec<String>,
    /// Turned off for this person, whoever listed them.
    pub disable: Vec<String>,
}

/// Which layer turned an extension on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum EnabledBy {
    /// `.oxplow/project.yaml`'s `extensions`.
    Team,
    /// `.oxplow/personal.yaml`'s `enableExtensions`.
    Personal,
}

/// What decides which extensions are on and where their sources are: the
/// team's list and sources in `.oxplow/project.yaml`, and a person's
/// overrides in `.oxplow/personal.yaml`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExtensionSettings {
    pub team: Vec<String>,
    pub team_sources: std::collections::BTreeMap<String, String>,
    pub personal: PersonalExtensions,
    pub personal_sources: std::collections::BTreeMap<String, String>,
    /// `enableBundledExtensions` as asked (the person's, else the team's).
    pub bundled_requested: Option<bool>,
    /// oxplow's own extensions run from their sources like any extension,
    /// not from the copies built in ([`bundled_from_sources`]).
    pub bundled_from_sources: bool,
}

/// Whether oxplow's own extensions run from their source folders: asked
/// (`enableBundledExtensions: false`) and a dev build. A shipped oxplow
/// always runs the copies built into it, so a cloned repo can't replace
/// oxplow's own commands.
pub fn bundled_from_sources(requested: Option<bool>, dev_build: bool) -> bool {
    dev_build && requested == Some(false)
}

/// The folder that holds extension sources by default, under a checkout.
pub const EXTENSION_SOURCES_DIR: &str = ".oxplow/extensions";

/// The team's list when `.oxplow/project.yaml` has no `extensions` key.
pub const DEFAULT_EXTENSIONS: &[&str] = &["oxplow-bundled"];

fn default_extensions() -> Vec<String> {
    DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect()
}

impl ExtensionSettings {
    pub fn of(config: &OxplowConfig) -> Self {
        Self {
            team: config.extensions.clone(),
            team_sources: config.extension_sources.clone(),
            personal: config.personal_extensions.clone(),
            personal_sources: config.personal_extension_sources.clone(),
            ..Self::bundled(
                config
                    .personal_enable_bundled_extensions
                    .or(config.enable_bundled_extensions),
            )
        }
    }

    /// The settings `requested` (`enableBundledExtensions`) makes, in
    /// this build.
    fn bundled(requested: Option<bool>) -> Self {
        Self {
            bundled_requested: requested,
            bundled_from_sources: bundled_from_sources(requested, cfg!(debug_assertions)),
            ..Self::default()
        }
    }

    /// The extensions on: (team ∪ personal enable) − personal disable, in
    /// that order, each with the layer that turned it on.
    pub fn enabled(&self) -> Vec<(String, EnabledBy)> {
        let mut out: Vec<(String, EnabledBy)> = Vec::new();
        let layers = self.team.iter().map(|n| (n, EnabledBy::Team)).chain(
            self.personal
                .enable
                .iter()
                .map(|n| (n, EnabledBy::Personal)),
        );
        for (name, layer) in layers {
            if !self.personal.disable.contains(name) && !out.iter().any(|(n, _)| n == name) {
                out.push((name.clone(), layer));
            }
        }
        out
    }

    pub fn is_enabled(&self, name: &str) -> bool {
        self.enabled().iter().any(|(n, _)| n == name)
    }

    /// Where `name`'s source is, in the checkout at `worktree`: the
    /// person's `extensionSources` entry, then the team's, then
    /// `.oxplow/extensions/<name>/`. A relative entry is the checkout's; `~/`
    /// is the home folder.
    pub fn source(&self, worktree: &Path, name: &str) -> std::path::PathBuf {
        let entry = self
            .personal_sources
            .get(name)
            .or_else(|| self.team_sources.get(name));
        match entry {
            Some(raw) => match raw.strip_prefix("~/") {
                Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
                None => worktree.join(raw),
            },
            None => worktree.join(EXTENSION_SOURCES_DIR).join(name),
        }
    }
}

/// [`ExtensionSettings`] for `project_dir`, without loading (or failing on)
/// the rest of either file: an unreadable or invalid block reads as empty.
/// The extension loader calls this on every load.
pub fn extension_settings(project_dir: impl AsRef<Path>) -> ExtensionSettings {
    #[derive(Deserialize, Default)]
    struct OnlyExtensions {
        #[serde(default)]
        extensions: Option<Vec<String>>,
        #[serde(default, rename = "enableExtensions")]
        enable: Vec<String>,
        #[serde(default, rename = "disableExtensions")]
        disable: Vec<String>,
        #[serde(default, rename = "extensionSources")]
        extension_sources: std::collections::BTreeMap<String, String>,
        #[serde(default, rename = "enableBundledExtensions")]
        enable_bundled: Option<bool>,
    }
    fn read(path: &Path) -> OnlyExtensions {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_yaml::from_str(&raw).ok())
            .unwrap_or_default()
    }
    let dir = project_dir.as_ref();
    let team = read(&config_path(dir));
    let personal = read(&personal_path(dir));
    ExtensionSettings {
        team: team.extensions.unwrap_or_else(default_extensions),
        team_sources: team.extension_sources,
        personal: PersonalExtensions {
            enable: personal.enable,
            disable: personal.disable,
        },
        personal_sources: personal.extension_sources,
        ..ExtensionSettings::bundled(personal.enable_bundled.or(team.enable_bundled))
    }
}

/// The built-in role names `ai.roles` accepts beside extension AI
/// profiles (`<extension>/<profile>`). Must match
/// `oxplow_ai::config::BuiltinRole` (a test in oxplow-app checks).
pub const AI_ROLE_NAMES: [&str; 3] = ["main", "summarize", "decide"];

/// Roles oxplow dropped because nothing asked them (`oxplow_ai`'s
/// `DROPPED_ROLES`): naming one fails saying so.
const DROPPED_AI_ROLES: [&str; 3] = ["fast", "embed", "review"];

/// One `ai.roles` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AiRoleOverride {
    pub provider: String,
    pub model: String,
    /// What the project forbids this binding (`restrict: [tools, sql]`);
    /// absent, nothing.
    #[serde(default)]
    pub restrict: std::collections::BTreeSet<oxplow_domain::ai::AiRestriction>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawAiBlock {
    #[serde(default)]
    roles: std::collections::BTreeMap<String, AiRoleOverride>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error(".oxplow/project.yaml parse error: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error(".oxplow/project.yaml validation: {0}")]
    Invalid(String),
    /// The user-global instances file ([`GlobalInstances`]).
    #[error("instances.yaml: {0}")]
    Instances(String),
}

/// Raw `zones:` row. `match` accepts a scalar or a sequence, so a
/// single-pattern zone reads as `match: crates/db/**` rather than a
/// one-element list.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawZoneRule {
    #[serde(rename = "match")]
    patterns: StringOrList,
    zone: String,
    #[serde(default)]
    color: Option<String>,
}

/// A YAML field that may be written as one string or a list of them.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
enum StringOrList {
    One(String),
    Many(Vec<String>),
}

impl StringOrList {
    fn into_vec(self) -> Vec<String> {
        match self {
            StringOrList::One(s) => vec![s],
            StringOrList::Many(v) => v,
        }
    }
}

/// Raw `generated:` block — `{ exclude: [...], include: [...] }`.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawGenerated {
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    include: Vec<String>,
}

/// Split a hex colour into 8-bit RGB channels. Accepts `#rrggbb`, the
/// `#rgb` shorthand (expanded the way CSS does, so `#abc` == `#aabbcc`), and
/// either spelling without the leading `#`. `None` for anything else, which is
/// what [`OxplowConfig::icon_tint`] validation rejects on.
pub fn parse_hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    let expanded = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => hex.to_string(),
        _ => return None,
    };
    if !expanded.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&expanded[i..i + 2], 16).ok();
    Some((channel(0)?, channel(2)?, channel(4)?))
}

/// Internal raw shape, used to validate before promoting to
/// `OxplowConfig`. Mirrors the TS `ParsedOxplowConfig` interface.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    /// Enabled agent harnesses by key, in priority order; the first is a new session's default. Absent enables every registered one.
    #[serde(default)]
    agents: Option<Vec<String>>,
    /// Display name; defaults to the project directory's basename.
    #[serde(rename = "projectName", default)]
    project_name: Option<String>,
    /// Extra language servers: `{ servers: [{ languageId, extensions, command, args }] }`. Runs programs.
    #[serde(default)]
    lsp: Option<RawLspBlock>,
    /// Text appended verbatim to every agent's system prompt. Steers every agent.
    #[serde(rename = "agentPromptAppend", default)]
    agent_prompt_append: Option<String>,
    /// File-snapshot retention in days; 0 disables pruning.
    #[serde(rename = "snapshotRetentionDays", default)]
    snapshot_retention_days: Option<f64>,
    /// Metric-capture retention in days; 0 (default) keeps everything.
    #[serde(rename = "metricRetentionDays", default)]
    metric_retention_days: Option<f64>,
    /// Keep per-run detail for only the newest N captures per producer; 0 disables the cap.
    #[serde(rename = "metricDetailMaxPerProducer", default)]
    metric_detail_max_per_producer: Option<f64>,
    /// Compact per-run detail older than this many days; 0 disables.
    #[serde(rename = "metricDetailRetentionDays", default)]
    metric_detail_retention_days: Option<f64>,
    /// Extra `exclude` / `include` paths layered over .gitignore for watching, snapshots and scans.
    #[serde(rename = "generated", default)]
    generated: Option<RawGenerated>,
    /// Largest file snapshotted by content; bigger files get a stat-only entry.
    #[serde(rename = "snapshotMaxFileBytes", default)]
    snapshot_max_file_bytes: Option<f64>,
    /// How long (ms) the Stop hook waits for the turn-end snapshot before moving on; the take keeps going and is recorded as over budget. Default 2000, minimum 100.
    #[serde(rename = "snapshotTurnBudgetMs", default)]
    snapshot_turn_budget_ms: Option<f64>,
    /// Inject the session-context block into every agent prompt.
    #[serde(rename = "injectSessionContext", default)]
    inject_session_context: Option<bool>,
    /// Hex colour composited behind the app icon so windows are tellable apart (macOS).
    #[serde(rename = "iconTint", default)]
    icon_tint: Option<String>,
    /// How the project's tests run: its test commands, the extra command patterns that count as a test or analysis run, and a hint injected into every agent prompt. Steers agents.
    #[serde(default)]
    testing: Option<RawTestingBlock>,
    /// Limits on the processes that run scripts: `maxConcurrent` (workers at once, minimum 1) and `memoryMb` (one worker's memory cap, minimum 64). Defaults: min(4, cores) and 512.
    #[serde(default)]
    scripts: Option<RawScriptsBlock>,
    /// Which metrics are on: `use` turns one on (with optional target/warnAt/failAt), `enabled: false` turns an on-by-default one off. Metrics are defined in extensions.
    #[serde(default)]
    metrics: Option<Vec<MetricEntry>>,
    /// Retired: read as an error naming the project's extension.
    #[serde(default)]
    #[schemars(skip)]
    collectors: Option<serde_yaml::Value>,
    /// Retired: read as an error naming the project's extension.
    #[serde(default)]
    #[schemars(skip)]
    measures: Option<serde_yaml::Value>,
    /// Retired: read as an error naming the project's extension.
    #[serde(default)]
    #[schemars(skip)]
    dimensions: Option<serde_yaml::Value>,
    /// Architectural zones: an ORDERED rule table, first match wins; `other`/`external` are reserved labels.
    #[serde(default)]
    zones: Option<Vec<RawZoneRule>>,
    /// Each agent harness's configuration by its key, e.g. `{ opencode: { model: "github-copilot/gpt-5-mini" } }`. Runs programs.
    #[serde(rename = "agentConfig", default)]
    agent_config: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    /// Retired: read as an error naming `agentConfig`.
    #[serde(rename = "agentModels", default)]
    #[schemars(skip)]
    agent_models: Option<serde_yaml::Value>,
    /// Instances of extension providers, by `<extension>/<instance id>`: `{ enabled, config, provider? }` (the provider's instance config; `provider` names which of the extension's providers an instance with its own id is). Enabling one runs its program once this machine approved it.
    #[serde(rename = "extensionInstances", default)]
    extension_instances: Option<std::collections::BTreeMap<String, ExtensionInstanceConfig>>,
    /// Each capability's active provider, by instance id: `{ work_items: issues }` (a provider's default instance has the provider's id). New work items file there; absent, oxplow's own. A provider that isn't running is a failure, never a fallback.
    #[serde(rename = "activeProviders", default)]
    active_providers: Option<std::collections::BTreeMap<String, String>>,
    /// Whose component shows for a replaceable core sub-component, by target: `{ work_item.board: task-views, vcs.history.graph: oxplow }` — an extension replacing it, or `oxplow` for oxplow's own. Unlisted, the only extension replacing it shows (several: oxplow's own); a bound target (`work_item.detail.state`) shows its provider's extension's, and may only be set to `oxplow`.
    #[serde(rename = "replacements", default)]
    replacements: Option<std::collections::BTreeMap<String, String>>,
    /// How long each event namespace's payloads and large content are kept, in days: `{ agent: { payloadDays: 60, contentDays: 14 } }`, over core's defaults (agent 30/14; test, code, collector, effect 90/30). An extension's namespace is kept no longer than its window; core state (snapshot, vcs, effort, …) is kept whole.
    #[serde(rename = "eventRetention", default)]
    event_retention: Option<
        std::collections::BTreeMap<String, oxplow_domain::events::retention::RetentionWindow>,
    >,
    /// AI role assignments `{ roles: { <role>: { provider, model } } }`, layered over the user's ai.yaml.
    #[serde(default)]
    ai: Option<RawAiBlock>,
    /// The extensions on for the whole team: `[names]`; absent, `[oxplow-bundled]`. Enables code. oxplow-foundation is always on and never listed; a person turns others on or off for themselves in `.oxplow/personal.yaml` (`enableExtensions`, `disableExtensions`).
    #[serde(default)]
    extensions: Option<Vec<String>>,
    /// Where an extension's source is when it isn't `.oxplow/extensions/<name>/`: `{ <name>: <folder> }`, relative to the checkout or absolute (`~/` is the home folder). Enables code.
    #[serde(rename = "extensionSources", default)]
    extension_sources: Option<std::collections::BTreeMap<String, String>>,
    /// Run oxplow's own extensions (oxplow-foundation, oxplow-bundled) from the copies built into it: the default. `false`, honored only by a dev build, runs them from their `extensionSources` folders and deploys them like any extension — how oxplow develops itself. Read when oxplow starts. Enables code.
    #[serde(rename = "enableBundledExtensions", default)]
    enable_bundled_extensions: Option<bool>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawScriptsBlock {
    #[serde(rename = "maxConcurrent", default)]
    max_concurrent: Option<f64>,
    #[serde(rename = "memoryMb", default)]
    memory_mb: Option<f64>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawTestingBlock {
    #[serde(default)]
    command: Option<String>,
    #[serde(rename = "fastCommand", default)]
    fast_command: Option<String>,
    #[serde(rename = "runPatterns", default)]
    run_patterns: Option<Vec<String>>,
    #[serde(rename = "analysisPatterns", default)]
    analysis_patterns: Option<Vec<String>>,
    #[serde(rename = "testPaths", default)]
    test_paths: Option<Vec<String>>,
    #[serde(rename = "agentHint", default)]
    agent_hint: Option<String>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawLspBlock {
    #[serde(default)]
    servers: Option<Vec<RawLspServer>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawLspServer {
    #[serde(rename = "languageId")]
    language_id: String,
    extensions: Vec<String>,
    command: String,
    #[serde(default)]
    args: Vec<String>,
}

/// Load `.oxplow/project.yaml` from `project_dir`, falling back to defaults
/// when the file is absent. The default `project_name` is the
/// basename of the resolved project directory.
pub fn load_project_config(project_dir: impl AsRef<Path>) -> Result<OxplowConfig, ConfigError> {
    let project_dir = project_dir.as_ref();
    let config_path = config_path(project_dir);
    let fallback_name = basename(project_dir);

    let personal = load_personal(project_dir)?;
    if !config_path.exists() {
        info!(
            config_path = %config_path.display(),
            "project config not found; using defaults"
        );
        return Ok(personal.onto(default_config(fallback_name)));
    }

    let raw = std::fs::read_to_string(&config_path)?;
    let doc: serde_yaml::Value = serde_yaml::from_str(&raw)?;
    let config = personal.onto(parse_project_config(doc, &fallback_name)?);
    info!(
        config_path = %config_path.display(),
        agents = ?config.agents,
        project_name = %config.project_name,
        lsp_servers = config.lsp_servers.len(),
        "loaded project config"
    );
    Ok(config)
}

/// A person's layer (`.oxplow/personal.yaml`), as it loads.
#[derive(Debug, Clone, Default)]
struct PersonalLayer {
    active_providers: std::collections::BTreeMap<String, String>,
    new_thread_session: Option<String>,
    extensions: PersonalExtensions,
    extension_sources: std::collections::BTreeMap<String, String>,
    enable_bundled_extensions: Option<bool>,
}

impl PersonalLayer {
    fn of(config: &OxplowConfig) -> Self {
        Self {
            active_providers: config.personal_active_providers.clone(),
            new_thread_session: config.personal_new_thread_session.clone(),
            extensions: config.personal_extensions.clone(),
            extension_sources: config.personal_extension_sources.clone(),
            enable_bundled_extensions: config.personal_enable_bundled_extensions,
        }
    }

    /// `config` with this layer in it.
    fn onto(self, config: OxplowConfig) -> OxplowConfig {
        OxplowConfig {
            personal_active_providers: self.active_providers,
            personal_new_thread_session: self.new_thread_session,
            personal_extensions: self.extensions,
            personal_extension_sources: self.extension_sources,
            personal_enable_bundled_extensions: self.enable_bundled_extensions,
            ..config
        }
    }

    /// As the file holds it.
    fn to_mapping(&self) -> serde_yaml::Mapping {
        let mut map = serde_yaml::Mapping::new();
        if !self.active_providers.is_empty() {
            map.insert(
                "activeProviders".into(),
                serde_yaml::to_value(&self.active_providers).expect("a map serializes"),
            );
        }
        if let Some(choice) = &self.new_thread_session {
            map.insert("newThreadSession".into(), choice.as_str().into());
        }
        if !self.extensions.enable.is_empty() {
            map.insert(
                "enableExtensions".into(),
                serde_yaml::to_value(&self.extensions.enable).expect("a list serializes"),
            );
        }
        if !self.extensions.disable.is_empty() {
            map.insert(
                "disableExtensions".into(),
                serde_yaml::to_value(&self.extensions.disable).expect("a list serializes"),
            );
        }
        if !self.extension_sources.is_empty() {
            map.insert(
                "extensionSources".into(),
                serde_yaml::to_value(&self.extension_sources).expect("a map serializes"),
            );
        }
        if let Some(on) = self.enable_bundled_extensions {
            map.insert("enableBundledExtensions".into(), on.into());
        }
        map
    }
}

/// The person's layer (`.oxplow/personal.yaml`); empty when the file is
/// absent.
fn load_personal(project_dir: &Path) -> Result<PersonalLayer, ConfigError> {
    let path = personal_path(project_dir);
    if !path.exists() {
        return Ok(Default::default());
    }
    let doc: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&path)?)?;
    parse_personal(doc)
}

/// Validate a personal layer: only [`PERSONAL_KEYS`], each as the project
/// file would take it.
fn parse_personal(doc: serde_yaml::Value) -> Result<PersonalLayer, ConfigError> {
    let map = match doc {
        serde_yaml::Value::Mapping(m) => m,
        serde_yaml::Value::Null => return Ok(Default::default()),
        _ => {
            return Err(ConfigError::Invalid(
                ".oxplow/personal.yaml is not a mapping".into(),
            ))
        }
    };
    let mut layer = PersonalLayer::default();
    for (key, value) in map {
        match key.as_str() {
            Some("activeProviders") => {
                let raw: std::collections::BTreeMap<String, String> = serde_yaml::from_value(value)
                    .map_err(|e| {
                        ConfigError::Invalid(format!(".oxplow/personal.yaml: activeProviders: {e}"))
                    })?;
                layer.active_providers = validate_active_providers(raw)
                    .map_err(|e| ConfigError::Invalid(format!(".oxplow/personal.yaml: {e}")))?;
            }
            Some("newThreadSession") => {
                let choice = value.as_str().map(str::trim).unwrap_or_default();
                if choice.is_empty() {
                    return Err(ConfigError::Invalid(
                        ".oxplow/personal.yaml: newThreadSession must be `ask`, `none` or an agent"
                            .into(),
                    ));
                }
                layer.new_thread_session = Some(choice.to_string());
            }
            Some(key @ ("enableExtensions" | "disableExtensions")) => {
                let names: Vec<String> = serde_yaml::from_value(value).map_err(|e| {
                    ConfigError::Invalid(format!(".oxplow/personal.yaml: {key}: {e}"))
                })?;
                if key == "enableExtensions" {
                    layer.extensions.enable = names;
                } else {
                    layer.extensions.disable = names;
                }
            }
            Some("extensionSources") => {
                layer.extension_sources = serde_yaml::from_value(value).map_err(|e| {
                    ConfigError::Invalid(format!(".oxplow/personal.yaml: extensionSources: {e}"))
                })?;
            }
            Some("enableBundledExtensions") => {
                layer.enable_bundled_extensions =
                    Some(serde_yaml::from_value(value).map_err(|e| {
                        ConfigError::Invalid(format!(
                            ".oxplow/personal.yaml: enableBundledExtensions: {e}"
                        ))
                    })?);
            }
            other => {
                return Err(ConfigError::Invalid(format!(
                    ".oxplow/personal.yaml may set only {} (found `{}`)",
                    PERSONAL_KEYS.join(", "),
                    other.unwrap_or("?")
                )))
            }
        }
    }
    Ok(layer)
}

/// `key`'s value in `config`'s personal layer, as the file holds it;
/// `None` when it's unset there.
pub fn personal_value(config: &OxplowConfig, key: &str) -> Option<serde_json::Value> {
    match key {
        "activeProviders" if !config.personal_active_providers.is_empty() => {
            serde_json::to_value(&config.personal_active_providers).ok()
        }
        "newThreadSession" => config
            .personal_new_thread_session
            .as_ref()
            .map(|c| serde_json::Value::String(c.clone())),
        "enableExtensions" if !config.personal_extensions.enable.is_empty() => {
            serde_json::to_value(&config.personal_extensions.enable).ok()
        }
        "disableExtensions" if !config.personal_extensions.disable.is_empty() => {
            serde_json::to_value(&config.personal_extensions.disable).ok()
        }
        "extensionSources" if !config.personal_extension_sources.is_empty() => {
            serde_json::to_value(&config.personal_extension_sources).ok()
        }
        "enableBundledExtensions" => config
            .personal_enable_bundled_extensions
            .map(serde_json::Value::Bool),
        _ => None,
    }
}

/// `layer` with `key` set (`Some`) or removed (`None`), validated as the
/// file would load.
fn personal_with(
    layer: serde_yaml::Mapping,
    key: &str,
    value: Option<&serde_json::Value>,
) -> Result<serde_yaml::Mapping, ConfigError> {
    if !PERSONAL_KEYS.contains(&key) {
        return Err(ConfigError::Invalid(format!(
            "`{key}` isn't a personal key ({})",
            PERSONAL_KEYS.join(", ")
        )));
    }
    let mut map = layer;
    let yaml_key = serde_yaml::Value::String(key.to_string());
    match value {
        Some(v) => {
            map.insert(yaml_key, to_yaml(v));
        }
        None => {
            map.remove(&yaml_key);
        }
    }
    parse_personal(serde_yaml::Value::Mapping(map.clone()))?;
    Ok(map)
}

/// `config` with `key` set (`Some`) or removed (`None`) in its personal
/// layer, validated as the file would load; the layer's other keys stay.
pub fn with_personal_key(
    config: &OxplowConfig,
    key: &str,
    value: Option<&serde_json::Value>,
) -> Result<OxplowConfig, ConfigError> {
    let map = personal_with(PersonalLayer::of(config).to_mapping(), key, value)?;
    let layer = parse_personal(serde_yaml::Value::Mapping(map))?;
    Ok(layer.onto(config.clone()))
}

/// Set (`Some`) or remove (`None`) `key` in a person's layer
/// (`.oxplow/personal.yaml`), validated as it would load. Only
/// [`PERSONAL_KEYS`]; the file goes when nothing is left in it.
pub fn write_personal_key(
    project_dir: impl AsRef<Path>,
    key: &str,
    value: Option<&serde_json::Value>,
) -> Result<(), ConfigError> {
    let path = personal_path(project_dir.as_ref());
    let current = if path.exists() {
        match serde_yaml::from_str::<serde_yaml::Value>(&std::fs::read_to_string(&path)?)? {
            serde_yaml::Value::Mapping(m) => m,
            _ => serde_yaml::Mapping::new(),
        }
    } else {
        serde_yaml::Mapping::new()
    };
    let doc = serde_yaml::Value::Mapping(personal_with(current, key, value)?);
    match &doc {
        serde_yaml::Value::Mapping(m) if m.is_empty() => {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
        }
        _ => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, serde_yaml::to_string(&doc)?)?;
        }
    }
    Ok(())
}

/// Validate a parsed `.oxplow/project.yaml` document exactly as
/// `load_project_config` does — the one path every config write takes,
/// whether a person edits the file or a command sets a key.
pub fn parse_project_config(
    doc: serde_yaml::Value,
    fallback_name: &str,
) -> Result<OxplowConfig, ConfigError> {
    let parsed: RawConfig = serde_yaml::from_value(doc)?;
    validate(parsed, fallback_name)
}

/// Write `key` into `.oxplow/project.yaml` as `value`, or remove it when
/// `None` — editing only that key's lines (tsk1028). Every other line of
/// the file is kept byte for byte: the person's formatting, comments, flow
/// style, and keys that happen to equal their default. A key's block is its
/// line at column 0 through the line before the next one, with the comment
/// lines directly above it; a new key is appended. The key's own value is
/// written block style, and comments inside it are not kept.
///
/// The caller validates the change first (`keys::with_key`); this checks
/// only that the edited text still reads as the same document with that one
/// key changed, and refuses rather than write anything else (a file whose
/// YAML ties blocks together — anchors, multiple documents — can't be
/// edited in place).
pub fn write_project_key(
    project_dir: impl AsRef<Path>,
    key: &str,
    value: Option<&serde_json::Value>,
) -> Result<(), ConfigError> {
    let path = config_path(project_dir.as_ref());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let original = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let block = value
        .map(|v| {
            let mut one = serde_yaml::Mapping::new();
            one.insert(serde_yaml::Value::String(key.to_string()), to_yaml(v));
            serde_yaml::to_string(&serde_yaml::Value::Mapping(one))
        })
        .transpose()?;
    let edited = edit_top_level_key(&original, key, block.as_deref());

    // The edit must change that key and nothing else.
    let as_map = |text: &str| -> Result<serde_yaml::Mapping, ConfigError> {
        match serde_yaml::from_str::<serde_yaml::Value>(text)? {
            serde_yaml::Value::Mapping(m) => Ok(m),
            serde_yaml::Value::Null => Ok(serde_yaml::Mapping::new()),
            _ => Err(ConfigError::Invalid(
                ".oxplow/project.yaml is not a mapping".into(),
            )),
        }
    };
    let mut expected = as_map(&original)?;
    let yaml_key = serde_yaml::Value::String(key.to_string());
    match value {
        Some(v) => {
            expected.insert(yaml_key, to_yaml(v));
        }
        None => {
            expected.remove(&yaml_key);
        }
    }
    if as_map(&edited)? != expected {
        return Err(ConfigError::Invalid(format!(
            "`{key}` can't be edited in place in .oxplow/project.yaml (its YAML ties keys \
             together, e.g. with anchors); edit the file by hand"
        )));
    }
    std::fs::write(path, edited)?;
    Ok(())
}

/// The top-level key a `.oxplow/project.yaml` line starts, if it does: a
/// `name:` at column 0 (not a comment, list item or flow value).
fn top_key(line: &str) -> Option<&str> {
    let first = line.chars().next()?;
    if first.is_whitespace() || matches!(first, '#' | '-' | '{' | '[' | '\'' | '"') {
        return None;
    }
    let (name, _) = line.split_once(':')?;
    Some(name.trim_end())
}

/// `text` with top-level `key`'s block replaced by `block` (or removed when
/// `None`; appended when the key is new). See [`write_project_key`].
fn edit_top_level_key(text: &str, key: &str, block: Option<&str>) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some(at) = lines.iter().position(|l| top_key(l) == Some(key)) else {
        let mut out = text.to_string();
        if let Some(block) = block {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(block);
        }
        return out;
    };
    // The block runs to the next top-level key, less the comments that
    // belong to that key (directly above it).
    let mut end = lines[at + 1..]
        .iter()
        .position(|l| top_key(l).is_some())
        .map_or(lines.len(), |i| at + 1 + i);
    while end > at + 1 && lines[end - 1].trim_start().starts_with('#') && end < lines.len() {
        end -= 1;
    }
    // Its own comments: the comment lines directly above it.
    let mut start = at;
    if block.is_none() {
        while start > 0 && lines[start - 1].trim_start().starts_with('#') {
            start -= 1;
        }
    }
    let mut out: String = lines[..start].concat();
    if let Some(block) = block {
        out.push_str(block);
    }
    out.push_str(&lines[end..].concat());
    out
}

/// The `.oxplow/project.yaml` document for `config`: only keys whose
/// value differs from the default, so a hand-edited file stays minimal.
/// `fallback_name` is the project name that needs no `projectName` key.
/// Pure: what `keys::with_key` validates a change through; the file itself
/// is written one key at a time (`write_project_key`).
pub fn render_project_config(config: &OxplowConfig, fallback_name: &str) -> serde_yaml::Mapping {
    config_entries(config, fallback_name)
        .into_iter()
        .filter(|e| e.set)
        .map(|e| (serde_yaml::Value::String(e.key.into()), e.value))
        .collect()
}

/// One project key's value in `config`, and whether the file sets it
/// (else it is the default — and `value` is what that default is).
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigEntry {
    pub key: &'static str,
    pub value: serde_yaml::Value,
    pub set: bool,
}

/// `v` as a YAML value, by way of JSON text. The one bridge from a config
/// type to YAML: serde_json's `arbitrary_precision` makes a number inside
/// a `serde_json::Value` a private struct when anything but serde_json
/// serializes it (`serde_yaml::to_value` writes
/// `{$serde_json::private::Number: '5'}` into the file), and some config
/// types hold a `Value` (an extension instance's `config`).
pub(crate) fn to_yaml<T: serde::Serialize>(v: &T) -> serde_yaml::Value {
    let text = serde_json::to_string(v).expect("config values serialize as json");
    serde_yaml::from_str(&text).expect("json text is yaml")
}

/// Every key the project file can hold, with the value `config` gives it —
/// the file's, or the default — and whether that differs from the default
/// (P6.H1: the effective-config view shows both). `render_project_config`
/// is the set ones.
pub fn config_entries(config: &OxplowConfig, fallback_name: &str) -> Vec<ConfigEntry> {
    let mut out = Vec::new();
    let mut put = |key: &'static str, value: serde_yaml::Value, set: bool| {
        out.push(ConfigEntry { key, value, set });
    };
    put("agents", to_yaml(&config.agents), !config.agents.is_empty());
    put(
        "projectName",
        if config.project_name.is_empty() {
            fallback_name.into()
        } else {
            config.project_name.clone().into()
        },
        !config.project_name.is_empty() && config.project_name != fallback_name,
    );
    put(
        "agentPromptAppend",
        config.agent_prompt_append.clone().into(),
        !config.agent_prompt_append.is_empty(),
    );
    put(
        "snapshotRetentionDays",
        config.snapshot_retention_days.into(),
        config.snapshot_retention_days != DEFAULT_SNAPSHOT_RETENTION_DAYS,
    );
    put(
        "metricRetentionDays",
        config.metric_retention_days.into(),
        config.metric_retention_days != DEFAULT_METRIC_RETENTION_DAYS,
    );
    put(
        "metricDetailMaxPerProducer",
        config.metric_detail_max_per_producer.into(),
        config.metric_detail_max_per_producer != DEFAULT_METRIC_DETAIL_MAX_PER_PRODUCER,
    );
    put(
        "metricDetailRetentionDays",
        config.metric_detail_retention_days.into(),
        config.metric_detail_retention_days != DEFAULT_METRIC_DETAIL_RETENTION_DAYS,
    );
    put(
        "generated",
        to_yaml(&config.generated),
        !config.generated.exclude.is_empty() || !config.generated.include.is_empty(),
    );
    put(
        "snapshotMaxFileBytes",
        config.snapshot_max_file_bytes.into(),
        config.snapshot_max_file_bytes != DEFAULT_SNAPSHOT_MAX_FILE_BYTES,
    );
    put(
        "snapshotTurnBudgetMs",
        config.snapshot_turn_budget_ms.into(),
        config.snapshot_turn_budget_ms != DEFAULT_SNAPSHOT_TURN_BUDGET_MS,
    );
    put(
        "injectSessionContext",
        config.inject_session_context.into(),
        config.inject_session_context != DEFAULT_INJECT_SESSION_CONTEXT,
    );
    put(
        "iconTint",
        config
            .icon_tint
            .as_ref()
            .map_or(serde_yaml::Value::Null, |t| t.as_str().into()),
        config.icon_tint.is_some(),
    );
    {
        let mut lsp = serde_yaml::Mapping::new();
        let servers: Vec<_> = config
            .lsp_servers
            .iter()
            .map(|s| {
                let mut m = serde_yaml::Mapping::new();
                m.insert("languageId".into(), s.language_id.clone().into());
                m.insert("extensions".into(), to_yaml(&s.extensions));
                m.insert("command".into(), s.command.clone().into());
                if !s.args.is_empty() {
                    m.insert("args".into(), to_yaml(&s.args));
                }
                serde_yaml::Value::Mapping(m)
            })
            .collect();
        lsp.insert("servers".into(), serde_yaml::Value::Sequence(servers));
        put(
            "lsp",
            serde_yaml::Value::Mapping(lsp),
            !config.lsp_servers.is_empty(),
        );
    }
    {
        let c = &config.testing;
        let mut m = serde_yaml::Mapping::new();
        if let Some(v) = &c.command {
            m.insert("command".into(), v.clone().into());
        }
        if let Some(v) = &c.fast_command {
            m.insert("fastCommand".into(), v.clone().into());
        }
        if !c.run_patterns.is_empty() {
            m.insert("runPatterns".into(), to_yaml(&c.run_patterns));
        }
        if !c.analysis_patterns.is_empty() {
            m.insert("analysisPatterns".into(), to_yaml(&c.analysis_patterns));
        }
        if !c.test_paths.is_empty() {
            m.insert("testPaths".into(), to_yaml(&c.test_paths));
        }
        if let Some(v) = &c.agent_hint {
            m.insert("agentHint".into(), v.clone().into());
        }
        let set = !m.is_empty();
        put("testing", serde_yaml::Value::Mapping(m), set);
    }
    {
        let defaults = ScriptsConfig::default();
        let mut m = serde_yaml::Mapping::new();
        if config.scripts.max_concurrent != defaults.max_concurrent {
            m.insert("maxConcurrent".into(), config.scripts.max_concurrent.into());
        }
        if config.scripts.memory_bytes != defaults.memory_bytes {
            m.insert(
                "memoryMb".into(),
                (config.scripts.memory_bytes / (1024 * 1024)).into(),
            );
        }
        let set = !m.is_empty();
        put("scripts", serde_yaml::Value::Mapping(m), set);
    }
    put(
        "metrics",
        serde_yaml::Value::Sequence(config.metrics.iter().map(minimal_yaml).collect()),
        !config.metrics.is_empty(),
    );
    put("zones", to_yaml(&config.zones), !config.zones.is_empty());
    put(
        "agentConfig",
        to_yaml(&config.agent_config),
        !config.agent_config.is_empty(),
    );
    put(
        "extensionInstances",
        to_yaml(&instances_as_written(&config.extension_instances)),
        !config.extension_instances.is_empty(),
    );
    put(
        "activeProviders",
        to_yaml(&config.active_providers),
        !config.active_providers.is_empty(),
    );
    put(
        "replacements",
        to_yaml(&config.replacements),
        !config.replacements.is_empty(),
    );
    put(
        "eventRetention",
        to_yaml(&config.event_retention),
        !config.event_retention.is_empty(),
    );
    put(
        "extensions",
        to_yaml(&config.extensions),
        config.extensions != default_extensions(),
    );
    put(
        "extensionSources",
        to_yaml(&config.extension_sources),
        !config.extension_sources.is_empty(),
    );
    put(
        "enableBundledExtensions",
        to_yaml(&config.enable_bundled_extensions),
        config.enable_bundled_extensions.is_some(),
    );
    {
        let mut ai = serde_yaml::Mapping::new();
        // A role restricted from nothing is written without `restrict`.
        let mut roles = to_yaml(&config.ai_roles);
        if let serde_yaml::Value::Mapping(roles) = &mut roles {
            for (_, role) in roles.iter_mut() {
                if let serde_yaml::Value::Mapping(role) = role {
                    if role
                        .get("restrict")
                        .and_then(serde_yaml::Value::as_sequence)
                        .is_some_and(Vec::is_empty)
                    {
                        role.remove("restrict");
                    }
                }
            }
        }
        ai.insert("roles".into(), roles);
        put(
            "ai",
            serde_yaml::Value::Mapping(ai),
            !config.ai_roles.is_empty(),
        );
    }
    out
}

/// Write a **global** metrics manifest (`global_config_dir()/metrics/<name>.yaml`)
/// — a clean `metrics:` doc holding `entries` (tsk235). Creates parent dirs.
/// Used by the Catalog "New metric" scaffold at global scope; the runner reads
/// these via [`load_global_metric_entries`].
pub fn write_global_metrics_file(path: &Path, entries: &[MetricEntry]) -> Result<(), ConfigError> {
    let seq: Vec<serde_yaml::Value> = entries.iter().map(minimal_yaml).collect();
    let mut doc = serde_yaml::Mapping::new();
    doc.insert("metrics".into(), serde_yaml::Value::Sequence(seq));
    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, yaml)?;
    Ok(())
}

/// One config entry as a minimal YAML mapping: its serde form (the very
/// keys [`load_project_config`] reads) without unset values (null, empty
/// lists and maps), so a hand-edited block stays minimal across UI-driven
/// writes. Generic on purpose: the per-field writers it replaced forgot
/// fields as the structs grew (tsk355 dropped entity metrics' `entity`,
/// `where`, … and broke loading).
fn minimal_yaml<T: Serialize>(entry: &T) -> serde_yaml::Value {
    fn prune(v: serde_yaml::Value) -> Option<serde_yaml::Value> {
        use serde_yaml::Value;
        match v {
            Value::Null => None,
            Value::Sequence(s) if s.is_empty() => None,
            Value::Mapping(m) => {
                let kept: serde_yaml::Mapping = m
                    .into_iter()
                    .filter_map(|(k, v)| prune(v).map(|v| (k, v)))
                    .collect();
                (!kept.is_empty()).then_some(Value::Mapping(kept))
            }
            other => Some(other),
        }
    }
    prune(to_yaml(entry)).unwrap_or_else(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
}

fn default_config(project_name: String) -> OxplowConfig {
    OxplowConfig {
        agents: Vec::new(),
        project_name,
        lsp_servers: Vec::new(),
        agent_prompt_append: String::new(),
        snapshot_retention_days: DEFAULT_SNAPSHOT_RETENTION_DAYS,
        metric_retention_days: DEFAULT_METRIC_RETENTION_DAYS,
        metric_detail_max_per_producer: DEFAULT_METRIC_DETAIL_MAX_PER_PRODUCER,
        metric_detail_retention_days: DEFAULT_METRIC_DETAIL_RETENTION_DAYS,
        generated: GeneratedConfig::default(),
        snapshot_max_file_bytes: DEFAULT_SNAPSHOT_MAX_FILE_BYTES,
        snapshot_turn_budget_ms: DEFAULT_SNAPSHOT_TURN_BUDGET_MS,
        inject_session_context: DEFAULT_INJECT_SESSION_CONTEXT,
        icon_tint: None,
        testing: TestingConfig::default(),
        scripts: ScriptsConfig::default(),
        metrics: Vec::new(),
        zones: Vec::new(),
        agent_config: Default::default(),
        extension_instances: std::collections::BTreeMap::new(),
        active_providers: std::collections::BTreeMap::new(),
        personal_active_providers: std::collections::BTreeMap::new(),
        personal_new_thread_session: None,
        replacements: std::collections::BTreeMap::new(),
        event_retention: std::collections::BTreeMap::new(),
        ai_roles: Default::default(),
        extensions: default_extensions(),
        extension_sources: Default::default(),
        personal_extensions: Default::default(),
        personal_extension_sources: Default::default(),
        enable_bundled_extensions: None,
        personal_enable_bundled_extensions: None,
    }
}

/// One instance of an extension's provider (`extensionInstances`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtensionInstanceConfig {
    /// Run it (once this machine approved the provider).
    #[serde(default)]
    pub enabled: bool,
    /// The instance's config, as the provider's `config_schema` describes.
    #[serde(default = "empty_object")]
    #[specta(type = oxplow_domain::Json)]
    pub config: serde_json::Value,
    /// How often its collectors are read, in minutes (absent:
    /// [`DEFAULT_SYNC_MINUTES`]; `0`: only when someone runs
    /// `oxplow.provider.sync`).
    // No `skip_serializing_if` (specta's unified mode can't express it):
    // `render_project_config` leaves an absent one out of the file.
    #[serde(rename = "syncMinutes", default)]
    pub sync_minutes: Option<u32>,
    /// Which of the extension's providers this is an instance of, for an
    /// instance whose id isn't a provider's own (`tracker/issues_acme:
    /// { provider: issues }` — a second account on the same tracker). Absent, the
    /// instance id is the provider id.
    #[serde(default)]
    pub provider: Option<String>,
}

impl Default for ExtensionInstanceConfig {
    /// Off and unconfigured: what a newly added instance is.
    fn default() -> Self {
        Self {
            enabled: false,
            config: empty_object(),
            sync_minutes: None,
            provider: None,
        }
    }
}

/// The file of this machine's **global** provider instances, in the
/// global config dir ([`global_config_dir`]).
pub const INSTANCES_FILE: &str = "instances.yaml";

/// Beside [`INSTANCES_FILE`]: how many times each global instance's
/// credentials changed (tsk842). The values are in the keychain, which
/// no oxplow can watch; each bumps the instance's count when it changes
/// one, and every other restarts the instance when it sees the count
/// move. Counts only — never a value.
pub const CREDENTIAL_GENERATIONS_FILE: &str = "instance-credentials.yaml";

/// The [`CREDENTIAL_GENERATIONS_FILE`]: `generations: { "<ext>/<id>": n }`.
pub struct CredentialGenerations;

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationsDoc {
    #[serde(default)]
    generations: std::collections::BTreeMap<String, u64>,
}

impl CredentialGenerations {
    /// Each global instance's count; a missing or unreadable file is
    /// none (no change is ever lost by that: the next bump writes it).
    pub fn load(dir: &Path) -> std::collections::BTreeMap<String, u64> {
        std::fs::read_to_string(dir.join(CREDENTIAL_GENERATIONS_FILE))
            .ok()
            .and_then(|text| serde_yaml::from_str::<GenerationsDoc>(&text).ok())
            .map(|doc| doc.generations)
            .unwrap_or_default()
    }

    /// Count a change of `instance`'s credentials, under the file's lock
    /// (every oxplow on the machine takes it): its new count.
    pub fn bump(dir: &Path, instance: &str) -> Result<u64, ConfigError> {
        use fs2::FileExt;
        let failed = |e: std::io::Error| ConfigError::Instances(e.to_string());
        let path = dir.join(CREDENTIAL_GENERATIONS_FILE);
        std::fs::create_dir_all(dir).map_err(failed)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(atomic::lock_path(&path))
            .map_err(failed)?;
        lock.lock_exclusive().map_err(failed)?;
        let mut doc = GenerationsDoc {
            generations: Self::load(dir),
        };
        let n = doc.generations.entry(instance.to_string()).or_default();
        *n += 1;
        let now = *n;
        let text =
            serde_yaml::to_string(&doc).map_err(|e| ConfigError::Instances(e.to_string()))?;
        atomic::write_atomic(&path, text.as_bytes()).map_err(failed)?;
        Ok(now)
    }
}

/// Provider instances that belong to the person rather than a project
/// (P9.B2): `instances: { "<extension>/<instance id>": { enabled, config,
/// provider? } }`, the same entries as a project's `extensionInstances`.
/// Each applies in every project that has its extension enabled; a
/// project's entry of the same name replaces it there.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalInstances {
    #[serde(default)]
    pub instances: std::collections::BTreeMap<String, ExtensionInstanceConfig>,
}

impl GlobalInstances {
    /// Load from `dir/instances.yaml`; a missing file is none.
    pub fn load(dir: &Path) -> Result<GlobalInstances, ConfigError> {
        let text = match std::fs::read_to_string(dir.join(INSTANCES_FILE)) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(GlobalInstances::default())
            }
            Err(e) => return Err(ConfigError::Instances(e.to_string())),
        };
        let loaded: GlobalInstances =
            serde_yaml::from_str(&text).map_err(|e| ConfigError::Instances(e.to_string()))?;
        loaded.validated()
    }

    /// Change `dir/instances.yaml` in one read-modify-write: under a lock
    /// every oxplow on the machine takes (`instances.yaml.lock`), `edit`
    /// the file as it is now, then validate and write it atomically — so
    /// two windows or two oxplows never drop each other's change, and a
    /// reader never sees half a file (tsk837). The outer error is the
    /// file's (it couldn't be locked, read, validated or written); the
    /// inner is `edit`'s refusal, which writes nothing.
    pub fn update<E>(
        dir: &Path,
        edit: impl FnOnce(
            &mut std::collections::BTreeMap<String, ExtensionInstanceConfig>,
        ) -> Result<(), E>,
    ) -> Result<Result<(), E>, ConfigError> {
        use fs2::FileExt;
        let failed = |e: std::io::Error| ConfigError::Instances(e.to_string());
        let path = dir.join(INSTANCES_FILE);
        std::fs::create_dir_all(dir).map_err(failed)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(atomic::lock_path(&path))
            .map_err(failed)?;
        lock.lock_exclusive().map_err(failed)?;
        let mut current = GlobalInstances::load(dir)?;
        if let Err(refused) = edit(&mut current.instances) {
            return Ok(Err(refused));
        }
        let checked = current.validated()?;
        let doc = serde_json::json!({ "instances": instances_as_written(&checked.instances) });
        let text =
            serde_yaml::to_string(&doc).map_err(|e| ConfigError::Instances(e.to_string()))?;
        atomic::write_atomic(&path, text.as_bytes()).map_err(failed)?;
        Ok(Ok(()))
    }

    /// The rules a project's `extensionInstances` follow, said of this file.
    fn validated(self) -> Result<GlobalInstances, ConfigError> {
        validate_extension_instances(self.instances)
            .map(|instances| GlobalInstances { instances })
            .map_err(|e| match e {
                ConfigError::Invalid(message) => ConfigError::Instances(
                    message.replacen("extensionInstances: ", "", 1).replacen(
                        "extensionInstances.",
                        "",
                        1,
                    ),
                ),
                other => other,
            })
    }
}

/// How often an instance's collectors are read when its config doesn't
/// say (`syncMinutes`).
pub const DEFAULT_SYNC_MINUTES: u32 = 5;

impl ExtensionInstanceConfig {
    /// How often its collectors are read; `None` for never on a schedule.
    pub fn sync_every(&self) -> Option<std::time::Duration> {
        match self.sync_minutes.unwrap_or(DEFAULT_SYNC_MINUTES) {
            0 => None,
            m => Some(std::time::Duration::from_secs(u64::from(m) * 60)),
        }
    }
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

/// `extensionInstances` as the file holds it: an instance's absent
/// `syncMinutes` is left out, not written `null`.
fn instances_as_written(
    instances: &std::collections::BTreeMap<String, ExtensionInstanceConfig>,
) -> serde_json::Value {
    let mut value = serde_json::to_value(instances).expect("instances serialize");
    if let Some(all) = value.as_object_mut() {
        for instance in all
            .values_mut()
            .filter_map(serde_json::Value::as_object_mut)
        {
            instance.retain(|_, v| !v.is_null());
        }
    }
    value
}

/// A provider or instance id: lowercase snake_case, starting with a
/// letter — a ref's provider segment and a command namespace as it stands.
fn is_instance_id(id: &str) -> bool {
    id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Validate `extensionInstances:`: keyed `<extension>/<instance id>` (the
/// id lowercase snake_case — a provider's own for its default instance),
/// each config an object, `provider` (when given) a provider id.
fn validate_extension_instances(
    raw: std::collections::BTreeMap<String, ExtensionInstanceConfig>,
) -> Result<std::collections::BTreeMap<String, ExtensionInstanceConfig>, ConfigError> {
    for (key, instance) in &raw {
        let Some((_, id)) = key.split_once('/').filter(|(ext, _)| !ext.is_empty()) else {
            return Err(ConfigError::Invalid(format!(
                "extensionInstances: `{key}` must be `<extension>/<instance id>` (the id lowercase \
                 snake_case: a provider's own, or another instance's with `provider: <id>`)"
            )));
        };
        if let Some(why) = oxplow_domain::capability::instance_id_problem(id) {
            return Err(ConfigError::Invalid(format!(
                "extensionInstances: `{key}` must be `<extension>/<instance id>`, and the \
                 instance id {why} (it is its refs' segment and its command namespace)"
            )));
        }
        if let Some(provider) = instance.provider.as_deref().filter(|p| !is_instance_id(p)) {
            return Err(ConfigError::Invalid(format!(
                "extensionInstances.{key}: `provider` names one of the extension's providers by \
                 id (lowercase snake_case), not `{provider}`"
            )));
        }
        if !instance.config.is_object() {
            return Err(ConfigError::Invalid(format!(
                "extensionInstances.{key}.config must be an object"
            )));
        }
    }
    Ok(raw)
}

/// Validate `replacements:`: each target replaceable; a bound one only
/// set to `oxplow` (its provider's extension decides otherwise).
fn validate_replacements(
    raw: std::collections::BTreeMap<String, String>,
) -> Result<std::collections::BTreeMap<String, String>, ConfigError> {
    for (target, whose) in &raw {
        match oxplow_domain::replaceable::replaceable(target) {
            None => {
                return Err(ConfigError::Invalid(format!(
                    "replacements: `{target}` isn't a replaceable component ({})",
                    oxplow_domain::replaceable::targets()
                )))
            }
            Some(t) if t.bound_to_provider && whose != oxplow_domain::replaceable::OXPLOWS_OWN => {
                return Err(ConfigError::Invalid(format!(
                    "replacements: `{target}` shows its provider's extension's or `oxplow`'s; \
                     no other extension can be chosen"
                )))
            }
            Some(_) => {}
        }
    }
    Ok(raw)
}

/// Validate `eventRetention:`: a namespace each — never core state, which
/// is kept whole — at windows of at least a day.
fn validate_event_retention(
    raw: std::collections::BTreeMap<String, oxplow_domain::events::retention::RetentionWindow>,
) -> Result<
    std::collections::BTreeMap<String, oxplow_domain::events::retention::RetentionWindow>,
    ConfigError,
> {
    for (namespace, window) in &raw {
        if namespace.is_empty()
            || !namespace
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(ConfigError::Invalid(format!(
                "eventRetention: `{namespace}` isn't an event namespace (lowercase letters, digits and `_`)"
            )));
        }
        if let Some(problem) = oxplow_domain::events::retention::window_problem(namespace, *window)
        {
            return Err(ConfigError::Invalid(format!(
                "eventRetention.{namespace}: {problem}"
            )));
        }
    }
    Ok(raw)
}

/// Validate `activeProviders:`: a choosable capability each
/// ([`oxplow_domain::capability`]), naming an implementation by its id (a
/// provider's default instance has the provider's).
fn validate_active_providers(
    raw: std::collections::BTreeMap<String, String>,
) -> Result<std::collections::BTreeMap<String, String>, ConfigError> {
    use oxplow_domain::capability;
    for (capability, provider) in &raw {
        if !capability::spec(capability).is_some_and(|c| c.choosable) {
            return Err(ConfigError::Invalid(format!(
                "activeProviders: `{capability}` isn't a capability whose implementation can be \
                 chosen ({})",
                capability::choosable()
                    .map(|c| c.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        if !is_instance_id(provider) {
            return Err(ConfigError::Invalid(format!(
                "activeProviders.{capability}: `{provider}` isn't an instance id (lowercase \
                 snake_case; a provider's default instance has the provider's id)"
            )));
        }
    }
    Ok(raw)
}

fn validate(raw: RawConfig, fallback_name: &str) -> Result<OxplowConfig, ConfigError> {
    let agents = validate_agents(raw.agents)?;

    let project_name = match raw.project_name {
        Some(name) => {
            let trimmed = name.trim().to_string();
            if trimmed.is_empty() {
                return Err(ConfigError::Invalid(
                    "projectName must be a non-empty string".into(),
                ));
            }
            trimmed
        }
        None => fallback_name.to_string(),
    };

    let agent_prompt_append = raw.agent_prompt_append.unwrap_or_default();

    let snapshot_retention_days = match raw.snapshot_retention_days {
        Some(n) if !n.is_finite() || n < 0.0 => {
            return Err(ConfigError::Invalid(
                "snapshotRetentionDays must be a non-negative number".into(),
            ));
        }
        Some(n) => n as u32,
        None => DEFAULT_SNAPSHOT_RETENTION_DAYS,
    };
    let metric_retention_days = match raw.metric_retention_days {
        Some(n) if !n.is_finite() || n < 0.0 => {
            return Err(ConfigError::Invalid(
                "metricRetentionDays must be a non-negative number".into(),
            ));
        }
        Some(n) => n as u32,
        None => DEFAULT_METRIC_RETENTION_DAYS,
    };
    let metric_detail_max_per_producer = match raw.metric_detail_max_per_producer {
        Some(n) if !n.is_finite() || n < 0.0 => {
            return Err(ConfigError::Invalid(
                "metricDetailMaxPerProducer must be a non-negative number".into(),
            ));
        }
        Some(n) => n as u32,
        None => DEFAULT_METRIC_DETAIL_MAX_PER_PRODUCER,
    };
    let metric_detail_retention_days = match raw.metric_detail_retention_days {
        Some(n) if !n.is_finite() || n < 0.0 => {
            return Err(ConfigError::Invalid(
                "metricDetailRetentionDays must be a non-negative number".into(),
            ));
        }
        Some(n) => n as u32,
        None => DEFAULT_METRIC_DETAIL_RETENTION_DAYS,
    };

    let generated = match raw.generated {
        Some(g) => GeneratedConfig {
            exclude: validate_generated_list(g.exclude, "generated.exclude")?,
            include: validate_generated_list(g.include, "generated.include")?,
        },
        None => GeneratedConfig::default(),
    };

    let zones = validate_zones(raw.zones.unwrap_or_default())?;

    let snapshot_max_file_bytes = match raw.snapshot_max_file_bytes {
        Some(n) if !n.is_finite() || n < 1024.0 => {
            return Err(ConfigError::Invalid(
                "snapshotMaxFileBytes must be a number >= 1024".into(),
            ));
        }
        Some(n) => n.floor() as u64,
        None => DEFAULT_SNAPSHOT_MAX_FILE_BYTES,
    };

    let snapshot_turn_budget_ms = match raw.snapshot_turn_budget_ms {
        Some(n) if !n.is_finite() || n < MIN_SNAPSHOT_TURN_BUDGET_MS as f64 => {
            return Err(ConfigError::Invalid(format!(
                "snapshotTurnBudgetMs must be a number >= {MIN_SNAPSHOT_TURN_BUDGET_MS}"
            )));
        }
        Some(n) => n.floor() as u64,
        None => DEFAULT_SNAPSHOT_TURN_BUDGET_MS,
    };

    let inject_session_context = raw
        .inject_session_context
        .unwrap_or(DEFAULT_INJECT_SESSION_CONTEXT);

    let icon_tint = match raw.icon_tint {
        Some(t) if parse_hex_rgb(&t).is_none() => {
            return Err(ConfigError::Invalid(format!(
                "iconTint must be a hex colour like \"#c2410c\" or \"#abc\"; got {t:?}"
            )));
        }
        other => other,
    };

    let testing = validate_testing(raw.testing)?;
    let scripts = validate_scripts(raw.scripts)?;
    let metrics = validate_metrics(raw.metrics, CatalogLayer::Project)?;
    for (key, moved) in [
        ("collectors", raw.collectors.is_some()),
        ("measures", raw.measures.is_some()),
        ("dimensions", raw.dimensions.is_some()),
    ] {
        if moved {
            return Err(ConfigError::Invalid(format!(
                "`{key}:` is an extension's: declare the project's in its own extension \
                 (`.oxplow/extensions/project/extension.yaml`)"
            )));
        }
    }

    if raw.agent_models.is_some() {
        return Err(ConfigError::Invalid(
            "agentModels is now agentConfig: write `agentConfig: { <agent>: { model: <model> } }`"
                .into(),
        ));
    }
    let agent_config = validate_agent_config(raw.agent_config.unwrap_or_default())?;

    let extension_instances =
        validate_extension_instances(raw.extension_instances.unwrap_or_default())?;
    let active_providers = validate_active_providers(raw.active_providers.unwrap_or_default())?;
    let replacements = validate_replacements(raw.replacements.unwrap_or_default())?;
    let event_retention = validate_event_retention(raw.event_retention.unwrap_or_default())?;

    let lsp_servers = match raw.lsp.and_then(|l| l.servers) {
        Some(servers) => {
            let mut out = Vec::with_capacity(servers.len());
            for (i, s) in servers.into_iter().enumerate() {
                if s.language_id.trim().is_empty() {
                    return Err(ConfigError::Invalid(format!(
                        "lsp.servers[{i}].languageId must be a non-empty string"
                    )));
                }
                if s.command.trim().is_empty() {
                    return Err(ConfigError::Invalid(format!(
                        "lsp.servers[{i}].command must be a non-empty string"
                    )));
                }
                if s.extensions.is_empty() {
                    return Err(ConfigError::Invalid(format!(
                        "lsp.servers[{i}].extensions must be a non-empty array"
                    )));
                }
                let mut exts = Vec::with_capacity(s.extensions.len());
                for (j, ext) in s.extensions.into_iter().enumerate() {
                    if !ext.starts_with('.') {
                        return Err(ConfigError::Invalid(format!(
                            "lsp.servers[{i}].extensions[{j}] must start with '.'"
                        )));
                    }
                    exts.push(ext.to_lowercase());
                }
                out.push(LspServerConfig {
                    language_id: s.language_id,
                    extensions: exts,
                    command: s.command,
                    args: s.args,
                });
            }
            out
        }
        None => Vec::new(),
    };

    Ok(OxplowConfig {
        agents,
        project_name,
        lsp_servers,
        agent_prompt_append,
        snapshot_retention_days,
        metric_retention_days,
        metric_detail_max_per_producer,
        metric_detail_retention_days,
        generated,
        snapshot_max_file_bytes,
        snapshot_turn_budget_ms,
        inject_session_context,
        icon_tint,
        testing,
        scripts,
        metrics,
        zones,
        agent_config,
        extension_instances,
        active_providers,
        personal_active_providers: std::collections::BTreeMap::new(),
        personal_new_thread_session: None,
        replacements,
        event_retention,
        ai_roles: validate_ai_roles(raw.ai)?,
        extensions: raw.extensions.unwrap_or_else(default_extensions),
        extension_sources: raw.extension_sources.unwrap_or_default(),
        personal_extensions: Default::default(),
        personal_extension_sources: Default::default(),
        enable_bundled_extensions: raw.enable_bundled_extensions,
        personal_enable_bundled_extensions: None,
    })
}

/// Validate `ai.roles`: known role names, non-empty provider and model.
/// `<extension>/<profile>`: the shape only. Whether an active extension
/// declares it is known at run time, where an unknown one goes unused.
fn is_ai_profile_name(role: &str) -> bool {
    role.split_once('/').is_some_and(|(extension, profile)| {
        !extension.is_empty()
            && extension
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            && profile.starts_with(|c: char| c.is_ascii_lowercase())
            && profile
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    })
}

fn validate_ai_roles(
    raw: Option<RawAiBlock>,
) -> Result<std::collections::BTreeMap<String, AiRoleOverride>, ConfigError> {
    let roles = raw.map(|b| b.roles).unwrap_or_default();
    for (role, o) in &roles {
        if DROPPED_AI_ROLES.contains(&role.as_str()) {
            return Err(ConfigError::Invalid(format!(
                "ai.roles.{role}: oxplow has no `{role}` role: nothing asked it; remove it \
                 (use one of {})",
                AI_ROLE_NAMES.join(", ")
            )));
        }
        if !AI_ROLE_NAMES.contains(&role.as_str()) && !is_ai_profile_name(role) {
            return Err(ConfigError::Invalid(format!(
                "ai.roles.{role}: unknown role (use one of {}, or an extension's AI profile as \
                 `<extension>/<profile>`)",
                AI_ROLE_NAMES.join(", ")
            )));
        }
        if o.provider.trim().is_empty() || o.model.trim().is_empty() {
            return Err(ConfigError::Invalid(format!(
                "ai.roles.{role} needs a provider and a model"
            )));
        }
    }
    Ok(roles)
}

/// Validate the `zones:` table. Each row needs at least one non-empty,
/// compilable glob and a label that isn't one of the computed sentinels
/// ([`ZONE_OTHER`] / [`ZONE_EXTERNAL`]) — declaring those would make a
/// real zone indistinguishable from "couldn't classify this".
///
/// Globs are compiled here (and thrown away) purely to fail at LOAD
/// time: a bad pattern in the file should be a config error the user
/// sees, not a rule that silently never matches.
fn validate_zones(raw: Vec<RawZoneRule>) -> Result<Vec<ZoneRuleConfig>, ConfigError> {
    let rules: Vec<ZoneRuleConfig> = raw
        .into_iter()
        .map(|r| ZoneRuleConfig {
            patterns: r.patterns.into_vec(),
            zone: r.zone,
            color: r.color,
        })
        .collect();
    validate_zone_rules(&rules)
}

/// Validate a zone table that's already in [`ZoneRuleConfig`] shape —
/// the path an agent's `set_zones` call takes, so a rule written over
/// MCP is held to exactly the same rules as one typed into the file.
/// Returns the trimmed table.
pub fn validate_zone_rules(rules: &[ZoneRuleConfig]) -> Result<Vec<ZoneRuleConfig>, ConfigError> {
    let mut out = Vec::with_capacity(rules.len());
    for (i, rule) in rules.iter().enumerate() {
        let zone = rule.zone.trim().to_string();
        if zone.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "zones[{i}].zone must be a non-empty label"
            )));
        }
        if zone == ZONE_OTHER || zone == ZONE_EXTERNAL {
            return Err(ConfigError::Invalid(format!(
                "zones[{i}].zone \"{zone}\" is reserved (oxplow computes it for \
                 unmatched files and out-of-repo import targets)"
            )));
        }
        let patterns = &rule.patterns;
        if patterns.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "zones[{i}].match must name at least one glob"
            )));
        }
        let mut cleaned = Vec::with_capacity(patterns.len());
        for (j, pattern) in patterns.iter().enumerate() {
            let trimmed = pattern.trim().to_string();
            if trimmed.is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "zones[{i}].match[{j}] must be a non-empty glob"
                )));
            }
            globset::GlobBuilder::new(&trimmed)
                .literal_separator(true)
                .build()
                .map_err(|e| {
                    ConfigError::Invalid(format!(
                        "zones[{i}].match[{j}] is not a valid glob (\"{trimmed}\"): {e}"
                    ))
                })?;
            cleaned.push(trimmed);
        }
        let color = match rule.color.as_deref() {
            Some(c) if parse_hex_rgb(c.trim()).is_none() => {
                return Err(ConfigError::Invalid(format!(
                    "zones[{i}].color must be a hex colour like #4f46e5 (got \"{c}\")"
                )));
            }
            Some(c) => Some(c.trim().to_string()),
            None => None,
        };
        out.push(ZoneRuleConfig {
            patterns: cleaned,
            zone,
            color,
        });
    }
    Ok(out)
}

/// Validate one `generated.exclude` / `generated.include` list: each
/// entry must be a non-empty, repo-relative path (no leading `/`, no
/// `..`). Returns the trimmed entries.
fn validate_generated_list(list: Vec<String>, label: &str) -> Result<Vec<String>, ConfigError> {
    let mut out = Vec::with_capacity(list.len());
    for (i, entry) in list.into_iter().enumerate() {
        let trimmed = entry.trim().trim_matches('/').to_string();
        if trimmed.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "{label}[{i}] must be a non-empty string"
            )));
        }
        if entry.trim().starts_with('/') {
            return Err(ConfigError::Invalid(format!(
                "{label}[{i}] must be a repo-relative path, not absolute (got \"{entry}\")"
            )));
        }
        if trimmed.split('/').any(|seg| seg == "..") {
            return Err(ConfigError::Invalid(format!(
                "{label}[{i}] must not contain `..` (got \"{entry}\")"
            )));
        }
        out.push(trimmed);
    }
    Ok(out)
}

/// Read-time presentation kinds a `metrics:` spec may declare (`displayKind`).
const METRIC_DISPLAY_KINDS: &[&str] = &["gauge", "findings", "test", "coverage", "event"];
/// Metric directions.
const METRIC_DIRECTIONS: &[&str] = &["higher-better", "lower-better", "neutral"];
/// Metric aggregations (mirror the engine's `Aggregation`): combine facts within
/// a capture.
const METRIC_AGGS: &[&str] = &["last", "sum", "avg", "min", "max", "count", "ratio"];
/// Aggregations an entity metric may declare (computed in SQL over its rows).
pub const ENTITY_METRIC_AGGS: &[&str] = &[
    "count",
    "count_distinct",
    "sum",
    "avg",
    "min",
    "max",
    "median",
    "p90",
];

/// An entity name an entity metric / dimension may name: a `v_*` view.
fn is_entity_name(s: &str) -> bool {
    s.len() > 2
        && s.starts_with("v_")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}
/// Formula binary ops a `metrics:` spec may declare.
const METRIC_FORMULA_OPS: &[&str] = &["add", "sub", "mul", "div"];
/// Catalog groupings a `metrics:` spec may declare.
const METRIC_CATEGORIES: &[&str] = &[
    "operational",
    "testing",
    "coverage",
    "static-quality",
    "custom",
];
/// Additivity-over-time a `measures:` entry may declare (mirrors the `measure`
/// table's CHECK).
const MEASURE_TEMPORAL_SEMANTICS: &[&str] = &["additive", "semi-additive", "non-additive"];
/// What ONE capture restates (tsk41). Deliberately NOT mirrored as a DB CHECK —
/// `temporal_semantics`' CHECK is exactly why adding a value there needs a
/// `measure` table rebuild (which would cascade-wipe every fact), so
/// `capture_scope` is validated here and in `CaptureScope::parse` instead.
const MEASURE_CAPTURE_SCOPES: &[&str] = &["complete", "per-path", "per-subject"];
/// Ratio-base role a `measures:` entry may declare.
/// Value types a `dimensions:` entry may declare (mirrors the `dimension`
/// table's CHECK).
const DIMENSION_VALUE_TYPES: &[&str] = &["categorical", "numeric", "temporal", "entity-ref"];

/// The namespace oxplow's own catalog keys (`oxplow.*`) belong to: only the
/// extension declaring `namespace: oxplow` (oxplow-foundation) defines them.
pub const OXPLOW_NAMESPACE: &str = "oxplow";

/// Why `key` can't be defined by `namespace` — the defining extension's
/// namespace, `None` for the project's or the person's own config — or
/// `None` when it can: `oxplow.*` is the `oxplow` namespace's.
pub fn reserved_key(key: &str, namespace: Option<&str>) -> Option<String> {
    (key.starts_with("oxplow.") && namespace != Some(OXPLOW_NAMESPACE)).then(|| {
        format!("key \"{key}\" uses the reserved \"oxplow.\" namespace, which is oxplow's own")
    })
}

/// Where a catalog block (`metrics:`, `measures:`, `dimensions:`) is
/// written, which decides what it may say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogLayer<'a> {
    /// `.oxplow/project.yaml`: the project's choices — which metrics are on
    /// (`use:`, an `enabled: false` marker) and their thresholds. Its
    /// definitions are its own extension's.
    Project,
    /// The person's own files in the global config dir.
    Global,
    /// An extension's manifest, in its namespace (see [`reserved_key`]).
    Extension(&'a str),
}

impl CatalogLayer<'_> {
    fn namespace(&self) -> Option<&str> {
        match self {
            CatalogLayer::Extension(ns) => Some(ns),
            _ => None,
        }
    }
}

/// Validate a `metrics:` block written in `layer`: namespaced keys, `oxplow.*`
/// only in the `oxplow` namespace ([`reserved_key`]), definitions (`key:`)
/// only outside the project. Each entry must be exactly one of the `use:` or
/// `key:` forms. Returns the cleaned entries (the scope resolution happens
/// later in [`resolve_metrics`]).
pub fn validate_metrics(
    raw: Option<Vec<MetricEntry>>,
    layer: CatalogLayer<'_>,
) -> Result<Vec<MetricEntry>, ConfigError> {
    let namespace = layer.namespace();
    let opt = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    // Each metric key may appear at most once in the project block. Two entries
    // for the same key (a `key:` define plus a `use:`, or two defines) would
    // each resolve to a `ResolvedMetric`, silently double-seeding and
    // double-computing the metric. Reject the collision instead.
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (i, e) in raw.into_iter().flatten().enumerate() {
        let use_key = opt(e.use_key);
        let key = opt(e.key);
        let (is_define, the_key) = match (&use_key, &key) {
            (Some(_), Some(_)) => {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] sets both `use` and `key`; use exactly one"
                )))
            }
            (None, None) => {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] must set either `use` (enable a catalog metric) or `key` (define one)"
                )))
            }
            (Some(u), None) => (false, u.clone()),
            (None, Some(k)) => (true, k.clone()),
        };
        if !the_key.contains('.') {
            return Err(ConfigError::Invalid(format!(
                "metrics[{i}] key \"{the_key}\" must be namespaced as \"<vendor>.<id>\""
            )));
        }
        if is_define && layer == CatalogLayer::Project {
            return Err(ConfigError::Invalid(format!(
                "metrics[{i}] defines `{the_key}`: the project's metrics are defined in its own \
                 extension (`.oxplow/extensions/project/extension.yaml`); `.oxplow/project.yaml` \
                 turns metrics on (`use:`) and off"
            )));
        }
        // A project may `use:` an `oxplow.*` metric but only the `oxplow`
        // namespace defines one.
        if is_define {
            if let Some(e) = reserved_key(&the_key, namespace) {
                return Err(ConfigError::Invalid(format!("metrics[{i}] {e}")));
            }
        }
        if !seen_keys.insert(the_key.clone()) {
            return Err(ConfigError::Invalid(format!(
                "metrics[{i}] key \"{the_key}\" appears more than once in the \
                 metrics block; declare it once (a single `use:` or `key:`)"
            )));
        }

        let direction = opt(e.direction);
        if let Some(d) = &direction {
            if !METRIC_DIRECTIONS.contains(&d.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] direction must be one of {METRIC_DIRECTIONS:?} (got \"{d}\")"
                )));
            }
        }
        let aggregation = opt(e.aggregation);
        let entity = opt(e.entity);
        let where_ = opt(e.where_);
        let time = opt(e.time);
        let value = opt(e.value);
        if let Some(a) = &aggregation {
            if entity.is_some() {
                if !ENTITY_METRIC_AGGS.contains(&a.as_str()) {
                    return Err(ConfigError::Invalid(format!(
                        "metrics[{i}] entity aggregation must be one of {ENTITY_METRIC_AGGS:?} (got \"{a}\")"
                    )));
                }
            } else if !METRIC_AGGS.contains(&a.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] aggregation must be one of {METRIC_AGGS:?} (got \"{a}\")"
                )));
            }
        }
        let display_kind = opt(e.display_kind);
        if let Some(k) = &display_kind {
            if !METRIC_DISPLAY_KINDS.contains(&k.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] displayKind must be one of {METRIC_DISPLAY_KINDS:?} (got \"{k}\")"
                )));
            }
        }
        let category = opt(e.category);
        if let Some(c) = &category {
            if !METRIC_CATEGORIES.contains(&c.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] category must be one of {METRIC_CATEGORIES:?} (got \"{c}\")"
                )));
            }
        }
        let source_measure = opt(e.source_measure);
        let filter = e.filter.map(|f| validate_filter(i, f)).transpose()?;
        let formula = e.formula.map(|f| validate_formula(i, f)).transpose()?;

        // The structural spec fields (measure/aggregation/filter/formula) are
        // inherent to the DEFINITION; a `use:` may only re-target thresholds.
        if !is_define
            && (source_measure.is_some()
                || aggregation.is_some()
                || filter.is_some()
                || formula.is_some()
                || entity.is_some()
                || where_.is_some()
                || time.is_some()
                || value.is_some())
        {
            return Err(ConfigError::Invalid(format!(
                "metrics[{i}] is a `use:` entry; it may only override target/warnAt/failAt, \
                 not the measure/aggregation/filter/formula (those are inherent to the definition)"
            )));
        }
        // A `key:` metric is either a measure aggregation OR a formula, never both,
        // never neither.
        if entity.is_none() && (where_.is_some() || time.is_some() || value.is_some()) {
            return Err(ConfigError::Invalid(format!(
                "metrics[{i}] sets `where`/`time`/`value` without `entity`"
            )));
        }
        if let Some(view) = &entity {
            if !is_entity_name(view) {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] entity must name a `v_*` view (got \"{view}\")"
                )));
            }
            if source_measure.is_some() || formula.is_some() || filter.is_some() {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] is an entity metric; it can't also set \
                     `sourceMeasure`, `formula` or `filter` (use `where`)"
                )));
            }
            let agg = aggregation.as_deref().unwrap_or("count");
            if agg != "count" && value.is_none() {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] entity aggregation `{agg}` needs a `value` expression"
                )));
            }
        } else if is_define {
            match (source_measure.is_some(), formula.is_some()) {
                (true, true) => {
                    return Err(ConfigError::Invalid(format!(
                        "metrics[{i}] sets both `sourceMeasure` and `formula`; use exactly one"
                    )))
                }
                (false, false) => {
                    return Err(ConfigError::Invalid(format!(
                        "metrics[{i}] defines key \"{the_key}\" but sets neither `sourceMeasure` \
                         (a measure aggregation) nor `formula` (a derived metric)"
                    )))
                }
                _ => {}
            }
        }

        let collector = opt(e.collector);
        if (e.default.is_some() || collector.is_some()) && !(is_define && namespace.is_some()) {
            return Err(ConfigError::Invalid(format!(
                "metrics[{i}] `default` and `collector` belong on an extension's metric definition"
            )));
        }
        if is_define && namespace.is_some() && e.enabled.is_some() {
            return Err(ConfigError::Invalid(format!(
                "metrics[{i}] an extension's metric says `default: off`, not `enabled` (the \
                 project turns metrics on and off)"
            )));
        }
        let (use_key, key) = if is_define {
            (None, Some(the_key))
        } else {
            (Some(the_key), None)
        };
        out.push(MetricEntry {
            use_key,
            key,
            enabled: e.enabled,
            default: e.default,
            collector,
            title: opt(e.title),
            source_measure,
            aggregation,
            filter,
            formula,
            unit: opt(e.unit),
            direction,
            display_kind,
            category,
            language: opt(e.language),
            description: opt(e.description),
            sliceable_dims: e
                .sliceable_dims
                .into_iter()
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty())
                .map(|d| dimension_key(&format!("metrics[{i}].sliceableDims"), d))
                .collect::<Result<_, _>>()?,
            target: e.target,
            warn_at: e.warn_at,
            fail_at: e.fail_at,
            entity,
            where_,
            time,
            value,
        });
    }
    Ok(out)
}

/// A dimension key a metric slices or filters by: its one, namespaced name
/// (tsk945) — `subject`, the raw-subject pseudo-dimension, aside.
fn dimension_key(at: &str, key: String) -> Result<String, ConfigError> {
    if key == "subject" || key.contains('.') {
        return Ok(key);
    }
    Err(ConfigError::Invalid(format!(
        "{at}: dimension `{key}` isn't namespaced — a conformed dimension is `oxplow.{key}`, \
         one of your own `<namespace>.{key}`"
    )))
}

/// Validate a spec's `filter:` block. `dimEq`, if present, must be a two-element
/// `[key, value]` list; both must be non-empty.
fn validate_filter(i: usize, f: FilterConfig) -> Result<FilterConfig, ConfigError> {
    let dim_eq = match f.dim_eq {
        Some(pair) => {
            let cleaned: Vec<String> = pair
                .into_iter()
                .map(|s| s.trim().to_string())
                .collect::<Vec<_>>();
            if cleaned.len() != 2 || cleaned.iter().any(|s| s.is_empty()) {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}].filter.dimEq must be a [key, value] pair of non-empty strings"
                )));
            }
            dimension_key(&format!("metrics[{i}].filter.dimEq"), cleaned[0].clone())?;
            Some(cleaned)
        }
        None => None,
    };
    Ok(FilterConfig {
        min_value: f.min_value,
        max_value: f.max_value,
        severity: f
            .severity
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        dim_eq,
    })
}

/// Validate a spec's `formula:` block: a known op over two non-empty metric keys.
fn validate_formula(i: usize, f: FormulaConfig) -> Result<FormulaConfig, ConfigError> {
    let op = f.op.trim().to_ascii_lowercase();
    if !METRIC_FORMULA_OPS.contains(&op.as_str()) {
        return Err(ConfigError::Invalid(format!(
            "metrics[{i}].formula.op must be one of {METRIC_FORMULA_OPS:?} (got \"{}\")",
            f.op
        )));
    }
    let left = f.left.trim().to_string();
    let right = f.right.trim().to_string();
    if left.is_empty() || right.is_empty() {
        return Err(ConfigError::Invalid(format!(
            "metrics[{i}].formula must set both `left` and `right` metric keys"
        )));
    }
    Ok(FormulaConfig { op, left, right })
}

/// Every metric definition (`key:` entry) by key, with the scope it came
/// from; a later scope wins (extension > global > built-in).
fn definition_catalog<'a>(
    builtin: &'a [MetricEntry],
    global: &'a [MetricEntry],
    extensions: &'a [ExtensionLayer<MetricEntry>],
) -> std::collections::HashMap<String, (String, &'a MetricEntry)> {
    let mut catalog = std::collections::HashMap::new();
    for (scope, entries) in scoped_layers(builtin, global, extensions) {
        for e in entries {
            if let Some(k) = e.key.as_deref() {
                catalog.insert(k.to_string(), (scope.clone(), e));
            }
        }
    }
    catalog
}

/// The project's `use:` keys that name no definition: normally typos,
/// which [`resolve_metrics`] skips. A **disable marker** (`enabled: false`)
/// for an extension metric isn't one — its key isn't a config definition, and
/// `seed_catalog` prunes it from config state.
pub fn unknown_uses(
    builtin: &[MetricEntry],
    global: &[MetricEntry],
    extensions: &[ExtensionLayer<MetricEntry>],
    project: &[MetricEntry],
) -> Vec<String> {
    let catalog = definition_catalog(builtin, global, extensions);
    project
        .iter()
        .filter(|e| e.enabled != Some(false))
        .filter_map(|e| e.use_key.clone())
        .filter(|k| !catalog.contains_key(k))
        .collect()
}

/// Resolve declared metric SPECS across the three scopes into the flat
/// [`ResolvedSpec`] list the runner consumes. Definitions (`key:` entries) from
/// built-in, then global, then project build a catalog by key (later scope wins →
/// precedence project > global > built-in). The **project's** entries are what's
/// *active*: a `key:` entry defines + enables (scope `project`); a `use:` entry
/// enables a catalog metric, layering its threshold overrides on top (scope = the
/// definition's scope). A `use:` referencing an unknown key is skipped
/// silently: this runs on every event, so [`unknown_uses`] reports those
/// once per load (tsk1076).
pub fn resolve_metrics(
    builtin: &[MetricEntry],
    global: &[MetricEntry],
    extensions: &[ExtensionLayer<MetricEntry>],
    project: &[MetricEntry],
) -> Vec<ResolvedSpec> {
    let catalog = definition_catalog(builtin, global, extensions);
    let mut out = Vec::new();
    for e in project {
        if let Some(uk) = e.use_key.as_deref() {
            if let Some((scope, def)) = catalog.get(uk) {
                out.push(resolve_one(uk, scope, def, Some(e)));
            }
        }
    }
    // An enabled extension's own definitions are resolved unless the project
    // mentions the key (a `use:` override or disable marker, handled above):
    // active, or listed inactive when `default: off`.
    let mentioned: std::collections::HashSet<&str> = project
        .iter()
        .filter_map(|e| e.use_key.as_deref())
        .collect();
    let mut seen = std::collections::HashSet::new();
    for (name, entries) in extensions {
        for e in entries {
            let Some(k) = e.key.as_deref() else { continue };
            if mentioned.contains(k) || !seen.insert(k.to_string()) {
                continue;
            }
            if let Some((scope, def)) = catalog.get(k) {
                if *scope == extension_scope(name) {
                    let mut spec = resolve_one(k, scope, def, None);
                    // An off-by-default one is listed, inactive, until a `use:`.
                    spec.enabled = def.default != Some(MetricDefault::Off);
                    out.push(spec);
                }
            }
        }
    }
    out
}

/// Entries one extension declares: `(extension name, entries)`.
pub type ExtensionLayer<T> = (String, Vec<T>);

/// The scope string for an extension's entries.
pub fn extension_scope(name: &str) -> String {
    oxplow_domain::refs::build::extension_ref(name)
}

/// The extension name in an `extension:<name>` scope.
pub fn scope_extension(scope: &str) -> Option<String> {
    oxplow_domain::refs::build::id_of(scope, oxplow_domain::refs::kind::EXTENSION).ok()
}

/// Scopes in precedence order (later wins): built-in, global, each
/// extension.
fn scoped_layers<'a, T>(
    builtin: &'a [T],
    global: &'a [T],
    extensions: &'a [ExtensionLayer<T>],
) -> Vec<(String, &'a [T])> {
    let mut out = vec![
        ("built-in".to_string(), builtin),
        ("global".to_string(), global),
    ];
    out.extend(
        extensions
            .iter()
            .map(|(n, e)| (extension_scope(n), e.as_slice())),
    );
    out
}

/// Build a [`ResolvedSpec`] from a definition entry `def` (in `scope`),
/// optionally layering threshold overrides from a `use:` entry `over`. Only the
/// thresholds (`target`/`warnAt`/`failAt`) are overridable — the structural spec
/// (measure/aggregation/filter/formula) is inherent to the definition.
fn resolve_one(
    key: &str,
    scope: &str,
    def: &MetricEntry,
    over: Option<&MetricEntry>,
) -> ResolvedSpec {
    let pick_f64 = |get: fn(&MetricEntry) -> Option<f64>| -> Option<f64> {
        over.and_then(get).or_else(|| get(def))
    };
    ResolvedSpec {
        key: key.to_string(),
        title: def.title.clone().unwrap_or_else(|| key.to_string()),
        source_measure: def.source_measure.clone(),
        aggregation: def.aggregation.clone().unwrap_or_else(|| "last".into()),
        filter: def.filter.clone(),
        formula: def.formula.clone(),
        unit: def.unit.clone(),
        direction: def.direction.clone().unwrap_or_else(|| "neutral".into()),
        display_kind: def.display_kind.clone().unwrap_or_else(|| "gauge".into()),
        category: def.category.clone(),
        language: def.language.clone(),
        description: def.description.clone(),
        sliceable_dims: def.sliceable_dims.clone(),
        target: pick_f64(|e| e.target),
        warn_at: pick_f64(|e| e.warn_at),
        fail_at: pick_f64(|e| e.fail_at),
        scope: scope.to_string(),
        collector: def.collector.clone(),
        // The `enabled` flag lives on the acting (project) entry — the `use:`
        // override for a use'd metric, else the `key:` definition. Default on.
        enabled: over.and_then(|o| o.enabled).or(def.enabled).unwrap_or(true),
        entity: def.entity.clone().map(|view| EntitySpec {
            view,
            where_: def.where_.clone(),
            time: def.time.clone(),
            value: def.value.clone(),
            aggregation: def.aggregation.clone().unwrap_or_else(|| "count".into()),
        }),
    }
}

/// Load metric definitions from the user-global scope
/// (`<global_dir>/metrics/*.yaml`). Each file is a `{ metrics: [ … ] }`
/// document (same shape as the `.oxplow/project.yaml` block). Best-effort: an unreadable
/// or malformed file is logged and skipped, never an error. Returns the entries
/// in filename order for deterministic precedence.
pub fn load_global_metric_entries(global_dir: &Path) -> Vec<MetricEntry> {
    #[derive(Deserialize)]
    struct Doc {
        #[serde(default)]
        metrics: Option<Vec<MetricEntry>>,
    }
    load_global_entries(global_dir, "metrics", |raw| {
        serde_yaml::from_str::<Doc>(raw)
            .ok()
            .map(|d| validate_metrics(d.metrics, CatalogLayer::Global).map_err(|e| e.to_string()))
    })
}

/// List `*.yaml`/`*.yml` files under `<global_dir>/<subdir>`, sorted by filename
/// for deterministic precedence. Empty when the directory is absent. Shared by
/// the global catalog loaders (metrics / measures / dimensions).
fn global_yaml_files(global_dir: &Path, subdir: &str) -> Vec<PathBuf> {
    let dir = global_dir.join(subdir);
    let Ok(read) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("yaml") | Some("yml")
            )
        })
        .collect();
    files.sort();
    files
}

/// Load one global catalog kind from `<global_dir>/<subdir>/*.yaml`, in
/// filename order. `parse` turns a file's raw text into `Some(Ok(entries))`,
/// `Some(Err(msg))` (well-formed YAML that fails validation → "malformed"), or
/// `None` (unreadable/unparseable → "unreadable"). Best-effort: a bad file is
/// logged and skipped. Shared by the four `load_global_*_entries` loaders — they
/// differ only in the doc field + validator, which live in `parse`.
fn load_global_entries<E>(
    global_dir: &Path,
    subdir: &str,
    parse: impl Fn(&str) -> Option<Result<Vec<E>, String>>,
) -> Vec<E> {
    let mut out = Vec::new();
    for path in global_yaml_files(global_dir, subdir) {
        match std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| parse(&raw))
        {
            Some(Ok(entries)) => out.extend(entries),
            Some(Err(e)) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping malformed global {} file", subdir)
            }
            None => {
                tracing::warn!(path = %path.display(), "skipping unreadable global {} file", subdir)
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Measures + dimensions (the fact-catalog authoring surface — epic tsk12, E)
// ---------------------------------------------------------------------------

/// Shared key check for the `measures:` / `dimensions:` catalogs: the key must
/// be present, namespaced `<vendor>.<id>`, definable by `namespace`
/// ([`reserved_key`]), and unique within its block. Returns the cleaned key.
/// `block` is the YAML block name for error messages.
fn validate_catalog_key(
    block: &str,
    i: usize,
    raw_key: Option<String>,
    namespace: Option<&str>,
    seen: &mut std::collections::HashSet<String>,
) -> Result<String, ConfigError> {
    let key = raw_key
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ConfigError::Invalid(format!("{block}[{i}] must set a `key`")))?;
    if !key.contains('.') {
        return Err(ConfigError::Invalid(format!(
            "{block}[{i}] key \"{key}\" must be namespaced as \"<vendor>.<id>\""
        )));
    }
    if let Some(e) = reserved_key(&key, namespace) {
        return Err(ConfigError::Invalid(format!("{block}[{i}] {e}")));
    }
    if !seen.insert(key.clone()) {
        return Err(ConfigError::Invalid(format!(
            "{block}[{i}] key \"{key}\" appears more than once in the {block} block"
        )));
    }
    Ok(key)
}

/// Validate a `measures:` block written in `layer` (an extension's or the
/// person's global files). Mirrors [`validate_metrics`]: namespaced keys,
/// `oxplow.*` only in the `oxplow` namespace, per-key uniqueness, known
/// temporalSemantics/captureScope enums. Definition-only (no `use:` form).
pub fn validate_measures(
    raw: Option<Vec<MeasureEntry>>,
    layer: CatalogLayer<'_>,
) -> Result<Vec<MeasureEntry>, ConfigError> {
    let namespace = layer.namespace();
    let opt = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, e) in raw.into_iter().flatten().enumerate() {
        let key = validate_catalog_key("measures", i, e.key, namespace, &mut seen)?;
        let temporal_semantics = match opt(e.temporal_semantics) {
            Some(s) => {
                if !MEASURE_TEMPORAL_SEMANTICS.contains(&s.as_str()) {
                    return Err(ConfigError::Invalid(format!(
                        "measures[{i}] temporalSemantics must be one of \
                         {MEASURE_TEMPORAL_SEMANTICS:?} (got \"{s}\")"
                    )));
                }
                Some(s)
            }
            None => None,
        };
        let capture_scope = match opt(e.capture_scope) {
            Some(s) => {
                if !MEASURE_CAPTURE_SCOPES.contains(&s.as_str()) {
                    return Err(ConfigError::Invalid(format!(
                        "measures[{i}] captureScope must be one of \
                         {MEASURE_CAPTURE_SCOPES:?} (got \"{s}\")"
                    )));
                }
                Some(s)
            }
            None => None,
        };
        out.push(MeasureEntry {
            key: Some(key),
            title: opt(e.title),
            unit: opt(e.unit),
            subject_kind: opt(e.subject_kind),
            temporal_semantics,
            capture_scope,
            description: opt(e.description),
        });
    }
    Ok(out)
}

/// Validate a `dimensions:` block written in `layer`. Mirrors
/// [`validate_measures`]: namespaced keys, `oxplow.*` only in the `oxplow`
/// namespace, per-key uniqueness, known valueType.
pub fn validate_dimensions(
    raw: Option<Vec<DimensionEntry>>,
    layer: CatalogLayer<'_>,
) -> Result<Vec<DimensionEntry>, ConfigError> {
    let namespace = layer.namespace();
    let opt = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, e) in raw.into_iter().flatten().enumerate() {
        let key = validate_catalog_key("dimensions", i, e.key, namespace, &mut seen)?;
        let value_type = match opt(e.value_type) {
            Some(s) => {
                if !DIMENSION_VALUE_TYPES.contains(&s.as_str()) {
                    return Err(ConfigError::Invalid(format!(
                        "dimensions[{i}] valueType must be one of \
                         {DIMENSION_VALUE_TYPES:?} (got \"{s}\")"
                    )));
                }
                Some(s)
            }
            None => None,
        };
        let vocabulary = e
            .vocabulary
            .into_iter()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect();
        let entity = opt(e.entity);
        let expr = opt(e.expr);
        let join = opt(e.join);
        match &entity {
            Some(view) => {
                if !is_entity_name(view) {
                    return Err(ConfigError::Invalid(format!(
                        "dimensions[{i}] entity must name a `v_*` view (got \"{view}\")"
                    )));
                }
                if expr.is_none() {
                    return Err(ConfigError::Invalid(format!(
                        "dimensions[{i}] is an entity dimension; it needs an `expr`"
                    )));
                }
                if e.promote {
                    return Err(ConfigError::Invalid(format!(
                        "dimensions[{i}] is an entity dimension; `promote` applies to fact dimensions only"
                    )));
                }
            }
            None if expr.is_some() || join.is_some() => {
                return Err(ConfigError::Invalid(format!(
                    "dimensions[{i}] sets `expr`/`join` without `entity`"
                )))
            }
            None => {}
        }
        out.push(DimensionEntry {
            key: Some(key),
            label: opt(e.label),
            value_type,
            subject_kind: opt(e.subject_kind),
            vocabulary,
            promote: e.promote,
            entity,
            expr,
            join,
        });
    }
    Ok(out)
}

/// Resolve declared measures across the global + project scopes into the flat
/// [`ResolvedMeasure`] list the boot seeder upserts into the `measure` catalog.
/// Both scopes are definition-only (a measure is declared, never "enabled"); a
/// project entry with the same key as a global one wins (precedence project >
/// global). First-seen order is preserved. The `oxplow.*` built-ins are the
/// migration seed and never flow through here.
pub fn resolve_measures(
    global: &[MeasureEntry],
    extensions: &[ExtensionLayer<MeasureEntry>],
) -> Vec<ResolvedMeasure> {
    let mut out: Vec<ResolvedMeasure> = Vec::new();
    let mut pos: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (scope, entries) in scoped_layers(&[], global, extensions) {
        for e in entries {
            let Some(key) = e.key.as_deref() else {
                continue;
            };
            let resolved = ResolvedMeasure {
                key: key.to_string(),
                title: e.title.clone().unwrap_or_else(|| key.to_string()),
                unit: e.unit.clone(),
                subject_kind: e.subject_kind.clone(),
                temporal_semantics: e
                    .temporal_semantics
                    .clone()
                    .unwrap_or_else(|| "semi-additive".into()),
                capture_scope: e.capture_scope.clone().unwrap_or_else(|| "complete".into()),
                scope: scope.clone(),
                description: e.description.clone(),
            };
            match pos.get(key) {
                Some(&i) => out[i] = resolved,
                None => {
                    pos.insert(key.to_string(), out.len());
                    out.push(resolved);
                }
            }
        }
    }
    out
}

/// Resolve declared dimensions across the global + project scopes (project >
/// global), analogous to [`resolve_measures`].
pub fn resolve_dimensions(
    global: &[DimensionEntry],
    extensions: &[ExtensionLayer<DimensionEntry>],
) -> Vec<ResolvedDimension> {
    let mut out: Vec<ResolvedDimension> = Vec::new();
    let mut pos: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (scope, entries) in scoped_layers(&[], global, extensions) {
        for e in entries {
            let Some(key) = e.key.as_deref() else {
                continue;
            };
            let resolved = ResolvedDimension {
                key: key.to_string(),
                label: e.label.clone().unwrap_or_else(|| key.to_string()),
                value_type: e.value_type.clone().unwrap_or_else(|| "categorical".into()),
                subject_kind: e.subject_kind.clone(),
                vocabulary: e.vocabulary.clone(),
                scope: scope.to_string(),
                promote: e.promote,
                entity: e.entity.clone().map(|view| EntityDimensionSpec {
                    view,
                    expr: e.expr.clone().unwrap_or_default(),
                    join: e.join.clone(),
                }),
            };
            match pos.get(key) {
                Some(&i) => out[i] = resolved,
                None => {
                    pos.insert(key.to_string(), out.len());
                    out.push(resolved);
                }
            }
        }
    }
    out
}

/// Load measure definitions from the user-global scope
/// (`<global_dir>/measures/*.yaml`, each a `{ measures: [ … ] }` doc).
/// Best-effort: a malformed/unreadable file is logged and skipped. Filename
/// order for deterministic precedence.
pub fn load_global_measure_entries(global_dir: &Path) -> Vec<MeasureEntry> {
    #[derive(Deserialize)]
    struct Doc {
        #[serde(default)]
        measures: Option<Vec<MeasureEntry>>,
    }
    load_global_entries(global_dir, "measures", |raw| {
        serde_yaml::from_str::<Doc>(raw)
            .ok()
            .map(|d| validate_measures(d.measures, CatalogLayer::Global).map_err(|e| e.to_string()))
    })
}

/// Load dimension definitions from the user-global scope
/// (`<global_dir>/dimensions/*.yaml`, each a `{ dimensions: [ … ] }` doc).
/// Best-effort; analogous to [`load_global_measure_entries`].
pub fn load_global_dimension_entries(global_dir: &Path) -> Vec<DimensionEntry> {
    #[derive(Deserialize)]
    struct Doc {
        #[serde(default)]
        dimensions: Option<Vec<DimensionEntry>>,
    }
    load_global_entries(global_dir, "dimensions", |raw| {
        serde_yaml::from_str::<Doc>(raw).ok().map(|d| {
            validate_dimensions(d.dimensions, CatalogLayer::Global).map_err(|e| e.to_string())
        })
    })
}

/// A harness key's shape: lowercase letters, digits, `_` and `-`, starting
/// with a letter. Whether one is registered is the app's to say.
fn is_agent_key(key: &str) -> bool {
    key.starts_with(|c: char| c.is_ascii_lowercase())
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn validate_agents(raw: Option<Vec<String>>) -> Result<Vec<String>, ConfigError> {
    let Some(agents) = raw else {
        return Ok(Vec::new());
    };
    if agents.is_empty() {
        return Err(ConfigError::Invalid(
            "agents must list at least one enabled agent (leave it out to enable every one)".into(),
        ));
    }
    let mut seen: Vec<String> = Vec::new();
    for agent in agents {
        if !is_agent_key(&agent) {
            return Err(ConfigError::Invalid(format!(
                "agents: `{agent}` isn't an agent key (lowercase letters, digits, `_`, `-`)"
            )));
        }
        if seen.contains(&agent) {
            return Err(ConfigError::Invalid(format!(
                "agents must not contain duplicates (got `{agent}`)"
            )));
        }
        seen.push(agent);
    }
    Ok(seen)
}

/// `agentConfig`: an object per harness key. Its fields are the harness's
/// to check.
fn validate_agent_config(
    raw: std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<std::collections::BTreeMap<String, serde_json::Value>, ConfigError> {
    for (agent, config) in &raw {
        if !is_agent_key(agent) {
            return Err(ConfigError::Invalid(format!(
                "agentConfig: `{agent}` isn't an agent key"
            )));
        }
        if !config.is_object() {
            return Err(ConfigError::Invalid(format!(
                "agentConfig.{agent} must be a map of settings"
            )));
        }
    }
    Ok(raw)
}

fn validate_scripts(raw: Option<RawScriptsBlock>) -> Result<ScriptsConfig, ConfigError> {
    let mut out = ScriptsConfig::default();
    let Some(raw) = raw else {
        return Ok(out);
    };
    let whole = |field: &str, n: f64, min: u64| {
        if n.is_finite() && n >= min as f64 {
            Ok(n.floor() as u64)
        } else {
            Err(ConfigError::Invalid(format!(
                "scripts.{field} must be a number >= {min}"
            )))
        }
    };
    if let Some(n) = raw.max_concurrent {
        out.max_concurrent = whole("maxConcurrent", n, 1)? as usize;
    }
    if let Some(n) = raw.memory_mb {
        out.memory_bytes = whole("memoryMb", n, MIN_SCRIPTS_MEMORY_MB)? * 1024 * 1024;
    }
    Ok(out)
}

fn validate_testing(raw: Option<RawTestingBlock>) -> Result<TestingConfig, ConfigError> {
    let Some(raw) = raw else {
        return Ok(TestingConfig::default());
    };
    let opt_trimmed = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let patterns = |field: &str, list: Option<Vec<String>>| {
        let mut out = Vec::new();
        for (i, p) in list.into_iter().flatten().enumerate() {
            let trimmed = p.trim().to_string();
            if trimmed.is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "testing.{field}[{i}] must be a non-empty string"
                )));
            }
            out.push(trimmed);
        }
        Ok(out)
    };
    // Globs are checked here, so a bad one fails the load rather than
    // silently marking nothing as a test.
    let globs = |field: &str, list: Option<Vec<String>>| {
        let list = patterns(field, list)?;
        for (i, glob) in list.iter().enumerate() {
            if let Err(e) = globset::GlobBuilder::new(glob)
                .literal_separator(true)
                .build()
            {
                return Err(ConfigError::Invalid(format!(
                    "testing.{field}[{i}] `{glob}` isn't a valid glob: {e}"
                )));
            }
        }
        Ok(list)
    };
    Ok(TestingConfig {
        command: opt_trimmed(raw.command),
        fast_command: opt_trimmed(raw.fast_command),
        run_patterns: patterns("runPatterns", raw.run_patterns)?,
        analysis_patterns: patterns("analysisPatterns", raw.analysis_patterns)?,
        test_paths: globs("testPaths", raw.test_paths)?,
        agent_hint: opt_trimmed(raw.agent_hint),
    })
}

pub(crate) fn basename(path: &Path) -> String {
    let resolved: PathBuf = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    resolved
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "oxplow".to_string())
}

#[cfg(test)]
mod global_config_dir_tests {
    use super::*;

    #[test]
    fn override_replaces_the_platform_dir() {
        let dir = global_config_dir_from(Some("/tmp/oxplow-dev".into())).unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/oxplow-dev"));
    }

    /// The override is used verbatim — no `net.voxland.oxplow` suffix —
    /// so `OXPLOW_HOME=<dir>` puts `session.json` directly in `<dir>`.
    #[test]
    fn override_is_not_suffixed_with_the_app_identifier() {
        let dir = global_config_dir_from(Some("/tmp/oxplow-dev".into())).unwrap();
        assert!(!dir.ends_with(APP_IDENTIFIER));
    }

    #[test]
    fn absent_override_uses_the_platform_dir() {
        let dir = global_config_dir_from(None).unwrap();
        assert!(dir.ends_with(APP_IDENTIFIER));
    }

    /// An exported-but-empty `OXPLOW_HOME` must not resolve to a
    /// relative "" and scatter global state through the cwd.
    #[test]
    fn empty_override_falls_back_to_the_platform_dir() {
        let dir = global_config_dir_from(Some("".into())).unwrap();
        assert!(dir.ends_with(APP_IDENTIFIER));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An instance's config is a `serde_json::Value`, and serde_json's
    /// `arbitrary_precision` makes a number a private struct when it's
    /// serialized by anything but serde_json — straight into
    /// `project.yaml` as `{$serde_json::private::Number: '5'}`. Every
    /// entry goes through JSON text, so the file holds the number.
    /// P7.A2: `activeProviders` names one provider per swappable
    /// capability, round-trips through the file, and refuses another
    /// capability or a malformed id.
    #[test]
    fn active_providers_name_one_provider_per_capability() {
        let parse = |yaml: &str| parse_project_config(serde_yaml::from_str(yaml).unwrap(), "demo");
        let config = parse("activeProviders: { work_items: issues }\n").unwrap();
        assert_eq!(config.active_providers["work_items"], "issues");
        let doc = render_project_config(&config, "demo");
        let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc)).unwrap();
        assert!(
            yaml.contains("activeProviders:\n  work_items: issues"),
            "{yaml}"
        );
        let err = parse("activeProviders: { vcs: jj }\n").unwrap_err();
        assert!(err.to_string().contains("work_items"), "{err}");
        let err = parse("activeProviders: { work_items: Issues-App }\n").unwrap_err();
        assert!(err.to_string().contains("instance id"), "{err}");
        assert!(parse("agents: [claude]\n")
            .unwrap()
            .active_providers
            .is_empty());
    }

    /// A person's own choice (`.oxplow/personal.yaml`, which git ignores)
    /// layers over the project's: the same `activeProviders` shape, and
    /// nothing else. Every choosable capability may be named; one core
    /// doesn't let a project choose may not.
    #[test]
    fn personal_choices_layer_over_the_projects() {
        let dir = tempfile::tempdir().unwrap();
        let oxplow = dir.path().join(".oxplow");
        std::fs::create_dir_all(&oxplow).unwrap();
        std::fs::write(
            oxplow.join("project.yaml"),
            "activeProviders: { work_items: issues }\n",
        )
        .unwrap();
        std::fs::write(
            oxplow.join("personal.yaml"),
            "activeProviders: { effort_policy: none }\n",
        )
        .unwrap();
        let config = load_project_config(dir.path()).unwrap();
        assert_eq!(
            config
                .active_providers
                .get("work_items")
                .map(String::as_str),
            Some("issues")
        );
        assert_eq!(config.active_providers.get("effort_policy"), None);
        assert_eq!(
            config
                .personal_active_providers
                .get("effort_policy")
                .map(String::as_str),
            Some("none")
        );
        std::fs::write(oxplow.join("personal.yaml"), "zones: []\n").unwrap();
        let err = load_project_config(dir.path()).unwrap_err().to_string();
        assert!(
            err.contains("personal.yaml") && err.contains("activeProviders"),
            "{err}"
        );

        let parse = |yaml: &str| parse_project_config(serde_yaml::from_str(yaml).unwrap(), "demo");
        assert!(parse("activeProviders: { snapshots: hashes }\n").is_ok());
        let err = parse("activeProviders: { vcs: jj }\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("snapshots"), "{err}");
    }

    /// A personal choice is written to, and removed from, `personal.yaml`
    /// alone.
    #[test]
    fn a_personal_choice_is_written_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        write_personal_key(
            dir.path(),
            "activeProviders",
            Some(&serde_json::json!({ "effort_policy": "none" })),
        )
        .unwrap();
        let config = load_project_config(dir.path()).unwrap();
        assert_eq!(
            config
                .personal_active_providers
                .get("effort_policy")
                .map(String::as_str),
            Some("none")
        );
        assert!(!config_path(dir.path()).exists(), "project.yaml untouched");
        write_personal_key(dir.path(), "activeProviders", None).unwrap();
        assert!(load_project_config(dir.path())
            .unwrap()
            .personal_active_providers
            .is_empty());
        assert!(write_personal_key(dir.path(), "zones", None).is_err());
    }

    /// What a new thread starts with is a person's own choice: `ask`,
    /// `none`, or an agent. It sits beside their capability choices in
    /// `personal.yaml`, and setting one leaves the other.
    #[test]
    fn a_new_thread_session_choice_is_personal() {
        let dir = tempfile::tempdir().unwrap();
        write_personal_key(
            dir.path(),
            "activeProviders",
            Some(&serde_json::json!({ "effort_policy": "none" })),
        )
        .unwrap();
        write_personal_key(
            dir.path(),
            "newThreadSession",
            Some(&serde_json::json!("claude")),
        )
        .unwrap();
        let config = load_project_config(dir.path()).unwrap();
        assert_eq!(
            config.personal_new_thread_session.as_deref(),
            Some("claude")
        );
        assert_eq!(config.personal_active_providers.len(), 1);
        assert_eq!(
            personal_value(&config, "newThreadSession"),
            Some(serde_json::json!("claude"))
        );

        // Setting one key in memory keeps the other.
        let next = with_personal_key(
            &config,
            "newThreadSession",
            Some(&serde_json::json!("none")),
        )
        .unwrap();
        assert_eq!(next.personal_new_thread_session.as_deref(), Some("none"));
        assert_eq!(next.personal_active_providers.len(), 1);

        assert!(
            write_personal_key(dir.path(), "newThreadSession", Some(&serde_json::json!("")))
                .is_err()
        );
        write_personal_key(dir.path(), "newThreadSession", None).unwrap();
        let config = load_project_config(dir.path()).unwrap();
        assert_eq!(config.personal_new_thread_session, None);
        assert_eq!(config.personal_active_providers.len(), 1);
    }

    /// tsk947: `eventRetention` sets a namespace's windows — a person's key.
    /// Core state (kept whole) and a window under a day are refused.
    #[test]
    fn event_retention_sets_a_namespaces_windows() {
        use oxplow_domain::events::retention::RetentionWindow;
        let parse = |yaml: &str| parse_project_config(serde_yaml::from_str(yaml).unwrap(), "demo");
        let config = parse(
            "eventRetention:\n  agent: { payloadDays: 60, contentDays: 14 }\n  acme_pr: { payloadDays: 7, contentDays: 3 }\n",
        )
        .unwrap();
        assert_eq!(
            config.event_retention.get("agent"),
            Some(&RetentionWindow::new(60, 14))
        );
        assert_eq!(
            config.event_retention.get("acme_pr"),
            Some(&RetentionWindow::new(7, 3))
        );
        let doc = render_project_config(&config, "demo");
        let again = parse_project_config(serde_yaml::Value::Mapping(doc), "demo").unwrap();
        assert_eq!(again.event_retention, config.event_retention);
        let err = parse("eventRetention:\n  snapshot: { payloadDays: 7, contentDays: 3 }\n")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("`snapshot`") && err.contains("kept whole"),
            "{err}"
        );
        // tsk985: windows are bounded — at least a week for a core
        // namespace oxplow reads back, a day for an extension's; at most a
        // hundred years; and a body is never kept longer than its payload.
        for (yaml, says) in [
            (
                "agent: { payloadDays: 3, contentDays: 3 }",
                "at least 7 days",
            ),
            (
                "acme_pr: { payloadDays: 0, contentDays: 0 }",
                "at least 1 day",
            ),
            (
                "agent: { payloadDays: 999999, contentDays: 14 }",
                "at most 36500 days",
            ),
            (
                "acme_pr: { payloadDays: 7, contentDays: 99999999999 }",
                "at most 36500 days",
            ),
            (
                "agent: { payloadDays: 30, contentDays: 60 }",
                "contentDays can't be longer than payloadDays",
            ),
        ] {
            let err = parse(&format!("eventRetention:\n  {yaml}\n"))
                .unwrap_err()
                .to_string();
            assert!(err.contains(says), "{yaml}: {err}");
        }
        assert!(
            parse("eventRetention:\n  agent: { payloadDays: 36500, contentDays: 36500 }\n").is_ok()
        );
        let plain = parse("agents: [claude]\n").unwrap();
        let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(render_project_config(
            &plain, "demo",
        )))
        .unwrap();
        assert!(!yaml.contains("eventRetention"), "{yaml}");
        // How long a project keeps its agents' activity is a person's call.
        assert!(crate::keys::HUMAN_ONLY_KEYS.contains(&"eventRetention"));
    }

    /// `replacements` says whose component shows, by target: an
    /// extension, or `oxplow` for oxplow's own. A bound target (the state
    /// control) can only be kept oxplow's — its provider's extension
    /// decides otherwise — and an unknown target is an error listing them.
    #[test]
    fn replacements_chooses_whose_component_shows() {
        let parse = |yaml: &str| parse_project_config(serde_yaml::from_str(yaml).unwrap(), "demo");
        let config = parse(
            "replacements: { vcs.history.graph: oxplow-bundled, work_item.board: task-views, work_item.detail.state: oxplow }\n",
        )
        .unwrap();
        assert_eq!(
            config
                .replacements
                .get("work_item.board")
                .map(String::as_str),
            Some("task-views")
        );
        assert_eq!(
            config
                .replacements
                .get("work_item.detail.state")
                .map(String::as_str),
            Some("oxplow")
        );
        let doc = render_project_config(&config, "demo");
        let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc)).unwrap();
        assert!(yaml.contains("vcs.history.graph: oxplow-bundled"), "{yaml}");
        let bound = parse("replacements: { work_item.detail.state: beads }\n").unwrap_err();
        assert!(
            bound
                .to_string()
                .contains("no other extension can be chosen"),
            "{bound}"
        );
        let unknown = parse("replacements: { vcs.conflict.resolver: x }\n").unwrap_err();
        assert!(unknown.to_string().contains("work_item.list"), "{unknown}");
        assert!(parse("agents: [claude]\n").unwrap().replacements.is_empty());
        assert!(
            parse("replacementsOff: [work_item.board]\n").is_err(),
            "the old key is gone"
        );
        assert!(crate::keys::HUMAN_ONLY_KEYS.contains(&"replacements"));
    }

    /// P9.B1: an instance is `<extension>/<instance id>`; a provider's
    /// default instance has the provider's id, and another one says which
    /// provider it is.
    #[test]
    fn an_instance_key_is_an_extension_and_a_snake_case_instance_id() {
        let parse = |yaml: &str| parse_project_config(serde_yaml::from_str(yaml).unwrap(), "demo");
        let config = parse(
            "extensionInstances:\n  my-tracker/issues: { enabled: true }\n  my-tracker/issues_acme: { enabled: true, provider: issues }\n",
        )
        .unwrap();
        assert_eq!(
            config.extension_instances["my-tracker/issues"].provider,
            None
        );
        assert_eq!(
            config.extension_instances["my-tracker/issues_acme"]
                .provider
                .as_deref(),
            Some("issues")
        );
        let doc = render_project_config(&config, "demo");
        let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc)).unwrap();
        assert!(yaml.contains("provider: issues"), "{yaml}");
        assert_eq!(
            yaml.matches("provider:").count(),
            1,
            "absent, it isn't written: {yaml}"
        );
        for bad in [
            "tracker/Issues",
            "tracker/issues-acme",
            "tracker/1st",
            "tracker/",
            "/issues",
            "tracker/a/b",
            "issues",
        ] {
            let err = parse(&format!(
                "extensionInstances:\n  \"{bad}\": {{ enabled: true }}\n"
            ))
            .unwrap_err()
            .to_string();
            assert!(err.contains("<extension>/<instance id>"), "{bad}: {err}");
        }
        let err = parse("extensionInstances:\n  tracker/acme: { provider: Lin-ear }\n")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("`provider`") && err.contains("Lin-ear"),
            "{err}"
        );
    }

    /// P9.B2: the user-global instances file — the same entries as a
    /// project's `extensionInstances`, for every project of this machine.
    #[test]
    fn global_instances_load_save_and_say_what_is_wrong() {
        let dir = tempfile::tempdir().unwrap();
        assert!(GlobalInstances::load(dir.path())
            .unwrap()
            .instances
            .is_empty());
        let mut global = GlobalInstances::default();
        global.instances.insert(
            "tracker/issues_acme".into(),
            ExtensionInstanceConfig {
                enabled: true,
                config: serde_json::json!({ "team": "ACME" }),
                sync_minutes: None,
                provider: Some("issues".into()),
            },
        );
        let written = global.instances.clone();
        GlobalInstances::update(dir.path(), |all| {
            *all = written;
            Ok::<_, ()>(())
        })
        .unwrap()
        .unwrap();
        let text = std::fs::read_to_string(dir.path().join(INSTANCES_FILE)).unwrap();
        assert!(
            text.contains("tracker/issues_acme") && text.contains("provider: issues"),
            "{text}"
        );
        assert!(
            !text.contains("syncMinutes"),
            "an absent schedule isn't written: {text}"
        );
        assert_eq!(GlobalInstances::load(dir.path()).unwrap(), global);

        std::fs::write(
            dir.path().join(INSTANCES_FILE),
            "instances:\n  tracker/Issues: { enabled: true }\n",
        )
        .unwrap();
        let err = GlobalInstances::load(dir.path()).unwrap_err().to_string();
        assert!(
            err.contains("instances.yaml") && err.contains("tracker/Issues"),
            "{err}"
        );
        assert!(!err.contains("project.yaml"), "{err}");
        std::fs::write(dir.path().join(INSTANCES_FILE), "extensionInstances: {}\n").unwrap();
        let err = GlobalInstances::load(dir.path()).unwrap_err().to_string();
        assert!(err.contains("instances.yaml"), "{err}");
        // An invalid set is never written.
        std::fs::remove_file(dir.path().join(INSTANCES_FILE)).unwrap();
        assert!(GlobalInstances::update(dir.path(), |all| {
            all.insert("nope".into(), ExtensionInstanceConfig::default());
            Ok::<_, ()>(())
        })
        .is_err());
        assert!(!dir.path().join(INSTANCES_FILE).exists());
        // An edit's refusal writes nothing.
        assert_eq!(
            GlobalInstances::update(dir.path(), |_| Err("no")).unwrap(),
            Err("no")
        );
    }

    /// tsk840: an instance's id is a provider id's kind of name — its refs'
    /// segment and its command namespace — so the same names are oxplow's
    /// own: `oxplow`, and a core namespace.
    #[test]
    fn an_instance_id_may_not_be_oxplows_own() {
        for id in ["oxplow", "code", "config", "work_item"] {
            let mut all = std::collections::BTreeMap::new();
            all.insert(
                format!("tracker/{id}"),
                ExtensionInstanceConfig {
                    provider: Some("issues".into()),
                    ..ExtensionInstanceConfig::default()
                },
            );
            let err = validate_extension_instances(all).unwrap_err().to_string();
            assert!(
                err.contains(&format!("`{id}` is reserved for oxplow")),
                "{err}"
            );
        }
    }

    /// tsk837: each change to the machine's instances is one
    /// read-modify-write of the file as it is then, under a lock other
    /// processes take too — so writers at once never drop each other's
    /// entry.
    #[test]
    fn global_instance_updates_at_once_all_land() {
        let dir = tempfile::tempdir().unwrap();
        let writers: Vec<_> = (0..16)
            .map(|n| {
                let dir = dir.path().to_path_buf();
                std::thread::spawn(move || {
                    GlobalInstances::update(&dir, |all| {
                        all.insert(
                            format!("tracker/issues_{n}"),
                            ExtensionInstanceConfig {
                                provider: Some("issues".into()),
                                ..ExtensionInstanceConfig::default()
                            },
                        );
                        Ok::<_, ()>(())
                    })
                    .unwrap()
                    .unwrap();
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        assert_eq!(
            GlobalInstances::load(dir.path()).unwrap().instances.len(),
            16
        );
    }

    #[test]
    fn rendered_config_holds_plain_numbers_from_json_values() {
        let mut config = default_config("demo".into());
        config.extension_instances.insert(
            "acme/github".into(),
            ExtensionInstanceConfig {
                enabled: true,
                config: serde_json::json!({ "pollMinutes": 5, "ratio": 0.5 }),
                sync_minutes: None,
                provider: None,
            },
        );
        let doc = render_project_config(&config, "demo");
        let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc)).unwrap();
        assert!(yaml.contains("pollMinutes: 5\n"), "{yaml}");
        assert!(
            !yaml.contains("syncMinutes"),
            "an absent schedule isn't written: {yaml}"
        );
        assert!(yaml.contains("ratio: 0.5\n"), "{yaml}");
        assert!(!yaml.contains("serde_json"), "{yaml}");
        let back = parse_project_config(serde_yaml::from_str(&yaml).unwrap(), "demo").unwrap();
        assert_eq!(
            back.extension_instances["acme/github"].config,
            serde_json::json!({ "pollMinutes": 5, "ratio": 0.5 })
        );
    }

    #[test]
    fn entity_metrics_and_dimensions_validate_and_resolve() {
        let yaml = r#"
metrics:
  - key: work.done
    title: Tasks completed
    entity: v_work_item
    where: "state = 'done'"
    time: closed_at
  - key: work.median_prio
    entity: v_work_item
    aggregation: median
    value: "e.rank"
dimensions:
  - key: work.prio
    entity: v_work_item
    expr: "json_extract(e.native, '$.priority')"
    join: "LEFT JOIN v_thread t ON t.id = e.thread_id"
"#;
        #[derive(Deserialize)]
        struct Doc {
            metrics: Option<Vec<MetricEntry>>,
            dimensions: Option<Vec<DimensionEntry>>,
        }
        let doc: Doc = serde_yaml::from_str(yaml).unwrap();
        let metrics = validate_metrics(doc.metrics, CatalogLayer::Extension("work")).unwrap();
        let extensions = vec![("work".to_string(), metrics)];
        let resolved = resolve_metrics(&[], &[], &extensions, &[]);
        assert_eq!(
            resolved[0].entity,
            Some(EntitySpec {
                view: "v_work_item".into(),
                where_: Some("state = 'done'".into()),
                time: Some("closed_at".into()),
                value: None,
                aggregation: "count".into(),
            })
        );
        assert_eq!(resolved[1].entity.as_ref().unwrap().aggregation, "median");
        let dims = validate_dimensions(doc.dimensions, CatalogLayer::Extension("work")).unwrap();
        let rd = resolve_dimensions(&[], &[("work".to_string(), dims)]);
        assert_eq!(
            rd[0].entity,
            Some(EntityDimensionSpec {
                view: "v_work_item".into(),
                expr: "json_extract(e.native, '$.priority')".into(),
                join: Some("LEFT JOIN v_thread t ON t.id = e.thread_id".into()),
            })
        );
    }

    #[test]
    fn entity_metric_mistakes_are_refused() {
        let bad = |yaml: &str| {
            let e: Vec<MetricEntry> = serde_yaml::from_str(yaml).unwrap();
            validate_metrics(Some(e), CatalogLayer::Global)
                .unwrap_err()
                .to_string()
        };
        assert!(bad("[{key: a.b, entity: tasks}]").contains("v_*"));
        assert!(
            bad("[{key: a.b, entity: v_work_item, aggregation: sum}]").contains("needs a `value`")
        );
        assert!(
            bad("[{key: a.b, entity: v_work_item, aggregation: ratio, value: x}]")
                .contains("entity aggregation")
        );
        assert!(
            bad("[{key: a.b, entity: v_work_item, sourceMeasure: m}]").contains("can't also set")
        );
        assert!(bad("[{key: a.b, sourceMeasure: m, where: x}]").contains("without `entity`"));
        assert!(bad("[{use: a.b, time: x}]").contains("`use:` entry"));
        let dim = |yaml: &str| {
            let e: Vec<DimensionEntry> = serde_yaml::from_str(yaml).unwrap();
            validate_dimensions(Some(e), CatalogLayer::Global)
                .unwrap_err()
                .to_string()
        };
        assert!(dim("[{key: a.b, entity: v_work_item}]").contains("needs an `expr`"));
        assert!(dim("[{key: a.b, expr: x}]").contains("without `entity`"));
        assert!(
            dim("[{key: a.b, entity: v_work_item, expr: x, promote: true}]")
                .contains("fact dimensions only")
        );
    }
    use tempfile::tempdir;

    /// Resolve the project config path under `<dir>/.oxplow/`, creating the
    /// `.oxplow` parent so a subsequent write succeeds. Used by tests that
    /// author a config file directly (simulating a user-edited file).
    /// tsk1028: a command's write changes only its key. The person's
    /// formatting, comments and default-valued keys (`agents: [claude]`)
    /// stay as written.
    #[test]
    fn a_key_write_leaves_the_rest_of_the_file_as_written() {
        let dir = tempdir().unwrap();
        let original = "# The cart project.\n\
                        agents: [claude]\n\
                        testing:\n  command: bun test   # with reports\n";
        std::fs::write(cfg_path(dir.path()), original).unwrap();
        let metrics = serde_json::json!([{ "use": "oxplow.fn_count" }]);
        write_project_key(dir.path(), "metrics", Some(&metrics)).unwrap();
        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        assert!(raw.starts_with(original), "the rest moved:\n{raw}");
        assert!(raw.ends_with("metrics:\n- use: oxplow.fn_count\n"), "{raw}");
        assert_eq!(load_project_config(dir.path()).unwrap().agents, ["claude"]);
    }

    /// Setting a key the file has replaces only that key's lines, under its
    /// comment; unsetting removes them, comment and all.
    #[test]
    fn a_key_write_replaces_or_removes_only_its_block() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "agents: [claude]\n\
             # Keep a week of history.\n\
             snapshotRetentionDays: 7\n\
             zones: []   # none yet\n",
        )
        .unwrap();
        write_project_key(
            dir.path(),
            "snapshotRetentionDays",
            Some(&serde_json::json!(14)),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(cfg_path(dir.path())).unwrap(),
            "agents: [claude]\n# Keep a week of history.\nsnapshotRetentionDays: 14\nzones: []   # none yet\n"
        );
        write_project_key(dir.path(), "snapshotRetentionDays", None).unwrap();
        assert_eq!(
            std::fs::read_to_string(cfg_path(dir.path())).unwrap(),
            "agents: [claude]\nzones: []   # none yet\n"
        );
    }

    /// A file that doesn't exist yet gets just the key.
    #[test]
    fn a_key_write_creates_the_file() {
        let dir = tempdir().unwrap();
        write_project_key(
            dir.path(),
            "snapshotRetentionDays",
            Some(&serde_json::json!(3)),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(config_path(dir.path())).unwrap(),
            "snapshotRetentionDays: 3\n"
        );
    }

    /// `cfg` as the renderer writes it: the round trip `keys::with_key`
    /// validates a change through.
    fn write_rendered(project_dir: &Path, cfg: &OxplowConfig) {
        let doc = render_project_config(cfg, &basename(project_dir));
        std::fs::write(
            cfg_path(project_dir),
            serde_yaml::to_string(&serde_yaml::Value::Mapping(doc)).unwrap(),
        )
        .unwrap();
    }

    fn cfg_path(project_dir: &Path) -> std::path::PathBuf {
        let p = config_path(project_dir);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        p
    }

    #[test]
    fn load_defaults_when_file_absent() {
        let dir = tempdir().unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert!(cfg.agents.is_empty(), "every registered harness");
        assert_eq!(cfg.snapshot_retention_days, DEFAULT_SNAPSHOT_RETENTION_DAYS);
        assert!(cfg.lsp_servers.is_empty());
        assert!(cfg.inject_session_context);
    }

    #[test]
    fn project_name_falls_back_to_basename() {
        let dir = tempdir().unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        let basename = dir.path().file_name().unwrap().to_string_lossy();
        assert_eq!(cfg.project_name, basename);
    }

    #[test]
    fn loads_enabled_agents_and_project_name() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "agents: [claude, codex]\nprojectName: explicit-name\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.agents, ["claude", "codex"]);
        assert_eq!(cfg.project_name, "explicit-name");
    }

    /// `agents` names harness keys, whatever is registered; the shape is
    /// checked here, the keys by the app.
    #[test]
    fn agents_are_keys_checked_for_shape() {
        let dir = tempdir().unwrap();
        let load = |yaml: &str| {
            std::fs::write(cfg_path(dir.path()), yaml).unwrap();
            load_project_config(dir.path())
        };
        assert_eq!(
            load("agents: [claude, my-agent_2]\n").unwrap().agents,
            ["claude", "my-agent_2"]
        );
        for (yaml, needle) in [
            ("agents: []\n", "at least one"),
            ("agents: [Claude]\n", "isn't an agent key"),
            ("agents: [codex, codex]\n", "duplicates"),
        ] {
            let err = load(yaml).unwrap_err().to_string();
            assert!(err.contains(needle), "{yaml}: {err}");
        }
    }

    /// The single-`agent` form is gone: `agents` is the one key.
    #[test]
    fn the_old_single_agent_key_is_refused() {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "agent: codex\n").unwrap();
        let err = load_project_config(dir.path()).unwrap_err().to_string();
        assert!(err.contains("agent"), "{err}");
    }

    #[test]
    fn rejects_empty_agents() {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "agents: []\n").unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("at least one")));
    }

    #[test]
    fn rejects_duplicate_agents() {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "agents: [claude, claude]\n").unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("duplicates")));
    }

    #[test]
    fn rejects_unknown_keys() {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "bogusKey: 1\n").unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));
    }

    #[test]
    fn rejects_empty_project_name() {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "projectName: \"   \"\n").unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("projectName")));
    }

    #[test]
    fn parses_lsp_servers() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            r#"
lsp:
  servers:
    - languageId: rust
      extensions: [.rs]
      command: rust-analyzer
      args: ["--quiet"]
"#,
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.lsp_servers.len(), 1);
        assert_eq!(cfg.lsp_servers[0].language_id, "rust");
        assert_eq!(cfg.lsp_servers[0].command, "rust-analyzer");
        assert_eq!(cfg.lsp_servers[0].args, vec!["--quiet"]);
    }

    #[test]
    fn rejects_lsp_extensions_without_dot() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            r#"
lsp:
  servers:
    - languageId: rust
      extensions: [rs]
      command: rust-analyzer
"#,
        )
        .unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("must start with '.'")));
    }

    #[test]
    fn generated_accepts_exclude_and_include_entries() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "generated:\n  exclude:\n    - target\n    - apps/desktop/dist\n  include:\n    - dist/keep.json\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(
            cfg.generated.exclude,
            vec!["target".to_string(), "apps/desktop/dist".to_string()]
        );
        assert_eq!(cfg.generated.include, vec!["dist/keep.json".to_string()]);
    }

    #[test]
    fn generated_defaults_to_empty_when_absent() {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "agents: [claude]\n").unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert!(cfg.generated.exclude.is_empty());
        assert!(cfg.generated.include.is_empty());
    }

    #[test]
    fn rejects_generated_absolute_path() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "generated:\n  exclude: [\"/etc/passwd\"]\n",
        )
        .unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("repo-relative")));
    }

    #[test]
    fn rejects_generated_include_parent_escape() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "generated:\n  include: [\"../sibling\"]\n",
        )
        .unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("..")));
    }

    #[test]
    fn write_round_trips_generated_exclude_include() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "generated:\n  exclude: [target]\n  include: [dist/keep.json]\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        write_rendered(dir.path(), &cfg);
        let reloaded = load_project_config(dir.path()).unwrap();
        assert_eq!(reloaded.generated.exclude, vec!["target".to_string()]);
        assert_eq!(
            reloaded.generated.include,
            vec!["dist/keep.json".to_string()]
        );
    }

    /// tsk251: `zones:` is an ORDERED table — first match wins — so the
    /// parsed order must be the file's order, and `match` takes either a
    /// single glob or a list of them.
    #[test]
    fn zones_parse_in_order_with_scalar_or_list_match() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "zones:\n\
             - match: [\"**/*_test.rs\", \"**/tests/**\"]\n  zone: test\n\
             - match: crates/oxplow-db/**\n  zone: store\n  color: \"#ea580c\"\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.zones.len(), 2);
        assert_eq!(cfg.zones[0].zone, "test");
        assert_eq!(
            cfg.zones[0].patterns,
            vec!["**/*_test.rs".to_string(), "**/tests/**".to_string()]
        );
        assert_eq!(cfg.zones[0].color, None);
        assert_eq!(cfg.zones[1].zone, "store");
        assert_eq!(
            cfg.zones[1].patterns,
            vec!["crates/oxplow-db/**".to_string()]
        );
        assert_eq!(cfg.zones[1].color.as_deref(), Some("#ea580c"));
    }

    #[test]
    fn zones_default_to_empty_when_absent() {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "agents: [claude]\n").unwrap();
        assert!(load_project_config(dir.path()).unwrap().zones.is_empty());
    }

    /// `other` and `external` are COMPUTED sentinels (unmatched file /
    /// out-of-repo import target). Declaring one would make a real zone
    /// indistinguishable from "we couldn't classify this".
    #[test]
    fn zones_reject_reserved_labels() {
        for label in ["other", "external"] {
            let dir = tempdir().unwrap();
            std::fs::write(
                cfg_path(dir.path()),
                format!("zones:\n- match: src/**\n  zone: {label}\n"),
            )
            .unwrap();
            let err = load_project_config(dir.path()).unwrap_err();
            assert!(
                matches!(&err, ConfigError::Invalid(msg) if msg.contains("reserved")),
                "{label} should be rejected as reserved, got {err:?}"
            );
        }
    }

    #[test]
    fn zones_reject_empty_and_unparseable_rules() {
        let cases = [
            ("zones:\n- match: src/**\n  zone: \"  \"\n", "zone"),
            ("zones:\n- match: \"\"\n  zone: ui\n", "match"),
            ("zones:\n- match: \"src/[\"\n  zone: ui\n", "glob"),
            (
                "zones:\n- match: src/**\n  zone: ui\n  color: nope\n",
                "color",
            ),
        ];
        for (yaml, needle) in cases {
            let dir = tempdir().unwrap();
            std::fs::write(cfg_path(dir.path()), yaml).unwrap();
            let err = load_project_config(dir.path()).unwrap_err();
            assert!(
                matches!(&err, ConfigError::Invalid(msg) if msg.contains(needle)),
                "expected a {needle} error for {yaml:?}, got {err:?}"
            );
        }
    }

    /// Every settings write rewrites `metrics:` and `dimensions:` from the
    /// parsed entries, so a field the writer forgets is silently dropped;
    /// an entity metric without its entity no longer validates and the
    /// whole config stops loading.
    #[test]
    fn write_round_trips_zones_preserving_order() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "zones:\n\
             - match: \"**/*_test.rs\"\n  zone: test\n\
             - match: crates/**\n  zone: backend\n  color: \"#abc\"\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        write_rendered(dir.path(), &cfg);
        let reloaded = load_project_config(dir.path()).unwrap();
        assert_eq!(reloaded.zones, cfg.zones);
        assert_eq!(reloaded.zones[0].zone, "test");
        assert_eq!(reloaded.zones[1].color.as_deref(), Some("#abc"));
    }

    #[test]
    fn fast_test_command_round_trips() {
        // tsk171: the coverage-free counterpart to `testCommand`, used for the
        // red/green loop so those runs still emit a report instead of being
        // dropped because the full instrumented run is too slow to repeat.
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "testing:\n  command: bun run test:collect\n  fastCommand: bun run test:fast\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(
            cfg.testing.fast_command.as_deref(),
            Some("bun run test:fast")
        );
        write_rendered(dir.path(), &cfg);
        let reloaded = load_project_config(dir.path()).unwrap();
        assert_eq!(
            reloaded.testing.fast_command.as_deref(),
            Some("bun run test:fast"),
            "survives a write/reload round-trip"
        );
        assert_eq!(
            reloaded.testing.command.as_deref(),
            Some("bun run test:collect"),
            "and doesn't displace the full command"
        );
    }

    #[test]
    fn managed_keys_covers_every_block_the_writer_emits() {
        // Every top-level key the renderer emits reads back the same: what
        // `keys::with_key` validates a command's change through (tsk164).
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "projectName: demo\n\
             snapshotRetentionDays: 3\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        write_rendered(dir.path(), &cfg);
        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&raw).unwrap();
        let serde_yaml::Value::Mapping(map) = doc else {
            panic!("config is a mapping");
        };
        // Every key present after a write must round-trip through a reload,
        // which is what being MANAGED buys.
        let reloaded = load_project_config(dir.path()).unwrap();
        assert_eq!(reloaded.project_name, "demo");
        assert_eq!(reloaded.snapshot_retention_days, 3);
        assert!(
            map.contains_key(serde_yaml::Value::String("projectName".into())),
            "projectName written, got:\n{raw}"
        );
    }

    #[test]
    fn agent_config_round_trips() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "agentConfig:\n  opencode:\n    model: github-copilot/gpt-5-mini\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(
            cfg.agent_config["opencode"]["model"],
            "github-copilot/gpt-5-mini"
        );
        write_rendered(dir.path(), &cfg);
        assert_eq!(
            load_project_config(dir.path()).unwrap().agent_config,
            cfg.agent_config
        );
    }

    #[test]
    fn the_team_list_and_the_personal_layer_decide_which_extensions_are_on() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "extensions: [oxplow-bundled, triage]\nextensionSources:\n  triage: tools/triage\n",
        )
        .unwrap();
        std::fs::write(
            personal_path(dir.path()),
            "enableExtensions: [mine, triage]\ndisableExtensions: [oxplow-bundled]\n\
             extensionSources:\n  mine: /abs/mine\n  triage: /abs/triage\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.extensions, vec!["oxplow-bundled", "triage"]);
        assert_eq!(cfg.personal_extensions.enable, vec!["mine", "triage"]);
        write_rendered(dir.path(), &cfg);

        let settings = extension_settings(dir.path());
        assert_eq!(
            settings.enabled(),
            vec![
                ("triage".to_string(), EnabledBy::Team),
                ("mine".to_string(), EnabledBy::Personal),
            ]
        );
        let wt = Path::new("/wt");
        // The person's source wins over the team's; a relative one is the checkout's.
        assert_eq!(settings.source(wt, "triage"), Path::new("/abs/triage"));
        assert_eq!(settings.source(wt, "mine"), Path::new("/abs/mine"));
        assert_eq!(
            settings.source(wt, "other"),
            Path::new("/wt/.oxplow/extensions/other")
        );
        let team_only = ExtensionSettings {
            personal_sources: Default::default(),
            ..settings.clone()
        };
        assert_eq!(
            team_only.source(wt, "triage"),
            Path::new("/wt/tools/triage")
        );
        // No list: the bundled extension is on; an empty one turns it off.
        assert_eq!(
            extension_settings(tempdir().unwrap().path()).enabled(),
            vec![("oxplow-bundled".to_string(), EnabledBy::Team)]
        );
        let none = tempdir().unwrap();
        std::fs::write(cfg_path(none.path()), "extensions: []\n").unwrap();
        let cfg = load_project_config(none.path()).unwrap();
        assert!(cfg.extensions.is_empty());
        write_rendered(none.path(), &cfg);
        assert!(extension_settings(none.path()).enabled().is_empty());
    }

    /// `enableBundledExtensions: false` runs oxplow's own extensions from
    /// their sources, only in a dev build; the person's layer wins.
    #[test]
    fn bundled_extensions_run_from_their_sources_only_when_a_dev_build_is_asked() {
        assert!(!bundled_from_sources(None, true));
        assert!(!bundled_from_sources(Some(true), true));
        assert!(bundled_from_sources(Some(false), true));
        assert!(!bundled_from_sources(Some(false), false), "a release build");
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "enableBundledExtensions: false\n").unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.enable_bundled_extensions, Some(false));
        write_rendered(dir.path(), &cfg);
        let settings = extension_settings(dir.path());
        assert_eq!(settings.bundled_requested, Some(false));
        assert_eq!(settings.bundled_from_sources, cfg!(debug_assertions));
        assert_eq!(ExtensionSettings::of(&cfg), settings);
        std::fs::write(personal_path(dir.path()), "enableBundledExtensions: true\n").unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.personal_enable_bundled_extensions, Some(true));
        let settings = extension_settings(dir.path());
        assert_eq!(settings.bundled_requested, Some(true));
        assert!(!settings.bundled_from_sources);
        assert_eq!(ExtensionSettings::of(&cfg), settings);
        assert!(crate::keys::HUMAN_ONLY_KEYS.contains(&"enableBundledExtensions"));
    }

    #[test]
    fn a_home_relative_source_expands() {
        let settings = ExtensionSettings {
            personal_sources: [("mine".to_string(), "~/exts/mine".to_string())].into(),
            ..Default::default()
        };
        assert_eq!(
            settings.source(Path::new("/wt"), "mine"),
            dirs::home_dir().unwrap().join("exts/mine")
        );
    }

    #[test]
    fn the_old_disabled_block_is_refused() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "extensions:\n  disabled: [oxplow-bundled]\n",
        )
        .unwrap();
        assert!(load_project_config(dir.path()).is_err());
    }

    #[test]
    fn ai_roles_round_trip_and_reject_unknown_roles() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "ai:\n  roles:\n    summarize: { provider: openrouter, model: openai/gpt-5-mini }\n    \
             acme/review: { provider: openrouter, model: big, restrict: [tools, streaming] }\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert!(
            cfg.ai_roles.contains_key("acme/review"),
            "an extension's AI profile"
        );
        assert_eq!(
            cfg.ai_roles.get("summarize"),
            Some(&AiRoleOverride {
                provider: "openrouter".into(),
                model: "openai/gpt-5-mini".into(),
                restrict: Default::default(),
            })
        );
        assert_eq!(
            cfg.ai_roles["acme/review"].restrict,
            [
                oxplow_domain::ai::AiRestriction::Tools,
                oxplow_domain::ai::AiRestriction::Streaming
            ]
            .into(),
            "what the project forbids the binding"
        );
        write_rendered(dir.path(), &cfg);
        let written = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        assert_eq!(
            written.matches("restrict").count(),
            1,
            "only a restricted role says so: {written}"
        );
        let again = load_project_config(dir.path()).unwrap();
        assert_eq!(again.ai_roles, cfg.ai_roles, "written back unchanged");

        for (yaml, needle) in [
            (
                "ai:\n  roles:\n    thinker: { provider: p, model: m }\n",
                "thinker",
            ),
            (
                "ai:\n  roles:\n    main: { provider: p, model: \"\" }\n",
                "main",
            ),
            (
                "ai:\n  roles:\n    acme/Review: { provider: p, model: m }\n",
                "acme/Review: unknown role",
            ),
            (
                "ai:\n  roles:\n    main: { provider: p, model: m, restrict: [network] }\n",
                "network",
            ),
            // A role nothing asked is gone: naming one says so.
            (
                "ai:\n  roles:\n    embed: { provider: p, model: m }\n",
                "ai.roles.embed: oxplow has no `embed` role",
            ),
        ] {
            std::fs::write(cfg_path(dir.path()), yaml).unwrap();
            let err = load_project_config(dir.path()).unwrap_err().to_string();
            assert!(err.contains(needle), "{err}");
        }
    }

    /// `agentModels` is retired: loading it says what replaced it. An
    /// `agentConfig` entry is a map under an agent key.
    #[test]
    fn agent_models_is_an_error_naming_agent_config() {
        let dir = tempdir().unwrap();
        for (yaml, needle) in [
            (
                "agentModels:\n  opencode: some/model\n",
                "agentModels is now agentConfig",
            ),
            ("agentConfig:\n  opencode: some/model\n", "map of settings"),
            ("agentConfig:\n  Bad: {}\n", "isn't an agent key"),
        ] {
            std::fs::write(cfg_path(dir.path()), yaml).unwrap();
            let err = load_project_config(dir.path()).unwrap_err().to_string();
            assert!(err.contains(needle), "{yaml}: {err}");
        }
    }

    #[test]
    fn rejects_lsp_missing_required_fields() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            r#"
lsp:
  servers:
    - languageId: rust
      command: rust-analyzer
"#,
        )
        .unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));
    }

    #[test]
    fn inject_session_context_round_trips() {
        let dir = tempdir().unwrap();
        let cfg = OxplowConfig {
            inject_session_context: false,
            ..default_config("test".into())
        };
        write_rendered(dir.path(), &cfg);
        let loaded = load_project_config(dir.path()).unwrap();
        assert!(!loaded.inject_session_context);
    }

    #[test]
    fn icon_tint_round_trips_and_defaults_to_unset() {
        let dir = tempdir().unwrap();
        assert_eq!(load_project_config(dir.path()).unwrap().icon_tint, None);

        let cfg = OxplowConfig {
            icon_tint: Some("#c2410c".into()),
            ..default_config("test".into())
        };
        write_rendered(dir.path(), &cfg);
        let loaded = load_project_config(dir.path()).unwrap();
        assert_eq!(loaded.icon_tint.as_deref(), Some("#c2410c"));
    }

    #[test]
    fn icon_tint_accepts_the_common_hex_spellings() {
        for spelling in ["#c2410c", "#C2410C", "#abc", "c2410c"] {
            let dir = tempdir().unwrap();
            std::fs::write(cfg_path(dir.path()), format!("iconTint: \"{spelling}\"\n")).unwrap();
            assert_eq!(
                load_project_config(dir.path())
                    .unwrap()
                    .icon_tint
                    .as_deref(),
                Some(spelling),
                "{spelling} should parse"
            );
        }
    }

    #[test]
    fn a_malformed_icon_tint_is_a_surfaced_config_error() {
        // Same stance as every other setting here: bad config is reported, not
        // silently dropped — a typo'd colour that quietly did nothing would be
        // read as "the feature is broken".
        for bad in ["", "#12", "#12345", "nope", "#gggggg", "rgb(1,2,3)"] {
            let dir = tempdir().unwrap();
            std::fs::write(cfg_path(dir.path()), format!("iconTint: \"{bad}\"\n")).unwrap();
            let err = load_project_config(dir.path()).unwrap_err();
            assert!(
                matches!(err, ConfigError::Invalid(ref m) if m.contains("iconTint")),
                "{bad:?} should be rejected naming the key, got {err:?}"
            );
        }
    }

    #[test]
    fn icon_tint_parses_to_rgb_channels() {
        // The shorthand expands the way CSS does, so #abc and #aabbcc agree.
        assert_eq!(parse_hex_rgb("#c2410c"), Some((0xc2, 0x41, 0x0c)));
        assert_eq!(parse_hex_rgb("c2410c"), Some((0xc2, 0x41, 0x0c)));
        assert_eq!(parse_hex_rgb("#ABC"), parse_hex_rgb("#aabbcc"));
        assert_eq!(parse_hex_rgb("#zzz"), None);
    }

    #[test]
    fn test_paths_are_globs_that_round_trip_and_a_bad_one_fails_the_load() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "testing:\n  testPaths: [\"spec/**\", \"**/it/**\"]\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.testing.test_paths, vec!["spec/**", "**/it/**"]);

        std::fs::write(cfg_path(dir.path()), "testing:\n  testPaths: [\"a{b\"]\n").unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(
            matches!(err, ConfigError::Invalid(ref m) if m.contains("testPaths[0]")),
            "{err:?}"
        );
        std::fs::write(cfg_path(dir.path()), "testing:\n  testPaths: [\" \"]\n").unwrap();
        assert!(load_project_config(dir.path()).is_err());
    }

    #[test]
    fn testing_defaults_empty_when_absent() {
        let dir = tempdir().unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.testing, TestingConfig::default());
    }

    #[test]
    fn parses_testing_block() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            r#"
testing:
  command: cargo cov
  agentHint: "Run tests with cargo cov"
  runPatterns:
    - cargo cov
    - bun test
  analysisPatterns: [lint:collect]
"#,
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.testing.command.as_deref(), Some("cargo cov"));
        assert_eq!(
            cfg.testing.agent_hint.as_deref(),
            Some("Run tests with cargo cov")
        );
        assert_eq!(cfg.testing.run_patterns, vec!["cargo cov", "bun test"]);
        assert_eq!(cfg.testing.analysis_patterns, vec!["lint:collect"]);
    }

    #[test]
    fn scripts_default_to_a_bounded_pool_and_512_mib() {
        let dir = tempdir().unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        let parallelism = std::thread::available_parallelism().map_or(1, |n| n.get());
        assert_eq!(cfg.scripts.max_concurrent, parallelism.min(4));
        assert_eq!(cfg.scripts.memory_bytes, 512 * 1024 * 1024);
        assert_eq!(cfg.scripts, ScriptsConfig::default());
    }

    #[test]
    fn scripts_block_sets_both_limits() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "scripts:\n  maxConcurrent: 2\n  memoryMb: 256\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.scripts.max_concurrent, 2);
        assert_eq!(cfg.scripts.memory_bytes, 256 * 1024 * 1024);
    }

    #[test]
    fn scripts_keys_are_independent() {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "scripts:\n  memoryMb: 128\n").unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.scripts.memory_bytes, 128 * 1024 * 1024);
        assert_eq!(
            cfg.scripts.max_concurrent,
            ScriptsConfig::default().max_concurrent
        );
    }

    #[test]
    fn scripts_minimums_are_one_slot_and_64_mib() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "scripts:\n  maxConcurrent: 1\n  memoryMb: 64\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.scripts.max_concurrent, 1);
        assert_eq!(cfg.scripts.memory_bytes, 64 * 1024 * 1024);
        for (text, key) in [
            ("scripts:\n  maxConcurrent: 0\n", "maxConcurrent"),
            ("scripts:\n  memoryMb: 63\n", "memoryMb"),
        ] {
            std::fs::write(cfg_path(dir.path()), text).unwrap();
            let err = load_project_config(dir.path()).unwrap_err().to_string();
            assert!(err.contains(key) && err.contains("scripts."), "got: {err}");
        }
    }

    #[test]
    fn scripts_bad_values_are_refused() {
        let dir = tempdir().unwrap();
        for text in [
            "scripts:\n  maxConcurrent: -1\n",
            "scripts:\n  maxConcurrent: lots\n",
            "scripts:\n  memoryMb: .nan\n",
            "scripts:\n  threads: 3\n",
        ] {
            std::fs::write(cfg_path(dir.path()), text).unwrap();
            assert!(load_project_config(dir.path()).is_err(), "accepted: {text}");
        }
    }

    #[test]
    fn scripts_round_trip_writes_only_what_differs() {
        let dir = tempdir().unwrap();
        let cfg = OxplowConfig {
            scripts: ScriptsConfig {
                max_concurrent: 3,
                memory_bytes: 128 * 1024 * 1024,
            },
            ..default_config("test".into())
        };
        write_rendered(dir.path(), &cfg);
        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        assert!(
            raw.contains("maxConcurrent: 3") && raw.contains("memoryMb: 128"),
            "{raw}"
        );
        assert_eq!(
            load_project_config(dir.path()).unwrap().scripts,
            cfg.scripts
        );

        write_rendered(dir.path(), &default_config("test".into()));
        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        assert!(!raw.contains("scripts"), "defaults are not written:\n{raw}");
    }

    /// tsk863: reports are read by collectors (`records:`) now; the old
    /// `collection:` block is a key the file doesn't have.
    #[test]
    fn a_collection_block_is_an_unknown_key() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "collection:\n  testCommand: cargo test\n",
        )
        .unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(
            matches!(&err, ConfigError::Parse(e) if e.to_string().contains("collection")),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_an_empty_run_pattern() {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "testing:\n  runPatterns: [\"  \"]\n").unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("runPatterns")));
    }

    #[test]
    fn testing_round_trips_through_write() {
        let dir = tempdir().unwrap();
        let cfg = OxplowConfig {
            testing: TestingConfig {
                command: Some("pytest".into()),
                fast_command: None,
                run_patterns: vec!["tox".into()],
                analysis_patterns: vec!["cargo clippy".into()],
                test_paths: vec!["spec/**".into()],
                agent_hint: Some("Run pytest, not bare python -m pytest".into()),
            },
            ..default_config("test".into())
        };
        write_rendered(dir.path(), &cfg);
        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        assert!(raw.contains("testing:"), "got:\n{raw}");
        let loaded = load_project_config(dir.path()).unwrap();
        assert_eq!(loaded.testing, cfg.testing);
    }

    /// Third-party keys that aren't part of oxplow's schema should
    /// survive a write. Comments still get stripped (no Rust YAML
    /// crate round-trips them), but the keys themselves persist —
    /// otherwise a sibling tool sharing .oxplow/project.yaml would lose its
    /// state every time the user touched oxplow's settings UI.
    #[test]
    fn a_key_write_preserves_unknown_top_level_keys() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "agents: [claude]\nthirdPartyTool:\n  enabled: true\n  values: [a, b]\n",
        )
        .unwrap();

        write_project_key(
            dir.path(),
            "snapshotRetentionDays",
            Some(&serde_json::json!(14)),
        )
        .unwrap();

        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        assert!(
            raw.contains("thirdPartyTool"),
            "third-party key should survive write, got:\n{raw}"
        );
        assert!(
            raw.contains("snapshotRetentionDays"),
            "managed key should still be present"
        );
    }

    fn load_from_yaml(yaml: &str) -> Result<OxplowConfig, ConfigError> {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), yaml).unwrap();
        load_project_config(dir.path())
    }

    /// Parse a `metrics:` document the way a global catalog file is
    /// validated (the layer that accepts `key:` definitions).
    fn global_metrics(yaml: &str) -> Result<Vec<MetricEntry>, String> {
        #[derive(Deserialize)]
        struct Doc {
            metrics: Option<Vec<MetricEntry>>,
        }
        let doc: Doc = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
        validate_metrics(doc.metrics, CatalogLayer::Global).map_err(|e| e.to_string())
    }

    /// The project's `metrics:` only `use:`s a metric: a `key:` definition
    /// is refused, naming where the project's metrics are defined.
    #[test]
    fn a_project_metric_definition_is_refused_but_a_use_loads() {
        let err = load_from_yaml(
            "metrics:\n  - key: acme.loc\n    sourceMeasure: acme.m\n    aggregation: count\n",
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("the project's metrics are defined in its own extension"),
            "{err}"
        );
        let cfg =
            load_from_yaml("metrics:\n  - use: acme.loc\n    target: 5\n    warnAt: 7\n").unwrap();
        assert_eq!(cfg.metrics.len(), 1);
        assert_eq!(cfg.metrics[0].use_key.as_deref(), Some("acme.loc"));
        assert_eq!(cfg.metrics[0].target, Some(5.0));
    }

    /// `collectors:`, `measures:` and `dimensions:` are an extension's, not
    /// project.yaml's: any of them fails the load.
    #[test]
    fn project_yaml_refuses_collectors_measures_and_dimensions() {
        for (key, yaml) in [
            (
                "collectors",
                "collectors:\n  - { id: acme.scan, runtime: starlark, entry: x.star, facts: [acme.todo] }\n",
            ),
            ("measures", "measures:\n  - key: acme.loc\n"),
            ("dimensions", "dimensions:\n  - key: acme.license\n"),
        ] {
            let err = load_from_yaml(yaml).unwrap_err().to_string();
            assert!(
                err.contains(&format!("`{key}:` is an extension's"))
                    && err.contains(".oxplow/extensions/project/extension.yaml"),
                "{key}: {err}"
            );
        }
    }

    /// tsk945: a dimension key has one name — its namespaced one. A metric
    /// declaring a bare one to slice or filter by is refused, saying what
    /// the key is called.
    #[test]
    fn a_metric_names_its_dimensions_by_their_namespaced_keys() {
        let metric = |extra: &str| -> Result<Vec<MetricEntry>, ConfigError> {
            let e: Vec<MetricEntry> = serde_yaml::from_str(&format!(
                "[{{key: repo.x, title: X, sourceMeasure: acme.m, aggregation: sum, {extra}}}]"
            ))
            .unwrap();
            validate_metrics(Some(e), CatalogLayer::Global)
        };
        for (extra, bare) in [
            ("sliceableDims: [language]", "language"),
            ("filter: { dimEq: [branch, main] }", "branch"),
        ] {
            let err = metric(extra).unwrap_err().to_string();
            assert!(
                err.contains(&format!("`{bare}`")) && err.contains(&format!("`oxplow.{bare}`")),
                "{err}"
            );
        }
        // `subject` is the raw-subject pseudo-dimension, not a catalog key.
        metric("sliceableDims: [subject, oxplow.language], filter: { dimEq: [acme.zone, api] }")
            .unwrap();
    }

    #[test]
    fn parses_both_metric_forms() {
        let metrics = global_metrics(
            r#"
metrics:
  - key: repo.unsafe_blocks
    title: "unsafe blocks"
    sourceMeasure: acme.ast_hit
    aggregation: sum
    direction: lower-better
    unit: count
    displayKind: findings
    sliceableDims: [acme.rule]
    filter: { dimEq: [acme.rule, unsafe_block] }
  - use: myglobal.todo_density
    target: 5
"#,
        )
        .unwrap();
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].key.as_deref(), Some("repo.unsafe_blocks"));
        assert_eq!(metrics[0].source_measure.as_deref(), Some("acme.ast_hit"));
        assert_eq!(metrics[0].aggregation.as_deref(), Some("sum"));
        assert_eq!(
            metrics[0].filter.as_ref().unwrap().dim_eq.as_deref(),
            Some(["acme.rule".to_string(), "unsafe_block".to_string()].as_slice())
        );
        assert_eq!(metrics[0].sliceable_dims, vec!["acme.rule".to_string()]);
        assert_eq!(metrics[1].use_key.as_deref(), Some("myglobal.todo_density"));
        assert_eq!(metrics[1].target, Some(5.0));
    }

    #[test]
    fn parses_formula_metric() {
        let metrics = global_metrics(
            r#"
metrics:
  - key: acme.bugs_per_kloc
    title: "bugs per KLOC"
    formula: { op: div, left: acme.bug_count, right: acme.kloc }
"#,
        )
        .unwrap();
        let f = metrics[0].formula.as_ref().unwrap();
        assert_eq!(f.op, "div");
        assert_eq!(f.left, "acme.bug_count");
        assert_eq!(f.right, "acme.kloc");
        assert!(metrics[0].source_measure.is_none());
    }

    #[test]
    fn formula_op_is_one_of_add_sub_mul_div() {
        for op in ["add", "sub", "mul", "div"] {
            assert!(
                global_metrics(&format!(
                    "metrics:\n  - key: a.b\n    formula: {{ op: {op}, left: a.c, right: a.d }}\n"
                ))
                .is_ok(),
                "{op}"
            );
        }
        // `div` is the only ratio primitive; there is no alias for it.
        assert!(global_metrics(
            "metrics:\n  - key: a.b\n    formula: { op: ratio, left: a.c, right: a.d }\n"
        )
        .is_err());
    }

    #[test]
    fn metric_validation_rejects_bad_entries() {
        // Reserved namespace for a definition.
        assert!(
            global_metrics("metrics:\n  - key: oxplow.foo\n    sourceMeasure: acme.m\n").is_err()
        );
        // `key:` with neither sourceMeasure nor formula.
        assert!(global_metrics("metrics:\n  - key: acme.foo\n    displayKind: gauge\n").is_err());
        // `key:` with BOTH sourceMeasure and formula.
        assert!(global_metrics(
            "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n    formula: { op: div, left: a.c, right: a.d }\n"
        )
        .is_err());
        // `use:` carrying a structural field (sourceMeasure).
        assert!(
            global_metrics("metrics:\n  - use: acme.foo\n    sourceMeasure: acme.m\n").is_err()
        );
        // both use and key.
        assert!(global_metrics("metrics:\n  - use: a.b\n    key: c.d\n").is_err());
        // un-namespaced key.
        assert!(global_metrics("metrics:\n  - key: foo\n    sourceMeasure: acme.m\n").is_err());
        // bad aggregation.
        assert!(global_metrics(
            "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n    aggregation: median\n"
        )
        .is_err());
        // bad displayKind.
        assert!(global_metrics(
            "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n    displayKind: sparkline\n"
        )
        .is_err());
        // a full valid spec is accepted.
        assert!(global_metrics(
            "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n    aggregation: count\n    displayKind: findings\n    category: static-quality\n"
        )
        .is_ok());
        // every category the built-in producer specs emit must round-trip: the app
        // WRITES those categories back into project.yaml, so a category the
        // validator rejects is a config the app can't re-read (it panicked at boot
        // on `coverage` — the built-in coverage specs' own category).
        for cat in METRIC_CATEGORIES {
            assert!(
                global_metrics(&format!(
                    "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n    category: {cat}\n"
                ))
                .is_ok(),
                "category {cat} should validate"
            );
        }
        assert!(global_metrics(
            "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n    category: coverage\n"
        )
        .is_ok());
        // the same key declared twice (a `key:` define + a `use:`).
        assert!(global_metrics(
            "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n  - use: a.b\n    target: 5\n"
        )
        .is_err());
        // the same key defined twice.
        assert!(global_metrics(
            "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n  - key: a.b\n    sourceMeasure: acme.n\n"
        )
        .is_err());
    }

    fn define(key: &str, target: Option<f64>) -> MetricEntry {
        MetricEntry {
            key: Some(key.into()),
            source_measure: Some("acme.m".into()),
            aggregation: Some("count".into()),
            target,
            ..Default::default()
        }
    }

    #[test]
    fn resolve_precedence_project_over_global_over_builtin() {
        let builtin = vec![define("oxplow.unsafe", Some(0.0))];
        let global = vec![define("oxplow.unsafe", Some(3.0))];
        // Project `use:`s the catalog key and overrides the target.
        let project = vec![MetricEntry {
            use_key: Some("oxplow.unsafe".into()),
            target: Some(7.0),
            ..Default::default()
        }];
        let resolved = resolve_metrics(&builtin, &global, &[], &project);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].key, "oxplow.unsafe");
        // The definition resolves at global scope (global > built-in), but the
        // project's `use:` override wins for the target.
        assert_eq!(resolved[0].scope, "global");
        assert_eq!(resolved[0].target, Some(7.0));
        // The measure comes from the (global) definition.
        assert_eq!(resolved[0].source_measure.as_deref(), Some("acme.m"));
    }

    #[test]
    fn resolve_extension_definition_is_active_and_scoped() {
        let extensions = vec![("acme".to_string(), vec![define("acme.loc", None)])];
        let resolved = resolve_metrics(&[], &[], &extensions, &[]);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].scope, "extension:acme");
        assert_eq!(resolved[0].display_kind, "gauge");
        assert_eq!(resolved[0].aggregation, "count");
    }

    #[test]
    fn resolve_carries_description_from_definition_not_override() {
        // A built-in/global definition declares the description; a `use:` entry's
        // own description is ignored (description is inherent, like trigger).
        let mut def = define("acme.loc", None);
        def.description = Some("Lines of code in the repo.".into());
        let global = vec![def];
        let project = vec![MetricEntry {
            use_key: Some("acme.loc".into()),
            description: Some("a project override that should be ignored".into()),
            ..Default::default()
        }];
        let resolved = resolve_metrics(&[], &global, &[], &project);
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].description.as_deref(),
            Some("Lines of code in the repo.")
        );
    }

    #[test]
    fn resolve_skips_unknown_use_key() {
        let project = vec![MetricEntry {
            use_key: Some("nope.missing".into()),
            ..Default::default()
        }];
        assert!(resolve_metrics(&[], &[], &[], &project).is_empty());
    }

    /// tsk1076: resolution runs on every event, so it reports nothing; the
    /// config's unknown `use:` keys are named once per load by
    /// `unknown_uses`, a disable marker for an extension key aside.
    #[test]
    fn unknown_uses_names_the_typos() {
        let builtin = vec![define("oxplow.unsafe", Some(0.0))];
        let project = vec![
            MetricEntry {
                use_key: Some("oxplow.unsafe".into()),
                ..Default::default()
            },
            MetricEntry {
                use_key: Some("nope.missing".into()),
                ..Default::default()
            },
            MetricEntry {
                use_key: Some("agent.tokens.total".into()),
                enabled: Some(false),
                ..Default::default()
            },
        ];
        assert_eq!(
            unknown_uses(&builtin, &[], &[], &project),
            vec!["nope.missing".to_string()]
        );
    }

    #[test]
    fn resolve_defaults_enabled_true() {
        let extensions = vec![("acme".to_string(), vec![define("acme.loc", None)])];
        let resolved = resolve_metrics(&[], &[], &extensions, &[]);
        assert!(resolved[0].enabled, "a bare definition is active");
    }

    #[test]
    fn resolve_marks_disabled_use_entry() {
        // A project `use:` disable marker over a known catalog def resolves with
        // `enabled: false` (NOT dropped — the Catalog still lists it, and
        // seed_catalog needs it to know to prune).
        let builtin = vec![define("oxplow.unsafe", Some(0.0))];
        let project = vec![MetricEntry {
            use_key: Some("oxplow.unsafe".into()),
            enabled: Some(false),
            ..Default::default()
        }];
        let resolved = resolve_metrics(&builtin, &[], &[], &project);
        assert_eq!(resolved.len(), 1);
        assert!(!resolved[0].enabled);
    }

    #[test]
    fn an_off_by_default_extension_metric_is_listed_inactive_until_used() {
        let mut def = define("acme.loc", None);
        def.default = Some(MetricDefault::Off);
        def.collector = Some("acme.scan".into());
        let extensions = vec![("acme".to_string(), vec![def])];
        let resolved = resolve_metrics(&[], &[], &extensions, &[]);
        assert_eq!(resolved.len(), 1);
        assert!(!resolved[0].enabled);
        assert_eq!(resolved[0].collector.as_deref(), Some("acme.scan"));
        let used = vec![MetricEntry {
            use_key: Some("acme.loc".into()),
            ..Default::default()
        }];
        let resolved = resolve_metrics(&[], &[], &extensions, &used);
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].enabled);
        assert_eq!(resolved[0].scope, "extension:acme");
    }

    #[test]
    fn default_and_collector_are_an_extension_definitions() {
        let mut def = define("acme.loc", None);
        def.default = Some(MetricDefault::Off);
        let err = validate_metrics(Some(vec![def.clone()]), CatalogLayer::Global)
            .unwrap_err()
            .to_string();
        assert!(err.contains("extension's metric definition"), "{err}");
        assert!(validate_metrics(Some(vec![def]), CatalogLayer::Extension("acme")).is_ok());
        let mut def = define("acme.loc", None);
        def.enabled = Some(false);
        let err = validate_metrics(Some(vec![def]), CatalogLayer::Extension("acme"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("`default: off`"), "{err}");
    }

    #[test]
    fn oxplow_keys_are_the_oxplow_namespaces() {
        let measure = || MeasureEntry {
            key: Some("oxplow.loc".into()),
            ..Default::default()
        };
        for layer in [CatalogLayer::Global, CatalogLayer::Extension("acme")] {
            let err = validate_measures(Some(vec![measure()]), layer)
                .unwrap_err()
                .to_string();
            assert!(err.contains("reserved"), "{err}");
            assert!(validate_metrics(Some(vec![define("oxplow.loc", None)]), layer).is_err());
        }
        assert!(validate_measures(
            Some(vec![measure()]),
            CatalogLayer::Extension(OXPLOW_NAMESPACE)
        )
        .is_ok());
        assert!(validate_metrics(
            Some(vec![define("oxplow.loc", None)]),
            CatalogLayer::Extension(OXPLOW_NAMESPACE)
        )
        .is_ok());
    }

    #[test]
    fn resolve_marks_disabled_extension_metric() {
        // The project turning an extension's metric off keeps its definition
        // but flags it off.
        let extensions = vec![("acme".to_string(), vec![define("acme.loc", None)])];
        let off = vec![MetricEntry {
            use_key: Some("acme.loc".into()),
            enabled: Some(false),
            ..Default::default()
        }];
        let resolved = resolve_metrics(&[], &[], &extensions, &off);
        assert_eq!(resolved.len(), 1);
        assert!(!resolved[0].enabled);
    }

    #[test]
    fn resolve_disable_marker_for_unknown_key_is_skipped_quietly() {
        // A disable marker for a producer/extension key (not a resolve-catalog def)
        // is skipped without a warning — seed_catalog prunes it from config state.
        let project = vec![MetricEntry {
            use_key: Some("agent.tokens.total".into()),
            enabled: Some(false),
            ..Default::default()
        }];
        assert!(resolve_metrics(&[], &[], &[], &project).is_empty());
    }

    #[test]
    fn disabled_marker_round_trips_through_write() {
        let dir = tempdir().unwrap();
        let cfg = OxplowConfig {
            metrics: vec![MetricEntry {
                use_key: Some("agent.tokens.total".into()),
                enabled: Some(false),
                ..Default::default()
            }],
            ..default_config("test".into())
        };
        write_rendered(dir.path(), &cfg);
        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        assert!(raw.contains("enabled: false"), "got:\n{raw}");
        let loaded = load_project_config(dir.path()).unwrap();
        assert_eq!(loaded.metrics, cfg.metrics);
    }

    #[test]
    fn metrics_round_trip_through_write() {
        let dir = tempdir().unwrap();
        let cfg = OxplowConfig {
            metrics: vec![MetricEntry {
                use_key: Some("acme.loc".into()),
                target: Some(2.0),
                ..Default::default()
            }],
            ..default_config("test".into())
        };
        write_rendered(dir.path(), &cfg);
        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        assert!(raw.contains("metrics:"), "got:\n{raw}");
        // No null fields written for unset options.
        assert!(!raw.contains("null"), "minimal write, got:\n{raw}");
        let loaded = load_project_config(dir.path()).unwrap();
        assert_eq!(loaded.metrics, cfg.metrics);
    }

    #[test]
    fn loads_global_metric_entries_from_dir() {
        let dir = tempdir().unwrap();
        let metrics_dir = dir.path().join("metrics");
        std::fs::create_dir_all(&metrics_dir).unwrap();
        std::fs::write(
            metrics_dir.join("a.yaml"),
            "metrics:\n  - key: myglobal.todo\n    sourceMeasure: myglobal.m\n    aggregation: count\n",
        )
        .unwrap();
        // A malformed file is skipped, not fatal.
        std::fs::write(metrics_dir.join("bad.yaml"), "metrics:\n  - key: nodot\n").unwrap();
        let entries = load_global_metric_entries(dir.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key.as_deref(), Some("myglobal.todo"));
    }

    // --- collectors (the fact producers, P7.B3) ------------------------------

    /// This repo's own `.oxplow/project.yaml` loads.
    #[test]
    fn this_repos_project_config_loads() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        load_project_config(&root).expect("the repo's project.yaml loads");
    }

    /// tsk865: `gauges:` is a key the file doesn't have (they're
    /// `collectors:`); nothing special-cases it.
    #[test]
    fn a_gauges_block_is_an_unknown_key() {
        let err = load_from_yaml("gauges:\n  - key: acme.foo\n").unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)), "{err:?}");
    }

    // --- measures + dimensions (workstream E) ------------------------------

    fn measure_entries(yaml: &str) -> Vec<MeasureEntry> {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn dimension_entries(yaml: &str) -> Vec<DimensionEntry> {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn parses_measures_and_dimensions_definitions() {
        let measures = validate_measures(
            Some(measure_entries(
                r#"
- key: acme.api_latency
  title: "API latency"
  unit: ms
  subjectKind: endpoint
  temporalSemantics: non-additive
  description: "p95 request latency"
"#,
            )),
            CatalogLayer::Extension("acme"),
        )
        .unwrap();
        assert_eq!(measures.len(), 1);
        let m = &measures[0];
        assert_eq!(m.key.as_deref(), Some("acme.api_latency"));
        assert_eq!(m.unit.as_deref(), Some("ms"));
        assert_eq!(m.subject_kind.as_deref(), Some("endpoint"));
        assert_eq!(m.temporal_semantics.as_deref(), Some("non-additive"));

        let dimensions = validate_dimensions(
            Some(dimension_entries(
                r#"
- key: acme.license
  label: License
  valueType: categorical
  vocabulary: [MIT, Apache-2.0, GPL-3.0]
- key: acme.endpoint
  valueType: entity-ref
  subjectKind: endpoint
  promote: true
"#,
            )),
            CatalogLayer::Extension("acme"),
        )
        .unwrap();
        assert_eq!(dimensions.len(), 2);
        assert_eq!(dimensions[0].key.as_deref(), Some("acme.license"));
        assert_eq!(
            dimensions[0].vocabulary,
            vec!["MIT", "Apache-2.0", "GPL-3.0"]
        );
        assert!(!dimensions[0].promote);
        assert_eq!(dimensions[1].value_type.as_deref(), Some("entity-ref"));
        assert!(dimensions[1].promote);
    }

    #[test]
    fn measure_validation_rejects_bad_entries() {
        let check =
            |yaml: &str| validate_measures(Some(measure_entries(yaml)), CatalogLayer::Global);
        // Reserved namespace.
        assert!(check("[{key: oxplow.foo}]").is_err());
        // Un-namespaced key.
        assert!(check("[{key: foo}]").is_err());
        // Missing key.
        assert!(check("[{title: nokey}]").is_err());
        // Bad temporalSemantics.
        assert!(check("[{key: acme.x, temporalSemantics: sideways}]").is_err());
        // Bad captureScope (tsk41).
        assert!(check("[{key: acme.x, captureScope: sometimes}]").is_err());
        // `per-path` is the tree-gauge scope and must be accepted.
        let valid = check("[{key: acme.x, captureScope: per-path}]").unwrap();
        assert_eq!(valid[0].capture_scope.as_deref(), Some("per-path"));
        let resolved = resolve_measures(&valid, &[]);
        assert_eq!(resolved[0].capture_scope, "per-path");
        // Default is `complete` — a capture restates the whole population.
        let plain = check("[{key: acme.y}]").unwrap();
        assert_eq!(resolve_measures(&plain, &[])[0].capture_scope, "complete");
        // Duplicate key.
        assert!(check("[{key: acme.x}, {key: acme.x}]").is_err());
        // componentRole is gone (tsk865): ratio components ride each fact.
        assert!(
            serde_yaml::from_str::<Vec<MeasureEntry>>("[{key: acme.x, componentRole: none}]")
                .is_err()
        );
    }

    #[test]
    fn dimension_validation_rejects_bad_entries() {
        let check =
            |yaml: &str| validate_dimensions(Some(dimension_entries(yaml)), CatalogLayer::Global);
        // Reserved namespace.
        assert!(check("[{key: oxplow.foo}]").is_err());
        // Un-namespaced key.
        assert!(check("[{key: foo}]").is_err());
        // Bad valueType.
        assert!(check("[{key: acme.x, valueType: blob}]").is_err());
        // Duplicate key.
        assert!(check("[{key: acme.x}, {key: acme.x}]").is_err());
        // A minimal valid dimension parses (defaults to categorical).
        assert!(check("[{key: acme.x}]").is_ok());
    }

    #[test]
    fn resolve_measures_precedence_and_defaults() {
        let global = vec![MeasureEntry {
            key: Some("acme.loc".into()),
            title: Some("Global LOC".into()),
            ..Default::default()
        }];
        // An extension redefines the same key AND adds a fresh one.
        let extensions = vec![(
            "acme".to_string(),
            vec![
                MeasureEntry {
                    key: Some("acme.loc".into()),
                    title: Some("Extension LOC".into()),
                    temporal_semantics: Some("additive".into()),
                    ..Default::default()
                },
                MeasureEntry {
                    key: Some("acme.churn".into()),
                    ..Default::default()
                },
            ],
        )];
        let resolved = resolve_measures(&global, &extensions);
        assert_eq!(resolved.len(), 2, "same key merges, distinct key adds");
        let loc = resolved.iter().find(|m| m.key == "acme.loc").unwrap();
        assert_eq!(loc.title, "Extension LOC", "extension wins over global");
        assert_eq!(loc.scope, "extension:acme");
        assert_eq!(loc.temporal_semantics, "additive");
        let churn = resolved.iter().find(|m| m.key == "acme.churn").unwrap();
        // Defaults applied.
        assert_eq!(churn.title, "acme.churn");
        assert_eq!(churn.temporal_semantics, "semi-additive");
    }

    #[test]
    fn resolve_dimensions_precedence_and_defaults() {
        let global = vec![DimensionEntry {
            key: Some("acme.license".into()),
            label: Some("Global label".into()),
            promote: false,
            ..Default::default()
        }];
        let extensions = vec![(
            "acme".to_string(),
            vec![DimensionEntry {
                key: Some("acme.license".into()),
                label: Some("License".into()),
                promote: true,
                ..Default::default()
            }],
        )];
        let resolved = resolve_dimensions(&global, &extensions);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].label, "License", "extension wins");
        assert_eq!(resolved[0].scope, "extension:acme");
        assert_eq!(resolved[0].value_type, "categorical", "default valueType");
        assert!(resolved[0].promote);
    }

    #[test]
    fn loads_global_measure_and_dimension_entries_from_dir() {
        let dir = tempdir().unwrap();
        let measures_dir = dir.path().join("measures");
        let dims_dir = dir.path().join("dimensions");
        std::fs::create_dir_all(&measures_dir).unwrap();
        std::fs::create_dir_all(&dims_dir).unwrap();
        std::fs::write(
            measures_dir.join("a.yaml"),
            "measures:\n  - key: myglobal.loc\n    unit: lines\n",
        )
        .unwrap();
        // A malformed file is skipped, not fatal.
        std::fs::write(measures_dir.join("bad.yaml"), "measures:\n  - key: nodot\n").unwrap();
        std::fs::write(
            dims_dir.join("d.yaml"),
            "dimensions:\n  - key: myglobal.license\n    label: License\n",
        )
        .unwrap();

        let measures = load_global_measure_entries(dir.path());
        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].key.as_deref(), Some("myglobal.loc"));
        let dims = load_global_dimension_entries(dir.path());
        assert_eq!(dims.len(), 1);
        assert_eq!(dims[0].key.as_deref(), Some("myglobal.license"));
    }
}
