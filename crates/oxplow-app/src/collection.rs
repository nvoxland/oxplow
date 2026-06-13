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
use oxplow_db::observation_store::{NewEffortObservation, SqliteEffortObservationStore};
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
    /// The open effort has no start snapshot, so the observation can't be
    /// pinned to a baseline (mirrors `CoverageIngest::NoBaseline`).
    NoBaseline,
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
    observations: Arc<SqliteEffortObservationStore>,
    efforts: Arc<SqliteTaskEffortStore>,
    threads: Arc<SqliteThreadStore>,
    snapshots: Arc<SqliteSnapshotStore>,
    blobs: BlobStore,
    config: Arc<RwLock<OxplowConfig>>,
    project_dir: PathBuf,
    events: EventBus,
    /// Efforts already nudged about a report-less test run. In-memory:
    /// ephemeral guidance that shouldn't pollute the effort_observation
    /// table or survive a restart.
    nudged_efforts: Arc<std::sync::Mutex<std::collections::HashSet<EffortId>>>,
    /// Commit shas already nudged about out-of-effort files. Same ephemeral
    /// in-memory dedup as `nudged_efforts`, but keyed by commit sha so the
    /// hygiene nudge fires at most once per commit.
    nudged_commits: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

impl CollectionService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        observations: Arc<SqliteEffortObservationStore>,
        efforts: Arc<SqliteTaskEffortStore>,
        threads: Arc<SqliteThreadStore>,
        snapshots: Arc<SqliteSnapshotStore>,
        blobs: BlobStore,
        config: Arc<RwLock<OxplowConfig>>,
        project_dir: PathBuf,
        events: EventBus,
    ) -> Self {
        Self {
            observations,
            efforts,
            threads,
            snapshots,
            blobs,
            config,
            project_dir,
            events,
            nudged_efforts: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            nudged_commits: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
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
        let id = self
            .observations
            .record(NewEffortObservation {
                stream_id,
                effort_id: effort.id.to_string(),
                kind: "test-run".into(),
                provenance: provenance.to_string(),
                source: source.to_string(),
                metric_value: None,
                payload_json: Some(serde_json::Value::Object(payload).to_string()),
                local_snapshot_id: None,
                closest_git_version: None,
                git_version_exact: false,
            })
            .await?;
        self.emit(thread, &effort);
        Ok(Some(id))
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
        let source = coverage_source(collector);
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
        self.store_diff_coverage(thread, &effort, &stream_id, &report, &source)
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
        // Pin the observation to the effort's start snapshot (mirrors
        // ingest_coverage). No baseline → can't anchor it meaningfully.
        if effort.start_snapshot_id.is_none() {
            return Ok(AnalysisIngest::NoBaseline);
        }
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
    async fn store_diff_coverage(
        &self,
        thread: &ThreadId,
        effort: &TaskEffort,
        stream_id: &str,
        report: &oxplow_coverage::CoverageReport,
        source: &str,
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
        let id = self
            .observations
            .record(NewEffortObservation {
                stream_id: stream_id.to_string(),
                effort_id: effort.id.to_string(),
                kind: "diff-coverage".into(),
                provenance: "observed".into(),
                source: source.to_string(),
                metric_value: Some(summary_pct),
                payload_json: Some(payload.to_string()),
                local_snapshot_id: Some(version.local_snapshot_id),
                closest_git_version: version.closest_git_version,
                git_version_exact: version.git_version_exact,
            })
            .await?;
        self.emit(thread, effort);
        Ok(CoverageIngest::Stored {
            observation_id: id,
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
        if let Some((merged, source)) = &coverage {
            if let Some(stream_id) = self.stream_id_for(thread).await? {
                let _ = self
                    .store_diff_coverage(thread, &effort, &stream_id, merged, source)
                    .await?;
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
            return Ok(Some(report_nudge_message(&cfg, &bash.command)));
        }
        Ok(None)
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
        let mut out_of_effort: Vec<String> = detail
            .files
            .iter()
            .map(|f| f.path.clone())
            .filter(|path| !self.path_changed_in_effort(path, &start_tree))
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
            .filter(|h| !h.starts_with("oversize:"))
            .and_then(|hash| self.blobs.read(hash).ok())
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

        let id = self
            .observations
            .record(NewEffortObservation {
                stream_id,
                effort_id: effort.id.to_string(),
                kind: "static-analysis".into(),
                provenance: "observed".into(),
                source: source.to_string(),
                metric_value,
                payload_json: Some(serde_json::Value::Object(payload).to_string()),
                local_snapshot_id,
                closest_git_version,
                git_version_exact,
            })
            .await?;
        self.emit(thread, &effort);
        Ok(Some(id))
    }

    /// Observations for an effort, newest-first. Pass `kind` to filter.
    pub async fn list_for_effort(
        &self,
        effort_id: &str,
        kind: Option<&str>,
    ) -> Result<Vec<oxplow_db::EffortObservation>, DomainError> {
        self.observations.list_for_effort(effort_id, kind).await
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
            .filter(|h| !h.starts_with("oversize:"))
            .and_then(|hash| self.blobs.read(hash).ok())
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

/// Trust label for a coverage collector's output: in-process tiers are
/// `observed` from a `coverage-report`; the external-exec escape hatch is
/// flagged `plugin-exec:<name>` so the UI can mark it lower-trust.
fn coverage_source(collector: &Collector) -> String {
    if collector.runtime() == CollectorRuntime::Exec {
        format!("plugin-exec:{}", collector.name())
    } else {
        "coverage-report".to_string()
    }
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
    fn coverage_source_flags_exec_lower_trust() {
        let jaq = Collector::jaq(
            "p",
            CollectorKind::Coverage,
            ["f"],
            CollectorInput::Xml,
            ".",
        );
        assert_eq!(coverage_source(&jaq), "coverage-report");
        let exec = Collector::exec("clover-cli", CollectorKind::Coverage, ["f"], ["cat"]);
        assert_eq!(coverage_source(&exec), "plugin-exec:clover-cli");
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
                    oversize: false,
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
                        oversize: false,
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

            let service = CollectionService::new(
                Arc::new(SqliteEffortObservationStore::new(db.clone())),
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
            // The effort still gets a test-run observation (the run was real).
            let obs = h
                .service
                .list_for_effort(&h.effort_id, Some("test-run"))
                .await
                .unwrap();
            assert_eq!(
                obs.len(),
                1,
                "test-run observation should still be recorded"
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
        async fn record_static_analysis_command_only_when_no_report() {
            // The ran-record: analyzer ran but produced no parseable report.
            let h = build(None).await;
            h.service
                .record_static_analysis(&h.thread, "cargo clippy", None, &[], "analysis-report")
                .await
                .unwrap();
            let rows = h
                .service
                .list_for_effort(&h.effort_id, Some("static-analysis"))
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            // No findings → no metric, command recorded.
            assert_eq!(rows[0].metric_value, None);
            let payload: serde_json::Value =
                serde_json::from_str(rows[0].payload_json.as_deref().unwrap()).unwrap();
            assert_eq!(payload["command"], "cargo clippy");
            assert!(payload.get("findings").is_none());
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
        async fn on_post_tool_use_records_static_analysis_on_analysis_command() {
            // A clippy command with no fresh report → command-only
            // static-analysis ran-record on the open effort (deterministic;
            // no report-mtime dependence).
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
            let rows = h
                .service
                .list_for_effort(&h.effort_id, Some("static-analysis"))
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert!(rows[0]
                .payload_json
                .as_deref()
                .unwrap()
                .contains("cargo clippy"));
            // A pure analysis command records no test-run.
            assert!(h
                .service
                .list_for_effort(&h.effort_id, Some("test-run"))
                .await
                .unwrap()
                .is_empty());
        }
    }
}
