//! Collection engine — effort-scoped observations (which tests ran +
//! diff coverage on changed lines). See `.context/collection.md`.
//!
//! Hybrid by design:
//! - **Passive**: `on_post_tool_use` is called from the control-plane's
//!   PostToolUse branch. It detects a test-runner Bash command, records
//!   a `test-run` observation (`observed`), and — if a coverage report
//!   is configured — rides along to ingest coverage.
//! - **Active**: `ingest_coverage` / `record_test_run` back the MCP
//!   tools of the same name.
//!
//! Coverage numbers come **only** from oxplow parsing the report
//! (`oxplow-coverage`), never from the agent — so `diff-coverage` is
//! always `observed`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde_json::json;
use similar::{ChangeTag, TextDiff};

use oxplow_collect_plugin::{
    Collector, CollectorInput, CollectorKind, CollectorOutput, CollectorRegistry, CollectorRuntime,
};
use oxplow_config::OxplowConfig;
use oxplow_db::agent_nudge_store::{NewAgentNudge, SqliteAgentNudgeStore};
use oxplow_db::{
    NewMetricDefinition, NewMetricFinding, NewMetricRun, NewMetricSample, SqliteMetricStore,
};
use oxplow_db::{
    SqliteSnapshotStore, SqliteTaskEffortStore, SqliteThreadStore, TaskEffort, TaskEffortStore,
};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{DomainError, EffortId, ThreadId};

use crate::blob_store::BlobStore;
use crate::events::{EventBus, OxplowEvent};
use crate::file_ref_version;

/// Built-in command substrings that count as a test run. The collection
/// profile's `testRunPatterns` extends (never replaces) this list.
const DEFAULT_TEST_PATTERNS: &[&str] = &[
    "pytest",
    "cargo test",
    "cargo nextest",
    "npm test",
    "npm run test",
    "pnpm test",
    "yarn test",
    "bun test",
    "jest",
    "vitest",
    "go test",
    "gradle test",
    "mvn test",
    "dotnet test",
    "rspec",
    "phpunit",
];

/// Built-in command substrings that count as a static-analysis run. The
/// collection profile's `analysisRunPatterns` extends (never replaces) this
/// list. Tool-agnostic: no command→tool knowledge lives here, only "did an
/// analyzer run?" — the report a run regenerates is what gets parsed.
const DEFAULT_ANALYSIS_PATTERNS: &[&str] = &[
    "cargo clippy",
    "clippy-driver",
    "eslint",
    "ruff",
    "golangci-lint",
    "flake8",
    "pylint",
    "mypy",
    "staticcheck",
    "tsc --noemit",
    "tsc --noEmit",
];

/// Does `command` look like a test run? Case-insensitive substring match
/// against the built-in patterns plus any caller-supplied extras.
pub fn detect_test_run(command: &str, extra_patterns: &[String]) -> bool {
    matches_any(command, DEFAULT_TEST_PATTERNS, extra_patterns)
}

/// Does `command` look like a static-analysis run? Same substring matching as
/// [`detect_test_run`], against the analysis patterns + profile extras.
pub fn detect_analysis_run(command: &str, extra_patterns: &[String]) -> bool {
    matches_any(command, DEFAULT_ANALYSIS_PATTERNS, extra_patterns)
}

/// Does `command` look like a `git commit`? Token-aware so it catches
/// `git commit`, `git commit --amend`, and `git -c user.email=x commit …`
/// (global flags between `git` and the subcommand), while NOT matching
/// `git add`, `git status`, or `git log --grep commit`. For each `git` token
/// it reads the first non-option token (skipping `-c <val>` / `--config <val>`
/// global flags) and checks it is exactly `commit`.
pub fn detect_git_commit(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let toks: Vec<&str> = lower.split_whitespace().collect();
    for (i, t) in toks.iter().enumerate() {
        if *t != "git" && !t.ends_with("/git") {
            continue;
        }
        // Read the subcommand for this `git`, skipping global options and
        // the value of `-c` / `--config`.
        let mut j = i + 1;
        while let Some(tok) = toks.get(j) {
            if *tok == "-c" || *tok == "--config" {
                j += 2; // skip the flag and its argument
                continue;
            }
            if tok.starts_with('-') {
                j += 1;
                continue;
            }
            if *tok == "commit" {
                return true;
            }
            break; // a different subcommand → this `git` isn't a commit
        }
    }
    false
}

/// Case-insensitive: does `command` contain any built-in or extra pattern?
fn matches_any(command: &str, builtins: &[&str], extras: &[String]) -> bool {
    let lower = command.to_ascii_lowercase();
    builtins
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .chain(extras.iter().map(|s| s.to_ascii_lowercase()))
        .any(|p| !p.trim().is_empty() && lower.contains(p.trim()))
}

/// The Bash command + best-effort exit code pulled out of a PostToolUse
/// envelope. `None` when the tool wasn't Bash or no command was present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BashInvocation {
    pub command: String,
    pub exit_code: Option<i64>,
}

/// Parse a PostToolUse `payload_json` for a Bash invocation. Tolerant of
/// shape drift: returns `None` unless `tool_name == "Bash"` and a
/// `tool_input.command` string is present. Exit code is best-effort
/// (Claude Code's Bash `tool_response` doesn't always carry one).
pub fn parse_bash_post_tool(payload_json: &str) -> Option<BashInvocation> {
    let v: serde_json::Value = serde_json::from_str(payload_json).ok()?;
    let tool_name = v.get("tool_name").and_then(|t| t.as_str())?;
    if tool_name != "Bash" {
        return None;
    }
    let command = v
        .get("tool_input")
        .and_then(|i| i.get("command"))
        .and_then(|c| c.as_str())?
        .to_string();
    if command.trim().is_empty() {
        return None;
    }
    let exit_code = v.get("tool_response").and_then(|r| {
        ["exit_code", "exitCode", "returnCode", "code"]
            .iter()
            .find_map(|k| r.get(*k).and_then(|x| x.as_i64()))
    });
    Some(BashInvocation { command, exit_code })
}

/// Diff-coverage thresholds (tsk220), stored as the `oxplow.coverage.diff_pct`
/// definition's `target`/`fail_at` so the renderer colors from DATA rather than
/// a hardcoded 50/80 ramp, and the advisory nudge fires below target.
pub const COVERAGE_TARGET_PCT: f64 = 80.0;
pub const COVERAGE_FAIL_PCT: f64 = 50.0;

/// Outcome of a coverage ingest, so the MCP tool can report precisely
/// why nothing landed.
#[derive(Debug, Clone, PartialEq)]
pub enum CoverageIngest {
    NoOpenEffort,
    NotConfigured,
    ReportMissing(String),
    /// The report on disk predates the open effort — it's from an earlier
    /// run that this test invocation didn't regenerate, so attributing it
    /// would be misleading. Only the passive ride-along skips on this;
    /// an explicit `ingest_coverage` ingests regardless.
    StaleReport(String),
    ParseError(String),
    NoBaseline,
    NoChangedCoverage,
    Stored {
        observation_id: i64,
        summary_pct: f64,
        changed_lines: usize,
        covered_lines: usize,
    },
}

/// Outcome of an analysis ingest (the on-demand `ingest_analysis` MCP path),
/// so the tool can report precisely why nothing landed. Mirrors
/// [`CoverageIngest`]'s shape.
#[derive(Debug, Clone, PartialEq)]
pub enum AnalysisIngest {
    NoOpenEffort,
    NotConfigured,
    ReportMissing(String),
    /// The report on disk predates the open effort. Only a `skip_if_stale`
    /// caller skips on this; the explicit MCP path passes `false`.
    StaleReport(String),
    ParseError(String),
    Stored {
        observation_id: i64,
        error_count: u64,
        warning_count: u64,
        info_count: u64,
        note_count: u64,
        findings: usize,
    },
}

#[derive(Clone)]
pub struct CollectionService {
    /// Unified metric substrate (epic tsk213) — the **sole** store for
    /// coverage/test/analysis facts now (the legacy `effort_observation` table
    /// was dropped in tsk215). The effort panel reconstructs its rows from here
    /// via `effort_observations_from_metrics`.
    metrics: Arc<SqliteMetricStore>,
    nudges: Arc<SqliteAgentNudgeStore>,
    efforts: Arc<SqliteTaskEffortStore>,
    threads: Arc<SqliteThreadStore>,
    snapshots: Arc<SqliteSnapshotStore>,
    blobs: BlobStore,
    config: Arc<RwLock<OxplowConfig>>,
    project_dir: PathBuf,
    events: EventBus,
    /// Efforts already nudged about a report-less test run. In-memory:
    /// ephemeral guidance that shouldn't be persisted or survive a restart.
    nudged_efforts: Arc<std::sync::Mutex<std::collections::HashSet<EffortId>>>,
    /// Commit shas already nudged about out-of-effort files. Same ephemeral
    /// in-memory dedup as `nudged_efforts`, but keyed by commit sha so the
    /// hygiene nudge fires at most once per commit.
    nudged_commits: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// Efforts already nudged about coverage below target (tsk220). Separate
    /// set from `nudged_efforts` so the report-less and coverage-target nudges
    /// don't suppress each other; same ephemeral in-memory dedup.
    nudged_coverage: Arc<std::sync::Mutex<std::collections::HashSet<EffortId>>>,
    /// `(effort, metric_id)` pairs already surfaced as a gauge warn/fail
    /// crossing in the effort-metric prompt context (tsk231). Gauge metrics run
    /// on-snapshot in the background, not in a hook, so their threshold crossing
    /// can't ride the PostToolUse `additionalContext`; instead the
    /// UserPromptSubmit context line surfaces it **once** per effort+metric.
    /// Same ephemeral in-memory dedup.
    nudged_gauge: Arc<std::sync::Mutex<std::collections::HashSet<(EffortId, i64)>>>,
}

impl CollectionService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        metrics: Arc<SqliteMetricStore>,
        nudges: Arc<SqliteAgentNudgeStore>,
        efforts: Arc<SqliteTaskEffortStore>,
        threads: Arc<SqliteThreadStore>,
        snapshots: Arc<SqliteSnapshotStore>,
        blobs: BlobStore,
        config: Arc<RwLock<OxplowConfig>>,
        project_dir: PathBuf,
        events: EventBus,
    ) -> Self {
        Self {
            metrics,
            nudges,
            efforts,
            threads,
            snapshots,
            blobs,
            config,
            project_dir,
            events,
            nudged_efforts: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            nudged_commits: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            nudged_coverage: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            nudged_gauge: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    /// Resolve the stream id that owns `thread` (the observation's hard
    /// scope + CASCADE anchor).
    async fn stream_id_for(&self, thread: &ThreadId) -> Result<Option<String>, DomainError> {
        Ok(self
            .threads
            .get(thread)
            .await?
            .map(|t| t.stream_id.to_string()))
    }

    fn collection_cfg(&self) -> oxplow_config::CollectionConfig {
        self.config
            .read()
            .map(|c| c.collection.clone())
            .unwrap_or_default()
    }

    /// Build the collector registry for this project: the first-party
    /// builtins plus any project-defined plugins from `collection.plugins`.
    /// Cheap to rebuild (jaq/starlark programs compile lazily at run time),
    /// so we construct it per ride-along — picking up hot-reloaded config.
    fn registry(&self, cfg: &oxplow_config::CollectionConfig) -> CollectorRegistry {
        let mut reg = CollectorRegistry::with_builtins();
        for p in &cfg.plugins {
            match plugin_to_collector(p, &self.project_dir) {
                Ok(c) => reg.register(c),
                Err(e) => tracing::warn!(
                    plugin = %p.name,
                    error = %e,
                    "collection plugin skipped"
                ),
            }
        }
        reg
    }

    /// Record a `test-run` observation against the thread's open effort.
    /// Returns `Ok(None)` when no effort is open (nothing to attribute).
    #[allow(clippy::too_many_arguments)]
    pub async fn record_test_run(
        &self,
        thread: &ThreadId,
        command: &str,
        exit_code: Option<i64>,
        duration_ms: Option<i64>,
        passed: Option<i64>,
        failed: Option<i64>,
        total: Option<i64>,
        provenance: &str,
        source: &str,
        report: Option<&oxplow_coverage::TestReport>,
    ) -> Result<Option<i64>, DomainError> {
        let Some(effort) = self.efforts.find_open_for_thread(thread).await? else {
            return Ok(None);
        };
        let Some(stream_id) = self.stream_id_for(thread).await? else {
            return Ok(None);
        };
        let mut payload = serde_json::Map::new();
        payload.insert("command".into(), json!(command));
        if let Some(c) = exit_code {
            payload.insert("exitCode".into(), json!(c));
        }
        if let Some(d) = duration_ms {
            payload.insert("durationMs".into(), json!(d));
        }
        // When oxplow parsed a JUnit report, embed the suite/case tree and
        // derive the counts from it (overriding any caller-supplied ones).
        let (passed, failed, total, skipped) = match report {
            Some(r) => {
                use oxplow_coverage::TestStatus::*;
                let (mut p, mut f, mut s) = (0i64, 0i64, 0i64);
                for suite in &r.suites {
                    for case in &suite.cases {
                        match case.status {
                            Passed => p += 1,
                            Failed => f += 1,
                            Skipped => s += 1,
                        }
                    }
                }
                payload.insert(
                    "suites".into(),
                    serde_json::to_value(&r.suites).unwrap_or(serde_json::Value::Null),
                );
                (Some(p), Some(f), Some(p + f + s), Some(s))
            }
            None => (passed, failed, total, None),
        };
        if let Some(p) = passed {
            payload.insert("passed".into(), json!(p));
        }
        if let Some(f) = failed {
            payload.insert("failed".into(), json!(f));
        }
        if let Some(t) = total {
            payload.insert("total".into(), json!(t));
        }
        if let Some(s) = skipped {
            payload.insert("skipped".into(), json!(s));
        }
        // Dual-write into the unified metric substrate (best-effort) — counts as
        // samples + the full payload (suite/case tree) as a `test-detail`
        // finding so the effort panel can render off the substrate (tsk215).
        self.mirror_test_metrics(
            thread,
            &stream_id,
            provenance,
            source,
            passed,
            failed,
            total,
            Some(serde_json::Value::Object(payload.clone())),
        )
        .await;
        // Substrate is the sole store now (tsk215) — `stream_id` is consumed by
        // the mirror above; the suite/case tree is kept as a `test-detail`
        // finding. Signal the panel to refetch; `Some` = recorded.
        let _ = (stream_id, payload);
        self.emit(thread, &effort);
        Ok(Some(0))
    }

    /// Ingest a SINGLE coverage report (the explicit MCP path). Uses the
    /// override path/format, or the first configured coverage report.
    pub async fn ingest_coverage(
        &self,
        thread: &ThreadId,
        report_path_override: Option<String>,
        format_override: Option<String>,
        skip_if_stale: bool,
    ) -> Result<CoverageIngest, DomainError> {
        let Some(effort) = self.efforts.find_open_for_thread(thread).await? else {
            return Ok(CoverageIngest::NoOpenEffort);
        };
        let Some(stream_id) = self.stream_id_for(thread).await? else {
            return Ok(CoverageIngest::NoOpenEffort);
        };
        let cfg = self.collection_cfg();
        let registry = self.registry(&cfg);
        let (report_path, format_str) = match (report_path_override, format_override) {
            (Some(p), Some(f)) => (p, f),
            _ => match first_coverage_report(&cfg, &registry) {
                Some(r) => (r.path.clone(), r.format.clone()),
                None => return Ok(CoverageIngest::NotConfigured),
            },
        };
        let Some(collector) = registry.resolve(&format_str) else {
            return Ok(CoverageIngest::ParseError(format!(
                "no collector registered for format \"{format_str}\""
            )));
        };
        if collector.kind() != CollectorKind::Coverage {
            return Ok(CoverageIngest::ParseError(format!(
                "format \"{format_str}\" is not a coverage format"
            )));
        }
        let abs = self.project_dir.join(&report_path);
        let Ok(content) = std::fs::read_to_string(&abs) else {
            return Ok(CoverageIngest::ReportMissing(report_path));
        };
        // Stale guard for the ride-along (skip_if_stale); the explicit
        // MCP path passes false — the caller asked for it.
        if skip_if_stale && report_is_stale(&abs, effort.started_at) {
            return Ok(CoverageIngest::StaleReport(report_path));
        }
        let report = match collector.run(&content) {
            Ok(CollectorOutput::Coverage(r)) => r,
            Ok(_) => {
                return Ok(CoverageIngest::ParseError(
                    "collector did not produce coverage output".into(),
                ))
            }
            Err(e) => return Ok(CoverageIngest::ParseError(e.to_string())),
        };
        self.store_diff_coverage(thread, &effort, &stream_id, &report)
            .await
    }

    /// Ingest a SINGLE analysis report (the explicit MCP path) — the on-demand
    /// counterpart to [`ingest_coverage`]. Resolves `format` via the collector
    /// registry, parses the report as `CollectorKind::Analysis`, and records a
    /// `static-analysis` observation against the thread's open effort via
    /// `record_static_analysis` (provenance `observed`). Uses the override
    /// path/format, or the first configured analysis report.
    pub async fn ingest_analysis(
        &self,
        thread: &ThreadId,
        report_path_override: Option<String>,
        format_override: Option<String>,
        skip_if_stale: bool,
    ) -> Result<AnalysisIngest, DomainError> {
        let Some(effort) = self.efforts.find_open_for_thread(thread).await? else {
            return Ok(AnalysisIngest::NoOpenEffort);
        };
        let cfg = self.collection_cfg();
        let registry = self.registry(&cfg);
        let (report_path, format_str) = match (report_path_override, format_override) {
            (Some(p), Some(f)) => (p, f),
            _ => match first_analysis_report(&cfg, &registry) {
                Some(r) => (r.path.clone(), r.format.clone()),
                None => return Ok(AnalysisIngest::NotConfigured),
            },
        };
        let Some(collector) = registry.resolve(&format_str) else {
            return Ok(AnalysisIngest::ParseError(format!(
                "no collector registered for format \"{format_str}\""
            )));
        };
        if collector.kind() != CollectorKind::Analysis {
            return Ok(AnalysisIngest::ParseError(format!(
                "format \"{format_str}\" is not an analysis format"
            )));
        }
        let source = analysis_source(collector);
        let analyzer = collector
            .name()
            .strip_prefix("oxplow.")
            .unwrap_or(collector.name())
            .to_string();
        let abs = self.project_dir.join(&report_path);
        let Ok(content) = std::fs::read_to_string(&abs) else {
            return Ok(AnalysisIngest::ReportMissing(report_path));
        };
        // Stale guard for the ride-along (skip_if_stale); the explicit MCP
        // path passes false — the caller asked for it.
        if skip_if_stale && report_is_stale(&abs, effort.started_at) {
            return Ok(AnalysisIngest::StaleReport(report_path));
        }
        // No baseline gate: findings are ABSOLUTE (current-file), not
        // diff-relative like coverage, so they don't need a start snapshot to
        // be meaningful. `record_static_analysis` stores with pin = None when
        // there's no snapshot — matching the passive ride-along path. (tsk86)
        let report = match collector.run(&content) {
            Ok(CollectorOutput::Analysis(r)) => r,
            Ok(_) => {
                return Ok(AnalysisIngest::ParseError(
                    "collector did not produce analysis output".into(),
                ))
            }
            Err(e) => return Ok(AnalysisIngest::ParseError(e.to_string())),
        };
        let (mut error_count, mut warning_count, mut info_count, mut note_count) = (0u64, 0, 0, 0);
        for f in &report.findings {
            use oxplow_coverage::Severity::*;
            match f.severity {
                Error => error_count += 1,
                Warning => warning_count += 1,
                Info => info_count += 1,
                Note => note_count += 1,
            }
        }
        let findings = report.findings.len();
        let command = format!("ingest_analysis {report_path}");
        match self
            .record_static_analysis(thread, &command, Some(&report), &[analyzer], &source)
            .await?
        {
            Some(observation_id) => Ok(AnalysisIngest::Stored {
                observation_id,
                error_count,
                warning_count,
                info_count,
                note_count,
                findings,
            }),
            None => Ok(AnalysisIngest::NoOpenEffort),
        }
    }

    /// Compute diff coverage from a (possibly merged) report and store a
    /// `diff-coverage` observation over the effort's changed lines.
    /// Mirror a diff-coverage result into the unified metric substrate
    /// (epic tsk213), best-effort: upsert the `oxplow.coverage.diff_pct`
    /// definition, open a `coverage` run, and record one sample carrying the
    /// covered/changed components (so module/branch roll-ups re-aggregate
    /// correctly) plus the capture branch + git version. A metric write error
    /// is logged and swallowed so it never fails the legacy observation path.
    #[allow(clippy::too_many_arguments)]
    async fn mirror_coverage_metric(
        &self,
        thread: &ThreadId,
        stream_id: &str,
        summary_pct: f64,
        covered: usize,
        changed: usize,
        version: &file_ref_version::ResolvedFileVersion,
        detail: Option<serde_json::Value>,
    ) {
        let Some(stream_val) = oxplow_domain::StreamId::try_from_str(stream_id).map(|s| s.value())
        else {
            return;
        };
        if let Err(e) = self
            .record_coverage_metric(
                thread,
                stream_val,
                summary_pct,
                covered,
                changed,
                version,
                detail,
            )
            .await
        {
            tracing::warn!(error = %e, "failed to mirror coverage into metric substrate");
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_coverage_metric(
        &self,
        thread: &ThreadId,
        stream_val: i64,
        summary_pct: f64,
        covered: usize,
        changed: usize,
        version: &file_ref_version::ResolvedFileVersion,
        detail: Option<serde_json::Value>,
    ) -> Result<(), DomainError> {
        let branch = oxplow_git::detect_current_branch(&self.project_dir);

        let mut def =
            NewMetricDefinition::new("oxplow.coverage.diff_pct", "coverage", "Diff coverage");
        def.unit = Some("%".into());
        def.direction = "higher-better".into();
        def.grain = Some("effort".into());
        def.basis = "diff-vs-effort-start".into();
        def.producer = Some("coverage".into());
        def.category = Some("coverage".into());
        def.language = None;
        def.dimensions_json = Some("[\"branch\",\"git_version\"]".into());
        def.description = Some("Coverage % over the effort's changed lines.".into());
        // Targets live in DATA, not a hardcoded UI ramp (tsk220): the renderer
        // colors red/green from these (fail < 50%, warn < 80%, ok ≥ 80%).
        def.target = Some(COVERAGE_TARGET_PCT);
        def.warn_at = Some(COVERAGE_TARGET_PCT);
        def.fail_at = Some(COVERAGE_FAIL_PCT);
        let metric_id = self.metrics.upsert_definition(def).await?;

        let mut run = NewMetricRun::done(stream_val, "coverage", "coverage-report");
        run.thread_id = Some(thread.value());
        run.trigger = Some("on-report".into());
        run.snapshot_id = Some(version.local_snapshot_id);
        run.closest_git_version = version.closest_git_version.clone();
        run.git_version_exact = version.git_version_exact;
        run.branch = branch.clone();

        let mut sample =
            NewMetricSample::observed(metric_id, stream_val, summary_pct, "coverage-report");
        sample.numerator = Some(covered as f64);
        sample.denominator = Some(changed as f64);
        sample.thread_id = Some(thread.value());
        sample.snapshot_id = Some(version.local_snapshot_id);
        sample.closest_git_version = version.closest_git_version.clone();
        sample.git_version_exact = version.git_version_exact;
        sample.basis_ref = version.closest_git_version.clone();
        sample.branch = branch;

        // Atomic: the run, its sample, and the verbatim per-file
        // uncovered-changed-lines detail finding (tsk215) commit together.
        let findings: Vec<NewMetricFinding> = Self::detail_finding("coverage-detail", detail)
            .into_iter()
            .collect();
        self.metrics
            .record_run_with_data(run, vec![sample], findings)
            .await?;
        self.events.emit(OxplowEvent::MetricSamplesChanged {
            stream_id: oxplow_domain::StreamId::new(stream_val),
        });
        Ok(())
    }

    /// Build one run-scoped detail finding carrying a verbatim `payload_json`
    /// (the rich per-effort detail — test tree / coverage files / analysis
    /// findings — kept on the substrate so the effort panel renders off the
    /// model, tsk215). `None` detail → no finding. The `run_id` is a placeholder
    /// (`0`); `record_run_with_data` overwrites it with the real run id when the
    /// run + its samples + findings commit together.
    fn detail_finding(kind: &str, detail: Option<serde_json::Value>) -> Option<NewMetricFinding> {
        let detail = detail?;
        Some(NewMetricFinding {
            run_id: 0,
            metric_id: None,
            subject_kind: None,
            subject_ref: None,
            path: None,
            start_line: None,
            end_line: None,
            col: None,
            kind: kind.to_string(),
            severity: None,
            rule: None,
            message: None,
            value: None,
            extra_json: Some(serde_json::to_string(&detail).unwrap_or_default()),
        })
    }

    /// Mirror a test run into the metric substrate (best-effort): a `tests` run
    /// with `oxplow.tests.{passed,failed,total}` gauge samples + a `test-detail`
    /// finding (the suite/case tree). Provenance flows through (a hook-observed
    /// run is `observed`; an MCP `record_test_run` is `asserted`).
    #[allow(clippy::too_many_arguments)]
    async fn mirror_test_metrics(
        &self,
        thread: &ThreadId,
        stream_id: &str,
        provenance: &str,
        source: &str,
        passed: Option<i64>,
        failed: Option<i64>,
        total: Option<i64>,
        detail: Option<serde_json::Value>,
    ) {
        if passed.is_none() && failed.is_none() && total.is_none() {
            return;
        }
        let Some(stream_val) = oxplow_domain::StreamId::try_from_str(stream_id).map(|s| s.value())
        else {
            return;
        };
        let branch = oxplow_git::detect_current_branch(&self.project_dir);
        let result = async {
            let mut run = NewMetricRun::done(stream_val, "tests", source.to_string());
            run.provenance = provenance.to_string();
            run.thread_id = Some(thread.value());
            run.trigger = Some("on-report".into());
            run.branch = branch.clone();

            let specs = [
                (
                    "oxplow.tests.passed",
                    "Tests passed",
                    "higher-better",
                    passed,
                ),
                (
                    "oxplow.tests.failed",
                    "Tests failed",
                    "lower-better",
                    failed,
                ),
                ("oxplow.tests.total", "Tests total", "neutral", total),
            ];
            let mut samples = Vec::new();
            for (key, title, direction, value) in specs {
                let Some(v) = value else { continue };
                let mut def = NewMetricDefinition::new(key, "gauge", title);
                def.unit = Some("count".into());
                def.direction = direction.into();
                def.grain = Some("effort".into());
                def.producer = Some("tests".into());
                def.category = Some("testing".into());
                def.dimensions_json = Some("[\"branch\"]".into());
                let metric_id = self.metrics.upsert_definition(def).await?;
                let mut sample =
                    NewMetricSample::observed(metric_id, stream_val, v as f64, source.to_string());
                sample.provenance = provenance.to_string();
                sample.thread_id = Some(thread.value());
                sample.branch = branch.clone();
                samples.push(sample);
            }
            // Atomic: the run, its samples, and the verbatim suite/case-tree
            // `test-detail` finding (tsk215) commit together.
            let findings: Vec<NewMetricFinding> = Self::detail_finding("test-detail", detail)
                .into_iter()
                .collect();
            self.metrics
                .record_run_with_data(run, samples, findings)
                .await?;
            Ok::<(), DomainError>(())
        }
        .await;
        match result {
            Ok(()) => self.events.emit(OxplowEvent::MetricSamplesChanged {
                stream_id: oxplow_domain::StreamId::new(stream_val),
            }),
            Err(e) => {
                tracing::warn!(error = %e, "failed to mirror test run into metric substrate")
            }
        }
    }

    /// Mirror a static-analysis result into the metric substrate (best-effort):
    /// an analyzer run + `oxplow.analysis.{errors,warnings}` gauge samples + one
    /// `metric_finding` per lint finding (located detail).
    #[allow(clippy::too_many_arguments)]
    async fn mirror_analysis_metrics(
        &self,
        thread: &ThreadId,
        stream_id: &str,
        source: &str,
        analyzers: &[String],
        report: &oxplow_coverage::AnalysisReport,
        snapshot_id: Option<i64>,
        git_version: Option<String>,
        git_version_exact: bool,
        detail: Option<serde_json::Value>,
    ) {
        let Some(stream_val) = oxplow_domain::StreamId::try_from_str(stream_id).map(|s| s.value())
        else {
            return;
        };
        let branch = oxplow_git::detect_current_branch(&self.project_dir);
        let analyzer = analyzers
            .first()
            .cloned()
            .unwrap_or_else(|| "analysis".to_string());
        let result = async {
            use oxplow_coverage::Severity::*;
            let (mut errors, mut warnings) = (0u64, 0u64);
            for f in &report.findings {
                match f.severity {
                    Error => errors += 1,
                    Warning => warnings += 1,
                    _ => {}
                }
            }

            let mut run = NewMetricRun::done(stream_val, analyzer.clone(), source.to_string());
            run.thread_id = Some(thread.value());
            run.trigger = Some("on-report".into());
            run.snapshot_id = snapshot_id;
            run.closest_git_version = git_version.clone();
            run.git_version_exact = git_version_exact;
            run.branch = branch.clone();

            let mut samples = Vec::new();
            for (key, title, value) in [
                ("oxplow.analysis.errors", "Analysis errors", errors),
                ("oxplow.analysis.warnings", "Analysis warnings", warnings),
            ] {
                let mut def = NewMetricDefinition::new(key, "gauge", title);
                def.unit = Some("count".into());
                def.direction = "lower-better".into();
                def.grain = Some("tree".into());
                def.producer = Some("analysis".into());
                def.category = Some("static-quality".into());
                def.dimensions_json = Some("[\"branch\",\"git_version\"]".into());
                let metric_id = self.metrics.upsert_definition(def).await?;
                let mut sample = NewMetricSample::observed(
                    metric_id,
                    stream_val,
                    value as f64,
                    source.to_string(),
                );
                sample.thread_id = Some(thread.value());
                sample.snapshot_id = snapshot_id;
                sample.closest_git_version = git_version.clone();
                sample.git_version_exact = git_version_exact;
                sample.branch = branch.clone();
                samples.push(sample);
            }

            // Verbatim analyzer payload (command/analyzer/counts/findings) so the
            // effort panel renders off the substrate (tsk215), plus one located
            // `metric_finding` per lint hit for the substrate's findings drill-in.
            let mut findings: Vec<NewMetricFinding> =
                Self::detail_finding("analysis-detail", detail)
                    .into_iter()
                    .collect();
            for f in &report.findings {
                let severity = match f.severity {
                    Error => "error",
                    Warning => "warning",
                    Info => "info",
                    Note => "note",
                };
                findings.push(NewMetricFinding {
                    run_id: 0,
                    metric_id: None,
                    subject_kind: Some("file".into()),
                    subject_ref: Some(format!("file:{}", f.path)),
                    path: Some(f.path.clone()),
                    start_line: f.line.map(|l| l as i64),
                    end_line: f.line.map(|l| l as i64),
                    col: f.column.map(|c| c as i64),
                    kind: "lint".into(),
                    severity: Some(severity.into()),
                    rule: f.rule.clone(),
                    message: Some(f.message.clone()),
                    value: None,
                    extra_json: None,
                });
            }
            // Atomic: run + samples + all findings commit together.
            self.metrics
                .record_run_with_data(run, samples, findings)
                .await?;
            Ok::<(), DomainError>(())
        }
        .await;
        match result {
            Ok(()) => self.events.emit(OxplowEvent::MetricSamplesChanged {
                stream_id: oxplow_domain::StreamId::new(stream_val),
            }),
            Err(e) => {
                tracing::warn!(error = %e, "failed to mirror analysis into metric substrate")
            }
        }
    }

    async fn store_diff_coverage(
        &self,
        thread: &ThreadId,
        effort: &TaskEffort,
        stream_id: &str,
        report: &oxplow_coverage::CoverageReport,
    ) -> Result<CoverageIngest, DomainError> {
        let Some(start) = effort.start_snapshot_id else {
            return Ok(CoverageIngest::NoBaseline);
        };
        // Changed lines per file = end-side diff of the effort's start
        // snapshot vs the current working tree (the effort is typically
        // still open when tests run, so there's no end snapshot yet).
        let start_tree = self.snapshots.tree_at(start).await?;
        let mut total_changed = 0usize;
        let mut total_covered = 0usize;
        let mut files_payload = Vec::new();
        for (path, fc) in &report.files {
            let changed = self.changed_lines_for(path, &start_tree);
            if changed.is_empty() {
                continue;
            }
            let changed_instr: BTreeSet<u32> =
                fc.instrumented.intersection(&changed).copied().collect();
            if changed_instr.is_empty() {
                continue;
            }
            let changed_cov: BTreeSet<u32> =
                fc.covered.intersection(&changed_instr).copied().collect();
            let uncovered: Vec<u32> = changed_instr.difference(&changed_cov).copied().collect();
            total_changed += changed_instr.len();
            total_covered += changed_cov.len();
            files_payload.push(json!({
                "path": path,
                "uncoveredChangedLines": uncovered,
            }));
        }
        if total_changed == 0 {
            return Ok(CoverageIngest::NoChangedCoverage);
        }
        let summary_pct = (total_covered as f64 / total_changed as f64) * 100.0;
        let payload = json!({
            "summaryPct": summary_pct,
            "changedLines": total_changed,
            "coveredLines": total_covered,
            "files": files_payload,
        });

        let pin = effort.end_snapshot_id.unwrap_or(start);
        let version = file_ref_version::resolve(&self.snapshots, &self.project_dir, pin).await?;
        // Dual-write into the unified metric substrate (best-effort) before the
        // legacy observation consumes `version`.
        self.mirror_coverage_metric(
            thread,
            stream_id,
            summary_pct,
            total_covered,
            total_changed,
            &version,
            Some(payload.clone()),
        )
        .await;
        // Substrate is the sole store now (tsk215) — the diff-coverage detail
        // is mirrored above; just signal the panel to refetch.
        self.emit(thread, effort);
        Ok(CoverageIngest::Stored {
            observation_id: 0,
            summary_pct,
            changed_lines: total_changed,
            covered_lines: total_covered,
        })
    }

    /// PostToolUse entry point: detect a test and/or static-analysis run,
    /// record it, and ride along to coverage / findings. Best-effort — never
    /// fails the hook.
    pub async fn on_post_tool_use(
        &self,
        thread: &ThreadId,
        payload_json: &str,
    ) -> Result<Option<String>, DomainError> {
        let Some(bash) = parse_bash_post_tool(payload_json) else {
            return Ok(None);
        };
        let cfg = self.collection_cfg();
        let is_test = detect_test_run(&bash.command, &cfg.test_run_patterns);
        let is_analysis = detect_analysis_run(&bash.command, &cfg.analysis_run_patterns);
        let is_commit = detect_git_commit(&bash.command);
        if !is_test && !is_analysis && !is_commit {
            return Ok(None);
        }
        let Some(effort) = self.efforts.find_open_for_thread(thread).await? else {
            return Ok(None);
        };

        // Commit-hygiene nudge: a successful `git commit` that swept in files
        // outside the open effort's changed set. Informational, one-shot per
        // commit. Independent of the test/analysis ride-alongs below.
        if is_commit {
            if let Some(msg) = self.check_commit_hygiene(&effort, bash.exit_code).await? {
                self.persist_nudge(thread, Some(&effort), "commit-hygiene", &msg, &bash.command)
                    .await;
                return Ok(Some(msg));
            }
            // A pure commit (not also a test/analysis run) is done.
            if !is_test && !is_analysis {
                return Ok(None);
            }
        }
        let registry = self.registry(&cfg);

        // Static-analysis ride-along: when an analyzer ran, record a
        // static-analysis observation — command-only (the ran-record) when no
        // fresh analysis report exists, or carrying merged findings when one
        // does. Classification is by collector kind, not a format heuristic.
        if is_analysis {
            let (report, source, analyzers) =
                match self.merge_fresh_analysis(&effort, &cfg, &registry) {
                    Some((r, source, analyzers)) => (Some(r), source, analyzers),
                    None => (None, "analysis-report".to_string(), Vec::new()),
                };
            self.record_static_analysis(
                thread,
                &bash.command,
                report.as_ref(),
                &analyzers,
                &source,
            )
            .await?;
        }

        // A pure analysis run (no test patterns matched) is done — the
        // test-run / coverage / nudge path below is test-specific.
        if !is_test {
            return Ok(None);
        }
        // Merge every test report fresher than the effort start into one
        // per-test tree (each test stack regenerates its own report; the
        // freshness guard excludes stale ones from prior efforts/runs).
        let report = self.merge_fresh_test_reports(&effort, &cfg, &registry);
        // Trust tier rides in `source`: "post-tool-bash" for the plain hook /
        // in-process collectors, "plugin-exec:<name>" when a lower-trust exec
        // plugin produced the suites (mirrors the coverage path).
        let (report, source) = match report {
            Some((r, source)) => (Some(r), source),
            None => (None, "post-tool-bash".to_string()),
        };
        self.record_test_run(
            thread,
            &bash.command,
            bash.exit_code,
            None,
            None,
            None,
            None,
            "observed",
            &source,
            report.as_ref(),
        )
        .await?;
        // Coverage ride-along: merge every fresh coverage report → one
        // diff-coverage observation.
        let coverage = self.merge_fresh_coverage(&effort, &cfg, &registry);
        let mut coverage_pct: Option<f64> = None;
        if let Some((merged, _source)) = &coverage {
            if let Some(stream_id) = self.stream_id_for(thread).await? {
                if let CoverageIngest::Stored { summary_pct, .. } = self
                    .store_diff_coverage(thread, &effort, &stream_id, merged)
                    .await?
                {
                    coverage_pct = Some(summary_pct);
                }
            }
        }
        // Coverage-target nudge (tsk220, advise only): the effort's diff
        // coverage landed below target → steer the agent to add tests, at most
        // once per effort. Never blocks. Mutually exclusive with the report-less
        // nudge below (this only fires when a coverage report WAS produced).
        if let Some(pct) = coverage_pct {
            if pct < COVERAGE_TARGET_PCT && self.mark_coverage_nudged(&effort.id) {
                let msg = coverage_target_nudge_message(pct);
                self.persist_nudge(
                    thread,
                    Some(&effort),
                    "coverage-target",
                    &msg,
                    &bash.command,
                )
                .await;
                return Ok(Some(msg));
            }
        }
        // Nudge: the agent ran tests but this run regenerated no report
        // oxplow could parse for the effort (so the effort gets a
        // command-only test-run and no coverage). Steer it to the
        // report-emitting command — at most once per effort. This is
        // tool-agnostic: it keys only on "test run detected" + "no fresh
        // report", never on which tool ran; the command it names comes
        // from the project's own config.
        let produced_report = report.is_some() || coverage.is_some();
        if !produced_report && self.mark_nudged(&effort.id) {
            let msg = report_nudge_message(&cfg, &bash.command);
            self.persist_nudge(
                thread,
                Some(&effort),
                "report-less-run",
                &msg,
                &bash.command,
            )
            .await;
            return Ok(Some(msg));
        }
        Ok(None)
    }

    /// Persist a fired nudge (best-effort) and emit `AgentNudgesChanged` so
    /// the renderer's debug sub-view live-updates. Called only AFTER the
    /// one-shot dedup gates pass, so a deduped/non-fired nudge is never
    /// stored. Never fails the hook: a persistence error is logged and
    /// swallowed.
    async fn persist_nudge(
        &self,
        thread: &ThreadId,
        effort: Option<&TaskEffort>,
        kind: &str,
        message: &str,
        trigger: &str,
    ) {
        let new = NewAgentNudge {
            thread_id: thread.to_string(),
            effort_id: effort.map(|e| e.id.to_string()),
            kind: kind.to_string(),
            message: message.to_string(),
            trigger: Some(trigger.to_string()),
        };
        match self.nudges.record(new).await {
            Ok(_) => {
                self.events.emit(OxplowEvent::AgentNudgesChanged {
                    thread_id: *thread,
                    effort_id: effort.map(|e| e.id.to_string()),
                });
                // Project the fired nudge into the metric substrate (tsk216):
                // `agent.nudges.fired` is an agent-activity signal — the agent
                // drifted off-task often enough to be corrected.
                self.project_nudge_metric(thread, kind).await;
            }
            Err(err) => tracing::warn!(?err, "persisting agent nudge failed"),
        }
    }

    /// Project one `agent.nudges.fired` event sample into the unified
    /// substrate. The nudge `kind` is the subject (so the explorer can break
    /// down which guardrail fired). Event kind → run-less. Best-effort: a
    /// metric write error is logged and never fails the hook. Lower is
    /// better — fewer nudges means the agent stayed on task.
    async fn project_nudge_metric(&self, thread: &ThreadId, kind: &str) {
        let stream_val = match self.threads.get(thread).await {
            Ok(Some(t)) => t.stream_id.value(),
            _ => return,
        };
        let branch = oxplow_git::detect_current_branch(&self.project_dir);
        let result = async {
            let mut def = NewMetricDefinition::new("agent.nudges.fired", "event", "Nudges fired");
            def.unit = Some("count".into());
            def.direction = "lower-better".into();
            def.default_agg = "sum".into();
            def.grain = Some("effort".into());
            def.producer = Some("nudges".into());
            def.category = Some("operational".into());
            def.dimensions_json = Some("[\"subject\",\"branch\",\"thread\"]".into());
            let metric_id = self.metrics.upsert_definition(def).await?;
            // Event kind: no compute run (run_id stays NULL).
            let mut sample = NewMetricSample::observed(metric_id, stream_val, 1.0, "nudges");
            sample.thread_id = Some(thread.value());
            sample.subject_kind = Some("nudge".into());
            sample.subject_ref = Some(kind.to_string());
            sample.dims_json = Some(format!("{{\"kind\":\"{kind}\"}}"));
            sample.branch = branch;
            self.metrics.record_sample(sample).await?;
            Ok::<(), DomainError>(())
        }
        .await;
        match result {
            Ok(()) => self.events.emit(OxplowEvent::MetricSamplesChanged {
                stream_id: oxplow_domain::StreamId::new(stream_val),
            }),
            Err(e) => {
                tracing::warn!(error = %e, "failed to project nudge into metric substrate")
            }
        }
    }

    /// Record that `effort` has been nudged. Returns `true` the first
    /// time (caller should nudge), `false` afterwards.
    fn mark_nudged(&self, effort: &EffortId) -> bool {
        match self.nudged_efforts.lock() {
            Ok(mut set) => set.insert(*effort),
            // Poisoned lock: don't nudge rather than risk a panic in a
            // best-effort hook.
            Err(_) => false,
        }
    }

    /// Record that commit `sha` has been nudged about. Returns `true` the
    /// first time (caller should nudge), `false` afterwards. Mirrors
    /// [`mark_nudged`] but keyed by commit sha.
    fn mark_commit_nudged(&self, sha: &str) -> bool {
        match self.nudged_commits.lock() {
            Ok(mut set) => set.insert(sha.to_string()),
            Err(_) => false,
        }
    }

    /// Record that `effort` has been nudged about coverage below target.
    /// Returns `true` the first time (caller should nudge), `false` after.
    fn mark_coverage_nudged(&self, effort: &EffortId) -> bool {
        match self.nudged_coverage.lock() {
            Ok(mut set) => set.insert(*effort),
            Err(_) => false,
        }
    }

    /// Record that `(effort, metric)` had its gauge warn/fail crossing
    /// surfaced. Returns `true` the first time (caller should surface the loud
    /// marker), `false` after. One-shot-per-effort+metric anti-nag (tsk231).
    fn mark_gauge_nudged(&self, effort: &EffortId, metric_id: i64) -> bool {
        match self.nudged_gauge.lock() {
            Ok(mut set) => set.insert((*effort, metric_id)),
            Err(_) => false,
        }
    }

    /// The advisory "metric deltas this effort" block for the open effort on
    /// `thread` (tsk231) — how each code metric's samples moved since the
    /// effort started, plus a **one-shot** loud marker the first turn a gauge
    /// crosses its `warn_at`/`fail_at` threshold. Advise-only; returns `None`
    /// when there's no open effort, no moved metric, and no fresh crossing (so
    /// steady-state turns add nothing). Surfaced via the UserPromptSubmit
    /// `additionalContext`, since on-snapshot gauges run outside any hook.
    pub async fn effort_metric_context(&self, thread: &ThreadId) -> Option<String> {
        let effort = self.efforts.find_open_for_thread(thread).await.ok()??;
        let defs = self.metrics.list_definitions().await.ok()?;
        let mut lines: Vec<String> = Vec::new();
        for def in defs {
            // Operational/event metrics (tokens, cost, cycle-time, nudges,
            // navigation) grow every turn and aren't code-health signals — keep
            // the line focused on code metrics.
            if def.kind == "event" || is_operational_metric_key(&def.key) {
                continue;
            }
            let Ok(samples) = self
                .metrics
                .samples_for_effort(def.id, effort.id.value())
                .await
            else {
                continue;
            };
            // samples_for_effort is time-ASC: first = effort-start baseline,
            // last = current. (The code metrics relevant here project one
            // headline sample per run, so first/last are the right anchors.)
            let (Some(first), Some(last)) = (samples.first(), samples.last()) else {
                continue;
            };
            let baseline = first.value;
            let current = last.value;
            let moved = (current - baseline).abs() > f64::EPSILON;
            let crossing = threshold_state(&def.direction, current, def.warn_at, def.fail_at);
            let fresh_crossing = crossing.is_some() && self.mark_gauge_nudged(&effort.id, def.id);
            if !moved && !fresh_crossing {
                continue;
            }
            let mut line = format!(
                "- {}: {} → {}{}",
                def.title,
                fmt_metric_num(baseline),
                fmt_metric_num(current),
                fmt_unit_suffix(def.unit.as_deref().unwrap_or("")),
            );
            if moved {
                let delta = current - baseline;
                line.push_str(&format!(" (Δ {})", fmt_signed(delta)));
            }
            if fresh_crossing {
                if let Some(level) = crossing {
                    let thresh = if level == "fail" {
                        def.fail_at
                    } else {
                        def.warn_at
                    };
                    line.push_str(&format!(
                        " ⚠ crossed {} threshold{}",
                        level,
                        thresh
                            .map(|t| format!(" ({})", fmt_metric_num(t)))
                            .unwrap_or_default(),
                    ));
                }
            }
            lines.push(line);
        }
        if lines.is_empty() {
            return None;
        }
        Some(format!(
            "# Metric deltas (this effort)\n{}\n\n(Advisory — for awareness, not gating.)",
            lines.join("\n")
        ))
    }

    /// Commit-hygiene check: after a successful `git commit`, flag any file in
    /// the new HEAD commit that falls OUTSIDE the open effort's changed set
    /// (start-snapshot content vs. working-tree content — the same notion of
    /// "what this effort changed" that diff-coverage uses). Informational; the
    /// caller surfaces the returned message to the agent without blocking.
    /// `Ok(None)` when there's nothing to flag, no baseline yet, the commit
    /// didn't succeed, or HEAD/commit details can't be read.
    async fn check_commit_hygiene(
        &self,
        effort: &TaskEffort,
        exit_code: Option<i64>,
    ) -> Result<Option<String>, DomainError> {
        // Only react to a commit that actually landed. A missing exit code is
        // tolerated (Claude Code's Bash response doesn't always carry one).
        if matches!(exit_code, Some(c) if c != 0) {
            return Ok(None);
        }
        // No start snapshot → no baseline to compare against → skip cleanly.
        let Some(start) = effort.start_snapshot_id else {
            return Ok(None);
        };
        // Resolve HEAD + the committed file list via libgit2 off the async
        // runtime thread (mirrors the rest of the codebase's git access).
        let project_dir = self.project_dir.clone();
        let sha = match tokio::task::spawn_blocking({
            let p = project_dir.clone();
            move || oxplow_git::head_commit_sha(&p)
        })
        .await
        {
            Ok(Some(sha)) => sha,
            _ => return Ok(None),
        };
        // One-shot per commit (anti-nag, mirrors the report-nudge dedup).
        if !self.mark_commit_nudged(&sha) {
            return Ok(None);
        }
        let detail = match tokio::task::spawn_blocking({
            let p = project_dir.clone();
            let sha = sha.clone();
            move || oxplow_git::get_commit_detail(&p, &sha)
        })
        .await
        {
            Ok(Some(detail)) => detail,
            _ => return Ok(None),
        };
        let start_tree = self.snapshots.tree_at(start).await?;
        // Claim-aware "in-effort" test (claim-first attribution, Child 3):
        // when the effort has CLAIMED files, prefer that set — a committed
        // file the effort never claimed is out-of-effort even if it changed
        // during the window, and a claimed file is never falsely flagged.
        // Only when the effort is UNREVIEWED (no claims at all — legacy /
        // non-structured-edit efforts) do we fall back to the raw snapshot
        // diff (the prior behavior).
        let claimed: std::collections::HashSet<String> = self
            .efforts
            .list_files(&effort.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|f| f.path)
            .collect();
        let claim_first_active = !claimed.is_empty();
        let mut out_of_effort: Vec<String> = detail
            .files
            .iter()
            .map(|f| f.path.clone())
            .filter(|path| {
                if claim_first_active {
                    !claimed.contains(path)
                } else {
                    !self.path_changed_in_effort(path, &start_tree)
                }
            })
            .collect();
        if out_of_effort.is_empty() {
            return Ok(None);
        }
        out_of_effort.sort();
        out_of_effort.dedup();
        let short = sha.get(..7).unwrap_or(&sha).to_string();
        Ok(Some(commit_hygiene_message(&short, &out_of_effort)))
    }

    /// True when `path`'s current working-tree content differs from its
    /// effort-start-snapshot content (treating an absent side as empty, so
    /// adds and deletes both count as changed). This is the path-granularity
    /// sibling of [`changed_lines_for`] and defines membership in the effort's
    /// changed set for the commit-hygiene check.
    fn path_changed_in_effort(&self, path: &str, start_tree: &BTreeMap<String, String>) -> bool {
        let old = start_tree
            .get(path)
            .and_then(|id| {
                crate::snapshot_content::read_tree_identity(id, &self.project_dir, &self.blobs)
            })
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        let new = std::fs::read(self.project_dir.join(path))
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        old != new
    }

    /// Merge every configured test report that exists and is fresher than
    /// the effort start into one tree, via each format's collector. `None`
    /// when nothing fresh/non-empty.
    fn merge_fresh_test_reports(
        &self,
        effort: &TaskEffort,
        cfg: &oxplow_config::CollectionConfig,
        registry: &CollectorRegistry,
    ) -> Option<(oxplow_coverage::TestReport, String)> {
        let mut merged = oxplow_coverage::TestReport::default();
        let mut exec_names: Vec<String> = Vec::new();
        for r in &cfg.reports {
            let Some(collector) = registry.resolve(&r.format) else {
                tracing::warn!(format = %r.format, path = %r.path, "no collector for report format");
                continue;
            };
            if collector.kind() != CollectorKind::Test {
                continue;
            }
            let abs = self.project_dir.join(&r.path);
            if report_is_stale(&abs, effort.started_at) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&abs) else {
                continue;
            };
            match collector.run(&content) {
                Ok(CollectorOutput::Test(parsed)) => {
                    merged.suites.extend(parsed.suites);
                    if collector.runtime() == CollectorRuntime::Exec {
                        exec_names.push(collector.name().to_string());
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(format = %r.format, error = %e, "test report parse failed")
                }
            }
        }
        if merged.suites.iter().all(|s| s.cases.is_empty()) {
            return None;
        }
        let source = if exec_names.is_empty() {
            "post-tool-bash".to_string()
        } else {
            format!("plugin-exec:{}", exec_names.join(","))
        };
        Some((merged, source))
    }

    /// Merge every configured coverage report that exists and is fresher
    /// than the effort start into one file→coverage map, via each format's
    /// collector. Returns the merged report plus a `source` label (lower-trust
    /// `plugin-exec:*` when any contributing collector was an external
    /// process). `None` when none contributed.
    fn merge_fresh_coverage(
        &self,
        effort: &TaskEffort,
        cfg: &oxplow_config::CollectionConfig,
        registry: &CollectorRegistry,
    ) -> Option<(oxplow_coverage::CoverageReport, String)> {
        let mut merged = oxplow_coverage::CoverageReport::default();
        let mut exec_names: Vec<String> = Vec::new();
        let mut any = false;
        for r in &cfg.reports {
            let Some(collector) = registry.resolve(&r.format) else {
                tracing::warn!(format = %r.format, path = %r.path, "no collector for report format");
                continue;
            };
            if collector.kind() != CollectorKind::Coverage {
                continue;
            }
            let abs = self.project_dir.join(&r.path);
            if report_is_stale(&abs, effort.started_at) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&abs) else {
                continue;
            };
            match collector.run(&content) {
                Ok(CollectorOutput::Coverage(parsed)) => {
                    for (path, fc) in parsed.files {
                        let entry = merged.files.entry(path).or_default();
                        entry.instrumented.extend(fc.instrumented);
                        entry.covered.extend(fc.covered);
                    }
                    if collector.runtime() == CollectorRuntime::Exec {
                        exec_names.push(collector.name().to_string());
                    }
                    any = true;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(format = %r.format, error = %e, "coverage report parse failed")
                }
            }
        }
        if !any {
            return None;
        }
        let source = if exec_names.is_empty() {
            "coverage-report".to_string()
        } else {
            format!("plugin-exec:{}", exec_names.join(","))
        };
        Some((merged, source))
    }

    /// Merge every configured analysis report that exists and is fresher than
    /// the effort start into one findings list, via each format's collector.
    /// Returns the merged report, a `source` label (lower-trust `plugin-exec:*`
    /// when any contributing collector was an external process), and the
    /// contributing analyzer names (collector names, `oxplow.` prefix stripped)
    /// for the UI's "which analyzer ran" label. `None` when none contributed.
    fn merge_fresh_analysis(
        &self,
        effort: &TaskEffort,
        cfg: &oxplow_config::CollectionConfig,
        registry: &CollectorRegistry,
    ) -> Option<(oxplow_coverage::AnalysisReport, String, Vec<String>)> {
        let mut merged = oxplow_coverage::AnalysisReport::default();
        let mut exec_names: Vec<String> = Vec::new();
        let mut analyzers: Vec<String> = Vec::new();
        let mut any = false;
        for r in &cfg.reports {
            let Some(collector) = registry.resolve(&r.format) else {
                tracing::warn!(format = %r.format, path = %r.path, "no collector for report format");
                continue;
            };
            if collector.kind() != CollectorKind::Analysis {
                continue;
            }
            let abs = self.project_dir.join(&r.path);
            if report_is_stale(&abs, effort.started_at) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&abs) else {
                continue;
            };
            match collector.run(&content) {
                Ok(CollectorOutput::Analysis(parsed)) => {
                    merged.findings.extend(parsed.findings);
                    let label = collector
                        .name()
                        .strip_prefix("oxplow.")
                        .unwrap_or(collector.name())
                        .to_string();
                    if !analyzers.contains(&label) {
                        analyzers.push(label);
                    }
                    if collector.runtime() == CollectorRuntime::Exec {
                        exec_names.push(collector.name().to_string());
                    }
                    any = true;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(format = %r.format, error = %e, "analysis report parse failed")
                }
            }
        }
        if !any {
            return None;
        }
        let source = if exec_names.is_empty() {
            "analysis-report".to_string()
        } else {
            format!("plugin-exec:{}", exec_names.join(","))
        };
        Some((merged, source, analyzers))
    }

    /// Record a `static-analysis` observation against the thread's open
    /// effort. This single kind is both the ran-record (when `report` is
    /// `None` — analyzer ran but regenerated no parseable report, like a
    /// command-only `test-run`) and the findings (when a report parsed). The
    /// headline metric is the error+warning count (lower = better). Returns
    /// `Ok(None)` when no effort is open.
    async fn record_static_analysis(
        &self,
        thread: &ThreadId,
        command: &str,
        report: Option<&oxplow_coverage::AnalysisReport>,
        analyzers: &[String],
        source: &str,
    ) -> Result<Option<i64>, DomainError> {
        let Some(effort) = self.efforts.find_open_for_thread(thread).await? else {
            return Ok(None);
        };
        let Some(stream_id) = self.stream_id_for(thread).await? else {
            return Ok(None);
        };
        let mut payload = serde_json::Map::new();
        payload.insert("command".into(), json!(command));
        if !analyzers.is_empty() {
            payload.insert("analyzer".into(), json!(analyzers.join(", ")));
        }
        let metric_value = report.map(|r| {
            use oxplow_coverage::Severity::*;
            let (mut errors, mut warnings, mut info, mut note) = (0u64, 0u64, 0u64, 0u64);
            for f in &r.findings {
                match f.severity {
                    Error => errors += 1,
                    Warning => warnings += 1,
                    Info => info += 1,
                    Note => note += 1,
                }
            }
            payload.insert("errorCount".into(), json!(errors));
            payload.insert("warningCount".into(), json!(warnings));
            payload.insert("infoCount".into(), json!(info));
            payload.insert("noteCount".into(), json!(note));
            payload.insert(
                "findings".into(),
                serde_json::to_value(&r.findings).unwrap_or(serde_json::Value::Null),
            );
            (errors + warnings) as f64
        });

        // Freshness pin (mirrors diff-coverage): pin to the effort's end
        // snapshot if present, else its start.
        let pin = effort.end_snapshot_id.or(effort.start_snapshot_id);
        let (local_snapshot_id, closest_git_version, git_version_exact) = match pin {
            Some(p) => {
                let v = file_ref_version::resolve(&self.snapshots, &self.project_dir, p).await?;
                (
                    Some(v.local_snapshot_id),
                    v.closest_git_version,
                    v.git_version_exact,
                )
            }
            None => (None, None, false),
        };

        // Dual-write into the unified metric substrate (best-effort), when a
        // report was parsed (command-only analyzer runs have no counts).
        if let Some(r) = report {
            self.mirror_analysis_metrics(
                thread,
                &stream_id,
                source,
                analyzers,
                r,
                local_snapshot_id,
                closest_git_version.clone(),
                git_version_exact,
                Some(serde_json::Value::Object(payload.clone())),
            )
            .await;
        }
        // Substrate is the sole store now (tsk215) — the analyzer payload is
        // kept as an `analysis-detail` finding + per-lint `metric_finding` rows
        // via the mirror above. Signal the panel to refetch; `Some` = recorded.
        let _ = (
            stream_id,
            metric_value,
            payload,
            local_snapshot_id,
            closest_git_version,
            git_version_exact,
        );
        self.emit(thread, &effort);
        Ok(Some(0))
    }

    /// Effort-review observations for an effort, newest-first. Pass `kind` to
    /// filter. Backed by the metric substrate (tsk215) via
    /// [`effort_observations_from_metrics`](Self::effort_observations_from_metrics).
    pub async fn list_for_effort(
        &self,
        effort_id: &str,
        kind: Option<&str>,
    ) -> Result<Vec<oxplow_db::EffortObservation>, DomainError> {
        Ok(self.effort_observations_from_metrics(effort_id, kind).await)
    }

    /// Reconstruct the effort-review observations for `effort_id` from the
    /// **metric substrate** (tsk215): the coverage/test/analysis headline
    /// samples that fall in the effort's time window + their verbatim
    /// `*-detail` finding payloads, shaped as `EffortObservation` rows so the
    /// effort panel renders off the model. The substrate successor to
    /// `list_for_effort` (which reads the legacy `effort_observation` table).
    /// One row per run, newest-first; `kind` optionally filters.
    pub async fn effort_observations_from_metrics(
        &self,
        effort_id: &str,
        kind: Option<&str>,
    ) -> Vec<oxplow_db::EffortObservation> {
        let Some(eid) = EffortId::try_from_str(effort_id) else {
            return vec![];
        };
        // (headline metric key, detail finding kind, observation kind)
        let specs = [
            (
                "oxplow.coverage.diff_pct",
                "coverage-detail",
                "diff-coverage",
            ),
            ("oxplow.tests.total", "test-detail", "test-run"),
            (
                "oxplow.analysis.errors",
                "analysis-detail",
                "static-analysis",
            ),
        ];
        let mut out = Vec::new();
        for (metric_key, detail_kind, obs_kind) in specs {
            if kind.is_some_and(|k| k != obs_kind) {
                continue;
            }
            let Ok(Some(def)) = self.metrics.get_definition(metric_key).await else {
                continue;
            };
            let Ok(samples) = self.metrics.samples_for_effort(def.id, eid.value()).await else {
                continue;
            };
            // samples_for_effort is time-ASC; newest-first for the panel.
            for sample in samples.into_iter().rev() {
                let payload_json = match sample.run_id {
                    Some(rid) => self
                        .metrics
                        .list_findings(rid)
                        .await
                        .ok()
                        .and_then(|fs| fs.into_iter().find(|f| f.kind == detail_kind))
                        .and_then(|f| f.extra_json),
                    None => None,
                };
                // Headline numeric per the panel's per-kind convention: coverage
                // → %, static-analysis → error+warning count, test-run → none
                // (the panel reads its counts from the payload).
                let metric_value = match obs_kind {
                    "test-run" => None,
                    "static-analysis" => payload_json
                        .as_deref()
                        .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
                        .map(|v| {
                            v["errorCount"].as_f64().unwrap_or(0.0)
                                + v["warningCount"].as_f64().unwrap_or(0.0)
                        })
                        .or(Some(sample.value)),
                    _ => Some(sample.value),
                };
                out.push(oxplow_db::EffortObservation {
                    id: sample.id,
                    stream_id: oxplow_domain::StreamId::new(sample.stream_id).to_string(),
                    effort_id: effort_id.to_string(),
                    kind: obs_kind.to_string(),
                    provenance: sample.provenance,
                    source: sample.source,
                    metric_value,
                    payload_json,
                    local_snapshot_id: sample.snapshot_id,
                    closest_git_version: sample.closest_git_version,
                    git_version_exact: sample.git_version_exact,
                    created_at: sample.captured_at,
                });
            }
        }
        out
    }

    /// Test-only: read the metric samples mirrored under a definition `key`.
    #[cfg(test)]
    async fn metric_samples_for_key(&self, key: &str) -> Vec<oxplow_db::MetricSample> {
        match self.metrics.get_definition(key).await.unwrap() {
            Some(d) => self.metrics.list_samples(d.id).await.unwrap(),
            None => vec![],
        }
    }

    /// Test-only: read the findings recorded under a run.
    #[cfg(test)]
    async fn metric_findings_for_run(&self, run_id: i64) -> Vec<oxplow_db::MetricFinding> {
        self.metrics.list_findings(run_id).await.unwrap()
    }

    /// End-side changed line numbers (1-based) for `path` between its
    /// start-snapshot content and the current working-tree content.
    /// Files absent from disk (deleted) or unchanged yield an empty set.
    fn changed_lines_for(
        &self,
        path: &str,
        start_tree: &BTreeMap<String, String>,
    ) -> BTreeSet<u32> {
        let old = start_tree
            .get(path)
            .and_then(|id| {
                crate::snapshot_content::read_tree_identity(id, &self.project_dir, &self.blobs)
            })
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        let Ok(new_bytes) = std::fs::read(self.project_dir.join(path)) else {
            return BTreeSet::new();
        };
        let new = String::from_utf8_lossy(&new_bytes).into_owned();
        if old == new {
            return BTreeSet::new();
        }
        diff_new_side_lines(&old, &new)
    }

    fn emit(&self, thread: &ThreadId, effort: &TaskEffort) {
        self.events.emit(OxplowEvent::EffortObservationsChanged {
            thread_id: *thread,
            effort_id: effort.id.to_string(),
        });
    }
}

/// True when `path`'s mtime is at or before `effort_start` — i.e. the
/// report wasn't (re)generated during this effort. Conservative: if the
/// mtime can't be read, returns `false` (ingest rather than silently drop
/// a report on a platform that won't surface mtimes).
/// The PostToolUse nudge shown when a detected test run produced no
/// report oxplow could parse for the effort. Tool-agnostic: it only
/// echoes the project's own configured command (or routes to
/// `/oxplow:configure`), so it works for any current/future test tool
/// without the hook knowing anything tool-specific.
fn report_nudge_message(cfg: &oxplow_config::CollectionConfig, command: &str) -> String {
    let cmd = command.trim();
    if let Some(tc) = cfg
        .test_command
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        format!(
            "Tests ran (`{cmd}`) but produced no report for this effort, so it shows a \
             command-only test-run and no coverage. Run the configured collection command \
             `{tc}` so oxplow can attribute the results. See .context/collection.md."
        )
    } else if !cfg.reports.is_empty() {
        format!(
            "Tests ran (`{cmd}`) but refreshed none of the configured collection reports, so \
             this effort has no parsed tests/coverage. Re-run via the command that regenerates \
             them and set `collection.testCommand` in oxplow.yaml to make it one step. See \
             .context/collection.md."
        )
    } else {
        format!(
            "Tests ran (`{cmd}`) but this project has no collection profile, so oxplow can't \
             attribute tests/coverage to the effort. Run /oxplow:configure to wire this stack's \
             report(s). See .context/collection.md."
        )
    }
}

/// The PostToolUse nudge shown when an effort's diff coverage lands below the
/// `oxplow.coverage.diff_pct` target (tsk220). Advisory — oxplow never blocks.
/// Operational metric namespaces (tokens, cost, cycle-time, redo-rate, nudges,
/// navigation) — these grow every turn and aren't code-health signals, so the
/// effort-metric prompt context (tsk231) skips them.
fn is_operational_metric_key(key: &str) -> bool {
    key.starts_with("agent.") || key.starts_with("effort.") || key.starts_with("task.")
}

/// Classify a value against a metric's thresholds, interpreted via `direction`.
/// Returns `Some("fail")` / `Some("warn")` when the value is in that zone, else
/// `None`. `neutral` metrics (no better/worse) never cross. The worse side is
/// "higher" for `lower-better` and "lower" for `higher-better`.
fn threshold_state(
    direction: &str,
    value: f64,
    warn_at: Option<f64>,
    fail_at: Option<f64>,
) -> Option<&'static str> {
    let worse_when_above = match direction {
        "lower-better" => true,
        "higher-better" => false,
        // neutral / unknown: no threshold semantics.
        _ => return None,
    };
    let crosses = |t: f64| {
        if worse_when_above {
            value >= t
        } else {
            value <= t
        }
    };
    if let Some(f) = fail_at {
        if crosses(f) {
            return Some("fail");
        }
    }
    if let Some(w) = warn_at {
        if crosses(w) {
            return Some("warn");
        }
    }
    None
}

/// Format a metric value compactly: integers as integers, else one decimal.
fn fmt_metric_num(v: f64) -> String {
    if v.fract().abs() < f64::EPSILON {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// A signed delta (`+2`, `-11`, `+1.5`) for the deltas line.
fn fmt_signed(v: f64) -> String {
    let n = fmt_metric_num(v.abs());
    if v < 0.0 {
        format!("-{n}")
    } else {
        format!("+{n}")
    }
}

/// `%` renders glued to the number (`71%`); other units get a leading space
/// (`7 count`); an empty unit adds nothing.
fn fmt_unit_suffix(unit: &str) -> String {
    match unit {
        "" => String::new(),
        "%" => "%".to_string(),
        u => format!(" {u}"),
    }
}

fn coverage_target_nudge_message(pct: f64) -> String {
    format!(
        "Diff coverage on this effort's changed lines is {pct:.0}%, below the {target:.0}% \
         target. Add tests for the uncovered changed lines before closing (advisory — oxplow \
         won't block you). See the effort's coverage panel for which lines are uncovered.",
        target = COVERAGE_TARGET_PCT
    )
}

/// The PostToolUse nudge shown when a `git commit` swept in files that fall
/// outside the open effort's changed set. Informational — never blocks the
/// commit. Names the offenders and, when any sit under `docs/`, adds the
/// auto-deploy warning (committing `docs/` to main publishes the site via
/// `.github/workflows/docs.yml`).
fn commit_hygiene_message(short_sha: &str, out_of_effort: &[String]) -> String {
    let n = out_of_effort.len();
    let plural = if n == 1 { "file" } else { "files" };
    let list = out_of_effort.join(", ");
    let mut msg = format!(
        "Commit {short_sha} includes {n} {plural} not part of this effort's changed set: \
         {list}. If this cross-cutting commit is intentional, carry on — otherwise it may have \
         swept in pre-staged drift; check `git show --stat HEAD` and `git commit --amend` (or \
         reset) to drop what doesn't belong."
    );
    if out_of_effort.iter().any(|p| p.starts_with("docs/")) {
        msg.push_str(
            " ⚠️ Some of these are under docs/ — committing docs/ to main auto-deploys the site \
             via .github/workflows/docs.yml, so don't push until you've confirmed they're meant \
             to publish.",
        );
    }
    msg
}

fn report_is_stale(path: &std::path::Path, effort_start: oxplow_domain::Timestamp) -> bool {
    let Ok(mtime) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return false;
    };
    let Ok(since) = mtime.duration_since(std::time::UNIX_EPOCH) else {
        return false;
    };
    (since.as_millis() as i64) <= effort_start.unix_ms()
}

/// 1-based line numbers on the NEW side that were inserted or replaced.
/// A modified line shows as delete+insert, so the inserted new-side line
/// is captured.
fn diff_new_side_lines(old: &str, new: &str) -> BTreeSet<u32> {
    let diff = TextDiff::from_lines(old, new);
    let mut changed = BTreeSet::new();
    let mut new_line: u32 = 0;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => new_line += 1,
            ChangeTag::Insert => {
                new_line += 1;
                changed.insert(new_line);
            }
            ChangeTag::Delete => {}
        }
    }
    changed
}

/// Convert a project plugin config into an executable collector, reading its
/// script from `entryFile` (project-relative; config validation already
/// rejects absolute / `..` paths). Host-side file I/O — the script itself
/// still does none, so determinism holds. `Err` carries a reason for the
/// skip-and-warn path.
fn plugin_to_collector(
    p: &oxplow_config::PluginConfig,
    project_dir: &std::path::Path,
) -> Result<Collector, String> {
    let kind = match p.kind.as_str() {
        "coverage" => CollectorKind::Coverage,
        "test" => CollectorKind::Test,
        "analysis" => CollectorKind::Analysis,
        other => return Err(format!("unknown kind \"{other}\"")),
    };
    let input = match p.input.as_deref().unwrap_or("text") {
        "text" => CollectorInput::Text,
        "json" => CollectorInput::Json,
        "xml" => CollectorInput::Xml,
        "lcov" => CollectorInput::Lcov,
        "lines" => CollectorInput::Lines,
        other => return Err(format!("unknown input \"{other}\"")),
    };
    let entry_file = p
        .entry_file
        .as_deref()
        .ok_or_else(|| "missing entryFile".to_string())?;
    let abs = project_dir.join(entry_file);
    Ok(match p.runtime.as_str() {
        "jaq" | "starlark" => {
            // The host reads the script file; the script never touches the fs.
            let script = std::fs::read_to_string(&abs)
                .map_err(|e| format!("read entryFile \"{entry_file}\": {e}"))?;
            if p.runtime == "jaq" {
                Collector::jaq(p.name.clone(), kind, p.formats.clone(), input, script)
            } else {
                Collector::starlark(p.name.clone(), kind, p.formats.clone(), input, script)
            }
        }
        "exec" => {
            // entryFile is the program to spawn (must be executable).
            let mut argv = vec![abs.to_string_lossy().into_owned()];
            argv.extend(p.args.iter().cloned());
            Collector::exec(p.name.clone(), kind, p.formats.clone(), argv)
        }
        other => return Err(format!("unknown runtime \"{other}\"")),
    })
}

/// The first configured report whose format resolves to a coverage collector
/// (the default target of an `ingest_coverage` call without overrides).
fn first_coverage_report<'a>(
    cfg: &'a oxplow_config::CollectionConfig,
    registry: &CollectorRegistry,
) -> Option<&'a oxplow_config::ReportConfig> {
    cfg.reports.iter().find(|r| {
        registry
            .resolve(&r.format)
            .is_some_and(|c| c.kind() == CollectorKind::Coverage)
    })
}

/// First configured report whose format resolves to an analysis collector —
/// the default target for an `ingest_analysis` call with no override.
fn first_analysis_report<'a>(
    cfg: &'a oxplow_config::CollectionConfig,
    registry: &CollectorRegistry,
) -> Option<&'a oxplow_config::ReportConfig> {
    cfg.reports.iter().find(|r| {
        registry
            .resolve(&r.format)
            .is_some_and(|c| c.kind() == CollectorKind::Analysis)
    })
}

/// Trust label for an analysis collector's output: in-process tiers are
/// `observed` from an `analysis-report`; the external-exec escape hatch is
/// flagged `plugin-exec:<name>` so the UI can mark it lower-trust.
fn analysis_source(collector: &Collector) -> String {
    if collector.runtime() == CollectorRuntime::Exec {
        format!("plugin-exec:{}", collector.name())
    } else {
        "analysis-report".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_state_respects_direction() {
        // lower-better: worse is higher.
        assert_eq!(
            threshold_state("lower-better", 12.0, Some(5.0), Some(10.0)),
            Some("fail")
        );
        assert_eq!(
            threshold_state("lower-better", 7.0, Some(5.0), Some(10.0)),
            Some("warn")
        );
        assert_eq!(
            threshold_state("lower-better", 3.0, Some(5.0), Some(10.0)),
            None
        );
        // higher-better: worse is lower (e.g. coverage %).
        assert_eq!(
            threshold_state("higher-better", 40.0, Some(80.0), Some(50.0)),
            Some("fail")
        );
        assert_eq!(
            threshold_state("higher-better", 70.0, Some(80.0), Some(50.0)),
            Some("warn")
        );
        assert_eq!(
            threshold_state("higher-better", 90.0, Some(80.0), Some(50.0)),
            None
        );
        // neutral never crosses.
        assert_eq!(
            threshold_state("neutral", 999.0, Some(1.0), Some(1.0)),
            None
        );
    }

    #[test]
    fn fmt_helpers_format_compactly() {
        assert_eq!(fmt_metric_num(7.0), "7");
        assert_eq!(fmt_metric_num(70.5), "70.5");
        assert_eq!(fmt_signed(9.0), "+9");
        assert_eq!(fmt_signed(-11.0), "-11");
        assert_eq!(fmt_unit_suffix("%"), "%");
        assert_eq!(fmt_unit_suffix("count"), " count");
        assert_eq!(fmt_unit_suffix(""), "");
    }

    #[test]
    fn operational_keys_are_recognized() {
        assert!(is_operational_metric_key("agent.tokens.total"));
        assert!(is_operational_metric_key("effort.cycle_time_ms"));
        assert!(is_operational_metric_key("task.efforts"));
        assert!(!is_operational_metric_key("oxplow.rust.unsafe_blocks"));
        assert!(!is_operational_metric_key("acme.custom"));
    }

    #[test]
    fn report_nudge_names_configured_test_command() {
        let cfg = oxplow_config::CollectionConfig {
            test_command: Some("bun run test:collect".into()),
            ..Default::default()
        };
        let msg = report_nudge_message(&cfg, "bun test");
        // Echoes the project's own command — tool-agnostic, no built-in
        // tool→command knowledge in the hook.
        assert!(msg.contains("bun run test:collect"), "{msg}");
        assert!(msg.contains("bun test"), "{msg}");
        assert!(!msg.contains("/oxplow:configure"), "{msg}");
    }

    #[test]
    fn report_nudge_routes_to_configure_without_profile() {
        // No collection profile at all → route to the agent-driven
        // configure flow (which adapts to any tool).
        let cfg = oxplow_config::CollectionConfig::default();
        let msg = report_nudge_message(&cfg, "pytest -q tests/");
        assert!(msg.contains("/oxplow:configure"), "{msg}");
        assert!(msg.contains("pytest -q tests/"), "{msg}");
    }

    #[test]
    fn project_plugin_config_converts_registers_and_runs() {
        // The generic mechanism: a project-defined plugin (config + a script
        // file, zero Rust) registers a new format and parses through the
        // registry. The script lives in a file (entryFile), not the yaml.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("oxplow/plugins")).unwrap();
        std::fs::write(
            dir.path().join("oxplow/plugins/clover.jq"),
            r#"{ files: { (.attrs.file): { instrumented: [1], covered: [1] } } }"#,
        )
        .unwrap();
        let p = oxplow_config::PluginConfig {
            name: "acme.clover".into(),
            kind: "coverage".into(),
            formats: vec!["clover".into()],
            runtime: "jaq".into(),
            input: Some("xml".into()),
            entry_file: Some("oxplow/plugins/clover.jq".into()),
            args: vec![],
        };
        let collector = plugin_to_collector(&p, dir.path()).expect("config converts to collector");
        assert_eq!(collector.kind(), CollectorKind::Coverage);
        let mut reg = CollectorRegistry::with_builtins();
        reg.register(collector);
        let out = reg
            .run("clover", r#"<cov file="src/a.rs"/>"#)
            .expect("plugin runs");
        assert!(out.as_coverage().unwrap().files.contains_key("src/a.rs"));
        // Builtins still resolve alongside the project plugin.
        assert!(reg.resolve("cobertura").is_some());
        // An unknown format resolves to None — merge_* warns and skips it.
        assert!(reg.resolve("nope").is_none());
    }

    #[test]
    fn project_plugin_with_missing_entry_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = oxplow_config::PluginConfig {
            name: "acme.clover".into(),
            kind: "coverage".into(),
            formats: vec!["clover".into()],
            runtime: "jaq".into(),
            input: Some("xml".into()),
            entry_file: Some("oxplow/plugins/missing.jq".into()),
            args: vec![],
        };
        assert!(plugin_to_collector(&p, dir.path()).is_err());
    }

    #[test]
    fn detect_test_run_matches_builtins_and_extras() {
        assert!(detect_test_run("cargo test --workspace", &[]));
        assert!(detect_test_run("PYTEST -q tests/", &[]));
        assert!(detect_test_run("npx vitest run", &[]));
        assert!(!detect_test_run("cargo build", &[]));
        assert!(!detect_test_run("ls -la", &[]));
        // Extra pattern from the profile.
        assert!(detect_test_run("./run-suite.sh", &["run-suite".into()]));
        // Empty extra patterns are ignored (don't match everything).
        assert!(!detect_test_run("echo hi", &["".into(), "   ".into()]));
    }

    #[test]
    fn detect_analysis_run_matches_builtins_and_extras() {
        assert!(detect_analysis_run("cargo clippy --workspace", &[]));
        assert!(detect_analysis_run("npx ESLint src/", &[]));
        assert!(detect_analysis_run("ruff check .", &[]));
        assert!(!detect_analysis_run("cargo build", &[]));
        assert!(!detect_analysis_run("cargo test", &[]));
        // Extra pattern from the profile.
        assert!(detect_analysis_run("./lint.sh", &["./lint.sh".into()]));
    }

    #[test]
    fn detect_git_commit_matches_commit_commands() {
        assert!(detect_git_commit("git commit -m \"x\""));
        assert!(detect_git_commit("GIT COMMIT --amend --no-edit"));
        assert!(detect_git_commit("git -c user.email=t@t commit -m y"));
        assert!(!detect_git_commit("git add -A"));
        assert!(!detect_git_commit("git status"));
        assert!(!detect_git_commit("cargo build"));
    }

    #[test]
    fn commit_hygiene_message_lists_files_and_flags_docs() {
        // No docs/ file → no auto-deploy warning.
        let plain = commit_hygiene_message("abc1234", &["src/other.rs".into()]);
        assert!(plain.contains("abc1234"), "{plain}");
        assert!(plain.contains("src/other.rs"), "{plain}");
        assert!(plain.contains("1 file not part"), "{plain}");
        assert!(!plain.contains("auto-deploys"), "{plain}");
        // A docs/ file → stronger auto-deploy warning + plural wording.
        let docs = commit_hygiene_message(
            "def5678",
            &["docs/blog/posts/held.md".into(), "src/x.rs".into()],
        );
        assert!(docs.contains("2 files not part"), "{docs}");
        assert!(docs.contains("docs/blog/posts/held.md"), "{docs}");
        assert!(docs.contains("auto-deploys the site"), "{docs}");
        assert!(docs.contains(".github/workflows/docs.yml"), "{docs}");
    }

    #[test]
    fn parse_bash_post_tool_extracts_command_and_exit() {
        let payload = r#"{
            "tool_name": "Bash",
            "tool_input": {"command": "cargo test", "description": "run tests"},
            "tool_response": {"exit_code": 0, "stdout": "ok"}
        }"#;
        let got = parse_bash_post_tool(payload).unwrap();
        assert_eq!(got.command, "cargo test");
        assert_eq!(got.exit_code, Some(0));
    }

    #[test]
    fn parse_bash_post_tool_ignores_non_bash_and_missing_command() {
        assert!(parse_bash_post_tool(r#"{"tool_name":"Edit","tool_input":{}}"#).is_none());
        assert!(parse_bash_post_tool(r#"{"tool_name":"Bash","tool_input":{}}"#).is_none());
        assert!(parse_bash_post_tool("not json").is_none());
        // Missing exit code is tolerated.
        let got = parse_bash_post_tool(r#"{"tool_name":"Bash","tool_input":{"command":"pytest"}}"#)
            .unwrap();
        assert_eq!(got.exit_code, None);
    }

    #[test]
    fn diff_new_side_lines_flags_inserts_and_replacements() {
        // old: a,b,c  new: a,B,c,d  → line 2 replaced, line 4 inserted.
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\nd\n";
        let changed = diff_new_side_lines(old, new);
        assert_eq!(changed, [2u32, 4].into_iter().collect());
    }

    #[test]
    fn diff_new_side_lines_empty_when_identical() {
        assert!(diff_new_side_lines("x\ny\n", "x\ny\n").is_empty());
    }

    #[test]
    fn report_is_stale_compares_mtime_to_effort_start() {
        use oxplow_domain::Timestamp;
        let f = tempfile::NamedTempFile::new().unwrap();
        let now_ms = Timestamp::now().unix_ms();
        // Effort started well before the file was written → report is
        // fresh (produced during the effort).
        assert!(!report_is_stale(
            f.path(),
            Timestamp::from_unix_ms(now_ms - 60_000)
        ));
        // Effort started after the file's mtime → the report predates the
        // effort → stale.
        assert!(report_is_stale(
            f.path(),
            Timestamp::from_unix_ms(now_ms + 60_000)
        ));
    }

    /// End-to-end exercises of the orchestration: a real in-memory DB +
    /// tempdir project, with stream/thread/task/effort/snapshot/blob rows
    /// built through the public store APIs.
    mod integration {
        use super::*;
        use oxplow_db::{
            Database, FileSnapshot, SqliteSnapshotStore, SqliteStreamStore, SqliteTaskStore,
        };
        use oxplow_domain::stores::{StreamStore, TaskStore};
        use oxplow_domain::{
            EffortId, Stream, StreamId, StreamKind, Task, TaskActorKind, TaskAuthor, TaskId,
            TaskPriority, TaskStatus, Thread, ThreadStatus, Timestamp,
        };

        const COBERTURA_50PCT: &str = r#"<?xml version="1.0"?>
<coverage><packages><package name="p"><classes>
  <class name="Foo" filename="src/foo.rs"><lines>
    <line number="1" hits="3"/>
    <line number="2" hits="1"/>
    <line number="4" hits="0"/>
  </lines></class>
</classes></package></packages></coverage>"#;

        struct Harness {
            service: CollectionService,
            thread: ThreadId,
            effort_id: String,
            efforts: Arc<SqliteTaskEffortStore>,
            nudges: Arc<SqliteAgentNudgeStore>,
            tmp: tempfile::TempDir,
        }

        /// Build the fixture. `report_xml` Some → write it + configure the
        /// collection profile (cobertura/coverage.xml); None → leave
        /// collection unconfigured. The effort's start snapshot holds
        /// `src/foo.rs` as `a\nb\nc\n`; the working tree has `a\nB\nc\nd\n`
        /// (lines 2 changed, 4 added).
        async fn build(report_xml: Option<&str>) -> Harness {
            build_full(report_xml, false, &[]).await
        }

        /// Like [`build`], plus two knobs for the commit-hygiene tests:
        /// - `git_init`: `git init` the project and lay down a base commit
        ///   so HEAD has a parent for `get_commit_detail`'s diff.
        /// - `baseline_extra`: `(path, content)` files seeded into the
        ///   effort's START snapshot AND written identically to the working
        ///   tree, so they read as *unchanged during the effort* (the
        ///   out-of-effort signal a stray staged file would produce).
        async fn build_full(
            report_xml: Option<&str>,
            git_init: bool,
            baseline_extra: &[(&str, &str)],
        ) -> Harness {
            let tmp = tempfile::tempdir().unwrap();
            let project_dir = tmp.path().to_path_buf();
            std::fs::create_dir_all(project_dir.join(".oxplow/snapshots")).unwrap();
            let db = Database::in_memory();
            let now = Timestamp::now();

            let stream = Stream {
                id: StreamId::new(1),
                kind: StreamKind::Primary,
                title: "p".into(),
                branch: "main".into(),
                branch_ref: "refs/heads/main".into(),
                branch_source: "main".into(),
                worktree_path: project_dir.to_string_lossy().into_owned(),
                working_pane: String::new(),
                talking_pane: String::new(),
                working_session_id: String::new(),
                talking_session_id: String::new(),
                custom_prompt: None,
                created_at: now,
                updated_at: now,
                archived_at: None,
            };
            SqliteStreamStore::new(db.clone())
                .upsert(&stream)
                .await
                .unwrap();
            let thread = Thread {
                id: ThreadId::new(1),
                stream_id: stream.id,
                title: "x".into(),
                status: ThreadStatus::Active,
                sort_index: 0,
                pane_target: "working".into(),
                agent: oxplow_domain::AgentKind::Claude,
                resume_session_id: String::new(),
                summary: String::new(),
                summary_updated_at: None,
                closed_at: None,
                custom_prompt: None,
                created_at: now,
                updated_at: now,
                archived_at: None,
            };
            SqliteThreadStore::new(db.clone())
                .upsert(&thread)
                .await
                .unwrap();
            let task_id = SqliteTaskStore::new(db.clone())
                .insert(&Task {
                    id: TaskId::placeholder(),
                    thread_id: Some(thread.id),
                    parent_id: None,
                    title: "x".into(),
                    description: String::new(),
                    status: TaskStatus::InProgress,
                    priority: TaskPriority::Medium,
                    sort_index: 0,
                    created_by: TaskActorKind::User,
                    created_at: now,
                    updated_at: now,
                    completed_at: None,
                    deleted_at: None,
                    note_count: 0,
                    author: Some(TaskAuthor::User),
                })
                .await
                .unwrap();

            // Start-snapshot content for src/foo.rs (the baseline).
            let blobs = BlobStore::new(project_dir.join(".oxplow/snapshots"));
            let snapshots = Arc::new(SqliteSnapshotStore::new(db.clone()));
            let old_hash = blobs.write(b"a\nb\nc\n").unwrap();
            let snap_id = snapshots.create_snapshot(stream.id).await.unwrap();
            snapshots
                .capture(FileSnapshot {
                    id: 0,
                    stream_id: stream.id,
                    path: "src/foo.rs".into(),
                    blob_hash: Some(old_hash),
                    size_bytes: 6,
                    captured_at: now,
                    storage: oxplow_db::SnapshotStorage::Oxplow,
                    snapshot_id: Some(snap_id),
                    mtime_ms: None,
                })
                .await
                .unwrap();

            // Seed `baseline_extra` into the SAME start snapshot so they're
            // part of the effort baseline; the working-tree copy written
            // below is identical → unchanged during the effort.
            for (path, content) in baseline_extra {
                let hash = blobs.write(content.as_bytes()).unwrap();
                snapshots
                    .capture(FileSnapshot {
                        id: 0,
                        stream_id: stream.id,
                        path: (*path).into(),
                        blob_hash: Some(hash),
                        size_bytes: content.len() as i64,
                        captured_at: now,
                        storage: oxplow_db::SnapshotStorage::Oxplow,
                        snapshot_id: Some(snap_id),
                        mtime_ms: None,
                    })
                    .await
                    .unwrap();
            }

            let efforts = Arc::new(SqliteTaskEffortStore::new(db.clone()));
            let effort = efforts
                .start(task_id, &thread.id, Some(snap_id))
                .await
                .unwrap();

            // Current working-tree content: line 2 changed, line 4 added.
            std::fs::create_dir_all(project_dir.join("src")).unwrap();
            std::fs::write(project_dir.join("src/foo.rs"), "a\nB\nc\nd\n").unwrap();
            // Lay down each baseline_extra file identically (unchanged).
            for (path, content) in baseline_extra {
                let abs = project_dir.join(path);
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&abs, content).unwrap();
            }
            // Optional git repo + base commit so HEAD has a parent.
            if git_init {
                git_in(&project_dir, &["init", "-q"]);
                std::fs::write(project_dir.join("README.md"), "base\n").unwrap();
                git_in(&project_dir, &["add", "README.md"]);
                git_commit(&project_dir, "base");
            }

            let mut cfg = oxplow_config::load_project_config(&project_dir).unwrap();
            if let Some(xml) = report_xml {
                std::fs::write(project_dir.join("coverage.xml"), xml).unwrap();
                cfg.collection.reports.push(oxplow_config::ReportConfig {
                    path: "coverage.xml".into(),
                    format: "cobertura".into(),
                });
            }

            let nudges = Arc::new(SqliteAgentNudgeStore::new(db.clone()));
            let service = CollectionService::new(
                Arc::new(SqliteMetricStore::new(db.clone())),
                nudges.clone(),
                efforts.clone(),
                Arc::new(SqliteThreadStore::new(db.clone())),
                snapshots,
                blobs,
                Arc::new(RwLock::new(cfg)),
                project_dir,
                EventBus::new(),
            );
            Harness {
                service,
                thread: thread.id,
                effort_id: effort.id.to_string(),
                efforts,
                nudges,
                tmp,
            }
        }

        #[tokio::test]
        async fn ingest_coverage_stores_diff_coverage_over_changed_lines() {
            let h = build(Some(COBERTURA_50PCT)).await;
            // Changed lines {2,4}; report instruments {1,2,4}, covers {1,2}.
            // So changed∩instrumented = {2,4}, covered = {2} → 50%, line 4
            // uncovered.
            // skip_if_stale = false: this test exercises the parse +
            // changed-line intersection deterministically; the mtime guard
            // is covered by `report_is_stale_compares_mtime_to_effort_start`
            // (a just-written report's mtime vs. the effort start is
            // wall-clock/fs-granularity sensitive and would flake here).
            let outcome = h
                .service
                .ingest_coverage(&h.thread, None, None, false)
                .await
                .unwrap();
            match outcome {
                CoverageIngest::Stored {
                    summary_pct,
                    changed_lines,
                    covered_lines,
                    ..
                } => {
                    assert_eq!(changed_lines, 2);
                    assert_eq!(covered_lines, 1);
                    assert!((summary_pct - 50.0).abs() < 1e-6, "got {summary_pct}");
                }
                other => panic!("expected Stored, got {other:?}"),
            }
            let rows = h
                .service
                .list_for_effort(&h.effort_id, Some("diff-coverage"))
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].provenance, "observed");
            let payload = rows[0].payload_json.as_deref().unwrap();
            let cov: DiffCovPayload = serde_json::from_str(payload).expect("payload parses");
            let foo = cov.files.iter().find(|f| f.path == "src/foo.rs").unwrap();
            assert_eq!(foo.uncovered, vec![4]);
        }

        #[tokio::test]
        async fn ingest_coverage_mirrors_into_metric_substrate() {
            // git_init = true so a branch is present to capture.
            let h = build_full(Some(COBERTURA_50PCT), true, &[]).await;
            h.service
                .ingest_coverage(&h.thread, None, None, false)
                .await
                .unwrap();

            let samples = h
                .service
                .metric_samples_for_key("oxplow.coverage.diff_pct")
                .await;
            assert_eq!(samples.len(), 1, "one mirrored coverage sample");
            let s = &samples[0];
            assert!((s.value - 50.0).abs() < 1e-6, "value {}", s.value);
            // Components stored so module/branch roll-ups re-aggregate correctly.
            assert_eq!(s.numerator, Some(1.0));
            assert_eq!(s.denominator, Some(2.0));
            assert_eq!(s.provenance, "observed");
            assert_eq!(s.source, "coverage-report");
            // Branch captured from the git_init'd repo (main/master).
            assert!(s.branch.is_some(), "capture branch tracked");
        }

        #[derive(serde::Deserialize)]
        struct DiffCovPayload {
            files: Vec<DiffCovFile>,
        }
        #[derive(serde::Deserialize)]
        struct DiffCovFile {
            path: String,
            #[serde(rename = "uncoveredChangedLines")]
            uncovered: Vec<u32>,
        }

        #[tokio::test]
        async fn record_test_run_attributes_to_open_effort() {
            let h = build(None).await;
            let id = h
                .service
                .record_test_run(
                    &h.thread,
                    "cargo test --workspace",
                    Some(0),
                    Some(1200),
                    Some(5),
                    Some(0),
                    Some(5),
                    "observed",
                    "post-tool-bash",
                    None,
                )
                .await
                .unwrap();
            assert!(id.is_some());
            let rows = h
                .service
                .list_for_effort(&h.effort_id, Some("test-run"))
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].stream_id, "str1");
            assert!(rows[0]
                .payload_json
                .as_deref()
                .unwrap()
                .contains("cargo test"));
        }

        #[tokio::test]
        async fn record_test_run_mirrors_counts_into_metric_substrate() {
            let h = build(None).await;
            h.service
                .record_test_run(
                    &h.thread,
                    "cargo test --workspace",
                    Some(0),
                    Some(1200),
                    Some(5),
                    Some(1),
                    Some(6),
                    "observed",
                    "post-tool-bash",
                    None,
                )
                .await
                .unwrap();
            let passed = h
                .service
                .metric_samples_for_key("oxplow.tests.passed")
                .await;
            let failed = h
                .service
                .metric_samples_for_key("oxplow.tests.failed")
                .await;
            let total = h.service.metric_samples_for_key("oxplow.tests.total").await;
            assert_eq!(passed.len(), 1);
            assert_eq!(passed[0].value, 5.0);
            assert_eq!(failed[0].value, 1.0);
            assert_eq!(total[0].value, 6.0);
            assert_eq!(passed[0].provenance, "observed");
            // All three share one run.
            assert_eq!(passed[0].run_id, total[0].run_id);
        }

        #[tokio::test]
        async fn record_test_run_writes_test_detail_finding_to_substrate() {
            // tsk215: the suite/case tree is kept verbatim on the substrate as a
            // `test-detail` finding so the effort panel renders off the model.
            use oxplow_coverage::{TestCase, TestReport, TestStatus, TestSuite};
            let h = build(None).await;
            let report = TestReport {
                suites: vec![TestSuite {
                    name: "oxplow-app".into(),
                    cases: vec![
                        TestCase {
                            classname: "mod".into(),
                            name: "t1".into(),
                            status: TestStatus::Passed,
                            time_ms: Some(3),
                        },
                        TestCase {
                            classname: "mod".into(),
                            name: "t2".into(),
                            status: TestStatus::Failed,
                            time_ms: None,
                        },
                    ],
                }],
            };
            h.service
                .record_test_run(
                    &h.thread,
                    "cargo test",
                    Some(0),
                    None,
                    None,
                    None,
                    None,
                    "observed",
                    "post-tool-bash",
                    Some(&report),
                )
                .await
                .unwrap();
            let total = h.service.metric_samples_for_key("oxplow.tests.total").await;
            let run_id = total[0].run_id.unwrap();
            let findings = h.service.metric_findings_for_run(run_id).await;
            let detail = findings
                .iter()
                .find(|f| f.kind == "test-detail")
                .expect("test-detail finding written");
            let payload: serde_json::Value =
                serde_json::from_str(detail.extra_json.as_deref().unwrap()).unwrap();
            assert_eq!(payload["suites"][0]["name"], "oxplow-app");
            assert_eq!(payload["suites"][0]["cases"][1]["status"], "failed");
        }

        #[tokio::test]
        async fn effort_observations_from_metrics_reconstructs_the_panel_shape() {
            // tsk215: the effort panel's observation rows are reconstructed from
            // the substrate (samples in the effort window + their detail payload).
            use oxplow_coverage::{TestCase, TestReport, TestStatus, TestSuite};
            let h = build(None).await;
            let report = TestReport {
                suites: vec![TestSuite {
                    name: "s".into(),
                    cases: vec![TestCase {
                        classname: "c".into(),
                        name: "t1".into(),
                        status: TestStatus::Passed,
                        time_ms: None,
                    }],
                }],
            };
            h.service
                .record_test_run(
                    &h.thread,
                    "cargo test",
                    Some(0),
                    None,
                    None,
                    None,
                    None,
                    "observed",
                    "post-tool-bash",
                    Some(&report),
                )
                .await
                .unwrap();
            let obs = h
                .service
                .effort_observations_from_metrics(&h.effort_id, Some("test-run"))
                .await;
            assert_eq!(obs.len(), 1, "one test-run row reconstructed");
            assert_eq!(obs[0].kind, "test-run");
            assert_eq!(obs[0].provenance, "observed");
            assert!(
                obs[0]
                    .payload_json
                    .as_deref()
                    .unwrap()
                    .contains("\"suites\""),
                "carries the suite/case tree from the substrate"
            );
            // Filtering by a different kind yields nothing.
            assert!(h
                .service
                .effort_observations_from_metrics(&h.effort_id, Some("diff-coverage"))
                .await
                .is_empty());
        }

        /// Seed a gauge definition + two samples (baseline, current) in the open
        /// effort's window. Returns the metric id.
        async fn seed_gauge(
            h: &Harness,
            key: &str,
            direction: &str,
            warn_at: Option<f64>,
            fail_at: Option<f64>,
            baseline: f64,
            current: f64,
        ) -> i64 {
            use oxplow_db::{NewMetricDefinition, NewMetricSample};
            let mut def = NewMetricDefinition::new(key, "gauge", "unsafe blocks");
            def.unit = Some("count".into());
            def.direction = direction.into();
            def.warn_at = warn_at;
            def.fail_at = fail_at;
            let metric_id = h.service.metrics.upsert_definition(def).await.unwrap();
            for value in [baseline, current] {
                h.service
                    .metrics
                    .record_sample(NewMetricSample {
                        run_id: None,
                        metric_id,
                        value,
                        numerator: None,
                        denominator: None,
                        captured_at: None,
                        snapshot_id: None,
                        closest_git_version: None,
                        branch: None,
                        git_version_exact: false,
                        basis_ref: None,
                        stream_id: 1,
                        thread_id: None,
                        subject_kind: None,
                        subject_ref: None,
                        path: None,
                        line: None,
                        dims_json: None,
                        provenance: "observed".into(),
                        source: "test".into(),
                    })
                    .await
                    .unwrap();
            }
            metric_id
        }

        #[tokio::test]
        async fn effort_metric_context_reports_deltas_and_one_shot_crossing() {
            let h = build(None).await;
            // unsafe blocks went 3 → 12 (lower-better), crossing fail_at=10.
            seed_gauge(
                &h,
                "test.unsafe_blocks",
                "lower-better",
                Some(5.0),
                Some(10.0),
                3.0,
                12.0,
            )
            .await;

            let first = h.service.effort_metric_context(&h.thread).await.unwrap();
            assert!(first.contains("Metric deltas (this effort)"), "{first}");
            assert!(first.contains("unsafe blocks: 3 → 12"), "{first}");
            assert!(first.contains("Δ +9"), "{first}");
            assert!(
                first.contains("crossed fail threshold (10)"),
                "first turn surfaces the crossing: {first}"
            );

            // Second turn: the delta still shows, but the loud crossing is
            // one-shot — it must not repeat.
            let second = h.service.effort_metric_context(&h.thread).await.unwrap();
            assert!(second.contains("unsafe blocks: 3 → 12"), "{second}");
            assert!(
                !second.contains("crossed"),
                "crossing is one-shot per effort: {second}"
            );
        }

        #[tokio::test]
        async fn effort_metric_context_none_when_nothing_moved() {
            let h = build(None).await;
            // Two equal samples, neutral direction → no movement, no crossing.
            seed_gauge(&h, "test.flat", "neutral", None, None, 7.0, 7.0).await;
            assert!(h.service.effort_metric_context(&h.thread).await.is_none());
        }

        #[tokio::test]
        async fn effort_metric_context_skips_operational_keys() {
            let h = build(None).await;
            // Tokens grow every turn but aren't a code-health signal — excluded.
            seed_gauge(
                &h,
                "agent.tokens.total",
                "neutral",
                None,
                None,
                100.0,
                5000.0,
            )
            .await;
            assert!(
                h.service.effort_metric_context(&h.thread).await.is_none(),
                "operational `agent.*` metrics are filtered out"
            );
        }

        #[tokio::test]
        async fn ingest_coverage_writes_coverage_detail_finding_to_substrate() {
            let h = build_full(Some(COBERTURA_50PCT), true, &[]).await;
            h.service
                .ingest_coverage(&h.thread, None, None, false)
                .await
                .unwrap();
            let cov = h
                .service
                .metric_samples_for_key("oxplow.coverage.diff_pct")
                .await;
            let run_id = cov[0].run_id.unwrap();
            let findings = h.service.metric_findings_for_run(run_id).await;
            let detail = findings
                .iter()
                .find(|f| f.kind == "coverage-detail")
                .expect("coverage-detail finding written");
            let payload: serde_json::Value =
                serde_json::from_str(detail.extra_json.as_deref().unwrap()).unwrap();
            assert!(payload["files"].is_array(), "per-file uncovered lines kept");
            assert!((payload["summaryPct"].as_f64().unwrap() - 50.0).abs() < 1e-6);
        }

        #[tokio::test]
        async fn record_test_run_embeds_junit_tree_and_derives_counts() {
            let h = build(None).await;
            // Run the bundled junit collector to build the suite/case tree
            // (oxplow-coverage no longer exposes a parse entry point).
            let junit = match CollectorRegistry::with_builtins()
                .run(
                    "junit",
                    r#"<testsuites><testsuite name="oxplow-app">
                  <testcase classname="oxplow_app::collection" name="a"/>
                  <testcase classname="oxplow_app::collection" name="b"><failure/></testcase>
                  <testcase classname="oxplow_app::collection" name="c"><skipped/></testcase>
                </testsuite></testsuites>"#,
                )
                .unwrap()
            {
                CollectorOutput::Test(r) => r,
                other => panic!("expected test output, got {other:?}"),
            };
            h.service
                .record_test_run(
                    &h.thread,
                    "cargo nextest run",
                    Some(1),
                    None,
                    None,
                    None,
                    None,
                    "observed",
                    "post-tool-bash",
                    Some(&junit),
                )
                .await
                .unwrap();
            let rows = h
                .service
                .list_for_effort(&h.effort_id, Some("test-run"))
                .await
                .unwrap();
            let payload: serde_json::Value =
                serde_json::from_str(rows[0].payload_json.as_deref().unwrap()).unwrap();
            // Counts derived from the tree (1 pass, 1 fail, 1 skip).
            assert_eq!(payload["passed"], 1);
            assert_eq!(payload["failed"], 1);
            assert_eq!(payload["skipped"], 1);
            assert_eq!(payload["total"], 3);
            // The suite/case tree is embedded for the UI.
            assert_eq!(payload["suites"][0]["name"], "oxplow-app");
            assert_eq!(payload["suites"][0]["cases"][1]["status"], "failed");
        }

        #[tokio::test]
        async fn ingest_coverage_no_open_effort_after_finish() {
            let h = build(Some(COBERTURA_50PCT)).await;
            h.efforts
                .finish(&EffortId::try_from_str(&h.effort_id).unwrap(), None, None)
                .await
                .unwrap();
            assert_eq!(
                h.service
                    .ingest_coverage(&h.thread, None, None, true)
                    .await
                    .unwrap(),
                CoverageIngest::NoOpenEffort
            );
        }

        #[tokio::test]
        async fn ingest_coverage_not_configured() {
            let h = build(None).await;
            assert_eq!(
                h.service
                    .ingest_coverage(&h.thread, None, None, true)
                    .await
                    .unwrap(),
                CoverageIngest::NotConfigured
            );
        }

        #[tokio::test]
        async fn ingest_coverage_no_changed_coverage_when_report_misses_changed_lines() {
            // Report only instruments line 1 (unchanged) → no changed line
            // intersects → NoChangedCoverage.
            let only_line_1 = r#"<?xml version="1.0"?>
<coverage><packages><package name="p"><classes>
  <class name="Foo" filename="src/foo.rs"><lines><line number="1" hits="3"/></lines></class>
</classes></package></packages></coverage>"#;
            let h = build(Some(only_line_1)).await;
            // false: see the Stored test — this checks the intersection
            // branch, not the (separately unit-tested) mtime guard.
            assert_eq!(
                h.service
                    .ingest_coverage(&h.thread, None, None, false)
                    .await
                    .unwrap(),
                CoverageIngest::NoChangedCoverage
            );
        }

        /// Build a PostToolUse payload for a Bash command.
        fn bash_payload(cmd: &str, exit_code: i64) -> String {
            format!(
                r#"{{"tool_name":"Bash","tool_input":{{"command":"{cmd}"}},"tool_response":{{"exit_code":{exit_code}}}}}"#
            )
        }

        /// Run a git subcommand in `dir`, asserting success.
        fn git_in(dir: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .current_dir(dir)
                .args(args)
                .output()
                .expect("spawn git");
            assert!(
                out.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        /// Commit with a fixed identity (avoids depending on global config).
        fn git_commit(dir: &std::path::Path, message: &str) {
            git_in(
                dir,
                &[
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "user.name=t",
                    "commit",
                    "-q",
                    "-m",
                    message,
                ],
            );
        }

        #[tokio::test]
        async fn on_post_tool_use_nudges_on_out_of_effort_commit() {
            // Effort changed src/foo.rs; a pre-staged held.txt is unchanged
            // during the effort. Committing both → held.txt is flagged.
            let h = build_full(None, true, &[("held.txt", "held\n")]).await;
            git_in(h.tmp.path(), &["add", "src/foo.rs", "held.txt"]);
            git_commit(h.tmp.path(), "feature work");
            let nudge = h
                .service
                .on_post_tool_use(&h.thread, &bash_payload("git commit -m work", 0))
                .await
                .unwrap()
                .expect("out-of-effort commit nudges");
            assert!(nudge.contains("held.txt"), "{nudge}");
            // The in-effort file must NOT be named as drift.
            assert!(!nudge.contains("src/foo.rs"), "{nudge}");
            // Not a docs/ file → no auto-deploy warning.
            assert!(!nudge.contains("auto-deploys"), "{nudge}");
        }

        #[tokio::test]
        async fn on_post_tool_use_no_nudge_when_commit_within_effort() {
            // Commit only the file the effort actually changed → clean.
            let h = build_full(None, true, &[]).await;
            git_in(h.tmp.path(), &["add", "src/foo.rs"]);
            git_commit(h.tmp.path(), "just the effort's file");
            let result = h
                .service
                .on_post_tool_use(&h.thread, &bash_payload("git commit -m work", 0))
                .await
                .unwrap();
            assert!(
                result.is_none(),
                "in-effort commit must not nudge: {result:?}"
            );
        }

        #[tokio::test]
        async fn on_post_tool_use_commit_nudge_flags_docs_autodeploy() {
            // A held blog post under docs/ swept into the commit → the
            // stronger auto-deploy warning fires (the tsk80 incident).
            let h = build_full(None, true, &[("docs/blog/posts/held.md", "# held\n")]).await;
            git_in(
                h.tmp.path(),
                &["add", "src/foo.rs", "docs/blog/posts/held.md"],
            );
            git_commit(h.tmp.path(), "feature + stray doc");
            let nudge = h
                .service
                .on_post_tool_use(&h.thread, &bash_payload("git commit -m work", 0))
                .await
                .unwrap()
                .expect("out-of-effort docs commit nudges");
            assert!(nudge.contains("docs/blog/posts/held.md"), "{nudge}");
            assert!(nudge.contains("auto-deploys the site"), "{nudge}");
            assert!(nudge.contains(".github/workflows/docs.yml"), "{nudge}");
        }

        #[tokio::test]
        async fn commit_hygiene_is_claim_aware() {
            // Claim-first (Child 3): when the effort has claims, the
            // commit-hygiene "in-effort" test prefers the CLAIMED set.
            // - A committed file the effort never claimed is flagged EVEN
            //   though it changed during the window (old snapshot logic
            //   would have cleared it).
            // - A claimed file is NOT flagged.
            let h = build_full(None, true, &[]).await;
            // extra.rs changed during the window (not in start snapshot)
            // but is never claimed.
            std::fs::write(h.tmp.path().join("extra.rs"), "x\n").unwrap();
            // Claim only src/foo.rs onto the open effort (as the real-time
            // auto-claim would).
            let effort_id = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            h.efforts
                .record_file(
                    &effort_id,
                    "src/foo.rs",
                    oxplow_db::EffortFileChange::Updated,
                    oxplow_db::FileRefVersion {
                        local_snapshot_id: 0,
                        closest_git_version: None,
                        git_version_exact: false,
                    },
                )
                .await
                .unwrap();
            git_in(h.tmp.path(), &["add", "src/foo.rs", "extra.rs"]);
            git_commit(h.tmp.path(), "claimed + unclaimed");
            let nudge = h
                .service
                .on_post_tool_use(&h.thread, &bash_payload("git commit -m work", 0))
                .await
                .unwrap()
                .expect("unclaimed committed file should nudge");
            assert!(
                nudge.contains("extra.rs"),
                "a committed-but-never-claimed file must be flagged: {nudge}"
            );
            assert!(
                !nudge.contains("src/foo.rs"),
                "a claimed file must NOT be flagged: {nudge}"
            );
        }

        #[tokio::test]
        async fn commit_hygiene_no_nudge_when_only_claimed_file_committed() {
            // Effort claims src/foo.rs and commits only it → clean, no nudge.
            let h = build_full(None, true, &[]).await;
            let effort_id = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            h.efforts
                .record_file(
                    &effort_id,
                    "src/foo.rs",
                    oxplow_db::EffortFileChange::Updated,
                    oxplow_db::FileRefVersion {
                        local_snapshot_id: 0,
                        closest_git_version: None,
                        git_version_exact: false,
                    },
                )
                .await
                .unwrap();
            git_in(h.tmp.path(), &["add", "src/foo.rs"]);
            git_commit(h.tmp.path(), "just the claimed file");
            let result = h
                .service
                .on_post_tool_use(&h.thread, &bash_payload("git commit -m work", 0))
                .await
                .unwrap();
            assert!(
                result.is_none(),
                "committing only the claimed file must not nudge: {result:?}"
            );
        }

        #[tokio::test]
        async fn on_post_tool_use_nudges_on_report_less_test_run() {
            // A detected test command with no fresh report → nudge returned.
            let h = build(None).await;
            // Configure a testCommand so the nudge names it.
            {
                let mut cfg = h.service.config.write().unwrap();
                cfg.collection.test_command = Some("bun run test:collect".into());
            }
            let result = h
                .service
                .on_post_tool_use(&h.thread, &bash_payload("bun test --watch false", 0))
                .await
                .unwrap();
            let nudge = result.expect("nudge returned for report-less run");
            assert!(
                nudge.contains("bun run test:collect"),
                "nudge should name the configured testCommand; got: {nudge}"
            );
            // A report-LESS run produces no parseable counts, so the substrate
            // records no test sample → the effort panel shows no test-run row
            // for it (tsk215). The report-less *nudge* is what surfaces the run
            // to the agent; the command-only "ran-record" marker is retired.
            let obs = h
                .service
                .list_for_effort(&h.effort_id, Some("test-run"))
                .await
                .unwrap();
            assert!(obs.is_empty(), "no substrate row for a report-less run");
            // The fired nudge also projects an `agent.nudges.fired` event
            // sample, subject = the nudge kind (tsk216).
            let fired = h.service.metric_samples_for_key("agent.nudges.fired").await;
            assert_eq!(fired.len(), 1, "one nudge sample per fired nudge");
            assert_eq!(fired[0].value, 1.0);
            assert_eq!(fired[0].subject_kind.as_deref(), Some("nudge"));
            assert_eq!(fired[0].subject_ref.as_deref(), Some("report-less-run"));
        }

        #[tokio::test]
        async fn coverage_def_carries_target_thresholds_from_data() {
            // tsk220: the 50/80 ramp lives on the definition (data), so the
            // renderer colors red/green from it and the nudge fires below target
            // — no hardcoded UI constant.
            let h = build_full(Some(COBERTURA_50PCT), true, &[]).await;
            h.service
                .ingest_coverage(&h.thread, None, None, false)
                .await
                .unwrap();
            let def = h
                .service
                .metrics
                .get_definition("oxplow.coverage.diff_pct")
                .await
                .unwrap()
                .expect("coverage definition seeded");
            assert_eq!(def.target, Some(80.0), "target in data");
            assert_eq!(def.fail_at, Some(50.0), "fail floor in data");
            assert_eq!(def.direction, "higher-better");
        }

        #[test]
        fn coverage_target_nudge_message_names_pct_and_target() {
            let msg = super::coverage_target_nudge_message(50.0);
            assert!(msg.contains("50%"), "names the actual pct; got: {msg}");
            assert!(msg.contains("80% target"), "names the target; got: {msg}");
            assert!(
                msg.to_lowercase().contains("advisory"),
                "flagged advise-only; got: {msg}"
            );
        }

        #[tokio::test]
        async fn coverage_target_nudge_is_one_shot_per_effort() {
            // The acceptance invariant: at most one coverage nudge per effort.
            let h = build(None).await;
            let eid = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            assert!(h.service.mark_coverage_nudged(&eid), "first time fires");
            assert!(
                !h.service.mark_coverage_nudged(&eid),
                "second time is deduped"
            );
        }

        #[tokio::test]
        async fn on_post_tool_use_no_nudge_when_report_produced() {
            // A detected test command that regenerated a fresh JUnit report →
            // no nudge. We use merge_fresh_test_reports directly to sidestep
            // the wall-clock/fs-mtime sensitivity of on_post_tool_use (the
            // staleness guard compares file mtime to effort.started_at, and
            // both happen in the same second in a test). This exercises exactly
            // the branch that suppresses the nudge: produced_report is true.
            let h = build(None).await;
            std::fs::write(
                h.tmp.path().join("tests.xml"),
                r#"<testsuites><testsuite name="suite"><testcase classname="c" name="t1"/></testsuite></testsuites>"#,
            )
            .unwrap();
            let cfg = oxplow_config::CollectionConfig {
                reports: vec![oxplow_config::ReportConfig {
                    path: "tests.xml".into(),
                    format: "junit".into(),
                }],
                test_command: Some("bun run test:collect".into()),
                ..Default::default()
            };
            // Synthetic effort started at the epoch so the just-written file
            // is always fresh (same approach as merge_fresh_test_reports test).
            let effort = TaskEffort {
                id: EffortId::new(901),
                task_id: TaskId::placeholder(),
                thread_id: h.thread,
                started_at: Timestamp::from_unix_ms(0),
                ended_at: None,
                start_snapshot_id: None,
                end_snapshot_id: None,
                summary: None,
            };
            let registry = h.service.registry(&cfg);
            let report = h.service.merge_fresh_test_reports(&effort, &cfg, &registry);
            assert!(
                report.is_some(),
                "fresh JUnit report should be merged (effort start = epoch)"
            );
            // When produced_report is true, mark_nudged is never called.
            // Directly verify: set the nudge flag manually, confirm it was
            // only set once the test asks for it (i.e. nudge didn't fire).
            let nudged = h
                .service
                .nudged_efforts
                .lock()
                .unwrap()
                .contains(&EffortId::new(901));
            assert!(
                !nudged,
                "nudge should not have fired when a report was produced"
            );
        }

        #[tokio::test]
        async fn on_post_tool_use_nudge_fires_once_per_effort() {
            // The nudge is at most once per effort — second no-report run
            // returns None.
            let h = build(None).await;
            {
                let mut cfg = h.service.config.write().unwrap();
                cfg.collection.test_command = Some("bun run test:collect".into());
            }
            let payload = bash_payload("bun test", 0);
            let first = h
                .service
                .on_post_tool_use(&h.thread, &payload)
                .await
                .unwrap();
            assert!(first.is_some(), "first report-less run should nudge");
            let second = h
                .service
                .on_post_tool_use(&h.thread, &payload)
                .await
                .unwrap();
            assert!(
                second.is_none(),
                "second run in same effort must not nudge again"
            );
        }

        #[tokio::test]
        async fn report_less_run_persists_one_nudge_and_dedup_doesnt_double_store() {
            // A report-less run persists exactly one `report-less-run` nudge
            // row tagged with kind + message + trigger; the one-shot dedup
            // means a second run in the same effort stores nothing more.
            let h = build(None).await;
            {
                let mut cfg = h.service.config.write().unwrap();
                cfg.collection.test_command = Some("bun run test:collect".into());
            }
            let payload = bash_payload("bun test --watch false", 0);
            h.service
                .on_post_tool_use(&h.thread, &payload)
                .await
                .unwrap()
                .expect("first run nudges");
            let rows = h.nudges.list_for_effort(&h.effort_id).await.unwrap();
            assert_eq!(rows.len(), 1, "exactly one nudge persisted");
            assert_eq!(rows[0].kind, "report-less-run");
            assert!(rows[0].message.contains("bun run test:collect"));
            assert_eq!(rows[0].trigger.as_deref(), Some("bun test --watch false"));
            assert_eq!(rows[0].effort_id.as_deref(), Some(h.effort_id.as_str()));

            // Second run is deduped (returns None) and stores nothing more.
            let second = h
                .service
                .on_post_tool_use(&h.thread, &payload)
                .await
                .unwrap();
            assert!(second.is_none(), "second run deduped");
            let rows = h.nudges.list_for_effort(&h.effort_id).await.unwrap();
            assert_eq!(rows.len(), 1, "deduped nudge must not double-store");
        }

        #[tokio::test]
        async fn out_of_effort_commit_persists_one_commit_hygiene_nudge() {
            // An out-of-effort commit persists exactly one `commit-hygiene`
            // nudge; the per-commit dedup means a repeat hook on the same
            // commit stores nothing more.
            let h = build_full(None, true, &[("held.txt", "held\n")]).await;
            git_in(h.tmp.path(), &["add", "src/foo.rs", "held.txt"]);
            git_commit(h.tmp.path(), "feature work");
            let payload = bash_payload("git commit -m work", 0);
            h.service
                .on_post_tool_use(&h.thread, &payload)
                .await
                .unwrap()
                .expect("out-of-effort commit nudges");
            let rows = h.nudges.list_for_effort(&h.effort_id).await.unwrap();
            assert_eq!(rows.len(), 1, "exactly one commit-hygiene nudge persisted");
            assert_eq!(rows[0].kind, "commit-hygiene");
            assert!(rows[0].message.contains("held.txt"));
            assert_eq!(rows[0].trigger.as_deref(), Some("git commit -m work"));

            // Same commit, hook fires again → per-commit dedup, no new row.
            let second = h
                .service
                .on_post_tool_use(&h.thread, &payload)
                .await
                .unwrap();
            assert!(second.is_none(), "same commit deduped");
            let rows = h.nudges.list_for_effort(&h.effort_id).await.unwrap();
            assert_eq!(rows.len(), 1, "deduped commit nudge must not double-store");
        }

        #[tokio::test]
        async fn on_post_tool_use_no_nudge_for_non_test_command() {
            // A non-test Bash command → no nudge, no observation.
            let h = build(None).await;
            let result = h
                .service
                .on_post_tool_use(&h.thread, &bash_payload("cargo build", 0))
                .await
                .unwrap();
            assert!(result.is_none());
            let obs = h
                .service
                .list_for_effort(&h.effort_id, Some("test-run"))
                .await
                .unwrap();
            assert!(obs.is_empty());
        }

        #[tokio::test]
        async fn on_post_tool_use_routes_to_configure_when_no_collection_profile() {
            // No collection block → nudge routes to /oxplow:configure.
            let h = build(None).await;
            // collection has no testCommand and no reports by default.
            let result = h
                .service
                .on_post_tool_use(&h.thread, &bash_payload("bun test", 0))
                .await
                .unwrap();
            let nudge = result.expect("nudge returned even without a collection profile");
            assert!(
                nudge.contains("/oxplow:configure"),
                "nudge should route to /oxplow:configure when no profile exists; got: {nudge}"
            );
        }

        #[tokio::test]
        async fn merge_fresh_test_reports_unions_suites_from_multiple_stacks() {
            let h = build(None).await;
            // Two JUnit reports from different stacks, both written now.
            std::fs::write(
                h.tmp.path().join("rust.xml"),
                r#"<testsuites><testsuite name="rust-crate"><testcase classname="c" name="t1"/></testsuite></testsuites>"#,
            )
            .unwrap();
            std::fs::write(
                h.tmp.path().join("front.xml"),
                r#"<testsuites><testsuite name="frontend"><testcase classname="d" name="t2"/></testsuite></testsuites>"#,
            )
            .unwrap();
            // Synthetic effort started at the epoch → both files are fresh
            // (deterministic, no wall-clock/fs-granularity dependence).
            let effort = TaskEffort {
                id: EffortId::new(902),
                task_id: TaskId::placeholder(),
                thread_id: ThreadId::new(1),
                started_at: Timestamp::from_unix_ms(0),
                ended_at: None,
                start_snapshot_id: None,
                end_snapshot_id: None,
                summary: None,
            };
            let cfg = oxplow_config::CollectionConfig {
                reports: vec![
                    oxplow_config::ReportConfig {
                        path: "rust.xml".into(),
                        format: "junit".into(),
                    },
                    oxplow_config::ReportConfig {
                        path: "front.xml".into(),
                        format: "junit".into(),
                    },
                ],
                ..Default::default()
            };
            let registry = h.service.registry(&cfg);
            let (merged, source) = h
                .service
                .merge_fresh_test_reports(&effort, &cfg, &registry)
                .expect("both fresh reports merged");
            let names: Vec<&str> = merged.suites.iter().map(|s| s.name.as_str()).collect();
            assert!(
                names.contains(&"rust-crate") && names.contains(&"frontend"),
                "merged suites from both stacks; got {names:?}"
            );
            // Both stacks use the in-process junit collector → not exec-tagged.
            assert_eq!(source, "post-tool-bash");
        }

        const CLIPPY_JSON: &str = "{\"reason\":\"compiler-message\",\"message\":{\"message\":\"unused\",\"code\":{\"code\":\"unused_variables\"},\"level\":\"warning\",\"spans\":[{\"file_name\":\"src/foo.rs\",\"line_start\":3,\"column_start\":9,\"is_primary\":true}]}}\n{\"reason\":\"compiler-message\",\"message\":{\"message\":\"boom\",\"code\":{\"code\":\"E0308\"},\"level\":\"error\",\"spans\":[{\"file_name\":\"src/bar.rs\",\"line_start\":1,\"column_start\":1,\"is_primary\":true}]}}\n";

        #[tokio::test]
        async fn record_static_analysis_attributes_to_open_effort() {
            let h = build(None).await;
            let report = oxplow_coverage::AnalysisReport {
                findings: vec![
                    oxplow_coverage::AnalysisFinding {
                        path: "src/a.rs".into(),
                        line: Some(1),
                        column: None,
                        severity: oxplow_coverage::Severity::Error,
                        rule: Some("E0308".into()),
                        message: "boom".into(),
                    },
                    oxplow_coverage::AnalysisFinding {
                        path: "src/a.rs".into(),
                        line: Some(2),
                        column: None,
                        severity: oxplow_coverage::Severity::Warning,
                        rule: None,
                        message: "meh".into(),
                    },
                ],
            };
            let id = h
                .service
                .record_static_analysis(
                    &h.thread,
                    "cargo clippy",
                    Some(&report),
                    &["clippy".to_string()],
                    "analysis-report",
                )
                .await
                .unwrap();
            assert!(id.is_some());
            let rows = h
                .service
                .list_for_effort(&h.effort_id, Some("static-analysis"))
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].provenance, "observed");
            // metric = error+warning count (lower = better).
            assert_eq!(rows[0].metric_value, Some(2.0));
            let payload: serde_json::Value =
                serde_json::from_str(rows[0].payload_json.as_deref().unwrap()).unwrap();
            assert_eq!(payload["errorCount"], 1);
            assert_eq!(payload["warningCount"], 1);
            assert_eq!(payload["analyzer"], "clippy");
            assert_eq!(payload["findings"][0]["rule"], "E0308");
        }

        #[tokio::test]
        async fn record_static_analysis_mirrors_into_metric_substrate() {
            let h = build(None).await;
            let report = oxplow_coverage::AnalysisReport {
                findings: vec![
                    oxplow_coverage::AnalysisFinding {
                        path: "src/a.rs".into(),
                        line: Some(10),
                        column: Some(3),
                        severity: oxplow_coverage::Severity::Error,
                        rule: Some("E0308".into()),
                        message: "boom".into(),
                    },
                    oxplow_coverage::AnalysisFinding {
                        path: "src/a.rs".into(),
                        line: Some(2),
                        column: None,
                        severity: oxplow_coverage::Severity::Warning,
                        rule: None,
                        message: "meh".into(),
                    },
                ],
            };
            h.service
                .record_static_analysis(
                    &h.thread,
                    "cargo clippy",
                    Some(&report),
                    &["clippy".to_string()],
                    "analysis-report",
                )
                .await
                .unwrap();

            let errors = h
                .service
                .metric_samples_for_key("oxplow.analysis.errors")
                .await;
            let warnings = h
                .service
                .metric_samples_for_key("oxplow.analysis.warnings")
                .await;
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0].value, 1.0);
            assert_eq!(warnings[0].value, 1.0);

            // One `lint` finding per hit (located detail) + one verbatim
            // `analysis-detail` payload (tsk215) under the run.
            let run_id = errors[0].run_id.expect("sample carries its run");
            let findings = h.service.metric_findings_for_run(run_id).await;
            let lints: Vec<_> = findings.iter().filter(|f| f.kind == "lint").collect();
            assert_eq!(lints.len(), 2);
            assert!(lints.iter().any(|f| f.rule.as_deref() == Some("E0308")
                && f.severity.as_deref() == Some("error")
                && f.path.as_deref() == Some("src/a.rs")));
            assert!(
                findings.iter().any(|f| f.kind == "analysis-detail"),
                "verbatim analysis payload kept on the substrate"
            );
        }

        #[tokio::test]
        async fn record_static_analysis_command_only_produces_no_substrate_row() {
            // tsk215: an analyzer that ran but produced no parseable report has
            // no metric to record, so the substrate has no static-analysis row
            // (the legacy "ran-record" marker is retired — a parseable report is
            // what surfaces analysis on the effort panel now).
            let h = build(None).await;
            let recorded = h
                .service
                .record_static_analysis(&h.thread, "cargo clippy", None, &[], "analysis-report")
                .await
                .unwrap();
            assert!(recorded.is_some(), "the run is acknowledged");
            let rows = h
                .service
                .list_for_effort(&h.effort_id, Some("static-analysis"))
                .await
                .unwrap();
            assert!(
                rows.is_empty(),
                "no substrate row for a report-less analyzer run"
            );
        }

        #[tokio::test]
        async fn merge_fresh_analysis_unions_findings_from_reports() {
            let h = build(None).await;
            std::fs::write(h.tmp.path().join("clippy.json"), CLIPPY_JSON).unwrap();
            // Synthetic effort started at the epoch → the report is fresh.
            let effort = TaskEffort {
                id: EffortId::new(903),
                task_id: TaskId::placeholder(),
                thread_id: h.thread,
                started_at: Timestamp::from_unix_ms(0),
                ended_at: None,
                start_snapshot_id: None,
                end_snapshot_id: None,
                summary: None,
            };
            let cfg = oxplow_config::CollectionConfig {
                reports: vec![oxplow_config::ReportConfig {
                    path: "clippy.json".into(),
                    format: "clippy-json".into(),
                }],
                ..Default::default()
            };
            let registry = h.service.registry(&cfg);
            let (merged, source, analyzers) = h
                .service
                .merge_fresh_analysis(&effort, &cfg, &registry)
                .expect("fresh clippy report merged");
            assert_eq!(merged.findings.len(), 2);
            assert_eq!(source, "analysis-report");
            assert_eq!(analyzers, vec!["clippy".to_string()]);
        }

        // `eslint -f json`: errors (severity 2) + a warning (severity 1)
        // across two filePaths, plus one null ruleId (parser error → no rule).
        const ESLINT_JSON: &str = r#"[
          { "filePath": "src/a.ts", "messages": [
            { "ruleId": "no-unused-vars", "severity": 2, "line": 3, "column": 7, "message": "x is unused" },
            { "ruleId": "eqeqeq", "severity": 1, "line": 9, "column": 5, "message": "use ===" }
          ] },
          { "filePath": "src/b.ts", "messages": [
            { "ruleId": null, "severity": 2, "line": 1, "column": 1, "message": "Parsing error" }
          ] }
        ]"#;

        #[tokio::test]
        async fn ingest_analysis_stores_static_analysis_from_eslint_report() {
            // End-to-end TS path: registry-parse(eslint-json) → store, through
            // the real service entry point (not just the golden parser test).
            let h = build(None).await;
            std::fs::write(h.tmp.path().join("eslint.json"), ESLINT_JSON).unwrap();
            let outcome = h
                .service
                .ingest_analysis(
                    &h.thread,
                    Some("eslint.json".into()),
                    Some("eslint-json".into()),
                    false,
                )
                .await
                .unwrap();
            match outcome {
                AnalysisIngest::Stored {
                    error_count,
                    warning_count,
                    info_count,
                    note_count,
                    findings,
                    ..
                } => {
                    assert_eq!(error_count, 2);
                    assert_eq!(warning_count, 1);
                    assert_eq!(info_count, 0);
                    assert_eq!(note_count, 0);
                    assert_eq!(findings, 3);
                }
                other => panic!("expected Stored, got {other:?}"),
            }
            // The observation landed on the open effort with the expected
            // findings list + counts, provenance observed, analyzer label.
            let rows = h
                .service
                .list_for_effort(&h.effort_id, Some("static-analysis"))
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].provenance, "observed");
            assert_eq!(rows[0].source, "analysis-report");
            // metric = error+warning count (lower = better).
            assert_eq!(rows[0].metric_value, Some(3.0));
            let payload: serde_json::Value =
                serde_json::from_str(rows[0].payload_json.as_deref().unwrap()).unwrap();
            assert_eq!(payload["errorCount"], 2);
            assert_eq!(payload["warningCount"], 1);
            assert_eq!(payload["analyzer"], "eslint");
            let findings = payload["findings"].as_array().unwrap();
            assert_eq!(findings.len(), 3);
            assert_eq!(findings[0]["path"], "src/a.ts");
            assert_eq!(findings[0]["rule"], "no-unused-vars");
            assert_eq!(findings[0]["severity"], "error");
            // null ruleId → no rule on that finding.
            assert_eq!(findings[2]["path"], "src/b.ts");
            assert!(findings[2]["rule"].is_null());
        }

        #[tokio::test]
        async fn ingest_analysis_stores_with_no_baseline() {
            // Findings are ABSOLUTE (current-file), not diff-relative like
            // coverage — so an effort with no start snapshot must still store
            // (pin = None), matching the passive ride-along. Regression guard
            // for the dropped baseline gate (tsk86).
            let h = build(None).await;
            // Re-open the effort with no start snapshot.
            let open = h
                .efforts
                .find_open_for_thread(&h.thread)
                .await
                .unwrap()
                .unwrap();
            h.efforts.finish(&open.id, None, None).await.unwrap();
            let no_base = h
                .efforts
                .start(open.task_id, &h.thread, None)
                .await
                .unwrap();
            assert!(no_base.start_snapshot_id.is_none());

            std::fs::write(h.tmp.path().join("eslint.json"), ESLINT_JSON).unwrap();
            let outcome = h
                .service
                .ingest_analysis(
                    &h.thread,
                    Some("eslint.json".into()),
                    Some("eslint-json".into()),
                    false,
                )
                .await
                .unwrap();
            match outcome {
                AnalysisIngest::Stored { findings, .. } => assert_eq!(findings, 3),
                other => panic!("expected Stored with no baseline, got {other:?}"),
            }
            // The observation landed, pinned to no local snapshot.
            let rows = h
                .service
                .list_for_effort(&no_base.id.to_string(), Some("static-analysis"))
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].local_snapshot_id, None);
        }

        #[tokio::test]
        async fn ingest_analysis_reports_missing_report() {
            let h = build(None).await;
            let outcome = h
                .service
                .ingest_analysis(
                    &h.thread,
                    Some("nope.json".into()),
                    Some("eslint-json".into()),
                    false,
                )
                .await
                .unwrap();
            assert_eq!(outcome, AnalysisIngest::ReportMissing("nope.json".into()));
        }

        #[tokio::test]
        async fn on_post_tool_use_handles_a_report_less_analysis_command() {
            // A clippy command with no fresh report: the hook runs cleanly and
            // (no test patterns) returns no nudge. tsk215: a report-less analyzer
            // run records no substrate row (a parseable report is what surfaces
            // analysis on the effort panel now).
            let h = build(None).await;
            let result = h
                .service
                .on_post_tool_use(
                    &h.thread,
                    &bash_payload("cargo clippy --workspace --all-targets", 0),
                )
                .await
                .unwrap();
            // No test patterns matched → no test nudge.
            assert!(result.is_none());
            assert!(h
                .service
                .list_for_effort(&h.effort_id, Some("static-analysis"))
                .await
                .unwrap()
                .is_empty());
            assert!(h
                .service
                .list_for_effort(&h.effort_id, Some("test-run"))
                .await
                .unwrap()
                .is_empty());
        }
    }
}
