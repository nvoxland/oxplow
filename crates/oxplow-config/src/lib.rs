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

/// How a configured metric is computed (the `compute:` block on a `metrics:`
/// entry). Mirrors [`PluginConfig`]'s runtime fields — the metric runner maps it
/// to a registered gauge collector. `report` is the report path for a
/// report-derived gauge; tree-derived gauges read the snapshot via `files()`
/// instead and leave it unset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
pub struct MetricComputeConfig {
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

/// One entry in the top-level `metrics:` block — the metric authoring surface
/// (epic tsk213, P3). Two forms, distinguished by which key is set:
/// - **`use:`** — enable an existing catalog metric by key (built-in/global),
///   optionally overriding `target`/`trigger`/`dimensions`/… for this project.
/// - **`key:`** — define a NEW metric (full definition + `compute:`).
///
/// The runner resolves these across the three scopes into `ResolvedMetric`s.
/// All non-discriminant fields are optional so both forms share one struct;
/// validation enforces the per-form rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default)]
#[serde(deny_unknown_fields)]
pub struct MetricEntry {
    /// `use:` form — the catalog key to enable.
    #[serde(rename = "use", default)]
    pub use_key: Option<String>,
    /// `key:` form — the new metric's namespaced key.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// `gauge` | `findings` | `test` | `coverage` | `event` (default `gauge`).
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub unit: Option<String>,
    /// `higher-better` | `lower-better` | `neutral` (default `neutral`).
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(rename = "defaultAgg", default)]
    pub default_agg: Option<String>,
    /// `effort` | `tree` | `file` | `entity`.
    #[serde(default)]
    pub grain: Option<String>,
    /// Language this metric measures (e.g. `rust`), for the catalog filter.
    #[serde(default)]
    pub language: Option<String>,
    /// One-line human description of what the metric measures (shown atop the
    /// Metric Detail page). Inherent to the definition — a `use:` can't override.
    #[serde(default)]
    pub description: Option<String>,
    /// Declared conformed-dimension keys this metric carries.
    #[serde(default)]
    pub dimensions: Vec<String>,
    #[serde(default)]
    pub target: Option<f64>,
    #[serde(rename = "warnAt", default)]
    pub warn_at: Option<f64>,
    #[serde(rename = "failAt", default)]
    pub fail_at: Option<f64>,
    /// `on-report` | `on-snapshot` | `on-effort-complete` | `manual` |
    /// `continuous` (default `manual`).
    #[serde(default)]
    pub trigger: Option<String>,
    /// How the metric computes (required for `key:` form; inherited from the
    /// catalog for `use:` form).
    #[serde(default)]
    pub compute: Option<MetricComputeConfig>,
}

/// A fully-resolved metric — the flat form the runner (oxplow-app) seeds into
/// `metric_definition` and runs. Produced by [`resolve_metrics`] after merging
/// the three scopes (built-in ∪ global ∪ project, precedence project > global >
/// built-in by key). Not serialized — purely an internal resolution result.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedMetric {
    pub key: String,
    pub title: String,
    pub kind: String,
    pub unit: Option<String>,
    pub direction: String,
    pub default_agg: String,
    pub grain: Option<String>,
    pub language: Option<String>,
    pub description: Option<String>,
    pub dimensions: Vec<String>,
    pub target: Option<f64>,
    pub warn_at: Option<f64>,
    pub fail_at: Option<f64>,
    /// `built-in` | `global` | `project`.
    pub scope: String,
    pub trigger: String,
    pub compute: MetricComputeConfig,
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
    /// `none` | `numerator` | `denominator` — ratio-base role (default `none`).
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
    /// Project-declared metrics (the `metrics:` block) — the author-able
    /// substrate surface (epic tsk213, P3). Each entry enables a catalog metric
    /// (`use:`) or defines a new one (`key:`). The runner resolves these across
    /// the built-in/global/project scopes; see [`resolve_metrics`].
    #[serde(default)]
    pub metrics: Vec<MetricEntry>,
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

/// Serialize one [`MetricEntry`] to a YAML mapping, omitting unset fields so a
/// hand-edited `metrics:` block stays minimal across UI-driven writes (mirrors
/// the per-field plugin serialization above).
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
    put_str("kind", &e.kind);
    put_str("unit", &e.unit);
    put_str("direction", &e.direction);
    put_str("defaultAgg", &e.default_agg);
    put_str("grain", &e.grain);
    put_str("language", &e.language);
    if !e.dimensions.is_empty() {
        m.insert(
            "dimensions".into(),
            serde_yaml::to_value(&e.dimensions).expect("dimensions serialize"),
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
    if let Some(t) = &e.trigger {
        m.insert("trigger".into(), t.clone().into());
    }
    if let Some(c) = &e.compute {
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
        m.insert("compute".into(), serde_yaml::Value::Mapping(cm));
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

/// Metric kinds a `metrics:` entry may declare.
const METRIC_KINDS: &[&str] = &["gauge", "findings", "test", "coverage", "event"];
/// Metric directions.
const METRIC_DIRECTIONS: &[&str] = &["higher-better", "lower-better", "neutral"];
/// Metric aggregations.
const METRIC_AGGS: &[&str] = &["last", "sum", "avg", "min", "max"];
/// Metric grains (the subject granularity a sample is recorded at).
const METRIC_GRAINS: &[&str] = &["effort", "tree", "file", "entity"];
/// Compute triggers.
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

        let kind = opt(e.kind);
        if let Some(k) = &kind {
            if !METRIC_KINDS.contains(&k.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] kind must be one of {METRIC_KINDS:?} (got \"{k}\")"
                )));
            }
        }
        let direction = opt(e.direction);
        if let Some(d) = &direction {
            if !METRIC_DIRECTIONS.contains(&d.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] direction must be one of {METRIC_DIRECTIONS:?} (got \"{d}\")"
                )));
            }
        }
        let default_agg = opt(e.default_agg);
        if let Some(a) = &default_agg {
            if !METRIC_AGGS.contains(&a.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] defaultAgg must be one of {METRIC_AGGS:?} (got \"{a}\")"
                )));
            }
        }
        let trigger = opt(e.trigger);
        if let Some(t) = &trigger {
            if !METRIC_TRIGGERS.contains(&t.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] trigger must be one of {METRIC_TRIGGERS:?} (got \"{t}\")"
                )));
            }
        }
        let grain = opt(e.grain);
        if let Some(g) = &grain {
            if !METRIC_GRAINS.contains(&g.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] grain must be one of {METRIC_GRAINS:?} (got \"{g}\")"
                )));
            }
        }

        // `use:` inherits compute from the catalog; `key:` must define it.
        let compute = match (is_define, e.compute) {
            (false, Some(_)) => {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] is a `use:` entry; it inherits `compute` from the catalog"
                )))
            }
            (false, None) => None,
            (true, None) => {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}] defines key \"{the_key}\" but has no `compute` block"
                )))
            }
            (true, Some(c)) => Some(validate_metric_compute(i, c)?),
        };

        let (use_key, key) = if is_define {
            (None, Some(the_key))
        } else {
            (Some(the_key), None)
        };
        out.push(MetricEntry {
            use_key,
            key,
            title: opt(e.title),
            kind,
            unit: opt(e.unit),
            direction,
            default_agg,
            grain,
            language: opt(e.language),
            description: opt(e.description),
            dimensions: e
                .dimensions
                .into_iter()
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty())
                .collect(),
            target: e.target,
            warn_at: e.warn_at,
            fail_at: e.fail_at,
            trigger,
            compute,
        });
    }
    Ok(out)
}

fn validate_metric_compute(
    i: usize,
    c: MetricComputeConfig,
) -> Result<MetricComputeConfig, ConfigError> {
    let runtime = c.runtime.trim().to_ascii_lowercase();
    if !PLUGIN_RUNTIMES.contains(&runtime.as_str()) {
        return Err(ConfigError::Invalid(format!(
            "metrics[{i}].compute.runtime must be jaq | starlark | exec (got \"{}\")",
            c.runtime
        )));
    }
    let input = match c.input.map(|s| s.trim().to_ascii_lowercase()) {
        Some(s) if !s.is_empty() => {
            if !PLUGIN_INPUTS.contains(&s.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "metrics[{i}].compute.input must be text | json | xml | lcov | lines (got \"{s}\")"
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
                "metrics[{i}].compute.entryFile is required (the script file path)"
            )))
        }
    };
    if Path::new(&entry_file).is_absolute() || entry_file.split('/').any(|c| c == "..") {
        return Err(ConfigError::Invalid(format!(
            "metrics[{i}].compute.entryFile must be a project-relative path without `..` (got \"{entry_file}\")"
        )));
    }
    Ok(MetricComputeConfig {
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

/// Resolve declared metrics across the three scopes into the flat
/// [`ResolvedMetric`] list the runner consumes. Definitions (`key:` entries)
/// from built-in, then global, then project build a catalog by key (later scope
/// wins → precedence project > global > built-in). The **project's** entries are
/// what's *active*: a `key:` entry defines + enables (scope `project`); a `use:`
/// entry enables a catalog metric, layering its override fields on top (scope =
/// the definition's scope). A `use:` referencing an unknown key is skipped with
/// a warning.
pub fn resolve_metrics(
    builtin: &[MetricEntry],
    global: &[MetricEntry],
    project: &[MetricEntry],
) -> Vec<ResolvedMetric> {
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
            // A project definition: it is its own resolved metric.
            out.push(resolve_one(k, "project", e, None));
        } else if let Some(uk) = e.use_key.as_deref() {
            match catalog.get(uk) {
                Some((scope, def)) => out.push(resolve_one(uk, scope, def, Some(e))),
                None => tracing::warn!(
                    key = uk,
                    "metrics: `use:` references an unknown catalog key; skipping"
                ),
            }
        }
    }
    out
}

/// Build a [`ResolvedMetric`] from a definition entry `def` (in `scope`),
/// optionally layering override fields from a `use:` entry `over`.
fn resolve_one(
    key: &str,
    scope: &str,
    def: &MetricEntry,
    over: Option<&MetricEntry>,
) -> ResolvedMetric {
    // Pick an override first, else the definition's value.
    let pick_str = |get: fn(&MetricEntry) -> &Option<String>| -> Option<String> {
        over.and_then(|o| get(o).clone())
            .or_else(|| get(def).clone())
    };
    let pick_f64 = |get: fn(&MetricEntry) -> Option<f64>| -> Option<f64> {
        over.and_then(get).or_else(|| get(def))
    };
    let dimensions = match over {
        Some(o) if !o.dimensions.is_empty() => o.dimensions.clone(),
        _ => def.dimensions.clone(),
    };
    ResolvedMetric {
        key: key.to_string(),
        title: pick_str(|e| &e.title).unwrap_or_else(|| key.to_string()),
        kind: pick_str(|e| &e.kind).unwrap_or_else(|| "gauge".into()),
        unit: pick_str(|e| &e.unit),
        direction: pick_str(|e| &e.direction).unwrap_or_else(|| "neutral".into()),
        default_agg: pick_str(|e| &e.default_agg).unwrap_or_else(|| "last".into()),
        grain: pick_str(|e| &e.grain),
        language: pick_str(|e| &e.language),
        // Description is inherent to the definition (like trigger) — a `use:`
        // override doesn't change what the metric *is*.
        description: def.description.clone(),
        dimensions,
        target: pick_f64(|e| e.target),
        warn_at: pick_f64(|e| e.warn_at),
        fail_at: pick_f64(|e| e.fail_at),
        scope: scope.to_string(),
        // trigger is inherent to the definition — when a metric is collected is a
        // property of what it measures, not a per-project knob. Like `compute`,
        // a `use:` entry can't override it (tsk290).
        trigger: def.trigger.clone().unwrap_or_else(|| "manual".into()),
        // compute always comes from the definition (use: entries can't set it).
        compute: def.compute.clone().unwrap_or_default(),
    }
}

/// Load metric definitions from the user-global scope
/// (`<global_dir>/metrics/*.yaml`). Each file is a `{ metrics: [ … ] }`
/// document (same shape as the `.oxplow/project.yaml` block). Best-effort: an unreadable
/// or malformed file is logged and skipped, never an error. Returns the entries
/// in filename order for deterministic precedence.
pub fn load_global_metric_entries(global_dir: &Path) -> Vec<MetricEntry> {
    #[derive(Deserialize)]
    struct GlobalMetricsDoc {
        #[serde(default)]
        metrics: Option<Vec<MetricEntry>>,
    }

    let mut out = Vec::new();
    for path in global_yaml_files(global_dir, "metrics") {
        let parsed = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_yaml::from_str::<GlobalMetricsDoc>(&raw).ok());
        match parsed {
            Some(doc) => match validate_metrics(doc.metrics) {
                Ok(entries) => out.extend(entries),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "skipping malformed global metrics file")
                }
            },
            None => {
                tracing::warn!(path = %path.display(), "skipping unreadable global metrics file")
            }
        }
    }
    out
}

/// List `*.yaml`/`*.yml` files under `<global_dir>/<subdir>`, sorted by filename
/// for deterministic precedence. Empty when the directory is absent. Shared by
/// the three global catalog loaders (metrics / measures / dimensions).
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
    let mut out = Vec::new();
    for path in global_yaml_files(global_dir, "measures") {
        let parsed = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_yaml::from_str::<Doc>(&raw).ok());
        match parsed {
            Some(doc) => match validate_measures(doc.measures) {
                Ok(entries) => out.extend(entries),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "skipping malformed global measures file")
                }
            },
            None => {
                tracing::warn!(path = %path.display(), "skipping unreadable global measures file")
            }
        }
    }
    out
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
    let mut out = Vec::new();
    for path in global_yaml_files(global_dir, "dimensions") {
        let parsed = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_yaml::from_str::<Doc>(&raw).ok());
        match parsed {
            Some(doc) => match validate_dimensions(doc.dimensions) {
                Ok(entries) => out.extend(entries),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "skipping malformed global dimensions file")
                }
            },
            None => {
                tracing::warn!(path = %path.display(), "skipping unreadable global dimensions file")
            }
        }
    }
    out
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
    kind: gauge
    title: "unsafe blocks"
    direction: lower-better
    unit: count
    trigger: on-snapshot
    dimensions: [language]
    compute: { runtime: starlark, entryFile: oxplow/metrics/unsafe.star }
  - use: myglobal.todo_density
    target: 5
"#,
        )
        .unwrap();
        assert_eq!(cfg.metrics.len(), 2);
        assert_eq!(cfg.metrics[0].key.as_deref(), Some("repo.unsafe_blocks"));
        assert_eq!(
            cfg.metrics[0]
                .compute
                .as_ref()
                .unwrap()
                .entry_file
                .as_deref(),
            Some("oxplow/metrics/unsafe.star")
        );
        assert_eq!(
            cfg.metrics[1].use_key.as_deref(),
            Some("myglobal.todo_density")
        );
        assert_eq!(cfg.metrics[1].target, Some(5.0));
    }

    #[test]
    fn metric_validation_rejects_bad_entries() {
        // Reserved namespace for a definition.
        assert!(load_from_yaml(
            "metrics:\n  - key: oxplow.foo\n    compute: { runtime: jaq, entryFile: x.jq }\n"
        )
        .is_err());
        // `key:` without compute.
        assert!(load_from_yaml("metrics:\n  - key: acme.foo\n    kind: gauge\n").is_err());
        // `use:` carrying compute.
        assert!(load_from_yaml(
            "metrics:\n  - use: acme.foo\n    compute: { runtime: jaq, entryFile: x.jq }\n"
        )
        .is_err());
        // both use and key.
        assert!(load_from_yaml("metrics:\n  - use: a.b\n    key: c.d\n").is_err());
        // un-namespaced key.
        assert!(load_from_yaml(
            "metrics:\n  - key: foo\n    compute: { runtime: jaq, entryFile: x.jq }\n"
        )
        .is_err());
        // bad runtime.
        assert!(load_from_yaml(
            "metrics:\n  - key: a.b\n    compute: { runtime: wasm, entryFile: x.jq }\n"
        )
        .is_err());
        // entryFile escaping the project root.
        assert!(load_from_yaml(
            "metrics:\n  - key: a.b\n    compute: { runtime: jaq, entryFile: ../x.jq }\n"
        )
        .is_err());
        // unknown grain.
        assert!(load_from_yaml(
            "metrics:\n  - key: a.b\n    grain: module\n    compute: { runtime: jaq, entryFile: x.jq }\n"
        )
        .is_err());
        // a valid grain is accepted.
        assert!(load_from_yaml(
            "metrics:\n  - key: a.b\n    grain: tree\n    compute: { runtime: jaq, entryFile: x.jq }\n"
        )
        .is_ok());
        // the same key declared twice (a `key:` define + a `use:`).
        assert!(load_from_yaml(
            "metrics:\n  - key: a.b\n    compute: { runtime: jaq, entryFile: x.jq }\n  - use: a.b\n    target: 5\n"
        )
        .is_err());
        // the same key defined twice.
        assert!(load_from_yaml(
            "metrics:\n  - key: a.b\n    compute: { runtime: jaq, entryFile: x.jq }\n  - key: a.b\n    compute: { runtime: jaq, entryFile: y.jq }\n"
        )
        .is_err());
    }

    fn define(key: &str, target: Option<f64>) -> MetricEntry {
        MetricEntry {
            key: Some(key.into()),
            target,
            compute: Some(MetricComputeConfig {
                runtime: "starlark".into(),
                entry_file: Some("m.star".into()),
                ..Default::default()
            }),
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
        // compute comes from the (global) definition.
        assert_eq!(resolved[0].compute.entry_file.as_deref(), Some("m.star"));
    }

    #[test]
    fn resolve_project_definition_is_active_and_scoped() {
        let project = vec![define("acme.loc", None)];
        let resolved = resolve_metrics(&[], &[], &project);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].scope, "project");
        assert_eq!(resolved[0].kind, "gauge");
        assert_eq!(resolved[0].trigger, "manual");
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
            "metrics:\n  - key: myglobal.todo\n    compute: { runtime: jaq, entryFile: t.jq }\n",
        )
        .unwrap();
        // A malformed file is skipped, not fatal.
        std::fs::write(metrics_dir.join("bad.yaml"), "metrics:\n  - key: nodot\n").unwrap();
        let entries = load_global_metric_entries(dir.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key.as_deref(), Some("myglobal.todo"));
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
