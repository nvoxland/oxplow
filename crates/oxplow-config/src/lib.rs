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

pub use oxplow_domain::AgentKind;

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
/// How many changed files one snapshot's symbol collection asks the
/// language servers about (P5.C6).
pub const DEFAULT_SYMBOLS_MAX_FILES_PER_SNAPSHOT: u32 = 50;
const DEFAULT_INJECT_SESSION_CONTEXT: bool = true;

/// An agent oxplow talks to over the Agent Client Protocol (tsk335): a
/// program that speaks ACP on its stdio. Presets cover the common ones
/// ([`acp_presets`]); `acpAgents:` in `.oxplow/project.yaml` adds or
/// overrides by name. A project entry names a program from the repo, so it
/// runs only once a person approved it (see `exec_consent`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AcpAgentConfig {
    /// Short name a thread picks it by (`claude`, `gemini`, `my-agent`).
    pub name: String,
    /// The program: a name on PATH or a path.
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment for the program.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

/// Where an ACP agent definition came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum AcpAgentSource {
    /// Built into oxplow; runs without approval.
    Preset,
    /// The project's `acpAgents:`; needs a person's approval to run.
    Project,
}

/// The built-in ACP agents: the vendors' ACP adapters.
pub fn acp_presets() -> Vec<AcpAgentConfig> {
    let preset = |name: &str, command: &str, args: &[&str]| AcpAgentConfig {
        name: name.into(),
        command: command.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        env: Default::default(),
    };
    vec![
        preset("claude", "claude-agent-acp", &[]),
        preset("gemini", "gemini", &["--acp"]),
        preset("codex", "codex-acp", &[]),
    ]
}

/// Presets, then the project's entries (a project entry replaces a
/// preset of the same name), in that order.
pub fn resolve_acp_agents(project: &[AcpAgentConfig]) -> Vec<(AcpAgentConfig, AcpAgentSource)> {
    let mut out: Vec<(AcpAgentConfig, AcpAgentSource)> = acp_presets()
        .into_iter()
        .map(|a| (a, AcpAgentSource::Preset))
        .collect();
    for a in project {
        match out.iter().position(|(p, _)| p.name == a.name) {
            Some(i) => out[i] = (a.clone(), AcpAgentSource::Project),
            None => out.push((a.clone(), AcpAgentSource::Project)),
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
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

/// A fact predicate on a `metrics:` spec (the `filter:` block) — the config
/// mirror of the engine's `FactFilter` (epic tsk12). A conjunctive predicate
/// keeping only the facts that match before aggregation: `minValue` for a
/// count-over-threshold (complexity ≥ N), `severity` for a lint slice, `dimEq`
/// for a conformed-dimension slice (`[oxplow.rule, unsafe_block]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default, schemars::JsonSchema)]
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
    /// Entity metric (tsk322): the `v_*` view it aggregates, instead of a
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
/// authoring surface (epic tsk12, workstream E). A measure is a *type of atomic
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
    /// Entity dimension (tsk322): the `v_*` view it slices, for entity
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
    /// Optional coverage-free counterpart to `test_command`, for the red/green
    /// loop (tsk171). It must still emit a test report (JUnit) so the
    /// progression lands in the effort's Tests panel, but it skips coverage
    /// instrumentation and should accept a filter argument.
    ///
    /// This exists because the alternative is worse. When the only
    /// report-emitting command is a full instrumented run of the whole suite,
    /// "route every invocation through it" is unfollowable in a TDD loop, so it
    /// gets dropped — and then NONE of the red→green runs are recorded. A
    /// weaker rule that is actually followed beats a stricter one that isn't.
    #[serde(rename = "fastTestCommand")]
    pub fast_test_command: Option<String>,
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
    pub snapshot_max_file_bytes: u64,
    /// How long the Stop hook waits for the turn-end snapshot, in ms.
    /// A take that runs longer keeps going in the background and is
    /// recorded as over budget. Default 2000.
    #[serde(rename = "snapshotTurnBudgetMs")]
    pub snapshot_turn_budget_ms: u64,
    /// How many of a snapshot's changed files the symbol collector asks
    /// the running language servers about; the rest are recorded as
    /// skipped. Default 50.
    #[serde(rename = "symbolsMaxFilesPerSnapshot")]
    pub symbols_max_files_per_snapshot: u32,
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
    /// YAML is handled by `write_project_config`, which builds its mapping by
    /// hand rather than through this derive.
    #[serde(rename = "iconTint")]
    pub icon_tint: Option<String>,
    /// Per-project collection profile (test + coverage instrumentation).
    pub collection: CollectionConfig,
    /// Project-declared metric SPECS (the `metrics:` block) — the author-able
    /// read surface (epic tsk12, E). Each entry enables a catalog metric
    /// (`use:`) or defines a new spec (`key:`) over a measure. The runner resolves
    /// these across the built-in/global/project scopes; see [`resolve_metrics`].
    #[serde(default)]
    pub metrics: Vec<MetricEntry>,
    /// The project's collectors (the `collectors:` block, P7.B3): fact
    /// producers that run on their trigger and record facts on the measures
    /// they declare. Owner [`collectors::PROJECT`].
    #[serde(default)]
    pub collectors: Vec<collectors::CollectorSpec>,
    /// The `collectors:` block as the file has it: what the writer puts
    /// back (a parsed spec isn't the shape the file declares).
    #[serde(skip)]
    pub collectors_yaml: Option<serde_json::Value>,
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
    /// Project-declared architectural zones (the `zones:` block) — an
    /// ORDERED table, first match wins. Empty (the default) means oxplow
    /// classifies nothing: every file is `other` and the Change-analysis
    /// zone surfaces stay empty until the project declares its own.
    #[serde(default)]
    pub zones: Vec<ZoneRuleConfig>,
    /// Per-agent launch model overrides, e.g.
    /// `agentModels: { opencode: "github-copilot/gpt-5-mini" }`.
    /// Only opencode consumes this today (its `-m provider/model`
    /// flag); claude/codex launch with their own defaults. Absent
    /// entries fall back to the built-in constant.
    #[serde(rename = "agentModels")]
    pub agent_models: std::collections::BTreeMap<AgentKind, String>,
    /// The project's ACP agents (`acpAgents:`), layered over
    /// [`acp_presets`] by [`resolve_acp_agents`].
    #[serde(rename = "acpAgents")]
    pub acp_agents: Vec<AcpAgentConfig>,
    /// The project's extension provider instances
    /// (`extensionInstances: { "<ext>/<id>": { enabled, config } }`).
    #[serde(rename = "extensionInstances")]
    pub extension_instances: std::collections::BTreeMap<String, ExtensionInstanceConfig>,
    /// Each swappable capability's active provider
    /// (`activeProviders: { work_items: linear }`); a capability absent
    /// here keeps oxplow's own.
    #[serde(rename = "activeProviders")]
    pub active_providers: std::collections::BTreeMap<String, String>,
    /// Core components no extension's replacement may take over
    /// (`replacementsOff: [work_item.board]`): oxplow's own shows there
    /// even when the active provider's extension replaces it.
    #[serde(rename = "replacementsOff")]
    pub replacements_off: std::collections::BTreeSet<String>,
    /// This project's AI role assignments (`ai: { roles: … }`), layered
    /// over the user-global `ai.yaml`. Keyed by role name (one of
    /// [`AI_ROLE_NAMES`]). Provider ids refer to each person's `ai.yaml`.
    #[serde(rename = "aiRoles")]
    pub ai_roles: std::collections::BTreeMap<String, AiRoleOverride>,
    /// Extensions turned off for this project (`extensions: { disabled:
    /// [...] }`), bundled ones included. Committed, so it's team-wide.
    #[serde(rename = "extensionsDisabled")]
    pub extensions_disabled: Vec<String>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawExtensionsBlock {
    #[serde(default)]
    disabled: Vec<String>,
}

/// Just the `extensions.disabled` list of `project_dir/.oxplow/project.yaml`,
/// without loading (or failing on) the rest of the file. The extension
/// loader calls this on every load, per worktree.
pub fn disabled_extensions(project_dir: impl AsRef<Path>) -> Vec<String> {
    #[derive(Deserialize)]
    struct OnlyExtensions {
        #[serde(default)]
        extensions: Option<RawExtensionsBlock>,
    }
    std::fs::read_to_string(config_path(project_dir.as_ref()))
        .ok()
        .and_then(|raw| serde_yaml::from_str::<OnlyExtensions>(&raw).ok())
        .and_then(|c| c.extensions)
        .map(|b| b.disabled)
        .unwrap_or_default()
}

/// Role names `ai.roles` accepts. Must match `oxplow_ai::config::Role`
/// (a test in oxplow-app checks).
pub const AI_ROLE_NAMES: [&str; 6] = ["main", "fast", "summarize", "embed", "decide", "review"];

/// One `ai.roles` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AiRoleOverride {
    pub provider: String,
    pub model: String,
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
    /// Enabled agent implementations, in priority order; the first is the default for new threads.
    #[serde(default)]
    agents: Option<Vec<AgentKind>>,
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
    /// How many of a snapshot's changed files the symbol collector asks the running language servers about (`v_symbol`); over the bound is recorded as skipped. Default 50; 0 turns collection off.
    #[serde(rename = "symbolsMaxFilesPerSnapshot", default)]
    symbols_max_files_per_snapshot: Option<f64>,
    /// Inject the session-context block into every agent prompt.
    #[serde(rename = "injectSessionContext", default)]
    inject_session_context: Option<bool>,
    /// Hex colour composited behind the app icon so windows are tellable apart (macOS).
    #[serde(rename = "iconTint", default)]
    icon_tint: Option<String>,
    /// Test and coverage collection: commands, report paths, run patterns, plugins. Runs programs.
    #[serde(default)]
    collection: Option<RawCollectionBlock>,
    /// Metric specs: enable a catalog metric (`use`) or define one (`key`) over a measure.
    #[serde(default)]
    metrics: Option<Vec<MetricEntry>>,
    /// Collectors: scripts (starlark, jaq) and programs (`runtime: exec`, approved by a person) that record facts on declared measures when their trigger fires. Runs programs.
    #[serde(default)]
    collectors: Option<serde_json::Value>,
    /// Custom fact types collectors may emit.
    #[serde(default)]
    measures: Option<Vec<MeasureEntry>>,
    /// Custom slice axes for facts.
    #[serde(default)]
    dimensions: Option<Vec<DimensionEntry>>,
    /// Architectural zones: an ORDERED rule table, first match wins; `other`/`external` are reserved labels.
    #[serde(default)]
    zones: Option<Vec<RawZoneRule>>,
    /// Per-agent launch model overrides, e.g. `{ opencode: "github-copilot/gpt-5-mini" }`.
    #[serde(rename = "agentModels", default)]
    agent_models: Option<std::collections::BTreeMap<AgentKind, String>>,
    /// The project's ACP agents, layered over the presets. Runs programs.
    #[serde(rename = "acpAgents", default)]
    acp_agents: Option<Vec<AcpAgentConfig>>,
    /// Instances of extension providers, by `<extension>/<instance id>`: `{ enabled, config, provider? }` (the provider's instance config; `provider` names which of the extension's providers an instance with its own id is). Enabling one runs its program once this machine approved it.
    #[serde(rename = "extensionInstances", default)]
    extension_instances: Option<std::collections::BTreeMap<String, ExtensionInstanceConfig>>,
    /// Each capability's active provider, by instance id: `{ work_items: linear }` (a provider's default instance has the provider's id). New work items file there; absent, oxplow's own. A provider that isn't running is a failure, never a fallback.
    #[serde(rename = "activeProviders", default)]
    active_providers: Option<std::collections::BTreeMap<String, String>>,
    /// Core components that stay oxplow's own, by target: `[work_item.board]`. Listed, an extension's replacement of it (the active provider's `ui.replacements`) isn't shown.
    #[serde(rename = "replacementsOff", default)]
    replacements_off: Option<Vec<String>>,
    /// AI role assignments `{ roles: { <role>: { provider, model } } }`, layered over the user's ai.yaml.
    #[serde(default)]
    ai: Option<RawAiBlock>,
    /// Extensions turned off for this project: `{ disabled: [names] }`.
    #[serde(default)]
    extensions: Option<RawExtensionsBlock>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawReport {
    path: String,
    format: String,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
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

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawCollectionBlock {
    #[serde(rename = "testCommand", default)]
    test_command: Option<String>,
    #[serde(rename = "fastTestCommand", default)]
    fast_test_command: Option<String>,
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

    if !config_path.exists() {
        info!(
            config_path = %config_path.display(),
            agents = ?vec![AgentKind::default()],
            "project config not found; using defaults"
        );
        return Ok(default_config(fallback_name));
    }

    let raw = std::fs::read_to_string(&config_path)?;
    let doc: serde_yaml::Value = serde_yaml::from_str(&raw)?;
    let config = parse_project_config(doc, &fallback_name)?;
    info!(
        config_path = %config_path.display(),
        agents = ?config.agents,
        project_name = %config.project_name,
        lsp_servers = config.lsp_servers.len(),
        "loaded project config"
    );
    Ok(config)
}

/// Validate a parsed `.oxplow/project.yaml` document exactly as
/// `load_project_config` does — the one path every config write takes,
/// whether a person edits the file or a command sets a key.
pub fn parse_project_config(
    doc: serde_yaml::Value,
    fallback_name: &str,
) -> Result<OxplowConfig, ConfigError> {
    if doc.get("gauges").is_some() {
        return Err(ConfigError::Invalid(GAUGES_RETIRED.into()));
    }
    let parsed: RawConfig = serde_yaml::from_value(doc)?;
    validate(parsed, fallback_name)
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

    // Every key the file schema knows (`keys::config_keys`) is ours to
    // render; anything else found in an existing file is copied through
    // verbatim (best-effort, since YAML→serde_yaml::Value→YAML is still
    // lossy on style). Deriving the set from the schema is what keeps a
    // new field from being "an extra" that re-inserts its stale on-disk
    // value over the one just written (tsk164, tsk411).
    let existing_extras: serde_yaml::Mapping = if path.exists() {
        match std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_yaml::from_str::<serde_yaml::Value>(&raw).ok())
        {
            Some(serde_yaml::Value::Mapping(m)) => m
                .into_iter()
                .filter(|(k, _)| match k {
                    serde_yaml::Value::String(s) => !keys::is_config_key(s),
                    _ => true,
                })
                .collect(),
            _ => serde_yaml::Mapping::new(),
        }
    } else {
        serde_yaml::Mapping::new()
    };

    let mut doc = render_project_config(config, &fallback_name);
    // Carry forward any unknown top-level keys the user (or a
    // sibling tool) added to .oxplow/project.yaml.
    for (k, v) in existing_extras {
        doc.insert(k, v);
    }

    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc))?;
    std::fs::write(path, yaml)?;
    Ok(())
}

/// The `.oxplow/project.yaml` document for `config`: only keys whose
/// value differs from the default, so a hand-edited file stays minimal.
/// `fallback_name` is the project name that needs no `projectName` key.
/// Pure — `write_project_config` adds the file's unknown keys and writes.
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
    put(
        "agents",
        to_yaml(&config.agents),
        config.agents != vec![AgentKind::default()],
    );
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
        "symbolsMaxFilesPerSnapshot",
        config.symbols_max_files_per_snapshot.into(),
        config.symbols_max_files_per_snapshot != DEFAULT_SYMBOLS_MAX_FILES_PER_SNAPSHOT,
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
        let c = &config.collection;
        let mut col = serde_yaml::Mapping::new();
        if let Some(v) = &c.test_command {
            col.insert("testCommand".into(), v.clone().into());
        }
        if let Some(v) = &c.fast_test_command {
            col.insert("fastTestCommand".into(), v.clone().into());
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
            col.insert("testRunPatterns".into(), to_yaml(&c.test_run_patterns));
        }
        if !c.analysis_run_patterns.is_empty() {
            col.insert(
                "analysisRunPatterns".into(),
                to_yaml(&c.analysis_run_patterns),
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
                    m.insert("formats".into(), to_yaml(&p.formats));
                    m.insert("runtime".into(), p.runtime.clone().into());
                    if let Some(input) = &p.input {
                        m.insert("input".into(), input.clone().into());
                    }
                    if let Some(entry_file) = &p.entry_file {
                        m.insert("entryFile".into(), entry_file.clone().into());
                    }
                    if !p.args.is_empty() {
                        m.insert("args".into(), to_yaml(&p.args));
                    }
                    serde_yaml::Value::Mapping(m)
                })
                .collect();
            col.insert("plugins".into(), serde_yaml::Value::Sequence(plugins));
        }
        let set = !col.is_empty();
        put("collection", serde_yaml::Value::Mapping(col), set);
    }
    put(
        "metrics",
        serde_yaml::Value::Sequence(config.metrics.iter().map(minimal_yaml).collect()),
        !config.metrics.is_empty(),
    );
    put(
        "collectors",
        config
            .collectors_yaml
            .as_ref()
            .map_or(serde_yaml::Value::Null, to_yaml),
        config.collectors_yaml.is_some(),
    );
    put(
        "measures",
        serde_yaml::Value::Sequence(config.measures.iter().map(minimal_yaml).collect()),
        !config.measures.is_empty(),
    );
    put(
        "dimensions",
        serde_yaml::Value::Sequence(
            config
                .dimensions
                .iter()
                .map(dimension_entry_to_yaml)
                .collect(),
        ),
        !config.dimensions.is_empty(),
    );
    put("zones", to_yaml(&config.zones), !config.zones.is_empty());
    put(
        "agentModels",
        to_yaml(&config.agent_models),
        !config.agent_models.is_empty(),
    );
    put(
        "acpAgents",
        to_yaml(&config.acp_agents),
        !config.acp_agents.is_empty(),
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
        "replacementsOff",
        to_yaml(&config.replacements_off),
        !config.replacements_off.is_empty(),
    );
    {
        let mut ext = serde_yaml::Mapping::new();
        ext.insert("disabled".into(), to_yaml(&config.extensions_disabled));
        put(
            "extensions",
            serde_yaml::Value::Mapping(ext),
            !config.extensions_disabled.is_empty(),
        );
    }
    {
        let mut ai = serde_yaml::Mapping::new();
        ai.insert("roles".into(), to_yaml(&config.ai_roles));
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

/// `measures:` / `collectors:` / `metrics:` entries as a
/// `.oxplow/project.yaml` snippet (empty lists omitted), each entry minimal
/// as the writer makes it. What a scaffold hands an agent to merge into the
/// file itself (tsk391). A collector is given as the YAML it's declared as.
pub fn entries_yaml(
    measures: &[MeasureEntry],
    collectors: &[serde_yaml::Value],
    metrics: &[MetricEntry],
) -> String {
    let mut doc = serde_yaml::Mapping::new();
    let mut put = |key: &str, seq: Vec<serde_yaml::Value>| {
        if !seq.is_empty() {
            doc.insert(key.into(), serde_yaml::Value::Sequence(seq));
        }
    };
    put("measures", measures.iter().map(minimal_yaml).collect());
    put("collectors", collectors.to_vec());
    put("metrics", metrics.iter().map(minimal_yaml).collect());
    serde_yaml::to_string(&doc).unwrap_or_default()
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

/// Serialize one [`DimensionEntry`] to a YAML mapping, omitting unset fields.
fn dimension_entry_to_yaml(e: &DimensionEntry) -> serde_yaml::Value {
    // `promote` is a plain bool; only `true` is worth writing.
    let mut v = minimal_yaml(e);
    if let serde_yaml::Value::Mapping(m) = &mut v {
        if m.get("promote") == Some(&serde_yaml::Value::Bool(false)) {
            m.remove("promote");
        }
    }
    v
}

fn default_config(project_name: String) -> OxplowConfig {
    OxplowConfig {
        agents: vec![AgentKind::default()],
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
        symbols_max_files_per_snapshot: DEFAULT_SYMBOLS_MAX_FILES_PER_SNAPSHOT,
        inject_session_context: DEFAULT_INJECT_SESSION_CONTEXT,
        icon_tint: None,
        collection: CollectionConfig::default(),
        metrics: Vec::new(),
        collectors: Vec::new(),
        collectors_yaml: None,
        measures: Vec::new(),
        dimensions: Vec::new(),
        zones: Vec::new(),
        agent_models: Default::default(),
        acp_agents: Vec::new(),
        extension_instances: std::collections::BTreeMap::new(),
        active_providers: std::collections::BTreeMap::new(),
        replacements_off: std::collections::BTreeSet::new(),
        ai_roles: Default::default(),
        extensions_disabled: Vec::new(),
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
    /// `provider.sync`).
    // No `skip_serializing_if` (specta's unified mode can't express it):
    // `render_project_config` leaves an absent one out of the file.
    #[serde(rename = "syncMinutes", default)]
    pub sync_minutes: Option<u32>,
    /// Which of the extension's providers this is an instance of, for an
    /// instance whose id isn't a provider's own (`tracker/linear_acme:
    /// { provider: linear }` — a second Linear workspace). Absent, the
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

    /// Validate and write to `dir/instances.yaml`.
    pub fn save(&self, dir: &Path) -> Result<(), ConfigError> {
        let checked = self.clone().validated()?;
        let doc = serde_json::json!({ "instances": instances_as_written(&checked.instances) });
        let text =
            serde_yaml::to_string(&doc).map_err(|e| ConfigError::Instances(e.to_string()))?;
        std::fs::create_dir_all(dir).map_err(|e| ConfigError::Instances(e.to_string()))?;
        std::fs::write(dir.join(INSTANCES_FILE), text)
            .map_err(|e| ConfigError::Instances(e.to_string()))
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
        let well_formed = key
            .split_once('/')
            .is_some_and(|(ext, id)| !ext.is_empty() && is_instance_id(id));
        if !well_formed {
            return Err(ConfigError::Invalid(format!(
                "extensionInstances: `{key}` must be `<extension>/<instance id>` (the id lowercase \
                 snake_case: a provider's own, or another instance's with `provider: <id>`)"
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

/// The capabilities whose active provider a project may choose
/// (`activeProviders`).
pub const SWAPPABLE_CAPABILITIES: &[&str] = &["work_items"];

/// Validate `replacementsOff:`: each a replaceable component's target
/// ([`oxplow_domain::replaceable`]); a repeat is one.
fn validate_replacements_off(
    raw: Vec<String>,
) -> Result<std::collections::BTreeSet<String>, ConfigError> {
    for target in &raw {
        if oxplow_domain::replaceable::replaceable(target).is_none() {
            return Err(ConfigError::Invalid(format!(
                "replacementsOff: `{target}` isn't a replaceable component ({})",
                oxplow_domain::replaceable::targets()
            )));
        }
    }
    Ok(raw.into_iter().collect())
}

/// Validate `activeProviders:`: a swappable capability each, naming an
/// instance by its id (a provider's default instance has the provider's).
fn validate_active_providers(
    raw: std::collections::BTreeMap<String, String>,
) -> Result<std::collections::BTreeMap<String, String>, ConfigError> {
    for (capability, provider) in &raw {
        if !SWAPPABLE_CAPABILITIES.contains(&capability.as_str()) {
            return Err(ConfigError::Invalid(format!(
                "activeProviders: `{capability}` isn't a capability whose provider can be chosen \
                 ({})",
                SWAPPABLE_CAPABILITIES.join(", ")
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

/// Validate `acpAgents:`: lowercase-dash names, unique, with a command.
fn validate_acp_agents(raw: Vec<AcpAgentConfig>) -> Result<Vec<AcpAgentConfig>, ConfigError> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(raw.len());
    for (i, mut a) in raw.into_iter().enumerate() {
        a.name = a.name.trim().to_string();
        a.command = a.command.trim().to_string();
        let ok_name = !a.name.is_empty()
            && a.name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok_name {
            return Err(ConfigError::Invalid(format!(
                "acpAgents[{i}].name must be lowercase letters, digits and dashes (got \"{}\")",
                a.name
            )));
        }
        if a.command.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "acpAgents[{i}] ({}) needs a command",
                a.name
            )));
        }
        if !seen.insert(a.name.clone()) {
            return Err(ConfigError::Invalid(format!(
                "acpAgents: `{}` is declared twice",
                a.name
            )));
        }
        out.push(a);
    }
    Ok(out)
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

    let symbols_max_files_per_snapshot = match raw.symbols_max_files_per_snapshot {
        Some(n) if !n.is_finite() || n < 0.0 => {
            return Err(ConfigError::Invalid(
                "symbolsMaxFilesPerSnapshot must be a number >= 0".into(),
            ));
        }
        Some(n) => n.floor() as u32,
        None => DEFAULT_SYMBOLS_MAX_FILES_PER_SNAPSHOT,
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

    let collection = validate_collection(raw.collection)?;
    let metrics = validate_metrics(raw.metrics)?;
    let collectors_yaml = raw.collectors;
    let collectors = parse_project_collectors(collectors_yaml.as_ref())?;
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

    let acp_agents = validate_acp_agents(raw.acp_agents.unwrap_or_default())?;
    let extension_instances =
        validate_extension_instances(raw.extension_instances.unwrap_or_default())?;
    let active_providers = validate_active_providers(raw.active_providers.unwrap_or_default())?;
    let replacements_off = validate_replacements_off(raw.replacements_off.unwrap_or_default())?;

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
        symbols_max_files_per_snapshot,
        inject_session_context,
        icon_tint,
        collection,
        metrics,
        collectors,
        collectors_yaml,
        measures,
        dimensions,
        zones,
        agent_models,
        acp_agents,
        extension_instances,
        active_providers,
        replacements_off,
        ai_roles: validate_ai_roles(raw.ai)?,
        extensions_disabled: raw.extensions.map(|b| b.disabled).unwrap_or_default(),
    })
}

/// Validate `ai.roles`: known role names, non-empty provider and model.
fn validate_ai_roles(
    raw: Option<RawAiBlock>,
) -> Result<std::collections::BTreeMap<String, AiRoleOverride>, ConfigError> {
    let roles = raw.map(|b| b.roles).unwrap_or_default();
    for (role, o) in &roles {
        if !AI_ROLE_NAMES.contains(&role.as_str()) {
            return Err(ConfigError::Invalid(format!(
                "ai.roles.{role}: unknown role (use one of {})",
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
/// Formula binary ops a `metrics:` spec may declare (`ratio` aliases `div`).
const METRIC_FORMULA_OPS: &[&str] = &["add", "sub", "mul", "div", "ratio"];
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
const MEASURE_COMPONENT_ROLES: &[&str] = &["none", "numerator", "denominator"];
/// Value types a `dimensions:` entry may declare (mirrors the `dimension`
/// table's CHECK).
const DIMENSION_VALUE_TYPES: &[&str] = &["categorical", "numeric", "temporal", "entity-ref"];

/// Validate the top-level `metrics:` block (the project scope). Mirrors the
/// plugin rules: namespaced keys, `oxplow.*` reserved for built-ins,
/// project-relative `entryFile`, known runtime/kind/trigger. Each entry must be
/// exactly one of the `use:` or `key:` forms. Returns the cleaned entries (the
/// three-scope resolution happens later in [`resolve_metrics`]).
pub fn validate_metrics(raw: Option<Vec<MetricEntry>>) -> Result<Vec<MetricEntry>, ConfigError> {
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
            entity,
            where_,
            time,
            value,
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

/// Gauges were folded into collectors (P7.B3): what loading a `gauges:`
/// block says.
pub const GAUGES_RETIRED: &str = "`gauges:` is now `collectors:` (each gauge is a collector that \
     records facts). Run `oxplow plugin migrate --project` to rewrite the block in place.";

/// The project's `collectors:` block, parsed with owner
/// [`collectors::PROJECT`]: every error at once, naming each collector.
fn parse_project_collectors(
    raw: Option<&serde_json::Value>,
) -> Result<Vec<collectors::CollectorSpec>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let value: serde_yaml::Value = to_yaml(raw);
    let (specs, errors) = collectors::parse_collectors(
        collectors::PROJECT,
        &value,
        &oxplow_domain::events::schema::is_core_type,
    );
    if errors.is_empty() {
        Ok(specs)
    } else {
        Err(ConfigError::Invalid(errors.join("; ")))
    }
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
    extensions: &[ExtensionLayer<MetricEntry>],
    project: &[MetricEntry],
) -> Vec<ResolvedSpec> {
    // Catalog of definitions by key, with the scope each came from.
    let mut catalog: std::collections::HashMap<String, (String, &MetricEntry)> =
        std::collections::HashMap::new();
    for (scope, entries) in scoped_layers(builtin, global, extensions, project) {
        for e in entries {
            if let Some(k) = e.key.as_deref() {
                catalog.insert(k.to_string(), (scope.clone(), e));
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
    // An enabled extension's own definitions are active unless the project
    // mentions the key (a `use:` override or disable marker, or its own
    // definition, all handled above).
    let mentioned: std::collections::HashSet<&str> = project
        .iter()
        .filter_map(|e| e.key.as_deref().or(e.use_key.as_deref()))
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
                    out.push(resolve_one(k, scope, def, None));
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
    format!("extension:{name}")
}

/// The extension name in an `extension:<name>` scope.
pub fn scope_extension(scope: &str) -> Option<&str> {
    scope.strip_prefix("extension:")
}

/// Scopes in precedence order (later wins): built-in, global, each
/// extension, project.
fn scoped_layers<'a, T>(
    builtin: &'a [T],
    global: &'a [T],
    extensions: &'a [ExtensionLayer<T>],
    project: &'a [T],
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
    out.push(("project".to_string(), project));
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
            .map(|d| validate_metrics(d.metrics).map_err(|e| e.to_string()))
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
pub fn validate_measures(raw: Option<Vec<MeasureEntry>>) -> Result<Vec<MeasureEntry>, ConfigError> {
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
pub fn validate_dimensions(
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
    project: &[MeasureEntry],
) -> Vec<ResolvedMeasure> {
    let mut out: Vec<ResolvedMeasure> = Vec::new();
    let mut pos: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (scope, entries) in scoped_layers(&[], global, extensions, project) {
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
    project: &[DimensionEntry],
) -> Vec<ResolvedDimension> {
    let mut out: Vec<ResolvedDimension> = Vec::new();
    let mut pos: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (scope, entries) in scoped_layers(&[], global, extensions, project) {
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
        fast_test_command: opt_trimmed(raw.fast_test_command),
        reports,
        test_run_patterns,
        analysis_run_patterns,
        agent_hint: opt_trimmed(raw.agent_hint),
        plugins,
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
        let config = parse("activeProviders: { work_items: linear }\n").unwrap();
        assert_eq!(config.active_providers["work_items"], "linear");
        let doc = render_project_config(&config, "demo");
        let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc)).unwrap();
        assert!(
            yaml.contains("activeProviders:\n  work_items: linear"),
            "{yaml}"
        );
        let err = parse("activeProviders: { vcs: jj }\n").unwrap_err();
        assert!(err.to_string().contains("work_items"), "{err}");
        let err = parse("activeProviders: { work_items: Linear-App }\n").unwrap_err();
        assert!(err.to_string().contains("instance id"), "{err}");
        assert!(parse("agents: [claude]\n")
            .unwrap()
            .active_providers
            .is_empty());
    }

    /// P9.A1: `replacementsOff` names core components a replacement may
    /// not take over; an unknown target is an error listing the real ones.
    #[test]
    fn replacements_off_names_replaceable_components() {
        let parse = |yaml: &str| parse_project_config(serde_yaml::from_str(yaml).unwrap(), "demo");
        let config = parse("replacementsOff: [work_item.board, work_item.board]\n").unwrap();
        assert_eq!(
            config.replacements_off.iter().collect::<Vec<_>>(),
            vec!["work_item.board"]
        );
        let doc = render_project_config(&config, "demo");
        let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc)).unwrap();
        assert!(
            yaml.contains("replacementsOff:\n- work_item.board"),
            "{yaml}"
        );
        let err = parse("replacementsOff: [vcs.history.graph]\n").unwrap_err();
        assert!(
            err.to_string().contains("work_item.board"),
            "lists what can be: {err}"
        );
        let plain = parse("agents: [claude]\n").unwrap();
        assert!(plain.replacements_off.is_empty());
        let doc = render_project_config(&plain, "demo");
        let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc)).unwrap();
        assert!(!yaml.contains("replacementsOff"), "{yaml}");
        // What renders a core region is a person's call, like the active
        // provider.
        assert!(crate::keys::HUMAN_ONLY_KEYS.contains(&"replacementsOff"));
    }

    /// P9.B1: an instance is `<extension>/<instance id>`; a provider's
    /// default instance has the provider's id, and another one says which
    /// provider it is.
    #[test]
    fn an_instance_key_is_an_extension_and_a_snake_case_instance_id() {
        let parse = |yaml: &str| parse_project_config(serde_yaml::from_str(yaml).unwrap(), "demo");
        let config = parse(
            "extensionInstances:\n  my-tracker/linear: { enabled: true }\n  my-tracker/linear_acme: { enabled: true, provider: linear }\n",
        )
        .unwrap();
        assert_eq!(
            config.extension_instances["my-tracker/linear"].provider,
            None
        );
        assert_eq!(
            config.extension_instances["my-tracker/linear_acme"]
                .provider
                .as_deref(),
            Some("linear")
        );
        let doc = render_project_config(&config, "demo");
        let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(doc)).unwrap();
        assert!(yaml.contains("provider: linear"), "{yaml}");
        assert_eq!(
            yaml.matches("provider:").count(),
            1,
            "absent, it isn't written: {yaml}"
        );
        for bad in [
            "tracker/Linear",
            "tracker/linear-acme",
            "tracker/1st",
            "tracker/",
            "/linear",
            "tracker/a/b",
            "linear",
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
            "tracker/linear_acme".into(),
            ExtensionInstanceConfig {
                enabled: true,
                config: serde_json::json!({ "team": "ACME" }),
                sync_minutes: None,
                provider: Some("linear".into()),
            },
        );
        global.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join(INSTANCES_FILE)).unwrap();
        assert!(
            text.contains("tracker/linear_acme") && text.contains("provider: linear"),
            "{text}"
        );
        assert!(
            !text.contains("syncMinutes"),
            "an absent schedule isn't written: {text}"
        );
        assert_eq!(GlobalInstances::load(dir.path()).unwrap(), global);

        std::fs::write(
            dir.path().join(INSTANCES_FILE),
            "instances:\n  tracker/Linear: { enabled: true }\n",
        )
        .unwrap();
        let err = GlobalInstances::load(dir.path()).unwrap_err().to_string();
        assert!(
            err.contains("instances.yaml") && err.contains("tracker/Linear"),
            "{err}"
        );
        assert!(!err.contains("project.yaml"), "{err}");
        std::fs::write(dir.path().join(INSTANCES_FILE), "extensionInstances: {}\n").unwrap();
        let err = GlobalInstances::load(dir.path()).unwrap_err().to_string();
        assert!(err.contains("instances.yaml"), "{err}");
        // An invalid set is never written.
        let mut bad = GlobalInstances::default();
        bad.instances
            .insert("nope".into(), ExtensionInstanceConfig::default());
        assert!(bad.save(dir.path()).is_err());
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

    /// A scaffold's entries render as a `project.yaml` snippet that loads
    /// back to the same entries (tsk391): the agent pastes it in.
    #[test]
    fn entries_yaml_loads_back() {
        let measure = MeasureEntry {
            key: Some("acme.x.count".into()),
            capture_scope: Some("per-path".into()),
            ..Default::default()
        };
        let metric = MetricEntry {
            key: Some("acme.x".into()),
            source_measure: Some("acme.x.count".into()),
            aggregation: Some("sum".into()),
            ..Default::default()
        };
        let collector: serde_yaml::Value = serde_yaml::from_str(
            "{ id: acme.x, runtime: starlark, entry: oxplow/collectors/acme_x.star, trigger: { on: [snapshot.taken] }, facts: [acme.x.count] }",
        )
        .unwrap();
        let yaml = entries_yaml(
            std::slice::from_ref(&measure),
            std::slice::from_ref(&collector),
            std::slice::from_ref(&metric),
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        std::fs::write(config_path(dir.path()), &yaml).unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.measures, vec![measure], "{yaml}");
        assert_eq!(cfg.collectors[0].id, "acme.x");
        assert_eq!(cfg.metrics, vec![metric]);
    }

    #[test]
    fn acp_agents_parse_validate_round_trip_and_layer_over_presets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        std::fs::write(
            config_path(dir.path()),
            "acpAgents:\n  - { name: gemini, command: /opt/gemini, args: [--acp, --yolo] }\n  - { name: mine, command: ./tools/agent, env: { MODE: fast } }\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.acp_agents.len(), 2);
        let resolved = resolve_acp_agents(&cfg.acp_agents);
        let names: Vec<(&str, AcpAgentSource)> = resolved
            .iter()
            .map(|(a, s)| (a.name.as_str(), *s))
            .collect();
        assert_eq!(
            names,
            vec![
                ("claude", AcpAgentSource::Preset),
                ("gemini", AcpAgentSource::Project),
                ("codex", AcpAgentSource::Preset),
                ("mine", AcpAgentSource::Project),
            ]
        );
        assert_eq!(resolved[1].0.args, vec!["--acp", "--yolo"]);
        // Written back as it was.
        write_project_config(dir.path(), &cfg).unwrap();
        assert_eq!(
            load_project_config(dir.path()).unwrap().acp_agents,
            cfg.acp_agents
        );
        for (bad, needle) in [
            (
                "acpAgents:\n  - { name: Bad Name, command: x }\n",
                "lowercase",
            ),
            (
                "acpAgents:\n  - { name: a, command: \"\" }\n",
                "needs a command",
            ),
            (
                "acpAgents:\n  - { name: a, command: x }\n  - { name: a, command: y }\n",
                "twice",
            ),
        ] {
            std::fs::write(config_path(dir.path()), bad).unwrap();
            let err = load_project_config(dir.path()).unwrap_err().to_string();
            assert!(err.contains(needle), "{bad}: {err}");
        }
    }

    #[test]
    fn entity_metrics_and_dimensions_validate_and_resolve() {
        let yaml = r#"
metrics:
  - key: work.done
    title: Tasks completed
    entity: v_task
    where: "status = 'done'"
    time: completed_at
  - key: work.median_prio
    entity: v_task
    aggregation: median
    value: "e.sort_index"
dimensions:
  - key: work.prio
    entity: v_task
    expr: "e.priority"
    join: "LEFT JOIN v_thread t ON t.id = e.thread_id"
"#;
        #[derive(Deserialize)]
        struct Doc {
            metrics: Option<Vec<MetricEntry>>,
            dimensions: Option<Vec<DimensionEntry>>,
        }
        let doc: Doc = serde_yaml::from_str(yaml).unwrap();
        let metrics = validate_metrics(doc.metrics).unwrap();
        let resolved = resolve_metrics(&[], &[], &[], &metrics);
        assert_eq!(
            resolved[0].entity,
            Some(EntitySpec {
                view: "v_task".into(),
                where_: Some("status = 'done'".into()),
                time: Some("completed_at".into()),
                value: None,
                aggregation: "count".into(),
            })
        );
        assert_eq!(resolved[1].entity.as_ref().unwrap().aggregation, "median");
        let dims = validate_dimensions(doc.dimensions).unwrap();
        let rd = resolve_dimensions(&[], &[], &dims);
        assert_eq!(
            rd[0].entity,
            Some(EntityDimensionSpec {
                view: "v_task".into(),
                expr: "e.priority".into(),
                join: Some("LEFT JOIN v_thread t ON t.id = e.thread_id".into()),
            })
        );
    }

    #[test]
    fn entity_metric_mistakes_are_refused() {
        let bad = |yaml: &str| {
            let e: Vec<MetricEntry> = serde_yaml::from_str(yaml).unwrap();
            validate_metrics(Some(e)).unwrap_err().to_string()
        };
        assert!(bad("[{key: a.b, entity: tasks}]").contains("v_*"));
        assert!(bad("[{key: a.b, entity: v_task, aggregation: sum}]").contains("needs a `value`"));
        assert!(
            bad("[{key: a.b, entity: v_task, aggregation: ratio, value: x}]")
                .contains("entity aggregation")
        );
        assert!(bad("[{key: a.b, entity: v_task, sourceMeasure: m}]").contains("can't also set"));
        assert!(bad("[{key: a.b, sourceMeasure: m, where: x}]").contains("without `entity`"));
        assert!(bad("[{use: a.b, time: x}]").contains("`use:` entry"));
        let dim = |yaml: &str| {
            let e: Vec<DimensionEntry> = serde_yaml::from_str(yaml).unwrap();
            validate_dimensions(Some(e)).unwrap_err().to_string()
        };
        assert!(dim("[{key: a.b, entity: v_task}]").contains("needs an `expr`"));
        assert!(dim("[{key: a.b, expr: x}]").contains("without `entity`"));
        assert!(dim("[{key: a.b, entity: v_task, expr: x, promote: true}]")
            .contains("fact dimensions only"));
    }
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

    /// The single-`agent` form is gone: `agents` is the one key.
    #[test]
    fn the_old_single_agent_key_is_refused() {
        let dir = tempdir().unwrap();
        std::fs::write(cfg_path(dir.path()), "agent: codex\n").unwrap();
        let err = load_project_config(dir.path()).unwrap_err().to_string();
        assert!(err.contains("agent"), "{err}");
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
    fn write_round_trips_entity_metrics_and_dimensions() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "metrics:\n\
             - key: work.open_bugs\n  title: Open Bugs\n  entity: v_task\n  aggregation: count\n  where: \"e.status = 'ready'\"\n  time: e.created_at\n\
             - key: work.mean_priority\n  entity: v_task\n  aggregation: avg\n  value: e.priority_rank\n\
             dimensions:\n\
             - key: work.thread_title\n  label: Thread\n  entity: v_task\n  expr: t.title\n  join: LEFT JOIN v_thread t ON t.id = e.thread_id\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        write_project_config(dir.path(), &cfg).unwrap();
        let reloaded = load_project_config(dir.path()).expect("still loads after a write");
        assert_eq!(reloaded.metrics, cfg.metrics);
        assert_eq!(reloaded.dimensions, cfg.dimensions);
        assert_eq!(
            reloaded.metrics[0].where_.as_deref(),
            Some("e.status = 'ready'")
        );
        assert_eq!(
            reloaded.dimensions[0].join.as_deref(),
            Some("LEFT JOIN v_thread t ON t.id = e.thread_id")
        );
    }

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
        write_project_config(dir.path(), &cfg).unwrap();
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
            "collection:\n  testCommand: bun run test:collect\n  fastTestCommand: bun run test:fast\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(
            cfg.collection.fast_test_command.as_deref(),
            Some("bun run test:fast")
        );
        write_project_config(dir.path(), &cfg).unwrap();
        let reloaded = load_project_config(dir.path()).unwrap();
        assert_eq!(
            reloaded.collection.fast_test_command.as_deref(),
            Some("bun run test:fast"),
            "survives a write/reload round-trip"
        );
        assert_eq!(
            reloaded.collection.test_command.as_deref(),
            Some("bun run test:collect"),
            "and doesn't displace the full command"
        );
    }

    /// The `collectors:` block is written back as the file declares it
    /// (a parsed spec isn't the declared shape), so a write that changes
    /// another key leaves it loading the same.
    #[test]
    fn a_write_keeps_the_collectors_block_as_declared() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "collectors:\n\
             - id: repo.scan_one\n  \
               doc: One\n  \
               runtime: starlark\n  \
               entry: oxplow/collectors/one.star\n  \
               trigger: { on: [snapshot.taken] }\n  \
               facts: [repo.one]\n",
        )
        .unwrap();
        let mut cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.collectors.len(), 1);
        cfg.snapshot_retention_days = 3;
        write_project_config(dir.path(), &cfg).unwrap();
        let reloaded = load_project_config(dir.path()).unwrap();
        assert_eq!(reloaded.collectors, cfg.collectors);
        assert_eq!(reloaded.snapshot_retention_days, 3);
    }

    #[test]
    fn managed_keys_covers_every_block_the_writer_emits() {
        // Guard against the next block drifting the same way tsk164's did: any
        // top-level key `write_project_config` serializes must be MANAGED, or
        // the stale on-disk copy silently wins.
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "collectors:\n\
             - id: repo.scan_one\n  \
               doc: One\n  \
               runtime: starlark\n  \
               entry: oxplow/collectors/one.star\n  \
               trigger: { on: [snapshot.taken] }\n  \
               facts: [repo.one]\n\
             projectName: demo\n\
             snapshotRetentionDays: 3\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        write_project_config(dir.path(), &cfg).unwrap();
        let raw = std::fs::read_to_string(cfg_path(dir.path())).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&raw).unwrap();
        let serde_yaml::Value::Mapping(map) = doc else {
            panic!("config is a mapping");
        };
        // Every key present after a write must round-trip through a reload,
        // which is what being MANAGED buys.
        let reloaded = load_project_config(dir.path()).unwrap();
        assert_eq!(reloaded.collectors.len(), 1);
        assert_eq!(reloaded.project_name, "demo");
        assert_eq!(reloaded.snapshot_retention_days, 3);
        assert!(
            map.contains_key(serde_yaml::Value::String("collectors".into())),
            "collectors block written, got:\n{raw}"
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
    fn disabled_extensions_round_trip_and_read_cheaply() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "extensions:\n  disabled: [oxplow-analytics]\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(cfg.extensions_disabled, vec!["oxplow-analytics"]);
        write_project_config(dir.path(), &cfg).unwrap();
        assert_eq!(disabled_extensions(dir.path()), vec!["oxplow-analytics"]);
        assert!(disabled_extensions(tempdir().unwrap().path()).is_empty());
    }

    #[test]
    fn ai_roles_round_trip_and_reject_unknown_roles() {
        let dir = tempdir().unwrap();
        std::fs::write(
            cfg_path(dir.path()),
            "ai:\n  roles:\n    summarize: { provider: openrouter, model: openai/gpt-5-mini }\n",
        )
        .unwrap();
        let cfg = load_project_config(dir.path()).unwrap();
        assert_eq!(
            cfg.ai_roles.get("summarize"),
            Some(&AiRoleOverride {
                provider: "openrouter".into(),
                model: "openai/gpt-5-mini".into()
            })
        );
        write_project_config(dir.path(), &cfg).unwrap();
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
        ] {
            std::fs::write(cfg_path(dir.path()), yaml).unwrap();
            let err = load_project_config(dir.path()).unwrap_err().to_string();
            assert!(err.contains(needle), "{err}");
        }
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
    fn icon_tint_round_trips_and_defaults_to_unset() {
        let dir = tempdir().unwrap();
        assert_eq!(load_project_config(dir.path()).unwrap().icon_tint, None);

        let cfg = OxplowConfig {
            icon_tint: Some("#c2410c".into()),
            ..default_config("test".into())
        };
        write_project_config(dir.path(), &cfg).unwrap();
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
                fast_test_command: None,
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
        // every category the built-in producer specs emit must round-trip: the app
        // WRITES those categories back into project.yaml, so a category the
        // validator rejects is a config the app can't re-read (it panicked at boot
        // on `coverage` — the built-in coverage specs' own category).
        for cat in METRIC_CATEGORIES {
            assert!(
                load_from_yaml(&format!(
                    "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n    category: {cat}\n"
                ))
                .is_ok(),
                "category {cat} should validate"
            );
        }
        assert!(load_from_yaml(
            "metrics:\n  - key: a.b\n    sourceMeasure: acme.m\n    category: coverage\n"
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
    fn resolve_project_definition_is_active_and_scoped() {
        let project = vec![define("acme.loc", None)];
        let resolved = resolve_metrics(&[], &[], &[], &project);
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

    #[test]
    fn resolve_defaults_enabled_true() {
        let project = vec![define("acme.loc", None)];
        let resolved = resolve_metrics(&[], &[], &[], &project);
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
    fn resolve_marks_disabled_key_definition() {
        // Disabling a config-DEFINED metric keeps its definition but flags it off.
        let mut def = define("acme.loc", None);
        def.enabled = Some(false);
        let resolved = resolve_metrics(&[], &[], &[], &[def]);
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

    // --- collectors (the fact producers, P7.B3) ------------------------------

    #[test]
    fn parses_the_collectors_block_as_the_projects() {
        let cfg = load_from_yaml(
            "collectors:\n  - { id: acme.scan, runtime: starlark, entry: oxplow/collectors/scan.star, trigger: { on: [snapshot.taken] }, facts: [acme.todo, acme.complexity] }\n",
        )
        .unwrap();
        assert_eq!(cfg.collectors.len(), 1);
        assert_eq!(cfg.collectors[0].id, "acme.scan");
        assert_eq!(
            cfg.collectors[0].facts,
            vec!["acme.complexity", "acme.todo"]
        );
        // An invalid one fails the load, naming it.
        let err = load_from_yaml(
            "collectors:\n  - { id: acme.scan, runtime: starlark, entry: x.star, trigger: { on: [nope.happened] }, facts: [acme.todo] }\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("acme.scan"), "{err}");
    }

    /// This repo's own `.oxplow/project.yaml` loads — its collectors
    /// included (migrated from `gauges:` in P7.B3).
    #[test]
    fn this_repos_project_config_loads() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cfg = load_project_config(&root).expect("the repo's project.yaml loads");
        assert!(
            cfg.collectors
                .iter()
                .any(|c| c.id == "repo.scan_type_coverage"),
            "{:?}",
            cfg.collectors.iter().map(|c| &c.id).collect::<Vec<_>>()
        );
    }

    /// `gauges:` is retired: loading one says how to migrate it.
    #[test]
    fn a_gauges_block_is_an_error_naming_the_migration() {
        let err = load_from_yaml(
            "gauges:\n  - key: acme.foo\n    emits: [acme.m]\n    compute: { runtime: starlark, entryFile: g.star }\n",
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("collectors:") && msg.contains("oxplow plugin migrate --project"),
            "{msg}"
        );
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
        let resolved = resolve_measures(&[], &[], &cfg.measures);
        assert_eq!(resolved[0].capture_scope, "per-path");
        let plain = load_from_yaml("measures:\n  - key: acme.y\n").unwrap();
        assert_eq!(
            resolve_measures(&[], &[], &plain.measures)[0].capture_scope,
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
        let resolved = resolve_measures(&global, &[], &project);
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
        let resolved = resolve_dimensions(&global, &[], &project);
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
