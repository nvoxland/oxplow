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

pub use oxplow_domain::AgentKind;

pub mod recent;
pub use recent::{RecentProject, RecentProjects};

pub mod session;
pub use session::SessionProjects;

/// Project-relative state directory holding oxplow's per-project config
/// and local data (DB, snapshots, wiki, …).
pub const OXPLOW_STATE_DIR: &str = ".oxplow";

/// Config file name, inside [`OXPLOW_STATE_DIR`]
/// (`<project>/.oxplow/project.yaml`).
pub const OXPLOW_CONFIG_FILE: &str = "project.yaml";

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

/// Global app-config dir (`<platform config dir>/net.voxland.oxplow`),
/// where launcher-level state like `recent-projects.json` and
/// `session.json` live. Matches Tauri's `app_config_dir()` on macOS /
/// Linux / Windows. `None` only if the platform config dir is
/// undiscoverable.
pub fn global_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_IDENTIFIER))
}

const DEFAULT_SNAPSHOT_RETENTION_DAYS: u32 = 7;
const DEFAULT_SNAPSHOT_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const DEFAULT_INJECT_SESSION_CONTEXT: bool = true;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct LspServerConfig {
    #[serde(rename = "languageId")]
    pub language_id: String,
    pub extensions: Vec<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// One test/coverage report the project's test run emits. `format` selects
/// the parser (collector): the built-ins are `lcov` | `cobertura` |
/// `jacoco-xml` (coverage) and `junit` (test results), plus any format a
/// project plugin (see [`PluginConfig`]) registers. The format name is no
/// longer gate-kept here — it's resolved against the collector registry at
/// collection time, so an unknown format surfaces as a warning rather than a
/// config load failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ReportConfig {
    pub path: String,
    pub format: String,
}

/// A project-defined collection plugin — the generic, kind-agnostic
/// definition mechanism. Mirrors `oxplow_collect_plugin::CollectorDescriptor`
/// but with plain-string `kind`/`runtime` so this crate stays dependency-light
/// (the collection layer maps it to a registered collector). `entry` is the
/// jaq/Starlark script (or the program for `exec`); `args` are extra exec
/// arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PluginConfig {
    pub name: String,
    /// What the plugin observes: `coverage` | `test`.
    pub kind: String,
    /// Format name(s) this plugin claims (resolved against `reports[].format`).
    pub formats: Vec<String>,
    /// Transform tier: `jaq` | `starlark` | `exec`.
    pub runtime: String,
    /// How the host pre-parses the report before the transform:
    /// `text` | `json` | `xml` | `lcov` | `lines` (default `text`). Applies to
    /// the in-process tiers (jaq/starlark); `exec` always gets raw content.
    // No `skip_serializing_if`: specta's unified-mode TS export forbids it.
    #[serde(default)]
    pub input: Option<String>,
    /// Project-relative path to the script file: the jaq/Starlark program, or
    /// the program to spawn for `exec`. Scripts live in their own files, not
    /// inline in `.oxplow/project.yaml`. Required for all three runtimes.
    #[serde(rename = "entryFile", default)]
    pub entry_file: Option<String>,
    /// Extra arguments for the `exec` runtime.
    #[serde(default)]
    pub args: Vec<String>,
}

/// How a gauge produces facts (the `compute:` block on a `gauges:` entry).
/// Mirrors [`PluginConfig`]'s runtime fields — the gauge runner maps it to a
/// registered collector. `report` is the report path for a report-derived
/// gauge; tree-derived gauges read the snapshot via `files()` instead and leave
/// it unset. (Renamed from `MetricComputeConfig` in epic tsk12, E: compute is a
/// property of the *gauge* that emits facts, not the *metric* that reads them.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
pub struct GaugeComputeConfig {
    /// Transform tier: `jaq` | `starlark` | `exec`.
    pub runtime: String,
    /// Host pre-parse for a report-derived gauge: `text` | `json` | `xml` |
    /// `lcov` | `lines` (default `text`). Ignored by `exec`.
    #[serde(default)]
    pub input: Option<String>,
    /// Project-relative path to the script file (jaq/Starlark program or the
    /// `exec` program). Required.
    #[serde(rename = "entryFile", default)]
    pub entry_file: Option<String>,
    /// Extra arguments for the `exec` runtime.
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional project-relative report path for a report-derived gauge.
    #[serde(default)]
    pub report: Option<String>,
}

/// A fact predicate on a `metrics:` spec (the `filter:` block) — the config
/// mirror of the engine's `FactFilter` (epic tsk12). A conjunctive predicate
/// keeping only the facts that match before aggregation: `minValue` for a
/// count-over-threshold (complexity ≥ N), `severity` for a lint slice, `dimEq`
/// for a conformed-dimension slice (`[oxplow.rule, unsafe_block]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default)]
#[serde(deny_unknown_fields)]
pub struct FilterConfig {
    /// Keep facts with `value >= minValue`.
    #[serde(rename = "minValue", default)]
    pub min_value: Option<f64>,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default)]
#[serde(deny_unknown_fields)]
pub struct FormulaConfig {
    /// `add` | `sub` | `mul` | `div` (`ratio` aliases `div`).
    pub op: String,
    /// The left operand metric key.
    pub left: String,
    /// The right operand metric key.
    pub right: String,
}

/// One entry in the top-level `metrics:` block — a **pure read-time SPEC** over a
/// measure (epic tsk12, E). A metric no longer *computes* anything: it names a
/// `sourceMeasure` + an `aggregation` (+ optional `filter`), or a `formula` over
/// other metrics, and the engine aggregates the durable facts a `gauges:` entry
/// emitted. Two forms, distinguished by which key is set:
/// - **`use:`** — enable an existing catalog metric by key (built-in/global),
///   optionally overriding `target`/thresholds for this project.
/// - **`key:`** — define a NEW spec (`sourceMeasure` + `aggregation`, or `formula`).
///
/// Resolved across the three scopes into [`ResolvedSpec`]s. All non-discriminant
/// fields are optional so both forms share one struct; validation enforces the
/// per-form rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default)]
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
    /// default-ON metric (producer/plugin) or a config-defined metric can be
    /// turned off without deleting its definition. Not a structural field, so a
    /// `use:` entry may carry it (unlike measure/aggregation/filter/formula).
    #[serde(default)]
    pub enabled: Option<bool>,
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
    /// Catalog grouping: `operational` | `testing` | `static-quality` | `custom`.
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
    /// `built-in` | `global` | `project`.
    pub scope: String,
    /// Whether this metric is active. Derived from the config entry's `enabled`
    /// flag (default `true`). A disabled spec is still resolved (so the Catalog
    /// can list it as an unchecked toggle), but `seed_catalog` prunes it from the
    /// `metric_spec` table so all spec-driven reads + producer collection stop.
    pub enabled: bool,
}

/// One entry in the top-level `gauges:` block — a **fact PRODUCER** (epic tsk12,
/// E). A gauge runs its `compute:` collector on its `trigger`, emitting atomic
/// facts on the measures it declares in `emits`. Unlike a metric there is no
/// `use:`/`key:` split — a gauge is always a definition (you declare the
/// producer; a project doesn't "enable" one). Resolved across global+project by
/// [`resolve_gauges`]; built-in gauges live in code, not config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default)]
#[serde(deny_unknown_fields)]
pub struct GaugeEntry {
    /// The gauge's namespaced key (`<vendor>.<id>`). Required.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// `on-report` | `on-snapshot` | `on-effort-complete` | `manual` |
    /// `continuous` (default `on-snapshot`).
    #[serde(default)]
    pub trigger: Option<String>,
    /// The measure keys this gauge is allowed to emit facts on (declare-to-collect:
    /// a fact on a measure outside this list is dropped). At least one required.
    #[serde(default)]
    pub emits: Vec<String>,
    /// How the gauge produces facts (required).
    #[serde(default)]
    pub compute: Option<GaugeComputeConfig>,
}

/// A fully-resolved gauge — the flat form the runner executes. Produced by
/// [`resolve_gauges`] (project > global). Not serialized.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedGauge {
    pub key: String,
    pub title: String,
    pub trigger: String,
    pub emits: Vec<String>,
    pub compute: GaugeComputeConfig,
    /// `built-in` | `global` | `project`.
    pub scope: String,
}

/// One entry in the top-level `measures:` block — the **measure catalog**
/// authoring surface (epic tsk12, workstream E). A measure is a *type of atomic
/// fact* a collector may emit (`oxplow.complexity`, `acme.api_latency`, …); the
/// `oxplow.*` built-ins are seeded by the DB migration, so config only *adds*
/// global/project measures. Unlike [`MetricEntry`] there is no `use:`/`key:`
/// split — a measure entry is always a definition (you declare the fact type,
/// you don't "enable" one). Resolved across the global+project scopes by
/// [`resolve_measures`] and seeded into the `measure` table at boot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default)]
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
    /// paths in its snapshot — which is what a **tree gauge over a per-commit delta**
    /// does. Such a measure is folded to the latest capture per (producer, path)
    /// before aggregating, so a repo-wide total stays correct while only changed
    /// files are rescanned. Set this on any measure a snapshot-triggered gauge
    /// emits per-file facts on (tsk41).
    #[serde(rename = "captureScope", default)]
    pub capture_scope: Option<String>,
    /// `none` | `numerator` | `denominator` — ratio-base role (default `none`).
    /// **Reserved / currently inert** (tsk15): still parsed + validated for
    /// back-compat (`deny_unknown_fields`), but no longer persisted — the
    /// `measure.component_role` column is dead (ratio components ride per-fact
    /// num/den). Kept as an authoring surface for a future component-role join.
    #[serde(rename = "componentRole", default)]
    pub component_role: Option<String>,
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
    pub component_role: String,
    /// `global` | `project` (built-ins are the migration seed, not config).
    pub scope: String,
    pub description: Option<String>,
}

/// One entry in the top-level `dimensions:` block — the **conformed-dimension
/// catalog** authoring surface (epic tsk12, workstream E). A dimension is a
/// slice axis that means the same thing to every fact that carries it
/// (`oxplow.severity`, `acme.license`, …), enabling cross-metric drill-across.
/// Like [`MeasureEntry`] it is definition-only; the `oxplow.*` built-ins are the
/// migration seed. Resolved by [`resolve_dimensions`] and seeded into the
/// `dimension` table at boot; `promote` requests a generated column + index
/// (catalog teeth).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default)]
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
}

/// Per-project collection profile (the `collection:` block). Written by
/// `/oxplow:configure` and read by the collection subsystem
/// (`.context/collection.md`): the Bash-hook detector reads
/// `test_run_patterns`, and the ride-along parses every `reports` entry
/// fresher than the effort start. A repo with several test stacks lists
/// each stack's report(s) here. All fields optional — an unconfigured
/// project collects nothing extra.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
pub struct CollectionConfig {
    /// Command that runs the project's tests (informational; surfaced to
    /// the agent so it knows how to produce the reports).
    #[serde(rename = "testCommand")]
    pub test_command: Option<String>,
    /// Reports the test run emits — coverage (lcov/cobertura/jacoco-xml)
    /// and/or test results (junit). oxplow parses each that is fresher
    /// than the effort start, so several stacks coexist.
    pub reports: Vec<ReportConfig>,
    /// Extra command substrings that count as a test run, on top of the
    /// built-in defaults (pytest, cargo test, jest, …).
    #[serde(rename = "testRunPatterns")]
    pub test_run_patterns: Vec<String>,
    /// Extra command substrings that count as a static-analysis run, on top
    /// of the built-in defaults (cargo clippy, eslint, ruff, …). Mirrors
    /// `test_run_patterns` for the analysis ride-along.
    #[serde(rename = "analysisRunPatterns")]
    pub analysis_run_patterns: Vec<String>,
    /// Free-form hint injected verbatim into every agent system prompt.
    /// Use it to tell the agent which test command to run, what coverage
    /// threshold to meet, etc. — anything project-specific the agent
    /// should know about the collection setup.
    #[serde(rename = "agentHint")]
    pub agent_hint: Option<String>,
    /// Project-defined collection plugins (jaq/starlark/exec parsers). Each
    /// registers the formats it claims, so a project can add support for a new
    /// report format without any change to oxplow itself.
    #[serde(default)]
    pub plugins: Vec<PluginConfig>,
}

impl CollectionConfig {
    /// Coverage reports (lcov / cobertura / jacoco-xml).
    pub fn coverage_reports(&self) -> impl Iterator<Item = &ReportConfig> {
        self.reports
            .iter()
            .filter(|r| !is_test_report_format(&r.format))
    }
    /// Test-result reports (junit).
    pub fn test_reports(&self) -> impl Iterator<Item = &ReportConfig> {
        self.reports
            .iter()
            .filter(|r| is_test_report_format(&r.format))
    }
}

/// `junit` is a test-result format; everything else known is coverage.
pub fn is_test_report_format(format: &str) -> bool {
    format.eq_ignore_ascii_case("junit")
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct OxplowConfig {
    /// Enabled agent implementations for this project, in priority order.
    /// The first entry is the default for newly-created threads.
    pub agents: Vec<AgentKind>,
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
    pub snapshot_max_file_bytes: u64,
    /// When true, the UserPromptSubmit hook injects a session-context
    /// block into every agent prompt.
    #[serde(rename = "injectSessionContext")]
    pub inject_session_context: bool,
    /// Per-project collection profile (test + coverage instrumentation).
    pub collection: CollectionConfig,
    /// Project-declared metric SPECS (the `metrics:` block) — the author-able
    /// read surface (epic tsk12, E). Each entry enables a catalog metric
    /// (`use:`) or defines a new spec (`key:`) over a measure. The runner resolves
    /// these across the built-in/global/project scopes; see [`resolve_metrics`].
    #[serde(default)]
    pub metrics: Vec<MetricEntry>,
    /// Project-declared gauges (the `gauges:` block) — the fact PRODUCERS (epic
    /// tsk12, E). Each runs its `compute:` collector on its trigger and emits
    /// facts on the measures it `emits`. Resolved by [`resolve_gauges`].
    #[serde(default)]
    pub gauges: Vec<GaugeEntry>,
    /// Project-declared measures (the `measures:` block) — custom fact TYPES a
    /// collector may emit (epic tsk12, workstream E). The `oxplow.*` built-ins
    /// are seeded by the DB migration; these add global/project ones. Resolved
    /// by [`resolve_measures`] and seeded into the `measure` catalog at boot.
    #[serde(default)]
    pub measures: Vec<MeasureEntry>,
    /// Project-declared dimensions (the `dimensions:` block) — custom conformed
    /// slice axes (epic tsk12, workstream E). Resolved by [`resolve_dimensions`]
    /// and seeded into the `dimension` catalog at boot.
    #[serde(default)]
    pub dimensions: Vec<DimensionEntry>,
    /// Per-agent launch model overrides, e.g.
    /// `agentModels: { opencode: "github-copilot/gpt-5-mini" }`.
    /// Only opencode consumes this today (its `-m provider/model`
    /// flag); claude/codex launch with their own defaults. Absent
    /// entries fall back to the built-in constant.
    #[serde(rename = "agentModels")]
    pub agent_models: std::collections::BTreeMap<AgentKind, String>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error(".oxplow/project.yaml parse error: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error(".oxplow/project.yaml validation: {0}")]
    Invalid(String),
}

/// Raw `generated:` block — `{ exclude: [...], include: [...] }`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGenerated {
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    include: Vec<String>,
}

/// Internal raw shape, used to validate before promoting to
/// `OxplowConfig`. Mirrors the TS `ParsedOxplowConfig` interface.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    agents: Option<Vec<AgentKind>>,
    #[serde(default)]
    agent: Option<AgentKind>,
    #[serde(rename = "projectName", default)]
    project_name: Option<String>,
    #[serde(default)]
    lsp: Option<RawLspBlock>,
    #[serde(rename = "agentPromptAppend", default)]
    agent_prompt_append: Option<String>,
    #[serde(rename = "snapshotRetentionDays", default)]
    snapshot_retention_days: Option<f64>,
    #[serde(rename = "generated", default)]
    generated: Option<RawGenerated>,
    #[serde(rename = "snapshotMaxFileBytes", default)]
    snapshot_max_file_bytes: Option<f64>,
    #[serde(rename = "injectSessionContext", default)]
    inject_session_context: Option<bool>,
    #[serde(default)]
    collection: Option<RawCollectionBlock>,
    #[serde(default)]
    metrics: Option<Vec<MetricEntry>>,
    #[serde(default)]
    gauges: Option<Vec<GaugeEntry>>,
    #[serde(default)]
    measures: Option<Vec<MeasureEntry>>,
    #[serde(default)]
    dimensions: Option<Vec<DimensionEntry>>,
    #[serde(rename = "agentModels", default)]
    agent_models: Option<std::collections::BTreeMap<AgentKind, String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReport {
    path: String,
    format: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlugin {
    name: String,
    kind: String,
    #[serde(default)]
    formats: Vec<String>,
    runtime: String,
    #[serde(default)]
    input: Option<String>,
    #[serde(rename = "entryFile", default)]
    entry_file: Option<String>,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCollectionBlock {
    #[serde(rename = "testCommand", default)]
    test_command: Option<String>,
    #[serde(default)]
    reports: Option<Vec<RawReport>>,
    // Back-compat: the pre-`reports` singular fields. Folded into
    // `reports` on load so existing .oxplow/project.yaml files keep working.
    #[serde(rename = "coverageReportPath", default)]
    coverage_report_path: Option<String>,
    #[serde(rename = "coverageFormat", default)]
    coverage_format: Option<String>,
    #[serde(rename = "testReportPath", default)]
    test_report_path: Option<String>,
    #[serde(rename = "testReportFormat", default)]
    test_report_format: Option<String>,
    #[serde(rename = "testRunPatterns", default)]
    test_run_patterns: Option<Vec<String>>,
    #[serde(rename = "analysisRunPatterns", default)]
    analysis_run_patterns: Option<Vec<String>>,
    #[serde(rename = "agentHint", default)]
    agent_hint: Option<String>,
    #[serde(default)]
    plugins: Option<Vec<RawPlugin>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLspBlock {
    #[serde(default)]
    servers: Option<Vec<RawLspServer>>,
}

#[derive(Debug, Deserialize)]
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

    if !config_path.exists() {
        info!(
            config_path = %config_path.display(),
            agents = ?vec![AgentKind::default()],
            "project config not found; using defaults"
        );
        return Ok(default_config(fallback_name));
    }

    let raw = std::fs::read_to_string(&config_path)?;
    let parsed: RawConfig = serde_yaml::from_str(&raw)?;
    let config = validate(parsed, &fallback_name)?;
    info!(
        config_path = %config_path.display(),
        agents = ?config.agents,
        project_name = %config.project_name,
        lsp_servers = config.lsp_servers.len(),
        "loaded project config"
    );
    Ok(config)
}

/// Re-serialize an `OxplowConfig` back to `.oxplow/project.yaml`.
///
/// **Comment preservation:** none of the maintained Rust YAML
/// crates (serde_yaml, yaml-rust2, saphyr) round-trip comments,
/// so YAML comments and exact whitespace in the user's original
/// file ARE LOST on write. What we do preserve:
///
/// - Any top-level keys the user added that aren't in oxplow's
///   schema (read here, copied through, written back). This
///   matters when a third tool shares `.oxplow/project.yaml`.
/// - The minimal-default behavior — keys whose value matches the
///   default are omitted entirely, so a hand-edited file stays
///   minimal across writes.
///
/// If you maintain heavy comments in `.oxplow/project.yaml`, prefer
/// editing the file by hand; oxplow only writes through the
/// settings UI's explicit save actions.
pub fn write_project_config(
    project_dir: impl AsRef<Path>,
    config: &OxplowConfig,
) -> Result<(), ConfigError> {
    let project_dir = project_dir.as_ref();
    let path = config_path(project_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let fallback_name = basename(project_dir);

    // Schema-managed keys we own. Anything outside this set found
    // in an existing file is copied through verbatim (best-effort,
    // since YAML→serde_yaml::Value→YAML is still lossy on style).
    const MANAGED_KEYS: &[&str] = &[
        "agent",
        "agents",
        "projectName",
        "agentPromptAppend",
        "snapshotRetentionDays",
        "generated",
        "snapshotMaxFileBytes",
        "injectSessionContext",
        "lsp",
        "collection",
        "metrics",
        "measures",
        "dimensions",
        "agentModels",
    ];

    let existing_extras: serde_yaml::Mapping = if path.exists() {
        match std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_yaml::from_str::<serde_yaml::Value>(&raw).ok())
        {
            Some(serde_yaml::Value::Mapping(m)) => m
                .into_iter()
                .filter(|(k, _)| match k {
                    serde_yaml::Value::String(s) => !MANAGED_KEYS.contains(&s.as_str()),
                    _ => true,
                })
                .collect(),
            _ => serde_yaml::Mapping::new(),
        }
    } else {
        serde_yaml::Mapping::new()
    };

    let mut doc = serde_yaml::Mapping::new();
    if config.agents != vec![AgentKind::default()] {
        doc.insert(
            "agents".into(),
            serde_yaml::to_value(&config.agents).expect("agents serialize"),
        );
    }
    if !config.project_name.is_empty() && config.project_name != fallback_name {
        doc.insert("projectName".into(), config.project_name.clone().into());
    }
    if !config.agent_prompt_append.is_empty() {
        doc.insert(
            "agentPromptAppend".into(),
            config.agent_prompt_append.clone().into(),
        );
    }
    if config.snapshot_retention_days != DEFAULT_SNAPSHOT_RETENTION_DAYS {
        doc.insert(
            "snapshotRetentionDays".into(),
            config.snapshot_retention_days.into(),
        );
    }
    if !config.generated.exclude.is_empty() || !config.generated.include.is_empty() {
        doc.insert(
            "generated".into(),
            serde_yaml::to_value(&config.generated).expect("generated paths serialize"),
        );
    }
    if config.snapshot_max_file_bytes != DEFAULT_SNAPSHOT_MAX_FILE_BYTES {
        doc.insert(
            "snapshotMaxFileBytes".into(),
            config.snapshot_max_file_bytes.into(),
        );
    }
    if config.inject_session_context != DEFAULT_INJECT_SESSION_CONTEXT {
        doc.insert(
            "injectSessionContext".into(),
            config.inject_session_context.into(),
        );
    }
    if !config.lsp_servers.is_empty() {
        let mut lsp = serde_yaml::Mapping::new();
        let servers: Vec<_> = config
            .lsp_servers
            .iter()
            .map(|s| {
                let mut m = serde_yaml::Mapping::new();
                m.insert("languageId".into(), s.language_id.clone().into());
                m.insert(
                    "extensions".into(),
                    serde_yaml::to_value(&s.extensions).expect("extensions serialize"),
                );
                m.insert("command".into(), s.command.clone().into());
                if !s.args.is_empty() {
                    m.insert(
                        "args".into(),
                        serde_yaml::to_value(&s.args).expect("args serialize"),
                    );
                }
                serde_yaml::Value::Mapping(m)
            })
            .collect();
        lsp.insert("servers".into(), serde_yaml::Value::Sequence(servers));
        doc.insert("lsp".into(), serde_yaml::Value::Mapping(lsp));
    }

    let c = &config.collection;
    if c.test_command.is_some()
        || !c.reports.is_empty()
        || !c.test_run_patterns.is_empty()
        || !c.analysis_run_patterns.is_empty()
        || c.agent_hint.is_some()
        || !c.plugins.is_empty()
    {
        let mut col = serde_yaml::Mapping::new();
        if let Some(v) = &c.test_command {
            col.insert("testCommand".into(), v.clone().into());
        }
        if !c.reports.is_empty() {
            let reports: Vec<_> = c
                .reports
                .iter()
                .map(|r| {
                    let mut m = serde_yaml::Mapping::new();
                    m.insert("path".into(), r.path.clone().into());
                    m.insert("format".into(), r.format.clone().into());
                    serde_yaml::Value::Mapping(m)
                })
                .collect();
            col.insert("reports".into(), serde_yaml::Value::Sequence(reports));
        }
        if !c.test_run_patterns.is_empty() {
            col.insert(
                "testRunPatterns".into(),
                serde_yaml::to_value(&c.test_run_patterns).expect("patterns serialize"),
            );
        }
        if !c.analysis_run_patterns.is_empty() {
            col.insert(
                "analysisRunPatterns".into(),
                serde_yaml::to_value(&c.analysis_run_patterns).expect("patterns serialize"),
            );
        }
        if let Some(v) = &c.agent_hint {
            col.insert("agentHint".into(), v.clone().into());
        }
        if !c.plugins.is_empty() {
            let plugins: Vec<_> = c
                .plugins
                .iter()
                .map(|p| {
                    let mut m = serde_yaml::Mapping::new();
                    m.insert("name".into(), p.name.clone().into());
                    m.insert("kind".into(), p.kind.clone().into());
                    m.insert(
                        "formats".into(),
                        serde_yaml::to_value(&p.formats).expect("formats serialize"),
                    );
                    m.insert("runtime".into(), p.runtime.clone().into());
                    if let Some(input) = &p.input {
                        m.insert("input".into(), input.clone().into());
                    }
                    if let Some(entry_file) = &p.entry_file {
                        m.insert("entryFile".into(), entry_file.clone().into());
                    }
                    if !p.args.is_empty() {
                        m.insert(
                            "args".into(),
                            serde_yaml::to_value(&p.args).expect("args serialize"),
                        );
                    }
                    serde_yaml::Value::Mapping(m)
                })
                .collect();
            col.insert("plugins".into(), serde_yaml::Value::Sequence(plugins));
        }
        doc.insert("collection".into(), serde_yaml::Value::Mapping(col));
    }

    if !config.metrics.is_empty() {
        let metrics: Vec<_> = config.metrics.iter().map(metric_entry_to_yaml).collect();
        doc.insert("metrics".into(), serde_yaml::Value::Sequence(metrics));
    }

    if !config.gauges.is_empty() {
        let gauges: Vec<_> = config.gauges.iter().map(gauge_entry_to_yaml).collect();
        doc.insert("gauges".into(), serde_yaml::Value::Sequence(gauges));
    }

    if !config.measures.is_empty() {
        let measures: Vec<_> = config.measures.iter().map(measure_entry_to_yaml).collect();
        doc.insert("measures".into(), serde_yaml::Value::Sequence(measures));
    }

    if !config.dimensions.is_empty() {
        let dimensions: Vec<_> = config
            .dimensions
            .iter()
            .map(dimension_entry_to_yaml)
            .collect();
        doc.insert("dimensions".into(), serde_yaml::Value::Sequence(dimensions));
    }

    if !config.agent_models.is_empty() {
        doc.insert(
            "agentModels".into(),
            serde_yaml::to_value(&config.agent_models).expect("agent models serialize"),
        );
    }

    // Carry forward any unknown top-level keys the user (or a
    // sibling tool) added to .oxplow/project.yaml.
    for (k, v) in existing_extras {
        doc.insert(k, v);
    }

    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc))?;
    std::fs::write(path, yaml)?;
    Ok(())
}

/// Write a **global** metrics manifest (`global_config_dir()/metrics/<name>.yaml`)
/// — a clean `metrics:` doc holding `entries` (tsk235). Creates parent dirs.
/// Used by the Catalog "New metric" scaffold at global scope; the runner reads
/// these via [`load_global_metric_entries`].
pub fn write_global_metrics_file(path: &Path, entries: &[MetricEntry]) -> Result<(), ConfigError> {
    let seq: Vec<serde_yaml::Value> = entries.iter().map(metric_entry_to_yaml).collect();
    let mut doc = serde_yaml::Mapping::new();
    doc.insert("metrics".into(), serde_yaml::Value::Sequence(seq));
    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, yaml)?;
    Ok(())
}

/// Write a **global** gauges manifest (`global_config_dir()/gauges/<name>.yaml`)
/// — a clean `gauges:` doc. Creates parent dirs. Loaded by
/// [`load_global_gauge_entries`]; used by the "New gauge" scaffold at global
/// scope (epic tsk12, E).
pub fn write_global_gauges_file(path: &Path, entries: &[GaugeEntry]) -> Result<(), ConfigError> {
    let seq: Vec<serde_yaml::Value> = entries.iter().map(gauge_entry_to_yaml).collect();
    let mut doc = serde_yaml::Mapping::new();
    doc.insert("gauges".into(), serde_yaml::Value::Sequence(seq));
    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, yaml)?;
    Ok(())
}

/// Write a **global** measures manifest (`global_config_dir()/measures/<name>.yaml`)
/// — a clean `measures:` doc. Creates parent dirs. Loaded by
/// [`load_global_measure_entries`]; used by the "New measure" scaffold at global
/// scope (epic tsk12, E).
pub fn write_global_measures_file(
    path: &Path,
    entries: &[MeasureEntry],
) -> Result<(), ConfigError> {
    let seq: Vec<serde_yaml::Value> = entries.iter().map(measure_entry_to_yaml).collect();
    let mut doc = serde_yaml::Mapping::new();
    doc.insert("measures".into(), serde_yaml::Value::Sequence(seq));
    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, yaml)?;
    Ok(())
}

/// Write a **global** dimensions manifest
/// (`global_config_dir()/dimensions/<name>.yaml`). Loaded by
/// [`load_global_dimension_entries`]; used by the "New dimension" scaffold.
pub fn write_global_dimensions_file(
    path: &Path,
    entries: &[DimensionEntry],
) -> Result<(), ConfigError> {
    let seq: Vec<serde_yaml::Value> = entries.iter().map(dimension_entry_to_yaml).collect();
    let mut doc = serde_yaml::Mapping::new();
    doc.insert("dimensions".into(), serde_yaml::Value::Sequence(seq));
    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, yaml)?;
    Ok(())
}

/// Serialize a [`GaugeComputeConfig`] to a YAML mapping (shared by the metric →
/// gauge migration and the `gauges:` writer).
fn gauge_compute_to_yaml(c: &GaugeComputeConfig) -> serde_yaml::Value {
    let mut cm = serde_yaml::Mapping::new();
    cm.insert("runtime".into(), c.runtime.clone().into());
    if let Some(i) = &c.input {
        cm.insert("input".into(), i.clone().into());
    }
    if let Some(f) = &c.entry_file {
        cm.insert("entryFile".into(), f.clone().into());
    }
    if !c.args.is_empty() {
        cm.insert(
            "args".into(),
            serde_yaml::to_value(&c.args).expect("args serialize"),
        );
    }
    if let Some(r) = &c.report {
        cm.insert("report".into(), r.clone().into());
    }
    serde_yaml::Value::Mapping(cm)
}

/// Serialize one [`MetricEntry`] (a spec) to a YAML mapping, omitting unset
/// fields so a hand-edited `metrics:` block stays minimal across UI-driven
/// writes (mirrors the per-field plugin serialization above).
fn metric_entry_to_yaml(e: &MetricEntry) -> serde_yaml::Value {
    let mut m = serde_yaml::Mapping::new();
    let mut put_str = |k: &str, v: &Option<String>| {
        if let Some(s) = v {
            m.insert(k.into(), s.clone().into());
        }
    };
    put_str("use", &e.use_key);
    put_str("key", &e.key);
    put_str("title", &e.title);
    put_str("sourceMeasure", &e.source_measure);
    put_str("aggregation", &e.aggregation);
    put_str("unit", &e.unit);
    put_str("direction", &e.direction);
    put_str("displayKind", &e.display_kind);
    put_str("category", &e.category);
    put_str("language", &e.language);
    put_str("description", &e.description);
    // Only the disable marker is written; a bare/enabled entry stays minimal.
    if let Some(b) = e.enabled {
        m.insert("enabled".into(), b.into());
    }
    if !e.sliceable_dims.is_empty() {
        m.insert(
            "sliceableDims".into(),
            serde_yaml::to_value(&e.sliceable_dims).expect("sliceableDims serialize"),
        );
    }
    if let Some(f) = &e.filter {
        m.insert(
            "filter".into(),
            serde_yaml::to_value(f).expect("filter serialize"),
        );
    }
    if let Some(f) = &e.formula {
        m.insert(
            "formula".into(),
            serde_yaml::to_value(f).expect("formula serialize"),
        );
    }
    if let Some(t) = e.target {
        m.insert("target".into(), t.into());
    }
    if let Some(t) = e.warn_at {
        m.insert("warnAt".into(), t.into());
    }
    if let Some(t) = e.fail_at {
        m.insert("failAt".into(), t.into());
    }
    serde_yaml::Value::Mapping(m)
}

/// Serialize one [`GaugeEntry`] to a YAML mapping, omitting unset fields.
fn gauge_entry_to_yaml(e: &GaugeEntry) -> serde_yaml::Value {
    let mut m = serde_yaml::Mapping::new();
    if let Some(k) = &e.key {
        m.insert("key".into(), k.clone().into());
    }
    if let Some(t) = &e.title {
        m.insert("title".into(), t.clone().into());
    }
    if let Some(t) = &e.trigger {
        m.insert("trigger".into(), t.clone().into());
    }
    if !e.emits.is_empty() {
        m.insert(
            "emits".into(),
            serde_yaml::to_value(&e.emits).expect("emits serialize"),
        );
    }
    if let Some(c) = &e.compute {
        m.insert("compute".into(), gauge_compute_to_yaml(c));
    }
    serde_yaml::Value::Mapping(m)
}

/// Serialize one [`MeasureEntry`] to a YAML mapping, omitting unset fields so a
/// hand-edited `measures:` block stays minimal across UI-driven writes.
fn measure_entry_to_yaml(e: &MeasureEntry) -> serde_yaml::Value {
    let mut m = serde_yaml::Mapping::new();
    let mut put_str = |k: &str, v: &Option<String>| {
        if let Some(s) = v {
            m.insert(k.into(), s.clone().into());
        }
    };
    put_str("key", &e.key);
    put_str("title", &e.title);
    put_str("unit", &e.unit);
    put_str("subjectKind", &e.subject_kind);
    put_str("temporalSemantics", &e.temporal_semantics);
    put_str("componentRole", &e.component_role);
    put_str("description", &e.description);
    serde_yaml::Value::Mapping(m)
}

/// Serialize one [`DimensionEntry`] to a YAML mapping, omitting unset fields.
fn dimension_entry_to_yaml(e: &DimensionEntry) -> serde_yaml::Value {
    let mut m = serde_yaml::Mapping::new();
    let mut put_str = |k: &str, v: &Option<String>| {
        if let Some(s) = v {
            m.insert(k.into(), s.clone().into());
        }
    };
    put_str("key", &e.key);
    put_str("label", &e.label);
    put_str("valueType", &e.value_type);
    put_str("subjectKind", &e.subject_kind);
    if !e.vocabulary.is_empty() {
        m.insert(
            "vocabulary".into(),
            serde_yaml::to_value(&e.vocabulary).expect("vocabulary serialize"),
        );
    }
    if e.promote {
        m.insert("promote".into(), true.into());
    }
    serde_yaml::Value::Mapping(m)
}

fn default_config(project_name: String) -> OxplowConfig {
    OxplowConfig {
        agents: vec![AgentKind::default()],
        project_name,
        lsp_servers: Vec::new(),
        agent_prompt_append: String::new(),
        snapshot_retention_days: DEFAULT_SNAPSHOT_RETENTION_DAYS,
        generated: GeneratedConfig::default(),
        snapshot_max_file_bytes: DEFAULT_SNAPSHOT_MAX_FILE_BYTES,
        inject_session_context: DEFAULT_INJECT_SESSION_CONTEXT,
        collection: CollectionConfig::default(),
        metrics: Vec::new(),
        gauges: Vec::new(),
        measures: Vec::new(),
        dimensions: Vec::new(),
        agent_models: Default::default(),
    }
}

fn validate(raw: RawConfig, fallback_name: &str) -> Result<OxplowConfig, ConfigError> {
    let agents = match (raw.agents, raw.agent) {
        (Some(_), Some(_)) => {
            return Err(ConfigError::Invalid(
                "configure either agents or the legacy agent key, not both".into(),
            ));
        }
        (agents, legacy_agent) => {
            validate_agents(agents.or_else(|| legacy_agent.map(|agent| vec![agent])))?
        }
    };

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

    let generated = match raw.generated {
        Some(g) => GeneratedConfig {
            exclude: validate_generated_list(g.exclude, "generated.exclude")?,
            include: validate_generated_list(g.include, "generated.include")?,
        },
        None => GeneratedConfig::default(),
    };

    let snapshot_max_file_bytes = match raw.snapshot_max_file_bytes {
        Some(n) if !n.is_finite() || n < 1024.0 => {
            return Err(ConfigError::Invalid(
                "snapshotMaxFileBytes must be a number >= 1024".into(),
            ));
        }
        Some(n) => n.floor() as u64,
        None => DEFAULT_SNAPSHOT_MAX_FILE_BYTES,
    };

    let inject_session_context = raw
        .inject_session_context
        .unwrap_or(DEFAULT_INJECT_SESSION_CONTEXT);

    let collection = validate_collection(raw.collection)?;
    let metrics = validate_metrics(raw.metrics)?;
    let gauges = validate_gauges(raw.gauges)?;
    let measures = validate_measures(raw.measures)?;
    let dimensions = validate_dimensions(raw.dimensions)?;

    let agent_models = {
        let mut out = std::collections::BTreeMap::new();
        for (agent, model) in raw.agent_models.unwrap_or_default() {
            let trimmed = model.trim().to_string();
            if trimmed.is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "agentModels.{} must be a non-empty string",
                    agent.as_str()
                )));
            }
            out.insert(agent, trimmed);
        }
        out
    };

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
        generated,
        snapshot_max_file_bytes,
        inject_session_context,
        collection,
        metrics,
        gauges,
        measures,
        dimensions,
        agent_models,
    })
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

/// Transform tiers a project plugin may declare. `builtin-rust` is
/// intentionally excluded — those are first-party, registered in code.
const PLUGIN_RUNTIMES: &[&str] = &["jaq", "starlark", "exec"];
/// Collector kinds a project plugin may target.
const PLUGIN_KINDS: &[&str] = &["coverage", "test", "analysis"];
/// Container pre-parsers a plugin may select for its input.
const PLUGIN_INPUTS: &[&str] = &["text", "json", "xml", "lcov", "lines"];

/// Read-time presentation kinds a `metrics:` spec may declare (`displayKind`).
const METRIC_DISPLAY_KINDS: &[&str] = &["gauge", "findings", "test", "coverage", "event"];
/// Metric directions.
const METRIC_DIRECTIONS: &[&str] = &["higher-better", "lower-better", "neutral"];
/// Metric aggregations (mirror the engine's `Aggregation`): combine facts within
/// a capture.
const METRIC_AGGS: &[&str] = &["last", "sum", "avg", "min", "max", "count", "ratio"];
/// Formula binary ops a `metrics:` spec may declare (`ratio` aliases `div`).
const METRIC_FORMULA_OPS: &[&str] = &["add", "sub", "mul", "div", "ratio"];
/// Catalog groupings a `metrics:` spec may declare.
const METRIC_CATEGORIES: &[&str] = &["operational", "testing", "static-quality", "custom"];
/// Gauge triggers (when a `gauges:` producer runs).
const METRIC_TRIGGERS: &[&str] = &[
    "on-report",
    "on-snapshot",
    "on-effort-complete",
    "manual",
    "continuous",
];

/// Additivity-over-time a `measures:` entry may declare (mirrors the `measure`
/// table's CHECK).
const MEASURE_TEMPORAL_SEMANTICS: &[&str] = &["additive", "semi-additive", "non-additive"];
/// What ONE capture restates (tsk41). Deliberately NOT mirrored as a DB CHECK —
/// `temporal_semantics`' CHECK is exactly why adding a value there needs a
/// `measure` table rebuild (which would cascade-wipe every fact), so
/// `capture_scope` is validated here and in `CaptureScope::parse` instead.
const MEASURE_CAPTURE_SCOPES: &[&str] = &["complete", "per-path"];
/// Ratio-base role a `measures:` entry may declare.
const MEASURE_COMPONENT_ROLES: &[&str] = &["none", "numerator", "denominator"];
/// Value types a `dimensions:` entry may declare (mirrors the `dimension`
/// table's CHECK).
const DIMENSION_VALUE_TYPES: &[&str] = &["categorical", "numeric", "temporal", "entity-ref"];

/// Validate the top-level `metrics:` block (the project scope). Mirrors the
/// plugin rules: namespaced keys, `oxplow.*` reserved for built-ins,
/// project-relative `entryFile`, known runtime/kind/trigger. Each entry must be
/// exactly one of the `use:` or `key:` forms. Returns the cleaned entries (the
/// three-scope resolution happens later in [`resolve_metrics`]).
fn validate_metrics(raw: Option<Vec<MetricEntry>>) -> Result<Vec<MetricEntry>, ConfigError> {
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
        // `oxplow.*` is reserved for built-ins; a project may `use:` one but not
        // `key:`-define under it (mirrors the plugin-name rule).
        if is_define && the_key.starts_with("oxplow.") {
            return Err(ConfigError::Invalid(format!(
                "metrics[{i}] key \"{the_key}\" uses the reserved \"oxplow.\" namespace"
            )));
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
        if let Some(a) = &aggregation {
            if !METRIC_AGGS.contains(&a.as_str()) {
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
                || formula.is_some())
        {
            return Err(ConfigError::Invalid(format!(
                "metrics[{i}] is a `use:` entry; it may only override target/warnAt/failAt, \
                 not the measure/aggregation/filter/formula (those are inherent to the definition)"
            )));
        }
        // A `key:` metric is either a measure aggregation OR a formula, never both,
        // never neither.
        if is_define {
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

        let (use_key, key) = if is_define {
            (None, Some(the_key))
        } else {
            (Some(the_key), None)
        };
        out.push(MetricEntry {
            use_key,
            key,
            enabled: e.enabled,
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
                .collect(),
            target: e.target,
            warn_at: e.warn_at,
            fail_at: e.fail_at,
        });
    }
    Ok(out)
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
            Some(cleaned)
        }
        None => None,
    };
    Ok(FilterConfig {
        min_value: f.min_value,
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

/// Validate the top-level `gauges:` block (the fact PRODUCERs). Namespaced keys,
/// `oxplow.*` reserved for built-ins, a known trigger, a non-empty `emits`
/// (declare-to-collect), and a valid `compute:` block. Definition-only (no
/// `use:`/`key:` split — a gauge is always declared).
fn validate_gauges(raw: Option<Vec<GaugeEntry>>) -> Result<Vec<GaugeEntry>, ConfigError> {
    let opt = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, e) in raw.into_iter().flatten().enumerate() {
        let key = validate_catalog_key("gauges", i, e.key, &mut seen)?;
        let trigger = opt(e.trigger);
        if let Some(t) = &trigger {
            if !METRIC_TRIGGERS.contains(&t.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "gauges[{i}] trigger must be one of {METRIC_TRIGGERS:?} (got \"{t}\")"
                )));
            }
        }
        let emits: Vec<String> = e
            .emits
            .into_iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect();
        if emits.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "gauges[{i}] must declare at least one measure in `emits` \
                 (a gauge may only emit facts on measures it declares)"
            )));
        }
        let compute = match e.compute {
            Some(c) => validate_gauge_compute(i, c)?,
            None => {
                return Err(ConfigError::Invalid(format!(
                    "gauges[{i}] key \"{key}\" has no `compute` block"
                )))
            }
        };
        out.push(GaugeEntry {
            key: Some(key),
            title: opt(e.title),
            trigger,
            emits,
            compute: Some(compute),
        });
    }
    Ok(out)
}

fn validate_gauge_compute(
    i: usize,
    c: GaugeComputeConfig,
) -> Result<GaugeComputeConfig, ConfigError> {
    let runtime = c.runtime.trim().to_ascii_lowercase();
    if !PLUGIN_RUNTIMES.contains(&runtime.as_str()) {
        return Err(ConfigError::Invalid(format!(
            "gauges[{i}].compute.runtime must be jaq | starlark | exec (got \"{}\")",
            c.runtime
        )));
    }
    let input = match c.input.map(|s| s.trim().to_ascii_lowercase()) {
        Some(s) if !s.is_empty() => {
            if !PLUGIN_INPUTS.contains(&s.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "gauges[{i}].compute.input must be text | json | xml | lcov | lines (got \"{s}\")"
                )));
            }
            Some(s)
        }
        _ => None,
    };
    let entry_file = c
        .entry_file
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let entry_file = match entry_file {
        Some(f) => f,
        None => {
            return Err(ConfigError::Invalid(format!(
                "gauges[{i}].compute.entryFile is required (the script file path)"
            )))
        }
    };
    if Path::new(&entry_file).is_absolute() || entry_file.split('/').any(|c| c == "..") {
        return Err(ConfigError::Invalid(format!(
            "gauges[{i}].compute.entryFile must be a project-relative path without `..` (got \"{entry_file}\")"
        )));
    }
    Ok(GaugeComputeConfig {
        runtime,
        input,
        entry_file: Some(entry_file),
        args: c.args.into_iter().map(|a| a.trim().to_string()).collect(),
        report: c
            .report
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    })
}

/// Resolve declared metric SPECS across the three scopes into the flat
/// [`ResolvedSpec`] list the runner consumes. Definitions (`key:` entries) from
/// built-in, then global, then project build a catalog by key (later scope wins →
/// precedence project > global > built-in). The **project's** entries are what's
/// *active*: a `key:` entry defines + enables (scope `project`); a `use:` entry
/// enables a catalog metric, layering its threshold overrides on top (scope = the
/// definition's scope). A `use:` referencing an unknown key is skipped with a
/// warning.
pub fn resolve_metrics(
    builtin: &[MetricEntry],
    global: &[MetricEntry],
    project: &[MetricEntry],
) -> Vec<ResolvedSpec> {
    // Catalog of definitions by key, with the scope each came from.
    let mut catalog: std::collections::HashMap<String, (&'static str, &MetricEntry)> =
        std::collections::HashMap::new();
    for (scope, entries) in [
        ("built-in", builtin),
        ("global", global),
        ("project", project),
    ] {
        for e in entries {
            if let Some(k) = e.key.as_deref() {
                catalog.insert(k.to_string(), (scope, e));
            }
        }
    }

    let mut out = Vec::new();
    for e in project {
        if let Some(k) = e.key.as_deref() {
            // A project definition: it is its own resolved spec.
            out.push(resolve_one(k, "project", e, None));
        } else if let Some(uk) = e.use_key.as_deref() {
            match catalog.get(uk) {
                Some((scope, def)) => out.push(resolve_one(uk, scope, def, Some(e))),
                // A `use:` of a key not in the resolve catalog is normally a typo.
                // The exception is a **disable marker** (`enabled: false`) for a
                // producer/plugin metric — those keys aren't config definitions,
                // so `seed_catalog` handles their pruning directly from config
                // state; skip it here silently rather than warn.
                None if e.enabled == Some(false) => {}
                None => tracing::warn!(
                    key = uk,
                    "metrics: `use:` references an unknown catalog key; skipping"
                ),
            }
        }
    }
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
        // The `enabled` flag lives on the acting (project) entry — the `use:`
        // override for a use'd metric, else the `key:` definition. Default on.
        enabled: over.and_then(|o| o.enabled).or(def.enabled).unwrap_or(true),
    }
}

/// Resolve declared gauges across the global + project scopes into the flat
/// [`ResolvedGauge`] list the runner executes. Both scopes are definition-only (a
/// gauge is declared, never "enabled"); a project entry with the same key as a
/// global one wins (precedence project > global). First-seen order is preserved.
/// Built-in gauges live in code and never flow through here.
pub fn resolve_gauges(global: &[GaugeEntry], project: &[GaugeEntry]) -> Vec<ResolvedGauge> {
    let mut out: Vec<ResolvedGauge> = Vec::new();
    let mut pos: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (scope, entries) in [("global", global), ("project", project)] {
        for e in entries {
            let Some(key) = e.key.as_deref() else {
                continue;
            };
            let resolved = ResolvedGauge {
                key: key.to_string(),
                title: e.title.clone().unwrap_or_else(|| key.to_string()),
                trigger: e.trigger.clone().unwrap_or_else(|| "on-snapshot".into()),
                emits: e.emits.clone(),
                compute: e.compute.clone().unwrap_or_default(),
                scope: scope.to_string(),
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
            .map(|d| validate_metrics(d.metrics).map_err(|e| e.to_string()))
    })
}

/// Load gauge definitions from the user-global scope
/// (`<global_dir>/gauges/*.yaml`, each a `{ gauges: [ … ] }` doc). Best-effort:
/// a malformed/unreadable file is logged and skipped. Filename order for
/// deterministic precedence.
pub fn load_global_gauge_entries(global_dir: &Path) -> Vec<GaugeEntry> {
    #[derive(Deserialize)]
    struct Doc {
        #[serde(default)]
        gauges: Option<Vec<GaugeEntry>>,
    }
    load_global_entries(global_dir, "gauges", |raw| {
        serde_yaml::from_str::<Doc>(raw)
            .ok()
            .map(|d| validate_gauges(d.gauges).map_err(|e| e.to_string()))
    })
}

/// List `*.yaml`/`*.yml` files under `<global_dir>/<subdir>`, sorted by filename
/// for deterministic precedence. Empty when the directory is absent. Shared by
/// the global catalog loaders (metrics / gauges / measures / dimensions).
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
/// be present, namespaced `<vendor>.<id>`, outside the reserved `oxplow.*`
/// namespace (those are the migration seed), and unique within its block.
/// Returns the cleaned key. `block` is the YAML block name for error messages.
fn validate_catalog_key(
    block: &str,
    i: usize,
    raw_key: Option<String>,
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
    if key.starts_with("oxplow.") {
        return Err(ConfigError::Invalid(format!(
            "{block}[{i}] key \"{key}\" uses the reserved \"oxplow.\" namespace"
        )));
    }
    if !seen.insert(key.clone()) {
        return Err(ConfigError::Invalid(format!(
            "{block}[{i}] key \"{key}\" appears more than once in the {block} block"
        )));
    }
    Ok(key)
}

/// Validate the top-level `measures:` block. Mirrors [`validate_metrics`]:
/// namespaced keys, `oxplow.*` reserved, per-key uniqueness, known
/// temporalSemantics/componentRole enums. Definition-only (no `use:` form).
fn validate_measures(raw: Option<Vec<MeasureEntry>>) -> Result<Vec<MeasureEntry>, ConfigError> {
    let opt = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, e) in raw.into_iter().flatten().enumerate() {
        let key = validate_catalog_key("measures", i, e.key, &mut seen)?;
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
        let component_role = match opt(e.component_role) {
            Some(s) => {
                if !MEASURE_COMPONENT_ROLES.contains(&s.as_str()) {
                    return Err(ConfigError::Invalid(format!(
                        "measures[{i}] componentRole must be one of \
                         {MEASURE_COMPONENT_ROLES:?} (got \"{s}\")"
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
            component_role,
            description: opt(e.description),
        });
    }
    Ok(out)
}

/// Validate the top-level `dimensions:` block. Mirrors [`validate_measures`]:
/// namespaced keys, `oxplow.*` reserved, per-key uniqueness, known valueType.
fn validate_dimensions(
    raw: Option<Vec<DimensionEntry>>,
) -> Result<Vec<DimensionEntry>, ConfigError> {
    let opt = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, e) in raw.into_iter().flatten().enumerate() {
        let key = validate_catalog_key("dimensions", i, e.key, &mut seen)?;
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
        out.push(DimensionEntry {
            key: Some(key),
            label: opt(e.label),
            value_type,
            subject_kind: opt(e.subject_kind),
            vocabulary,
            promote: e.promote,
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
pub fn resolve_measures(global: &[MeasureEntry], project: &[MeasureEntry]) -> Vec<ResolvedMeasure> {
    let mut out: Vec<ResolvedMeasure> = Vec::new();
    let mut pos: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (scope, entries) in [("global", global), ("project", project)] {
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
                component_role: e.component_role.clone().unwrap_or_else(|| "none".into()),
                scope: scope.to_string(),
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
    project: &[DimensionEntry],
) -> Vec<ResolvedDimension> {
    let mut out: Vec<ResolvedDimension> = Vec::new();
    let mut pos: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (scope, entries) in [("global", global), ("project", project)] {
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
            .map(|d| validate_measures(d.measures).map_err(|e| e.to_string()))
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
        serde_yaml::from_str::<Doc>(raw)
            .ok()
            .map(|d| validate_dimensions(d.dimensions).map_err(|e| e.to_string()))
    })
}

fn validate_agents(raw: Option<Vec<AgentKind>>) -> Result<Vec<AgentKind>, ConfigError> {
    let agents = raw.unwrap_or_else(|| vec![AgentKind::default()]);
    if agents.is_empty() {
        return Err(ConfigError::Invalid(
            "agents must list at least one enabled agent".into(),
        ));
    }
    let mut seen = Vec::new();
    for agent in agents {
        if seen.contains(&agent) {
            return Err(ConfigError::Invalid(format!(
                "agents must not contain duplicates (got {agent:?})"
            )));
        }
        seen.push(agent);
    }
    Ok(seen)
}

/// Require a non-empty (already-trimmed) report format. The *value* is no
/// longer gate-kept against a hardcoded list — format names resolve against
/// the collector registry at collection time, so plugin-provided formats work
/// and an unknown one surfaces as a warning, not a config load failure.
fn require_format(field: &str, fmt: &str) -> Result<(), ConfigError> {
    if fmt.is_empty() {
        return Err(ConfigError::Invalid(format!(
            "collection.{field} must be a non-empty string"
        )));
    }
    Ok(())
}

fn validate_plugins(raw: Option<Vec<RawPlugin>>) -> Result<Vec<PluginConfig>, ConfigError> {
    let mut plugins = Vec::new();
    for (i, p) in raw.into_iter().flatten().enumerate() {
        let name = p.name.trim().to_string();
        if name.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "collection.plugins[{i}].name must be a non-empty string"
            )));
        }
        // Names are namespaced `<vendor>.<id>`; `oxplow.` is reserved for the
        // first-party built-ins so a project can't impersonate them.
        if name.starts_with("oxplow.") {
            return Err(ConfigError::Invalid(format!(
                "collection.plugins[{i}].name \"{name}\" uses the reserved \"oxplow.\" namespace"
            )));
        }
        if !name.contains('.') {
            return Err(ConfigError::Invalid(format!(
                "collection.plugins[{i}].name \"{name}\" must be namespaced as \"<vendor>.<id>\" (e.g. acme.clover)"
            )));
        }
        let kind = p.kind.trim().to_ascii_lowercase();
        if !PLUGIN_KINDS.contains(&kind.as_str()) {
            return Err(ConfigError::Invalid(format!(
                "collection.plugins[{i}].kind must be coverage | test | analysis (got \"{}\")",
                p.kind
            )));
        }
        let runtime = p.runtime.trim().to_ascii_lowercase();
        if !PLUGIN_RUNTIMES.contains(&runtime.as_str()) {
            return Err(ConfigError::Invalid(format!(
                "collection.plugins[{i}].runtime must be jaq | starlark | exec (got \"{}\")",
                p.runtime
            )));
        }
        let mut formats = Vec::new();
        for (j, f) in p.formats.into_iter().enumerate() {
            let f = f.trim().to_string();
            if f.is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "collection.plugins[{i}].formats[{j}] must be a non-empty string"
                )));
            }
            formats.push(f);
        }
        if formats.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "collection.plugins[{i}].formats must list at least one format"
            )));
        }
        let input = match p.input.map(|s| s.trim().to_ascii_lowercase()) {
            Some(s) if !s.is_empty() => {
                if !PLUGIN_INPUTS.contains(&s.as_str()) {
                    return Err(ConfigError::Invalid(format!(
                        "collection.plugins[{i}].input must be text | json | xml | lcov | lines (got \"{s}\")"
                    )));
                }
                Some(s)
            }
            _ => None,
        };
        let entry_file = p
            .entry_file
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let entry_file = match entry_file {
            Some(f) => f,
            None => {
                return Err(ConfigError::Invalid(format!(
                    "collection.plugins[{i}].entryFile is required (the script file path)"
                )))
            }
        };
        if Path::new(&entry_file).is_absolute() || entry_file.split('/').any(|c| c == "..") {
            return Err(ConfigError::Invalid(format!(
                "collection.plugins[{i}].entryFile must be a project-relative path \
                 without `..` (got \"{entry_file}\")"
            )));
        }
        let args = p.args.into_iter().map(|a| a.trim().to_string()).collect();
        plugins.push(PluginConfig {
            name,
            kind,
            formats,
            runtime,
            input,
            entry_file: Some(entry_file),
            args,
        });
    }
    Ok(plugins)
}

fn validate_collection(raw: Option<RawCollectionBlock>) -> Result<CollectionConfig, ConfigError> {
    let Some(raw) = raw else {
        return Ok(CollectionConfig::default());
    };
    let opt_trimmed = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

    let mut reports = Vec::new();
    // The `reports` list (canonical).
    for (i, r) in raw.reports.into_iter().flatten().enumerate() {
        let path = r.path.trim().to_string();
        let format = r.format.trim().to_string();
        if path.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "collection.reports[{i}].path must be a non-empty string"
            )));
        }
        require_format(&format!("reports[{i}].format"), &format)?;
        reports.push(ReportConfig { path, format });
    }
    // Back-compat: fold the old singular fields into `reports`.
    if let (Some(path), Some(format)) = (
        opt_trimmed(raw.coverage_report_path),
        opt_trimmed(raw.coverage_format),
    ) {
        require_format("coverageFormat", &format)?;
        reports.push(ReportConfig { path, format });
    }
    if let (Some(path), Some(format)) = (
        opt_trimmed(raw.test_report_path),
        opt_trimmed(raw.test_report_format),
    ) {
        require_format("testReportFormat", &format)?;
        reports.push(ReportConfig { path, format });
    }

    let validate_patterns = |field: &str, list: Option<Vec<String>>| {
        let mut out = Vec::new();
        for (i, p) in list.into_iter().flatten().enumerate() {
            let trimmed = p.trim().to_string();
            if trimmed.is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "collection.{field}[{i}] must be a non-empty string"
                )));
            }
            out.push(trimmed);
        }
        Ok(out)
    };
    let test_run_patterns = validate_patterns("testRunPatterns", raw.test_run_patterns)?;
    let analysis_run_patterns =
        validate_patterns("analysisRunPatterns", raw.analysis_run_patterns)?;
    let plugins = validate_plugins(raw.plugins)?;
    Ok(CollectionConfig {
        test_command: opt_trimmed(raw.test_command),
        reports,
        test_run_patterns,
        analysis_run_patterns,
        agent_hint: opt_trimmed(raw.agent_hint),
        plugins,
    })
}

fn basename(path: &Path) -> String {
    let resolved: PathBuf = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    resolved
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "oxplow".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Resolve the project config path under `<dir>/.oxplow/`, creating the
    /// `.oxplow` parent so a subsequent write succeeds. Used by tests that
    /// author a config file directly (simulating a user-edited file).
    fn cfg_path(project_dir: &Path) -> std::path::PathBuf {
        let p = config_path(project_dir);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        p
    }

    #[test]
    fn load_defaults_when_file_absent() {
        let dir = tempdir().unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.agents, vec![AgentKind::Claude]);
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
        assert_eq!(cfg.agents, vec![AgentKind::Claude, AgentKind::Codex]);
        assert_eq!(cfg.project_name, "explicit-name");
    }

    #[test]
    fn loads_all_three_agent_kinds() {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "agents: [claude, codex, opencode]\n").unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(
            cfg.agents,
            vec![AgentKind::Claude, AgentKind::Codex, AgentKind::Opencode]
        );
    }

    #[test]
    fn loads_legacy_agent_as_single_enabled_agent() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "agent: codex\nprojectName: explicit-name\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.agents, vec![AgentKind::Codex]);
        assert_eq!(cfg.project_name, "explicit-name");
    }

    #[test]
    fn rejects_agents_and_legacy_agent_together() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "agent: claude\nagents: [claude, codex]\n",
        )
        .unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("not both")));
    }

    #[test]
    fn rejects_invalid_agent_in_agents() {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "agents: [emacs]\n").unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));
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
        write_project_config(dir.path(), &cfg).unwrap();
        let reloaded = load_project_config(dir.path()).unwrap();
        assert_eq!(reloaded.generated.exclude, vec!["target".to_string()]);
        assert_eq!(
            reloaded.generated.include,
            vec!["dist/keep.json".to_string()]
        );
    }

    #[test]
    fn agent_models_round_trip() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "agentModels:\n  opencode: github-copilot/gpt-5-mini\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(
            cfg.agent_models
                .get(&AgentKind::Opencode)
                .map(String::as_str),
            Some("github-copilot/gpt-5-mini")
        );
        write_project_config(dir.path(), &cfg).unwrap();
        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        assert!(raw.contains("agentModels:"), "got:\n{raw}");
        assert!(
            raw.contains("opencode: github-copilot/gpt-5-mini"),
            "got:\n{raw}"
        );
    }

    #[test]
    fn agent_models_rejects_unknown_agent_and_blank_model() {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "agentModels:\n  goose: some/model\n").unwrap();
        assert!(matches!(
            load_project_config(dir.path()).unwrap_err(),
            ConfigError::Parse(_)
        ));
        std::fs::write(cfg_path(dir.path()), "agentModels:\n  opencode: \"  \"\n").unwrap();
        assert!(matches!(
            load_project_config(dir.path()).unwrap_err(),
            ConfigError::Invalid(msg) if msg.contains("agentModels.opencode")
        ));
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
        write_project_config(dir.path(), &cfg).unwrap();
        let loaded = load_project_config(dir.path()).unwrap();
        assert!(!loaded.inject_session_context);
    }

    #[test]
    fn collection_defaults_empty_when_absent() {
        let dir = tempdir().unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.collection, CollectionConfig::default());
    }

    #[test]
    fn parses_collection_block() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            r#"
collection:
  testCommand: cargo cov
  agentHint: "Run tests with cargo cov"
  reports:
    - { path: target/coverage/lcov.info, format: lcov }
    - { path: target/nextest/default/junit.xml, format: junit }
    - { path: apps/desktop/test-report.xml, format: junit }
  testRunPatterns:
    - cargo cov
    - bun test
"#,
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.collection.test_command.as_deref(), Some("cargo cov"));
        assert_eq!(
            cfg.collection.agent_hint.as_deref(),
            Some("Run tests with cargo cov")
        );
        assert_eq!(cfg.collection.reports.len(), 3);
        assert_eq!(cfg.collection.coverage_reports().count(), 1);
        assert_eq!(cfg.collection.test_reports().count(), 2);
        assert_eq!(cfg.collection.reports[0].format, "lcov");
        assert_eq!(
            cfg.collection.test_run_patterns,
            vec!["cargo cov", "bun test"]
        );
    }

    #[test]
    fn back_compat_singular_fields_fold_into_reports() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "collection:\n  coverageReportPath: cov.info\n  coverageFormat: lcov\n  testReportPath: j.xml\n  testReportFormat: junit\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.collection.reports.len(), 2);
        assert_eq!(cfg.collection.coverage_reports().count(), 1);
        assert_eq!(cfg.collection.test_reports().next().unwrap().path, "j.xml");
    }

    #[test]
    fn accepts_unrecognized_report_format_for_registry_resolution() {
        // Format names are no longer gate-kept here — a plugin-provided format
        // resolves against the collector registry at collection time.
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "collection:\n  reports:\n    - { path: x.tap, format: tap }\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.collection.reports[0].format, "tap");
    }

    #[test]
    fn rejects_empty_report_format() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "collection:\n  reports:\n    - { path: x.tap, format: \"\" }\n",
        )
        .unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("format")));
    }

    #[test]
    fn parses_project_plugin_definition() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "collection:\n  reports:\n    - { path: c.xml, format: clover }\n  plugins:\n    - name: acme.clover\n      kind: coverage\n      formats: [clover]\n      runtime: jaq\n      entryFile: oxplow/plugins/clover.jq\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.collection.plugins.len(), 1);
        let p = &cfg.collection.plugins[0];
        assert_eq!(p.name, "acme.clover");
        assert_eq!(p.kind, "coverage");
        assert_eq!(p.formats, vec!["clover"]);
        assert_eq!(p.runtime, "jaq");
        assert_eq!(p.entry_file.as_deref(), Some("oxplow/plugins/clover.jq"));
    }

    #[test]
    fn rejects_plugin_entry_file_escaping_project() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "collection:\n  plugins:\n    - name: acme.x\n      kind: coverage\n      formats: [x]\n      runtime: jaq\n      entryFile: ../../etc/passwd\n",
        )
        .unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("entryFile")));
    }

    #[test]
    fn rejects_plugin_in_reserved_oxplow_namespace() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "collection:\n  plugins:\n    - name: oxplow.clover\n      kind: coverage\n      formats: [clover]\n      runtime: jaq\n      entryFile: p.jq\n",
        )
        .unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("oxplow.")));
    }

    #[test]
    fn rejects_plugin_without_namespace_prefix() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "collection:\n  plugins:\n    - name: clover\n      kind: coverage\n      formats: [clover]\n      runtime: jaq\n      entryFile: p.jq\n",
        )
        .unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("namespaced")));
    }

    #[test]
    fn rejects_plugin_with_unknown_runtime() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "collection:\n  plugins:\n    - name: acme.x\n      kind: coverage\n      formats: [x]\n      runtime: wasm\n      entryFile: p.jq\n",
        )
        .unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("runtime")));
    }

    #[test]
    fn rejects_plugin_missing_entry_file() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "collection:\n  plugins:\n    - name: acme.x\n      kind: test\n      formats: [x]\n      runtime: starlark\n",
        )
        .unwrap();
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(msg) if msg.contains("entryFile")));
    }

    #[test]
    fn collection_round_trips_through_write() {
        let dir = tempdir().unwrap();
        let cfg = OxplowConfig {
            collection: CollectionConfig {
                test_command: Some("pytest".into()),
                reports: vec![
                    ReportConfig {
                        path: "coverage.xml".into(),
                        format: "cobertura".into(),
                    },
                    ReportConfig {
                        path: "junit.xml".into(),
                        format: "junit".into(),
                    },
                ],
                test_run_patterns: vec!["tox".into()],
                analysis_run_patterns: vec!["cargo clippy".into()],
                agent_hint: Some("Run pytest, not bare python -m pytest".into()),
                plugins: vec![PluginConfig {
                    name: "acme.clover".into(),
                    kind: "coverage".into(),
                    formats: vec!["clover".into()],
                    runtime: "jaq".into(),
                    input: Some("xml".into()),
                    entry_file: Some("oxplow/plugins/clover.jq".into()),
                    args: vec![],
                }],
            },
            ..default_config("test".into())
        };
        write_project_config(dir.path(), &cfg).unwrap();
        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        assert!(raw.contains("collection:"), "got:\n{raw}");
        let loaded = load_project_config(dir.path()).unwrap();
        assert_eq!(loaded.collection, cfg.collection);
    }

    /// Third-party keys that aren't part of oxplow's schema should
    /// survive a write. Comments still get stripped (no Rust YAML
    /// crate round-trips them), but the keys themselves persist —
    /// otherwise a sibling tool sharing .oxplow/project.yaml would lose its
    /// state every time the user touched oxplow's settings UI.
    #[test]
    fn write_preserves_unknown_top_level_keys() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "agents: [claude]\nthirdPartyTool:\n  enabled: true\n  values: [a, b]\n",
        )
        .unwrap();

        let cfg = OxplowConfig {
            snapshot_retention_days: 14,
            ..default_config("test".into())
        };
        write_project_config(dir.path(), &cfg).unwrap();

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

    #[test]
    fn parses_both_metric_forms() {
        let cfg = load_from_yaml(
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
        assert_eq!(cfg.metrics.len(), 2);
        assert_eq!(cfg.metrics[0].key.as_deref(), Some("repo.unsafe_blocks"));
        assert_eq!(
            cfg.metrics[0].source_measure.as_deref(),
            Some("acme.ast_hit")
        );
        assert_eq!(cfg.metrics[0].aggregation.as_deref(), Some("sum"));
        assert_eq!(
            cfg.metrics[0].filter.as_ref().unwrap().dim_eq.as_deref(),
            Some(["acme.rule".to_string(), "unsafe_block".to_string()].as_slice())
        );
        assert_eq!(cfg.metrics[0].sliceable_dims, vec!["acme.rule".to_string()]);
        assert_eq!(
            cfg.metrics[1].use_key.as_deref(),
            Some("myglobal.todo_density")
        );
        assert_eq!(cfg.metrics[1].target, Some(5.0));
    }

    #[test]
    fn parses_formula_metric() {
        let cfg = load_from_yaml(
            r#"
metrics:
  - key: acme.bugs_per_kloc
    title: "bugs per KLOC"
    formula: { op: div, left: acme.bug_count, right: acme.kloc }
"#,
        )
        .unwrap();
        let f = cfg.metrics[0].formula.as_ref().unwrap();
        assert_eq!(f.op, "div");
        assert_eq!(f.left, "acme.bug_count");
        assert_eq!(f.right, "acme.kloc");
        assert!(cfg.metrics[0].source_measure.is_none());
    }

    #[test]
    fn metric_validation_rejects_bad_entries() {
        // Reserved namespace for a definition.
        assert!(
            load_from_yaml("metrics:\n  - key: oxplow.foo\n    sourceMeasure: acme.m\n").is_err()
        );
        // `key:` with neither sourceMeasure nor formula.
        assert!(load_from_yaml("metrics:\n  - key: acme.foo\n    displayKind: gauge\n").is_err());
        // `key:` with BOTH sourceMeasure and formula.
        assert!(load_from_yaml(
            "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n    formula: { op: div, left: a.c, right: a.d }\n"
        )
        .is_err());
        // `use:` carrying a structural field (sourceMeasure).
        assert!(
            load_from_yaml("metrics:\n  - use: acme.foo\n    sourceMeasure: acme.m\n").is_err()
        );
        // both use and key.
        assert!(load_from_yaml("metrics:\n  - use: a.b\n    key: c.d\n").is_err());
        // un-namespaced key.
        assert!(load_from_yaml("metrics:\n  - key: foo\n    sourceMeasure: acme.m\n").is_err());
        // bad aggregation.
        assert!(load_from_yaml(
            "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n    aggregation: median\n"
        )
        .is_err());
        // bad displayKind.
        assert!(load_from_yaml(
            "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n    displayKind: sparkline\n"
        )
        .is_err());
        // a full valid spec is accepted.
        assert!(load_from_yaml(
            "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n    aggregation: count\n    displayKind: findings\n    category: static-quality\n"
        )
        .is_ok());
        // the same key declared twice (a `key:` define + a `use:`).
        assert!(load_from_yaml(
            "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n  - use: a.b\n    target: 5\n"
        )
        .is_err());
        // the same key defined twice.
        assert!(load_from_yaml(
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
        let resolved = resolve_metrics(&builtin, &global, &project);
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
    fn resolve_project_definition_is_active_and_scoped() {
        let project = vec![define("acme.loc", None)];
        let resolved = resolve_metrics(&[], &[], &project);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].scope, "project");
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
        let resolved = resolve_metrics(&[], &global, &project);
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
        assert!(resolve_metrics(&[], &[], &project).is_empty());
    }

    #[test]
    fn resolve_defaults_enabled_true() {
        let project = vec![define("acme.loc", None)];
        let resolved = resolve_metrics(&[], &[], &project);
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
        let resolved = resolve_metrics(&builtin, &[], &project);
        assert_eq!(resolved.len(), 1);
        assert!(!resolved[0].enabled);
    }

    #[test]
    fn resolve_marks_disabled_key_definition() {
        // Disabling a config-DEFINED metric keeps its definition but flags it off.
        let mut def = define("acme.loc", None);
        def.enabled = Some(false);
        let resolved = resolve_metrics(&[], &[], &[def]);
        assert_eq!(resolved.len(), 1);
        assert!(!resolved[0].enabled);
    }

    #[test]
    fn resolve_disable_marker_for_unknown_key_is_skipped_quietly() {
        // A disable marker for a producer/plugin key (not a resolve-catalog def)
        // is skipped without a warning — seed_catalog prunes it from config state.
        let project = vec![MetricEntry {
            use_key: Some("agent.tokens.total".into()),
            enabled: Some(false),
            ..Default::default()
        }];
        assert!(resolve_metrics(&[], &[], &project).is_empty());
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
        write_project_config(dir.path(), &cfg).unwrap();
        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        assert!(raw.contains("enabled: false"), "got:\n{raw}");
        let loaded = load_project_config(dir.path()).unwrap();
        assert_eq!(loaded.metrics, cfg.metrics);
    }

    #[test]
    fn metrics_round_trip_through_write() {
        let dir = tempdir().unwrap();
        let cfg = OxplowConfig {
            metrics: vec![define("acme.loc", Some(2.0))],
            ..default_config("test".into())
        };
        write_project_config(dir.path(), &cfg).unwrap();
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

    // --- gauges (the fact PRODUCERs — workstream E) -------------------------

    #[test]
    fn parses_gauges_block() {
        let cfg = load_from_yaml(
            r#"
gauges:
  - key: acme.scan
    title: "Acme scan"
    trigger: on-snapshot
    emits: [acme.complexity, acme.todo]
    compute: { runtime: starlark, entryFile: oxplow/gauges/scan.star }
"#,
        )
        .unwrap();
        assert_eq!(cfg.gauges.len(), 1);
        assert_eq!(cfg.gauges[0].key.as_deref(), Some("acme.scan"));
        assert_eq!(cfg.gauges[0].emits, vec!["acme.complexity", "acme.todo"]);
        assert_eq!(
            cfg.gauges[0]
                .compute
                .as_ref()
                .unwrap()
                .entry_file
                .as_deref(),
            Some("oxplow/gauges/scan.star")
        );
    }

    #[test]
    fn gauge_validation_rejects_bad_entries() {
        // Reserved namespace.
        assert!(load_from_yaml(
            "gauges:\n  - key: oxplow.foo\n    emits: [acme.m]\n    compute: { runtime: jaq, entryFile: x.jq }\n"
        )
        .is_err());
        // No emits (declare-to-collect requires at least one).
        assert!(load_from_yaml(
            "gauges:\n  - key: acme.foo\n    compute: { runtime: jaq, entryFile: x.jq }\n"
        )
        .is_err());
        // No compute.
        assert!(load_from_yaml("gauges:\n  - key: acme.foo\n    emits: [acme.m]\n").is_err());
        // Bad runtime.
        assert!(load_from_yaml(
            "gauges:\n  - key: acme.foo\n    emits: [acme.m]\n    compute: { runtime: wasm, entryFile: x.jq }\n"
        )
        .is_err());
        // entryFile escaping the project root.
        assert!(load_from_yaml(
            "gauges:\n  - key: acme.foo\n    emits: [acme.m]\n    compute: { runtime: jaq, entryFile: ../x.jq }\n"
        )
        .is_err());
        // A full valid gauge is accepted.
        assert!(load_from_yaml(
            "gauges:\n  - key: acme.foo\n    emits: [acme.m]\n    compute: { runtime: starlark, entryFile: g.star }\n"
        )
        .is_ok());
    }

    #[test]
    fn resolve_gauges_project_over_global() {
        let g = |key: &str, file: &str| GaugeEntry {
            key: Some(key.into()),
            emits: vec!["acme.m".into()],
            compute: Some(GaugeComputeConfig {
                runtime: "starlark".into(),
                entry_file: Some(file.into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let global = vec![g("acme.scan", "global.star")];
        let project = vec![g("acme.scan", "project.star")];
        let resolved = resolve_gauges(&global, &project);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].scope, "project");
        assert_eq!(
            resolved[0].compute.entry_file.as_deref(),
            Some("project.star")
        );
        // Default trigger when unset.
        assert_eq!(resolved[0].trigger, "on-snapshot");
    }

    #[test]
    fn gauges_round_trip_through_write() {
        let dir = tempdir().unwrap();
        let cfg = OxplowConfig {
            gauges: vec![GaugeEntry {
                key: Some("acme.scan".into()),
                emits: vec!["acme.m".into()],
                compute: Some(GaugeComputeConfig {
                    runtime: "starlark".into(),
                    entry_file: Some("g.star".into()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..default_config("test".into())
        };
        write_project_config(dir.path(), &cfg).unwrap();
        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        assert!(raw.contains("gauges:"), "got:\n{raw}");
        assert!(!raw.contains("null"), "minimal write, got:\n{raw}");
        let loaded = load_project_config(dir.path()).unwrap();
        assert_eq!(loaded.gauges, cfg.gauges);
    }

    // --- measures + dimensions (workstream E) ------------------------------

    #[test]
    fn parses_measures_and_dimensions_blocks() {
        let cfg = load_from_yaml(
            r#"
measures:
  - key: acme.api_latency
    title: "API latency"
    unit: ms
    subjectKind: endpoint
    temporalSemantics: non-additive
    componentRole: numerator
    description: "p95 request latency"
dimensions:
  - key: acme.license
    label: License
    valueType: categorical
    vocabulary: [MIT, Apache-2.0, GPL-3.0]
  - key: acme.endpoint
    valueType: entity-ref
    subjectKind: endpoint
    promote: true
"#,
        )
        .unwrap();
        assert_eq!(cfg.measures.len(), 1);
        let m = &cfg.measures[0];
        assert_eq!(m.key.as_deref(), Some("acme.api_latency"));
        assert_eq!(m.unit.as_deref(), Some("ms"));
        assert_eq!(m.subject_kind.as_deref(), Some("endpoint"));
        assert_eq!(m.temporal_semantics.as_deref(), Some("non-additive"));
        assert_eq!(m.component_role.as_deref(), Some("numerator"));

        assert_eq!(cfg.dimensions.len(), 2);
        assert_eq!(cfg.dimensions[0].key.as_deref(), Some("acme.license"));
        assert_eq!(
            cfg.dimensions[0].vocabulary,
            vec!["MIT", "Apache-2.0", "GPL-3.0"]
        );
        assert!(!cfg.dimensions[0].promote);
        assert_eq!(cfg.dimensions[1].value_type.as_deref(), Some("entity-ref"));
        assert!(cfg.dimensions[1].promote);
    }

    #[test]
    fn measure_validation_rejects_bad_entries() {
        // Reserved namespace.
        assert!(load_from_yaml("measures:\n  - key: oxplow.foo\n").is_err());
        // Un-namespaced key.
        assert!(load_from_yaml("measures:\n  - key: foo\n").is_err());
        // Missing key.
        assert!(load_from_yaml("measures:\n  - title: nokey\n").is_err());
        // Bad temporalSemantics.
        assert!(
            load_from_yaml("measures:\n  - key: acme.x\n    temporalSemantics: sideways\n")
                .is_err()
        );
        // Bad componentRole.
        assert!(load_from_yaml("measures:\n  - key: acme.x\n    componentRole: pivot\n").is_err());
        // Bad captureScope (tsk41).
        assert!(
            load_from_yaml("measures:\n  - key: acme.x\n    captureScope: sometimes\n").is_err()
        );
        // `per-path` is the tree-gauge scope and must be accepted.
        let cfg =
            load_from_yaml("measures:\n  - key: acme.x\n    captureScope: per-path\n").unwrap();
        assert_eq!(cfg.measures[0].capture_scope.as_deref(), Some("per-path"));
        // Default is `complete` — a capture restates the whole population.
        let resolved = resolve_measures(&[], &cfg.measures);
        assert_eq!(resolved[0].capture_scope, "per-path");
        let plain = load_from_yaml("measures:\n  - key: acme.y\n").unwrap();
        assert_eq!(
            resolve_measures(&[], &plain.measures)[0].capture_scope,
            "complete"
        );
        // Duplicate key.
        assert!(load_from_yaml("measures:\n  - key: acme.x\n  - key: acme.x\n").is_err());
        // A minimal valid measure parses.
        assert!(load_from_yaml("measures:\n  - key: acme.x\n").is_ok());
    }

    #[test]
    fn dimension_validation_rejects_bad_entries() {
        // Reserved namespace.
        assert!(load_from_yaml("dimensions:\n  - key: oxplow.foo\n").is_err());
        // Un-namespaced key.
        assert!(load_from_yaml("dimensions:\n  - key: foo\n").is_err());
        // Bad valueType.
        assert!(load_from_yaml("dimensions:\n  - key: acme.x\n    valueType: blob\n").is_err());
        // Duplicate key.
        assert!(load_from_yaml("dimensions:\n  - key: acme.x\n  - key: acme.x\n").is_err());
        // A minimal valid dimension parses (defaults to categorical).
        assert!(load_from_yaml("dimensions:\n  - key: acme.x\n").is_ok());
    }

    #[test]
    fn resolve_measures_precedence_and_defaults() {
        let global = vec![MeasureEntry {
            key: Some("acme.loc".into()),
            title: Some("Global LOC".into()),
            ..Default::default()
        }];
        // Project redefines the same key AND adds a fresh one.
        let project = vec![
            MeasureEntry {
                key: Some("acme.loc".into()),
                title: Some("Project LOC".into()),
                temporal_semantics: Some("additive".into()),
                ..Default::default()
            },
            MeasureEntry {
                key: Some("acme.churn".into()),
                ..Default::default()
            },
        ];
        let resolved = resolve_measures(&global, &project);
        assert_eq!(resolved.len(), 2, "same key merges, distinct key adds");
        let loc = resolved.iter().find(|m| m.key == "acme.loc").unwrap();
        assert_eq!(loc.title, "Project LOC", "project wins over global");
        assert_eq!(loc.scope, "project");
        assert_eq!(loc.temporal_semantics, "additive");
        let churn = resolved.iter().find(|m| m.key == "acme.churn").unwrap();
        // Defaults applied.
        assert_eq!(churn.title, "acme.churn");
        assert_eq!(churn.temporal_semantics, "semi-additive");
        assert_eq!(churn.component_role, "none");
    }

    #[test]
    fn resolve_dimensions_precedence_and_defaults() {
        let global = vec![DimensionEntry {
            key: Some("acme.license".into()),
            label: Some("Global label".into()),
            promote: false,
            ..Default::default()
        }];
        let project = vec![DimensionEntry {
            key: Some("acme.license".into()),
            label: Some("License".into()),
            promote: true,
            ..Default::default()
        }];
        let resolved = resolve_dimensions(&global, &project);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].label, "License", "project wins");
        assert_eq!(resolved[0].scope, "project");
        assert_eq!(resolved[0].value_type, "categorical", "default valueType");
        assert!(resolved[0].promote);
    }

    #[test]
    fn measures_and_dimensions_round_trip_through_write() {
        let dir = tempdir().unwrap();
        let cfg = OxplowConfig {
            measures: vec![MeasureEntry {
                key: Some("acme.loc".into()),
                unit: Some("lines".into()),
                temporal_semantics: Some("additive".into()),
                ..Default::default()
            }],
            dimensions: vec![DimensionEntry {
                key: Some("acme.license".into()),
                label: Some("License".into()),
                vocabulary: vec!["MIT".into(), "Apache-2.0".into()],
                promote: true,
                ..Default::default()
            }],
            ..default_config("test".into())
        };
        write_project_config(dir.path(), &cfg).unwrap();
        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        assert!(raw.contains("measures:"), "got:\n{raw}");
        assert!(raw.contains("dimensions:"), "got:\n{raw}");
        assert!(!raw.contains("null"), "minimal write, got:\n{raw}");
        let loaded = load_project_config(dir.path()).unwrap();
        assert_eq!(loaded.measures, cfg.measures);
        assert_eq!(loaded.dimensions, cfg.dimensions);
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
