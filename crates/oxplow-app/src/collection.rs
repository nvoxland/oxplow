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
use oxplow_db::{NewFact, NewMetricCapture, SqliteFactStore};
use oxplow_db::{
    SqliteAttributionStore, SqliteSnapshotStore, SqliteTaskEffortStore, SqliteThreadStore,
    TaskEffort, TaskEffortStore, STATE_CLAIMED,
};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{DomainError, EffortId, TaskId, ThreadId};

use crate::blob_store::BlobStore;
use crate::events::{EventBus, OxplowEvent};
use crate::file_ref_version;
use crate::metric_engine::threshold_state;

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

/// Executables that only READ. A sub-command leading with one of these can
/// MENTION a test/analysis pattern (a grep needle, an `echo`d reminder, a path)
/// without being an actual run — so the substring detector must skip it. Keeps
/// `grep test:collect …` / `cat … | … nextest …` from registering phantom runs
/// (and firing the report-less nudge).
const READ_ONLY_EXECUTABLES: &[&str] = &[
    "grep", "egrep", "fgrep", "rg", "ag", "ack", "echo", "printf", "cat", "bat", "less", "more",
    "head", "tail", "sed", "awk", "cut", "tr", "sort", "uniq", "comm", "diff", "wc", "ls", "find",
    "fd", "stat", "jq", "yq", "column",
];

/// The leading executable of one (operator-split) sub-command: lowercased,
/// basename only, skipping leading `VAR=val` env assignments (so the
/// `OXPLOW_TASK=tsk42` attribution token doesn't mask the real command).
/// `None` for an empty sub-command.
fn subcommand_exec(sub: &str) -> Option<String> {
    for tok in sub.split_whitespace() {
        if let Some((key, _)) = tok.split_once('=') {
            // UPPER_SNAKE=value → an env assignment prefix; keep scanning.
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                continue;
            }
        }
        let base = tok.rsplit(['/', '\\']).next().unwrap_or(tok);
        return Some(base.to_ascii_lowercase());
    }
    None
}

/// True when a sub-command's executable only reads (so a pattern match inside it
/// is incidental, not a run).
fn subcommand_is_read_only(sub: &str) -> bool {
    subcommand_exec(sub).is_some_and(|e| READ_ONLY_EXECUTABLES.contains(&e.as_str()))
}

/// Case-insensitive: does `command` actually INVOKE any built-in or extra
/// pattern? The command is split into sub-commands on shell operators
/// (`&&` / `||` / `;` / `|` / newline); a sub-command whose leading executable
/// only reads (grep/echo/cat/…) is ignored. So a command that merely *mentions*
/// a pattern (e.g. `grep test:collect .oxplow/project.yaml`) no longer counts as a run,
/// while a real `cd app && OXPLOW_TASK=tsk1 bun run test:collect` still does.
fn matches_any(command: &str, builtins: &[&str], extras: &[String]) -> bool {
    let pats: Vec<String> = builtins
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .chain(extras.iter().map(|s| s.to_ascii_lowercase()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if pats.is_empty() {
        return false;
    }
    let normalized = command
        .replace("&&", "\n")
        .replace("||", "\n")
        .replace([';', '|'], "\n");
    normalized.split('\n').any(|sub| {
        let lower = sub.to_ascii_lowercase();
        pats.iter().any(|p| lower.contains(p.as_str())) && !subcommand_is_read_only(sub)
    })
}

/// The optional `OXPLOW_TASK=<id>` attribution token an agent prefixes onto a
/// test command so the passive PostToolUse ride-along can pin the run to EXACTLY
/// that task's open effort (`find_open_for_task`), even with several efforts
/// open. Accepts the human id (`tsk42`) or a bare number (`42`). Returns `None`
/// when absent/unparseable (the run then uses the single-open auto rule).
fn parse_task_token(command: &str) -> Option<TaskId> {
    const KEY: &str = "OXPLOW_TASK=";
    let idx = command.find(KEY)?;
    let val = command[idx + KEY.len()..]
        .split_whitespace()
        .next()?
        .trim_matches(['"', '\'']);
    TaskId::try_from_str(val).or_else(|| val.parse::<i64>().ok().map(TaskId::new))
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

/// Cap on each in-memory nudge-dedup set. These are keyed by effort (or commit
/// sha) and only ever grew before — a slow but unbounded leak in a long-lived
/// daemon, since efforts are never explicitly forgotten (and the substrate may
/// age-sweep them away entirely). Insertion-ordered eviction past the cap keeps
/// memory bounded while preserving the one-shot guarantee for every recently
/// active effort; only entries older than `NUDGE_DEDUP_CAP` distinct keys ago
/// (long-closed efforts) can re-arm, which is harmless.
const NUDGE_DEDUP_CAP: usize = 1024;

/// A `HashSet` with a bounded size and oldest-first eviction. Used for the
/// ephemeral in-memory nudge-dedup sets so they can't grow without limit.
struct BoundedSet<T> {
    set: std::collections::HashSet<T>,
    order: std::collections::VecDeque<T>,
    cap: usize,
}

impl<T: std::hash::Hash + Eq + Clone> BoundedSet<T> {
    fn new(cap: usize) -> Self {
        Self {
            set: std::collections::HashSet::new(),
            order: std::collections::VecDeque::new(),
            cap,
        }
    }

    /// Insert `v`; return `true` the first time it's seen, `false` if already
    /// present. Evicts the oldest entry once over capacity.
    fn insert(&mut self, v: T) -> bool {
        if !self.set.insert(v.clone()) {
            return false;
        }
        self.order.push_back(v);
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        true
    }

    fn contains(&self, v: &T) -> bool {
        self.set.contains(v)
    }
}

#[derive(Clone)]
pub struct CollectionService {
    /// Durable fact layer (epic tsk12): coverage/test/analysis producers
    /// dual-write atomic facts here beside the legacy samples/findings. The
    /// aggregation engine reads these; the samples are the rebuildable cache.
    facts: Arc<SqliteFactStore>,
    nudges: Arc<SqliteAgentNudgeStore>,
    efforts: Arc<SqliteTaskEffortStore>,
    threads: Arc<SqliteThreadStore>,
    snapshots: Arc<SqliteSnapshotStore>,
    blobs: BlobStore,
    config: Arc<RwLock<OxplowConfig>>,
    project_dir: PathBuf,
    events: EventBus,
    /// Kind-agnostic attribution ledger (tsk262/263) — runs (test/coverage/
    /// analysis) record their claim state here. A run is auto-attributed to the
    /// open effort at record time only when unambiguous (`find_single_open_for_thread`);
    /// the concurrent case is resolved by the close reconcile + the agent's claim.
    attribution: Arc<SqliteAttributionStore>,
    /// Efforts already nudged about a report-less test run. In-memory:
    /// ephemeral guidance that shouldn't be persisted or survive a restart.
    /// Bounded (see [`BoundedSet`]) so it can't leak in a long-lived daemon.
    nudged_efforts: Arc<std::sync::Mutex<BoundedSet<EffortId>>>,
    /// Commit shas already nudged about out-of-effort files. Same ephemeral
    /// bounded in-memory dedup as `nudged_efforts`, but keyed by commit sha so
    /// the hygiene nudge fires at most once per commit.
    nudged_commits: Arc<std::sync::Mutex<BoundedSet<String>>>,
    /// Efforts already nudged about coverage below target (tsk220). Separate
    /// set from `nudged_efforts` so the report-less and coverage-target nudges
    /// don't suppress each other; same ephemeral bounded in-memory dedup.
    nudged_coverage: Arc<std::sync::Mutex<BoundedSet<EffortId>>>,
    /// `(effort, metric_id)` pairs already surfaced as a gauge warn/fail
    /// crossing in the effort-metric prompt context (tsk231). Gauge metrics run
    /// on-snapshot in the background, not in a hook, so their threshold crossing
    /// can't ride the PostToolUse `additionalContext`; instead the
    /// UserPromptSubmit context line surfaces it **once** per effort+metric.
    /// Same ephemeral bounded in-memory dedup.
    nudged_gauge: Arc<std::sync::Mutex<BoundedSet<(EffortId, i64)>>>,
}

impl CollectionService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        facts: Arc<SqliteFactStore>,
        nudges: Arc<SqliteAgentNudgeStore>,
        efforts: Arc<SqliteTaskEffortStore>,
        threads: Arc<SqliteThreadStore>,
        snapshots: Arc<SqliteSnapshotStore>,
        blobs: BlobStore,
        config: Arc<RwLock<OxplowConfig>>,
        project_dir: PathBuf,
        events: EventBus,
        attribution: Arc<SqliteAttributionStore>,
    ) -> Self {
        Self {
            facts,
            nudges,
            efforts,
            threads,
            snapshots,
            blobs,
            config,
            project_dir,
            events,
            attribution,
            nudged_efforts: Arc::new(std::sync::Mutex::new(BoundedSet::new(NUDGE_DEDUP_CAP))),
            nudged_commits: Arc::new(std::sync::Mutex::new(BoundedSet::new(NUDGE_DEDUP_CAP))),
            nudged_coverage: Arc::new(std::sync::Mutex::new(BoundedSet::new(NUDGE_DEDUP_CAP))),
            nudged_gauge: Arc::new(std::sync::Mutex::new(BoundedSet::new(NUDGE_DEDUP_CAP))),
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
        task: Option<TaskId>,
    ) -> Result<Option<i64>, DomainError> {
        // OBSERVE: record the run regardless of effort; attribution is separate
        // (tsk263). We only need the stream to record into the substrate.
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
        // Resolve the owning effort ONCE — used to stamp the fact-capture below
        // (so `captures_for_effort` attributes it, tsk37) and to claim the run in
        // the ledger at the tail. Same resolution the auto-claim uses.
        let owning = self.resolve_owning_effort(thread, task).await;
        let owning_val = owning.as_ref().map(|e| e.id.value());

        // Write the run CAPTURE into the durable fact layer (epic tsk12): one
        // fact on `oxplow.test_case` per case, the pass/fail/skip status carried
        // as the `oxplow.status` dimension (and the suite as `oxplow.test_suite`)
        // so Count() sliced by status reconstructs the passed/failed/total
        // headline. The capture IS the run (T-E1, tsk48): it carries the verbatim
        // payload in `detail_json` and its id is what the ledger claims. Recorded
        // even report-less (observe-always) — but a run that MEASURED nothing
        // (no report, no asserted counts) records under the `test-run` producer,
        // not `tests`: an empty `tests` capture reads as "suite ran, found 0
        // tests" to the zero-fill/currency logic (tsk44) and would collapse the
        // semi-additive oxplow.tests.* timeline to 0. Asserted counts (the MCP
        // sub-agent path) synthesize status-sliced facts — no case identity, but
        // the counts are real case-grain measurements the specs must read.
        let counted = passed.is_some() || failed.is_some() || total.is_some();
        let mut capture_id: Option<i64> = None;
        if let Some(stream_val) =
            oxplow_domain::StreamId::try_from_str(&stream_id).map(|s| s.value())
        {
            let dual = async {
                let mut facts = Vec::new();
                if let Some(r) = report {
                    let Some(measure) = self.facts.get_measure("oxplow.test_case").await? else {
                        return Ok::<Option<i64>, DomainError>(None);
                    };
                    use oxplow_coverage::TestStatus::*;
                    for suite in &r.suites {
                        for case in &suite.cases {
                            let status = match case.status {
                                Passed => "passed",
                                Failed => "failed",
                                Skipped => "skipped",
                            };
                            facts.push(NewFact {
                                subject_kind: Some("test".into()),
                                subject_ref: Some(format!(
                                    "test:{}::{}",
                                    case.classname, case.name
                                )),
                                dims_json: serde_json::to_string(&json!({
                                    "oxplow.status": status,
                                    "oxplow.test_suite": suite.name,
                                }))
                                .ok(),
                                ..NewFact::new(measure.id, 1.0)
                            });
                        }
                    }
                } else if counted {
                    let Some(measure) = self.facts.get_measure("oxplow.test_case").await? else {
                        return Ok::<Option<i64>, DomainError>(None);
                    };
                    let p = passed.unwrap_or(0).max(0);
                    let f = failed.unwrap_or(0).max(0);
                    let s = total.map(|t| (t - p - f).max(0)).unwrap_or(0);
                    for (status, n) in [("passed", p), ("failed", f), ("skipped", s)] {
                        for _ in 0..n {
                            facts.push(NewFact {
                                subject_kind: Some("test".into()),
                                dims_json: serde_json::to_string(&json!({
                                    "oxplow.status": status,
                                }))
                                .ok(),
                                ..NewFact::new(measure.id, 1.0)
                            });
                        }
                    }
                }
                let branch = oxplow_git::detect_current_branch(&self.project_dir);
                let snapshot_id = self
                    .snapshots
                    .latest_snapshot_id_for_stream(oxplow_domain::StreamId::new(stream_val))
                    .await
                    .ok()
                    .flatten();
                // `tests` = a measurement (report or asserted counts — a zero
                // here is a real "found 0"); `test-run` = a run record only,
                // invisible to the tests metric timeline.
                let producer = if report.is_some() || counted {
                    "tests"
                } else {
                    "test-run"
                };
                let mut capture = NewMetricCapture::done(stream_val, producer, source.to_string());
                capture.provenance = provenance.to_string();
                capture.thread_id = Some(thread.value());
                capture.trigger = Some("on-report".into());
                capture.branch = branch;
                capture.snapshot_id = snapshot_id;
                capture.effort_id = owning_val;
                capture.detail_json = Self::capture_detail(
                    "test-detail",
                    &serde_json::Value::Object(payload.clone()),
                );
                let id = self.facts.record_facts(capture, facts).await?;
                Ok(Some(id))
            }
            .await;
            match dual {
                Ok(id) => {
                    capture_id = id;
                    self.events.emit(OxplowEvent::MetricSamplesChanged {
                        stream_id: oxplow_domain::StreamId::new(stream_val),
                    });
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to write the test-run capture")
                }
            }
        }
        let _ = (stream_id, payload);
        // ATTRIBUTE via the unified run ledger (the effort resolved above), then
        // refresh the panel for the effort it landed on (if any). Observe-always:
        // the run is already recorded above regardless of effort. The claimed ref
        // is the CAPTURE id (T-E1) — the legacy run row is no longer the identity.
        if let (Some(cid), Some(effort)) = (capture_id, owning.as_ref()) {
            self.claim_run(effort, cid).await;
            self.emit(thread, effort);
        }
        Ok(capture_id)
    }

    /// Attribute a just-recorded run (`run:<id>`) to an effort via the unified
    /// `"run"` ledger (tsk269), riding only oxplow's MCP contract + effort state,
    /// never any agent internals.
    ///
    /// Naming a `task` is **EXACT-or-nothing** (tsk271): resolve that task's open
    /// effort via `find_open_for_task` and claim the run for it, even under
    /// concurrency — a dispatched sub-agent self-attributes by naming its own
    /// task, with oxplow never seeing "which sub-agent". When the named task has
    /// NO open effort, the run is **left unclaimed** — it is NOT auto-attributed
    /// to whatever single effort happens to be open, since that effort belongs to
    /// a different task and claiming it would be a *wrong-exact* mis-attribution
    /// (the design otherwise guarantees "less exact, never wrong-exact"). An
    /// UNNAMED run uses the AUTO optimization — claim only when exactly one effort
    /// is open (`find_single_open_for_thread`). Either way the unclaimed case
    /// defers to the close reconcile + window-dominance + the agent's claim, and
    /// the run is always recorded (observe-always), never dropped.
    ///
    /// Returns the effort it attributed to (for a panel refresh). Best-effort: a
    /// ledger write error never fails the host path.
    async fn auto_attribute_run(
        &self,
        thread: &ThreadId,
        run_id: i64,
        task: Option<TaskId>,
    ) -> Option<TaskEffort> {
        let attribute_to = self.resolve_owning_effort(thread, task).await;
        if let Some(effort) = attribute_to.as_ref() {
            self.claim_run(effort, run_id).await;
        }
        attribute_to
    }

    /// The effort a just-produced run/capture belongs to, by the SAME
    /// exact-or-nothing (a named task's open effort) / single-open (unnamed)
    /// resolution the run auto-claim uses (tsk271). A named task is
    /// exact-or-nothing — never the single-open thread guess, which could claim a
    /// DIFFERENT task's effort. Used both to claim the run in the ledger AND to
    /// stamp `metric_capture.effort_id`, so the fact-attribution read
    /// (`captures_for_effort`, T-D) attributes the producer's facts (tsk37).
    async fn resolve_owning_effort(
        &self,
        thread: &ThreadId,
        task: Option<TaskId>,
    ) -> Option<TaskEffort> {
        match task {
            Some(tid) => self.efforts.find_open_for_task(tid).await.ok().flatten(),
            None => self
                .efforts
                .find_single_open_for_thread(thread)
                .await
                .ok()
                .flatten(),
        }
    }

    /// Claim `run:<id>` for an effort in the unified run ledger (best-effort — a
    /// ledger write error never fails the host path).
    async fn claim_run(&self, effort: &TaskEffort, run_id: i64) {
        let _ = self
            .attribution
            .set_state(
                &effort.id,
                "run",
                &format!("run:{run_id}"),
                STATE_CLAIMED,
                None,
            )
            .await;
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
        // OBSERVE-ALWAYS (tsk270): record absolute coverage regardless of effort;
        // the effort-relative diff is derived at read.
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
        if skip_if_stale && report_is_stale(&abs, report_fresh_floor()) {
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
        self.observe_coverage(thread, &stream_id, &report).await
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
        // OBSERVE-ALWAYS (tsk269): analysis findings are absolute, so we record
        // regardless of open-effort count; `record_static_analysis` attributes via
        // the ledger.
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
        if skip_if_stale && report_is_stale(&abs, report_fresh_floor()) {
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

    /// The capture-spine detail envelope (T-E1, tsk48): the verbatim per-run
    /// payload wrapped as `{"kind": <detail kind>, "payload": {…}}`, stored in
    /// `metric_capture.detail_json`. The kind discriminates the three run
    /// producers (test-detail / coverage-detail / analysis-detail) for the
    /// observations panel + the read-time diff-coverage derivation.
    fn capture_detail(kind: &str, payload: &serde_json::Value) -> Option<String> {
        serde_json::to_string(&json!({ "kind": kind, "payload": payload })).ok()
    }

    /// Content identity for an idempotent report ingest (tsk14): a hash over the
    /// producer, the basis it was measured against (git version + snapshot), and
    /// the verbatim payload envelope, so a REPLAYED report (a hook that fired
    /// twice, `ingest_coverage`/`ingest_analysis` called again) dedupes to the
    /// same capture instead of double-counting additive facts. `None` when
    /// there's no payload to identify by — such a capture always inserts fresh.
    fn ingest_idempotency_key(
        producer: &str,
        git_version: Option<&str>,
        snapshot_id: Option<i64>,
        detail_json: Option<&str>,
    ) -> Option<String> {
        let detail = detail_json?;
        let identity = format!(
            "{producer}|{}|{}|{detail}",
            git_version.unwrap_or(""),
            snapshot_id.map(|s| s.to_string()).unwrap_or_default(),
        );
        Some(crate::blob_store::BlobStore::hash(identity.as_bytes()))
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
    ) -> Option<i64> {
        let stream_val = oxplow_domain::StreamId::try_from_str(stream_id).map(|s| s.value())?;
        let branch = oxplow_git::detect_current_branch(&self.project_dir);
        let analyzer = analyzers
            .first()
            .cloned()
            .unwrap_or_else(|| "analysis".to_string());
        // The capture-spine copy of the verbatim payload (T-E1, tsk48) — taken
        // before the legacy detail-finding write consumes `detail`.
        let capture_detail_json = detail
            .as_ref()
            .and_then(|d| Self::capture_detail("analysis-detail", d));
        let dual = async {
            let Some(measure) = self.facts.get_measure("oxplow.lint_hit").await? else {
                return Ok::<Option<i64>, DomainError>(None);
            };
            // A CLEAN report still writes its (empty) capture — "this
            // analysis ran and found nothing" is what lets the errors/
            // warnings series drop back to zero (tsk44).
            use oxplow_coverage::Severity::*;
            let mut facts = Vec::with_capacity(report.findings.len());
            for f in &report.findings {
                let severity = match f.severity {
                    Error => "error",
                    Warning => "warning",
                    Info => "info",
                    Note => "note",
                };
                facts.push(NewFact {
                    subject_kind: Some("file".into()),
                    subject_ref: Some(format!("file:{}", f.path)),
                    path: Some(f.path.clone()),
                    line: f.line.map(|l| l as i64),
                    severity: Some(severity.into()),
                    rule: f.rule.clone(),
                    detail: Some(f.message.clone()),
                    ..NewFact::new(measure.id, 1.0)
                });
            }
            // Stamp the owning effort (single-open, matching the run
            // auto-claim below) so `captures_for_effort` attributes these
            // lint facts (tsk37).
            let owning_val = self
                .resolve_owning_effort(thread, None)
                .await
                .map(|e| e.id.value());
            let mut capture =
                NewMetricCapture::done(stream_val, analyzer.clone(), source.to_string());
            capture.thread_id = Some(thread.value());
            capture.trigger = Some("on-report".into());
            capture.snapshot_id = snapshot_id;
            capture.closest_git_version = git_version.clone();
            capture.git_version_exact = git_version_exact;
            capture.branch = branch.clone();
            capture.effort_id = owning_val;
            capture.detail_json = capture_detail_json;
            capture.idempotency_key = Self::ingest_idempotency_key(
                &analyzer,
                git_version.as_deref(),
                snapshot_id,
                capture.detail_json.as_deref(),
            );
            let id = self.facts.record_facts(capture, facts).await?;
            Ok(Some(id))
        }
        .await;
        match dual {
            Ok(capture_id) => {
                self.events.emit(OxplowEvent::MetricSamplesChanged {
                    stream_id: oxplow_domain::StreamId::new(stream_val),
                });
                capture_id
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to write the analysis capture");
                None
            }
        }
    }

    /// OBSERVE-ALWAYS coverage (tsk270): record the **absolute** whole-report
    /// coverage — per-file instrumented/covered line-sets, verbatim, in the
    /// `coverage-detail` finding + an `oxplow.coverage.abs_pct` headline + the
    /// run (pinned to the stream's current snapshot) — with NO effort baseline.
    /// The effort-relative diff-coverage is derived later at READ from these
    /// line-sets ([`diff_coverage_for_effort`]). Attribution rides the unified
    /// `"run"` ledger (auto-claimed when unambiguous, else reconciled/claimed).
    async fn observe_coverage(
        &self,
        thread: &ThreadId,
        stream_id: &str,
        report: &oxplow_coverage::CoverageReport,
    ) -> Result<CoverageIngest, DomainError> {
        let Some((abs_pct, total_cov, total_instr, payload)) = coverage_abs_payload(report) else {
            return Ok(CoverageIngest::NoChangedCoverage);
        };

        // Pin to the stream's current snapshot — the code state the report
        // measured — independent of any effort (observe-always).
        let pin = match oxplow_domain::StreamId::try_from_str(stream_id) {
            Some(s) => self
                .snapshots
                .latest_snapshot_id_for_stream(s)
                .await
                .ok()
                .flatten(),
            None => None,
        };
        let version = match pin {
            Some(p) => file_ref_version::resolve(&self.snapshots, &self.project_dir, p).await?,
            None => file_ref_version::ResolvedFileVersion {
                local_snapshot_id: 0,
                closest_git_version: None,
                git_version_exact: false,
            },
        };
        // The owning effort stamps the coverage capture AND receives the ledger
        // claim below — the capture IS the run now (T-E1, tsk48).
        let attribute_to = self.resolve_owning_effort(thread, None).await;
        let owning_val = attribute_to.as_ref().map(|e| e.id.value());

        // The run CAPTURE (epic tsk12): one fact on `oxplow.coverage` per file,
        // value = its line-%, numerator/denominator = covered/instrumented
        // counts so the engine re-derives the headline as Σcovered/Σinstrumented
        // (non-additive ratio) instead of averaging pre-rolled percentages; the
        // verbatim per-file line-sets ride in `detail_json` (the read-time
        // diff-coverage derivation + observations panel read them, T-E1).
        let mut capture_id: Option<i64> = None;
        if let Some(stream_val) =
            oxplow_domain::StreamId::try_from_str(stream_id).map(|s| s.value())
        {
            let dual = async {
                let Some(measure) = self.facts.get_measure("oxplow.coverage").await? else {
                    return Ok::<Option<i64>, DomainError>(None);
                };
                let mut facts = Vec::new();
                for (path, fc) in &report.files {
                    let instr = fc.instrumented.len();
                    if instr == 0 {
                        continue;
                    }
                    let covered = fc.covered.len();
                    let pct = covered as f64 / instr as f64 * 100.0;
                    facts.push(NewFact {
                        numerator: Some(covered as f64),
                        denominator: Some(instr as f64),
                        subject_kind: Some("file".into()),
                        subject_ref: Some(format!("file:{path}")),
                        path: Some(path.clone()),
                        ..NewFact::new(measure.id, pct)
                    });
                }
                let branch = oxplow_git::detect_current_branch(&self.project_dir);
                let snapshot_id =
                    (version.local_snapshot_id != 0).then_some(version.local_snapshot_id);
                let mut capture = NewMetricCapture::done(stream_val, "coverage", "coverage-report");
                capture.thread_id = Some(thread.value());
                capture.trigger = Some("on-report".into());
                capture.snapshot_id = snapshot_id;
                capture.closest_git_version = version.closest_git_version.clone();
                capture.git_version_exact = version.git_version_exact;
                capture.basis_ref = version.closest_git_version.clone();
                capture.branch = branch;
                capture.effort_id = owning_val;
                capture.detail_json = Self::capture_detail("coverage-detail", &payload);
                capture.idempotency_key = Self::ingest_idempotency_key(
                    "coverage",
                    version.closest_git_version.as_deref(),
                    snapshot_id,
                    capture.detail_json.as_deref(),
                );
                let id = self.facts.record_facts(capture, facts).await?;
                Ok(Some(id))
            }
            .await;
            match dual {
                Ok(id) => {
                    capture_id = id;
                    self.events.emit(OxplowEvent::MetricSamplesChanged {
                        stream_id: oxplow_domain::StreamId::new(stream_val),
                    });
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to write the coverage capture")
                }
            }
        }
        // ATTRIBUTE via the unified run ledger (the capture id is the ref), then
        // refresh the panel for the effort it landed on (if any).
        if let (Some(cid), Some(effort)) = (capture_id, attribute_to.as_ref()) {
            self.claim_run(effort, cid).await;
            self.emit(thread, effort);
        }

        Ok(CoverageIngest::Stored {
            observation_id: 0,
            summary_pct: abs_pct,
            changed_lines: total_instr,
            covered_lines: total_cov,
        })
    }

    /// Derive the effort-relative **diff-coverage** from a run's stored ABSOLUTE
    /// per-file line-sets, against the effort's start snapshot (tsk270). The body
    /// is the pre-tsk270 `store_diff_coverage` computation, moved to read time so
    /// a coverage run claimed AFTER the effort closed still produces a diff and
    /// nothing is frozen at reconcile. Returns `(summary_pct, diff_payload)` or
    /// `None` when the effort has no start snapshot or no changed instrumented
    /// lines overlap. `diff_payload` is the same shape the panel already renders.
    async fn diff_coverage_for_effort(
        &self,
        effort: &TaskEffort,
        abs_payload: &serde_json::Value,
    ) -> Result<Option<(f64, serde_json::Value)>, DomainError> {
        let Some(start) = effort.start_snapshot_id else {
            return Ok(None);
        };
        let start_tree = self.snapshots.tree_at(start).await?;
        let to_set = |v: Option<&serde_json::Value>| -> BTreeSet<u32> {
            v.and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_u64().map(|n| n as u32))
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut total_changed = 0usize;
        let mut total_covered = 0usize;
        let mut files_payload = Vec::new();
        for f in abs_payload
            .get("files")
            .and_then(|v| v.as_array())
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let Some(path) = f.get("path").and_then(|p| p.as_str()) else {
                continue;
            };
            let changed = self.changed_lines_for(path, &start_tree);
            if changed.is_empty() {
                continue;
            }
            let instrumented = to_set(f.get("instrumented"));
            let covered = to_set(f.get("covered"));
            let changed_instr: BTreeSet<u32> =
                instrumented.intersection(&changed).copied().collect();
            if changed_instr.is_empty() {
                continue;
            }
            let changed_cov: BTreeSet<u32> =
                covered.intersection(&changed_instr).copied().collect();
            let uncovered: Vec<u32> = changed_instr.difference(&changed_cov).copied().collect();
            total_changed += changed_instr.len();
            total_covered += changed_cov.len();
            files_payload.push(json!({ "path": path, "uncoveredChangedLines": uncovered }));
        }
        if total_changed == 0 {
            return Ok(None);
        }
        let summary_pct = (total_covered as f64 / total_changed as f64) * 100.0;
        Ok(Some((
            summary_pct,
            json!({
                "summaryPct": summary_pct,
                "changedLines": total_changed,
                "coveredLines": total_covered,
                "files": files_payload,
            }),
        )))
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
        // OBSERVE-ALWAYS (tsk269): tests + analysis are recorded regardless of how
        // many efforts are open — attribution is deferred to the ledger, never a
        // precondition for recording. The single open effort (if any) is resolved
        // here only for the effort-RELATIVE advisories (commit-hygiene, coverage,
        // nudges), which legitimately no-op when ambiguous.
        let effort_opt = self.efforts.find_single_open_for_thread(thread).await?;

        // Commit-hygiene nudge: a successful `git commit` that swept in files
        // outside the open effort's changed set. Effort-relative advisory — only
        // with a single open effort. Independent of the ride-alongs below.
        if is_commit {
            if let Some(effort) = &effort_opt {
                if let Some(msg) = self.check_commit_hygiene(effort, bash.exit_code).await? {
                    self.persist_nudge(thread, Some(effort), "commit-hygiene", &msg, &bash.command)
                        .await;
                    return Ok(Some(msg));
                }
            }
            // A pure commit (not also a test/analysis run) is done.
            if !is_test && !is_analysis {
                return Ok(None);
            }
        }
        let registry = self.registry(&cfg);
        let floor = report_fresh_floor();

        // Static-analysis ride-along (OBSERVE-ALWAYS): when an analyzer ran,
        // record a static-analysis observation — command-only (the ran-record)
        // when no fresh analysis report exists, or carrying merged findings when
        // one does. Classification is by collector kind, not a format heuristic.
        if is_analysis {
            let (report, source, analyzers) =
                match self.merge_fresh_analysis(floor, &cfg, &registry) {
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
        // Merge every fresh test report into one per-test tree (each test stack
        // regenerates its own report; the freshness window excludes stale ones
        // from prior runs/other stacks).
        let report = self.merge_fresh_test_reports(floor, &cfg, &registry);
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
            // Exact attribution when the agent prefixed `OXPLOW_TASK=<id>`
            // (find_open_for_task — survives concurrent efforts); otherwise the
            // single-open auto rule attributes it (tsk265/tsk271).
            parse_task_token(&bash.command),
        )
        .await?;
        // Coverage ride-along (OBSERVE-ALWAYS, tsk270): record the ABSOLUTE
        // report regardless of effort; the effort-relative diff is derived later
        // at read. Keep the merged report so the coverage-target nudge can derive
        // this effort's diff-coverage when a single effort is open.
        let coverage = self.merge_fresh_coverage(floor, &cfg, &registry);
        if let Some((merged, _source)) = &coverage {
            if let Some(stream_id) = self.stream_id_for(thread).await? {
                let _ = self.observe_coverage(thread, &stream_id, merged).await?;
            }
        }
        // Nudges below are effort-RELATIVE (key/dedup per effort), so they only
        // run with a single open effort. The runs above are already recorded.
        let Some(effort) = effort_opt else {
            return Ok(None);
        };
        // Derive THIS effort's diff-coverage from the absolute report (read-time)
        // for the coverage-target nudge.
        let coverage_pct = match &coverage {
            Some((merged, _)) => match coverage_abs_payload(merged) {
                Some((_, _, _, abs_payload)) => self
                    .diff_coverage_for_effort(&effort, &abs_payload)
                    .await?
                    .map(|(pct, _)| pct),
                None => None,
            },
            None => None,
        };
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
            // One fact on the `oxplow.nudge` event measure (value 1), the nudge
            // kind as subject so Sum() reconstructs the fired count (epic tsk12;
            // the legacy sample write is gone, T-E2).
            if let Some(measure) = self.facts.get_measure("oxplow.nudge").await? {
                let fact = NewFact {
                    subject_kind: Some("nudge".into()),
                    subject_ref: Some(kind.to_string()),
                    dims_json: Some(format!("{{\"kind\":\"{kind}\"}}")),
                    ..NewFact::new(measure.id, 1.0)
                };
                let owning_val = self
                    .resolve_owning_effort(thread, None)
                    .await
                    .map(|e| e.id.value());
                let mut capture = NewMetricCapture::done(stream_val, "nudges", "nudges");
                capture.thread_id = Some(thread.value());
                capture.trigger = Some("continuous".into());
                capture.branch = branch;
                capture.effort_id = owning_val;
                self.facts.record_facts(capture, vec![fact]).await?;
            }
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

    /// Has `(effort, metric)` already had its crossing surfaced? A read-only
    /// peek: [`effort_metric_context`] decides which crossings are fresh during
    /// its loop but defers the actual `mark_gauge_nudged` to its await-free tail
    /// (so a hook timeout mid-loop can't consume a one-shot the agent never saw).
    /// A poisoned lock reads as "already nudged" — suppress rather than nag.
    fn gauge_already_nudged(&self, effort: &EffortId, metric_id: i64) -> bool {
        match self.nudged_gauge.lock() {
            Ok(set) => set.contains(&(*effort, metric_id)),
            Err(_) => true,
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
        // Advisory prompt — only when the open effort is unambiguous; under
        // parallel sub-agents we can't say whose deltas these are (tsk263).
        let effort = self
            .efforts
            .find_single_open_for_thread(thread)
            .await
            .ok()??;
        // Shared attribution core (tsk253): the same per-family roll-up the
        // task-page panel reads (`effort_metric_deltas`), so the prompt and the
        // UI report the SAME baseline→current — file-attributed for gauges, so
        // under overlapping efforts the agent sees only its own effort's effect.
        // The one-shot warn/fail crossing nudge is layered on top here.
        let deltas = self.effort_metric_deltas(&effort.id.to_string()).await;
        if deltas.is_empty() {
            return None;
        }
        // key → metric-spec id, for the one-shot crossing dedup (keyed by that id
        // in `nudged_gauge`); the delta carries the key, not the id.
        let id_by_key: std::collections::HashMap<String, i64> = self
            .facts
            .list_specs()
            .await
            .ok()?
            .into_iter()
            .map(|s| (s.key, s.id))
            .collect();
        let mut lines: Vec<String> = Vec::new();
        // Crossings surfaced this turn — marked consumed only in the await-free
        // tail below, never inside the loop (a hook timeout could otherwise drop
        // the response after a one-shot was already consumed mid-loop).
        let mut fresh_crossings: Vec<i64> = Vec::new();
        for d in &deltas {
            // Operational/event metrics (tokens, cost, cycle-time, nudges,
            // navigation) grow every turn and aren't code-health signals — keep
            // the line focused on code metrics.
            if d.kind == "event" || crate::attribution::is_operational_metric_key(&d.key) {
                continue;
            }
            let metric_id = id_by_key.get(&d.key).copied();
            // Peek (don't consume) the one-shot here; consume after the loop.
            let fresh_crossing = d.crossing.is_some()
                && metric_id.is_some_and(|id| !self.gauge_already_nudged(&effort.id, id));
            if fresh_crossing {
                if let Some(id) = metric_id {
                    fresh_crossings.push(id);
                }
            }
            if !d.changed && !fresh_crossing {
                continue;
            }
            let mut line = format!(
                "- {}: {} → {}{}",
                d.title,
                fmt_metric_num(d.baseline.unwrap_or(d.current)),
                fmt_metric_num(d.current),
                fmt_unit_suffix(d.unit.as_deref().unwrap_or("")),
            );
            if d.changed {
                if let Some(delta) = d.delta {
                    line.push_str(&format!(" (Δ {})", fmt_signed(delta)));
                }
            }
            if fresh_crossing {
                if let Some(level) = d.crossing.as_deref() {
                    let thresh = if level == "fail" {
                        d.fail_at
                    } else {
                        d.warn_at
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
        // Await-free tail: consume the one-shots now that the context is fully
        // built and about to be returned. The timeout that bounds the hook only
        // fires at await points, so nothing between here and the response can
        // drop a marker we've consumed.
        for metric_id in fresh_crossings {
            self.mark_gauge_nudged(&effort.id, metric_id);
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
        floor: oxplow_domain::Timestamp,
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
            if report_is_stale(&abs, floor) {
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
        floor: oxplow_domain::Timestamp,
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
            if report_is_stale(&abs, floor) {
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
        floor: oxplow_domain::Timestamp,
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
            if report_is_stale(&abs, floor) {
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

    /// Record a `static-analysis` observation. OBSERVE-ALWAYS (tsk269): analysis
    /// findings are absolute (current-file, not effort-relative), so the run is
    /// recorded regardless of how many efforts are open and attributed via the
    /// unified `"run"` ledger. This single kind is both the ran-record (when
    /// `report` is `None` — analyzer ran but regenerated no parseable report) and
    /// the findings (when a report parsed). The headline metric is the
    /// error+warning count (lower = better). Returns `Ok(None)` only when there's
    /// no stream or nothing was recorded.
    async fn record_static_analysis(
        &self,
        thread: &ThreadId,
        command: &str,
        report: Option<&oxplow_coverage::AnalysisReport>,
        analyzers: &[String],
        source: &str,
    ) -> Result<Option<i64>, DomainError> {
        let Some(stream_id) = self.stream_id_for(thread).await? else {
            return Ok(None);
        };
        // Optional single open effort — for the snapshot pin + panel refresh only;
        // attribution rides the ledger (auto-claimed below when unambiguous).
        let effort = self.efforts.find_single_open_for_thread(thread).await?;
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

        // Snapshot pin: the effort's end-or-start snapshot when one is open, else
        // the stream's current snapshot (the code state the analyzer ran against)
        // — so observe-always still pins the run to a code state under 0/N efforts.
        let pin = match &effort {
            Some(e) => e.end_snapshot_id.or(e.start_snapshot_id),
            None => match oxplow_domain::StreamId::try_from_str(&stream_id) {
                Some(s) => self
                    .snapshots
                    .latest_snapshot_id_for_stream(s)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            },
        };
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
        // report was parsed (command-only analyzer runs have no counts → no run).
        let run_id = if let Some(r) = report {
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
            .await
        } else {
            None
        };
        let _ = (
            stream_id,
            metric_value,
            payload,
            local_snapshot_id,
            closest_git_version,
            git_version_exact,
        );
        // Attribute the run via the unified ledger, then refresh the panel for the
        // effort it landed on (command-only runs have no run → refresh the single
        // open effort if any).
        let attribute_to = match run_id {
            Some(rid) => self.auto_attribute_run(thread, rid, None).await,
            None => effort,
        };
        if let Some(e) = attribute_to.as_ref() {
            self.emit(thread, e);
        }
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
        // Coverage derives its effort-relative diff against the effort's start
        // snapshot, so load the effort once (tsk270).
        let effort = self.efforts.get_effort(&eid).await.ok().flatten();
        // Every run kind is observe-always → attribute by the unified ledger
        // CLAIM (exact under concurrency), never a time window (which would mix
        // concurrent efforts' runs). The capture IS the run (T-E1, tsk48): the
        // claimed refs are capture ids, and the verbatim payload rides in the
        // capture's `detail_json` envelope.
        let mut caps: Vec<oxplow_db::MetricCapture> = Vec::new();
        for id in self
            .attribution
            .list_refs(&eid, "run", STATE_CLAIMED)
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|r| r.strip_prefix("run:").and_then(|s| s.parse::<i64>().ok()))
        {
            if let Ok(Some(c)) = self.facts.get_capture(id).await {
                caps.push(c);
            }
        }
        // Newest-first for the panel.
        caps.sort_by(|a, b| b.captured_at.cmp(&a.captured_at).then(b.id.cmp(&a.id)));
        let mut out = Vec::new();
        for c in caps {
            let Some(envelope) = c
                .detail_json
                .as_deref()
                .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
            else {
                continue;
            };
            let obs_kind = match envelope["kind"].as_str() {
                Some("coverage-detail") => "diff-coverage",
                Some("test-detail") => "test-run",
                Some("analysis-detail") => "static-analysis",
                _ => continue,
            };
            if kind.is_some_and(|k| k != obs_kind) {
                continue;
            }
            let payload = envelope
                .get("payload")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            // Headline numeric + payload per the panel's per-kind convention:
            // coverage → derive THIS effort's diff from the run's ABSOLUTE
            // detail (skip when no overlap/baseline); static-analysis →
            // error+warning count; test-run → none (panel reads the payload;
            // report-less runs — no `total` — stay off the panel, as before).
            let (payload_json, metric_value) = match obs_kind {
                "diff-coverage" => {
                    let derived = match effort.as_ref() {
                        Some(eff) => self
                            .diff_coverage_for_effort(eff, &payload)
                            .await
                            .ok()
                            .flatten(),
                        None => None,
                    };
                    match derived {
                        Some((pct, diff_payload)) => (Some(diff_payload.to_string()), Some(pct)),
                        None => continue,
                    }
                }
                "static-analysis" => {
                    let mv = payload["errorCount"].as_f64().unwrap_or(0.0)
                        + payload["warningCount"].as_f64().unwrap_or(0.0);
                    (serde_json::to_string(&payload).ok(), Some(mv))
                }
                _ => {
                    if payload.get("total").is_none() {
                        continue;
                    }
                    (serde_json::to_string(&payload).ok(), None)
                }
            };
            out.push(oxplow_db::EffortObservation {
                id: c.id,
                stream_id: oxplow_domain::StreamId::new(c.stream_id).to_string(),
                effort_id: effort_id.to_string(),
                kind: obs_kind.to_string(),
                provenance: c.provenance.clone(),
                source: c.source.clone(),
                metric_value,
                payload_json,
                local_snapshot_id: c.snapshot_id,
                closest_git_version: c.closest_git_version.clone(),
                git_version_exact: c.git_version_exact,
                created_at: c.captured_at,
            });
        }
        out
    }

    /// Roll every metric up over a single effort for the task/effort page — the
    /// structured sibling of [`effort_metric_context`](Self::effort_metric_context)
    /// (which builds the agent-prompt text). Reads the spec catalog and, per
    /// family, aggregates the effort's own facts (epic tsk12, T-D; see metrics.md):
    /// - **per-file gauges** (`File`): Σ over the effort's *claimed* files
    ///   (`task_effort_file`) of `(current − baseline)` fact value — the slice this
    ///   effort actually moved, even on a branch shared with another effort. With
    ///   no claimed files it falls back to the repo-wide before→after.
    /// - **run + operational** (`Run`/`Window`): before→after (or `sum` flow) over
    ///   the facts of the effort's OWN captures (`metric_capture.effort_id`,
    ///   stamped at ingest — tsk37), so overlapping efforts stay disjoint.
    /// - **coverage** (`Coverage`): effort-relative diff derived at read (still on
    ///   the legacy detail payload — a documented special case).
    ///
    /// Returns only metrics the effort moved/touched, grouped code-health →
    /// coverage → tests → operational, then by title.
    pub async fn effort_metric_deltas(&self, effort_id: &str) -> Vec<oxplow_db::EffortMetricDelta> {
        let Some(eid) = EffortId::try_from_str(effort_id) else {
            return vec![];
        };
        let Ok(Some(effort)) = self.efforts.get_effort(&eid).await else {
            return vec![];
        };
        // Every metric is a spec now (built-in ∪ producer ∪ config); each read
        // aggregates the spec's source measure's facts (epic tsk12, T-D).
        let Ok(specs) = self.facts.list_specs().await else {
            return vec![];
        };
        // The effort's OWN captures (stamped `effort_id` at ingest, tsk37) — the
        // attribution spine for the run + operational families. File gauges are
        // snapshot scans (unstamped), so they read by claimed files × time below.
        // Full rows (not just ids): an EMPTY capture (a clean analysis run) is a
        // zero record the stamped read fills in (tsk44).
        let effort_caps = self
            .facts
            .captures_for_effort(effort.id.value())
            .await
            .unwrap_or_default();
        let claimed: Vec<String> = self
            .efforts
            .list_files(&eid)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|f| f.path)
            .collect();
        // The effort's stream (via its thread): gauge captures are per-worktree
        // scans, so the File family must not read another stream's facts (tsk43).
        let stream = self
            .threads
            .get(&effort.thread_id)
            .await
            .ok()
            .flatten()
            .map(|t| t.stream_id.value());

        use crate::attribution::{classify_effort_attribution, EffortAttributionFamily};
        // Per-call memo: several File-family specs share one `source_measure`
        // (e.g. the count-over-threshold specs over `oxplow.complexity`), so
        // load each measure's full history once instead of once per spec
        // (tsk17). Rebuilt every call — no cross-call staleness.
        let mut fact_cache: std::collections::HashMap<
            i64,
            std::sync::Arc<Vec<oxplow_db::FactRow>>,
        > = std::collections::HashMap::new();
        let mut out: Vec<oxplow_db::EffortMetricDelta> = Vec::new();
        for spec in &specs {
            // One classifier (in `attribution.rs`, beside the write-side
            // `AttributionKind` each family maps to) decides the family; this match
            // is the only place each family's read computation is named (tsk274).
            let row = match classify_effort_attribution(spec) {
                EffortAttributionFamily::File => {
                    self.file_delta_from_facts(spec, &effort, &claimed, stream, &mut fact_cache)
                        .await
                }
                // Coverage stays effort-relative + on the legacy detail payload
                // (line-sets aren't in facts yet) — derive the diff at read via the
                // spec's legacy definition (tsk270, T-D scope guard).
                EffortAttributionFamily::Coverage => {
                    self.coverage_delta_for_spec(spec, &effort).await
                }
                // Run + operational read identically now: before→after / `sum` over
                // the facts of the effort's own captures (the tsk37 spine).
                EffortAttributionFamily::Run | EffortAttributionFamily::Window => {
                    self.effort_stamped_delta(spec, &effort_caps).await
                }
            };
            if let Some(row) = row {
                out.push(row);
            }
        }
        out.sort_by(|a, b| {
            effort_metric_group_order(a)
                .cmp(&effort_metric_group_order(b))
                .then_with(|| a.title.cmp(&b.title))
        });
        out
    }

    /// Per-file attribution for a code gauge, over facts: Σ over the effort's
    /// claimed files of the file's `(current − baseline)` value, treating a file
    /// absent from a capture as 0 (sparse emission — how a drop-to-zero is
    /// detected). Facts are scoped to the effort's `stream` (gauge captures are
    /// per-worktree scans — another stream's values must not pollute the delta,
    /// tsk43). Baseline capture = the latest before the effort started; current
    /// = the latest at/before the effort end (the newest when open; a capture
    /// STAMPED with this effort — an on-effort-complete gauge landing just after
    /// the close — also counts). A closed effort with no capture in its window
    /// yields `None` — never a post-close capture's repo changes. `None` when
    /// the measure is unknown or nothing moved.
    async fn file_delta_from_facts(
        &self,
        spec: &oxplow_db::MetricSpec,
        effort: &TaskEffort,
        claimed: &[String],
        stream: Option<i64>,
        fact_cache: &mut std::collections::HashMap<i64, std::sync::Arc<Vec<oxplow_db::FactRow>>>,
    ) -> Option<oxplow_db::EffortMetricDelta> {
        let measure_key = spec.source_measure.as_deref()?;
        let measure = self.facts.get_measure(measure_key).await.ok().flatten()?;
        let filter = spec_fact_filter(spec).ok()?;
        // The spec's aggregation decides what one kept fact contributes: a
        // `count` spec counts offenders (each fact = 1) — summing their raw
        // values would report Σ complexity where the Metrics page counts
        // functions, and feed a value-sum into count-calibrated thresholds.
        // Everything else keeps the Σ-of-values read (correct for the per-file
        // `sum` gauges; the min/avg-style aggregations have no meaningful
        // per-file Σ and don't reach the File family today).
        let agg = crate::metric_engine::Aggregation::parse(&spec.aggregation)?;
        let contribution = |f: &&oxplow_db::FactRow| -> f64 {
            match agg {
                crate::metric_engine::Aggregation::Count => 1.0,
                _ => f.value,
            }
        };
        // Load this measure's full history once per `effort_metric_deltas` call
        // and reuse it across every spec sharing the measure (tsk17).
        let facts = match fact_cache.get(&measure.id) {
            Some(cached) => cached.clone(),
            None => {
                let loaded =
                    std::sync::Arc::new(self.facts.facts_for_measure(measure.id).await.ok()?);
                fact_cache.insert(measure.id, loaded.clone());
                loaded
            }
        };
        let kept: Vec<&oxplow_db::FactRow> = facts
            .iter()
            .filter(|f| filter.matches(f))
            .filter(|f| stream.map_or(true, |s| f.stream_id == s))
            .collect();
        if kept.is_empty() {
            return None;
        }
        // Distinct captures in time-ascending order (facts arrive oldest-first),
        // plus which of them this effort stamped (on-effort-complete gauges).
        let mut caps: Vec<(i64, oxplow_domain::Timestamp)> = Vec::new();
        let mut stamped: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for f in &kept {
            if !caps.iter().any(|(id, _)| *id == f.capture_id) {
                caps.push((f.capture_id, f.captured_at));
            }
            if f.effort_id == Some(effort.id.value()) {
                stamped.insert(f.capture_id);
            }
        }
        // Splice in the producers' remaining captures — including EMPTY zero-hit
        // scans (tsk44): "scanned, found nothing" must be eligible as the
        // baseline/current capture, or a drop-to-zero during the effort is
        // invisible (every kept fact predates it). Stream-scoped like the facts.
        let producers: std::collections::BTreeSet<String> =
            kept.iter().map(|f| f.producer.clone()).collect();
        if let Ok(all_caps) = self
            .facts
            .captures_for_producers(producers.into_iter().collect())
            .await
        {
            for c in all_caps {
                if stream.map_or(true, |s| c.stream_id == s)
                    && !caps.iter().any(|(id, _)| *id == c.id)
                {
                    caps.push((c.id, c.captured_at));
                    if c.effort_id == Some(effort.id.value()) {
                        stamped.insert(c.id);
                    }
                }
            }
            caps.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        }
        let baseline_cap = caps
            .iter()
            .rev()
            .find(|(_, at)| *at < effort.started_at)
            .map(|(id, _)| *id);
        let current_cap = caps
            .iter()
            .rev()
            .find(|(id, at)| match effort.ended_at {
                Some(end) => *at <= end || stamped.contains(id),
                None => true,
            })
            .map(|(id, _)| *id);
        // A closed effort with no capture in (or stamped into) its window has
        // nothing attributable — never fabricate a drop-to-zero against a
        // pre-effort baseline, and never read a post-close capture (tsk43).
        current_cap?;
        // A claimed file's value in a capture: the kept facts on that path,
        // combined per the spec's aggregation (count ⇒ each fact is 1), else 0.
        let file_value = |cap: Option<i64>, path: &str| -> f64 {
            cap.map(|c| {
                kept.iter()
                    .filter(|f| f.capture_id == c && f.path.as_deref() == Some(path))
                    .map(contribution)
                    .sum()
            })
            .unwrap_or(0.0)
        };
        // Repo total in a capture = every kept fact in it, same aggregation (the
        // crossing badge reflects the repo total, not the claimed-file slice).
        let repo_total = |cap: Option<i64>| -> f64 {
            cap.map(|c| {
                kept.iter()
                    .filter(|f| f.capture_id == c)
                    .map(contribution)
                    .sum()
            })
            .unwrap_or(0.0)
        };
        let crossing = threshold_state(
            &spec.direction,
            repo_total(current_cap),
            spec.warn_at,
            spec.fail_at,
        )
        .map(str::to_string);

        // Per-file attribution needs path-grained facts. A repo-scalar gauge
        // (facts with no path) sums 0/0 over the claimed paths and would
        // silently drop the row — its movement is the repo-wide window (tsk43).
        let path_grained = kept.iter().any(|f| f.path.is_some());
        if claimed.is_empty() || !path_grained {
            // No claimed files (an early effort) or no per-file grain → the
            // repo-wide before→after, so the movement still surfaces.
            let baseline = repo_total(baseline_cap);
            let current = repo_total(current_cap);
            if baseline == 0.0 && current == 0.0 {
                return None;
            }
            let changed = (current - baseline).abs() > f64::EPSILON;
            return Some(effort_delta_row_spec(
                spec,
                DeltaCalc {
                    agg: "level",
                    baseline: Some(baseline),
                    current,
                    delta: changed.then_some(current - baseline),
                    changed,
                    attributed_files: None,
                    sample_count: kept.len() as i64,
                    latest_run_id: current_cap,
                    crossing,
                },
            ));
        }

        let mut baseline = 0.0;
        let mut current = 0.0;
        let mut attributed = 0i64;
        for p in claimed {
            let b = file_value(baseline_cap, p);
            let c = file_value(current_cap, p);
            if b != 0.0 || c != 0.0 {
                attributed += 1;
            }
            baseline += b;
            current += c;
        }
        // The effort's files never carried this metric → nothing to show.
        if baseline == 0.0 && current == 0.0 {
            return None;
        }
        let changed = (current - baseline).abs() > f64::EPSILON;
        Some(effort_delta_row_spec(
            spec,
            DeltaCalc {
                agg: "files",
                baseline: Some(baseline),
                current,
                delta: changed.then_some(current - baseline),
                changed,
                attributed_files: Some(attributed),
                sample_count: kept.len() as i64,
                latest_run_id: current_cap,
                crossing,
            },
        ))
    }

    /// Before→after (or `sum` flow) for a run/operational metric over the facts of
    /// the effort's OWN captures (`metric_capture.effort_id`, stamped at ingest —
    /// tsk37). One series point per capture (within-capture aggregation is the
    /// spec's); `sum`-aggregation specs (token/turn/nudge flows) are summed across
    /// captures, everything else is first→last. A count/sum spec's EMPTY effort
    /// capture (a clean analysis run — tsk44) reads as an explicit 0 point, so
    /// "3 errors → 0" shows in the panel. `None` when the effort has no
    /// captures carrying (or zero-recording) this metric's facts.
    async fn effort_stamped_delta(
        &self,
        spec: &oxplow_db::MetricSpec,
        effort_caps: &[oxplow_db::MetricCapture],
    ) -> Option<oxplow_db::EffortMetricDelta> {
        if effort_caps.is_empty() {
            return None;
        }
        let measure_key = spec.source_measure.as_deref()?;
        let measure = self.facts.get_measure(measure_key).await.ok().flatten()?;
        let agg = crate::metric_engine::Aggregation::parse(&spec.aggregation)?;
        let filter = spec_fact_filter(spec).ok()?;
        let cap_ids: Vec<i64> = effort_caps.iter().map(|c| c.id).collect();
        let facts = self
            .facts
            .facts_for_captures(measure.id, cap_ids)
            .await
            .ok()?;
        let mut series = crate::metric_engine::aggregate_series(&facts, agg, &filter, None);
        if matches!(
            agg,
            crate::metric_engine::Aggregation::Count | crate::metric_engine::Aggregation::Sum
        ) {
            // Which producers emit this metric's slice — from the effort's own
            // kept facts, else the measure's global facts (the effort whose only
            // run was clean). An effort capture from those producers that
            // aggregated to no point is an explicit zero.
            let mut producers: std::collections::BTreeSet<String> = facts
                .iter()
                .filter(|f| filter.matches(f))
                .map(|f| f.producer.clone())
                .collect();
            if producers.is_empty() {
                if let Ok(all) = self.facts.facts_for_measure(measure.id).await {
                    producers = all
                        .iter()
                        .filter(|f| filter.matches(f))
                        .map(|f| f.producer.clone())
                        .collect();
                }
            }
            let have: std::collections::HashSet<i64> =
                series.iter().map(|p| p.capture_id).collect();
            for c in effort_caps {
                if producers.contains(&c.producer) && !have.contains(&c.id) {
                    series.push(crate::metric_engine::SeriesPoint {
                        capture_id: c.id,
                        captured_at: c.captured_at,
                        value: 0.0,
                        numerator: None,
                        denominator: None,
                        group: None,
                        branch: c.branch.clone(),
                        provenance: Some(c.provenance.clone()),
                        git_version: c.closest_git_version.clone(),
                        source: Some(c.source.clone()),
                    });
                }
            }
            series.sort_by(|a, b| {
                a.captured_at
                    .cmp(&b.captured_at)
                    .then(a.capture_id.cmp(&b.capture_id))
            });
        }
        let (first, last) = (series.first()?, series.last()?);
        let latest = Some(last.capture_id);
        if spec.aggregation == "sum" {
            let total: f64 = series.iter().map(|p| p.value).sum();
            if total == 0.0 {
                return None;
            }
            let crossing = threshold_state(&spec.direction, total, spec.warn_at, spec.fail_at)
                .map(str::to_string);
            return Some(effort_delta_row_spec(
                spec,
                DeltaCalc {
                    agg: "sum",
                    baseline: None,
                    current: total,
                    delta: Some(total),
                    changed: true,
                    attributed_files: None,
                    sample_count: facts.len() as i64,
                    latest_run_id: latest,
                    crossing,
                },
            ));
        }
        let baseline = first.value;
        let current = last.value;
        let changed = (current - baseline).abs() > f64::EPSILON;
        let crossing = threshold_state(&spec.direction, current, spec.warn_at, spec.fail_at)
            .map(str::to_string);
        Some(effort_delta_row_spec(
            spec,
            DeltaCalc {
                agg: "level",
                baseline: Some(baseline),
                current,
                delta: changed.then_some(current - baseline),
                changed,
                attributed_files: None,
                sample_count: facts.len() as i64,
                latest_run_id: latest,
                crossing,
            },
        ))
    }

    /// Coverage effort-delta (the `Coverage` family, tsk270): coverage is
    /// **observe-always** (absolute) AND **effort-relative** (diff vs the
    /// effort's start snapshot), so neither the time window nor a stored value
    /// is right. For each coverage run CAPTURE this effort claimed (ledger —
    /// the capture is the run, T-E1), derive its diff-coverage from the
    /// capture's ABSOLUTE per-file line-sets (`detail_json`) at read, then
    /// before→after over the derived sequence.
    async fn coverage_delta_for_spec(
        &self,
        spec: &oxplow_db::MetricSpec,
        effort: &TaskEffort,
    ) -> Option<oxplow_db::EffortMetricDelta> {
        let mut caps: Vec<oxplow_db::MetricCapture> = Vec::new();
        for id in self
            .attribution
            .list_refs(&effort.id, "run", STATE_CLAIMED)
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|r| r.strip_prefix("run:").and_then(|s| s.parse::<i64>().ok()))
        {
            if let Ok(Some(c)) = self.facts.get_capture(id).await {
                caps.push(c);
            }
        }
        caps.sort_by(|a, b| a.captured_at.cmp(&b.captured_at).then(a.id.cmp(&b.id)));
        let mut derived: Vec<f64> = Vec::new();
        let mut latest_cap = None;
        for c in &caps {
            let payload = c
                .detail_json
                .as_deref()
                .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                .filter(|env| env["kind"].as_str() == Some("coverage-detail"))
                .and_then(|env| env.get("payload").cloned());
            if let Some(abs) = payload {
                if let Ok(Some((pct, _))) = self.diff_coverage_for_effort(effort, &abs).await {
                    derived.push(pct);
                    latest_cap = Some(c.id);
                }
            }
        }
        let (first, last) = (derived.first()?, derived.last()?);
        let (baseline, current) = (*first, *last);
        let changed = (current - baseline).abs() > f64::EPSILON;
        let crossing = threshold_state(&spec.direction, current, spec.warn_at, spec.fail_at)
            .map(str::to_string);
        Some(effort_delta_row_spec(
            spec,
            DeltaCalc {
                agg: "level",
                baseline: Some(baseline),
                current,
                delta: changed.then_some(current - baseline),
                changed,
                attributed_files: None,
                sample_count: derived.len() as i64,
                latest_run_id: latest_cap,
                crossing,
            },
        ))
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
            "Tests ran (`{cmd}`) but produced no report, so this run won't appear in the \
             effort's Tests panel — only report-emitting runs do. Run EVERY test invocation \
             (including failing/red-phase and single-test runs, not just the final green one) \
             via `{tc}` in the foreground so they all show."
        )
    } else if !cfg.reports.is_empty() {
        format!(
            "Tests ran (`{cmd}`) but refreshed none of the configured collection reports, so \
             this effort has no parsed tests/coverage. Re-run via the command that regenerates \
             them and set `collection.testCommand` in .oxplow/project.yaml to make it one step."
        )
    } else {
        format!(
            "Tests ran (`{cmd}`) but this project has no collection profile, so oxplow can't \
             attribute tests/coverage to the effort. Run /oxplow:configure to wire this stack's \
             report(s)."
        )
    }
}

/// Effort-panel group ordering: code-health gauges first, then coverage, tests,
/// static-analysis, operational. Drives the grouped rendering on the task page.
fn effort_metric_group_order(d: &oxplow_db::EffortMetricDelta) -> u8 {
    match d.category.as_deref() {
        Some("coverage") => 1,
        Some("testing") => 2,
        Some("static-quality") => 3,
        Some("operational") => 4,
        _ => 0,
    }
}

/// The computed half of an [`oxplow_db::EffortMetricDelta`] — the per-family
/// numbers, joined with the spec's metadata by [`effort_delta_row_spec`].
struct DeltaCalc {
    agg: &'static str,
    baseline: Option<f64>,
    current: f64,
    delta: Option<f64>,
    changed: bool,
    attributed_files: Option<i64>,
    sample_count: i64,
    latest_run_id: Option<i64>,
    crossing: Option<String>,
}

/// Build an effort-metric row from a metric SPEC + the computed `DeltaCalc`.
/// `kind` ← the spec's `display_kind`; the `latest_run_id` field carries a
/// capture id (epic tsk12, T-D/T-E1).
fn effort_delta_row_spec(
    spec: &oxplow_db::MetricSpec,
    c: DeltaCalc,
) -> oxplow_db::EffortMetricDelta {
    oxplow_db::EffortMetricDelta {
        key: spec.key.clone(),
        title: spec.title.clone(),
        unit: spec.unit.clone(),
        direction: spec.direction.clone(),
        kind: spec.display_kind.clone(),
        category: spec.category.clone(),
        language: spec.language.clone(),
        agg: c.agg.to_string(),
        baseline: c.baseline,
        current: c.current,
        delta: c.delta,
        changed: c.changed,
        attributed_files: c.attributed_files,
        sample_count: c.sample_count,
        target: spec.target,
        warn_at: spec.warn_at,
        fail_at: spec.fail_at,
        crossing: c.crossing,
        latest_run_id: c.latest_run_id,
    }
}

/// A spec's `filter_json` as a [`FactFilter`](crate::metric_engine::FactFilter)
/// (the empty filter when absent) — the effort-read counterpart of the engine's
/// private `spec_filter`. A malformed predicate is surfaced, never ignored.
fn spec_fact_filter(
    spec: &oxplow_db::MetricSpec,
) -> Result<crate::metric_engine::FactFilter, DomainError> {
    match spec.filter_json.as_deref() {
        Some(j) => crate::metric_engine::FactFilter::from_json(j),
        None => Ok(crate::metric_engine::FactFilter::default()),
    }
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

/// How far back a report's mtime can be and still count as "fresh" for passive
/// ingestion (tsk269). Replaces the effort-start floor so collection works at
/// 0/N open efforts (observe-always). A report regenerated by the command that
/// just ran is seconds old; stale reports from prior runs/other stacks fall
/// outside the window and are skipped — same router/anti-replay intent as the
/// old effort-start floor, minus the effort dependency. 10 minutes is generous
/// for a slow suite that finishes writing well after its command started.
const REPORT_FRESH_WINDOW_MS: i64 = 10 * 60 * 1000;

/// Build the ABSOLUTE coverage payload from a report (tsk270): whole-report %
/// plus per-file instrumented/covered line-sets, stored verbatim so the
/// effort-relative diff can be derived later at read. Returns
/// `(abs_pct, covered, instrumented, payload)`; `None` when nothing is
/// instrumented.
fn coverage_abs_payload(
    report: &oxplow_coverage::CoverageReport,
) -> Option<(f64, usize, usize, serde_json::Value)> {
    let mut total_instr = 0usize;
    let mut total_cov = 0usize;
    let mut files_payload = Vec::new();
    for (path, fc) in &report.files {
        if fc.instrumented.is_empty() {
            continue;
        }
        let covered_instr: BTreeSet<u32> =
            fc.covered.intersection(&fc.instrumented).copied().collect();
        total_instr += fc.instrumented.len();
        total_cov += covered_instr.len();
        files_payload.push(json!({
            "path": path,
            "instrumented": fc.instrumented.iter().copied().collect::<Vec<u32>>(),
            "covered": covered_instr.iter().copied().collect::<Vec<u32>>(),
        }));
    }
    if total_instr == 0 {
        return None;
    }
    let abs_pct = (total_cov as f64 / total_instr as f64) * 100.0;
    Some((
        abs_pct,
        total_cov,
        total_instr,
        json!({ "absPct": abs_pct, "files": files_payload }),
    ))
}

/// The freshness floor: reports touched at/before this are stale. `now` minus
/// the window. (Passed where the effort-start `Timestamp` used to be.)
fn report_fresh_floor() -> oxplow_domain::Timestamp {
    oxplow_domain::Timestamp::from_unix_ms(
        oxplow_domain::Timestamp::now().unix_ms() - REPORT_FRESH_WINDOW_MS,
    )
}

/// True when `path`'s mtime is at/before `floor` — i.e. NOT freshly regenerated.
fn report_is_stale(path: &std::path::Path, floor: oxplow_domain::Timestamp) -> bool {
    let Ok(mtime) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return false;
    };
    let Ok(since) = mtime.duration_since(std::time::UNIX_EPOCH) else {
        return false;
    };
    (since.as_millis() as i64) <= floor.unix_ms()
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
    fn bounded_set_dedups_and_evicts_oldest() {
        let mut s = BoundedSet::new(2);
        assert!(s.insert(1)); // new
        assert!(!s.insert(1)); // already present
        assert!(s.insert(2));
        assert!(s.contains(&1) && s.contains(&2));
        // Inserting a third evicts the oldest (1).
        assert!(s.insert(3));
        assert!(!s.contains(&1), "oldest entry evicted past cap");
        assert!(s.contains(&2) && s.contains(&3));
        // 1 is forgotten, so it inserts fresh again (re-arms).
        assert!(s.insert(1));
    }

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
    fn report_nudge_has_no_repo_specific_context_path() {
        // The nudge ships in oxplow-app and fires in every downstream
        // project — it must never point at this repo's own `.context/`
        // docs, which don't exist in a user's project.
        let with_cmd = oxplow_config::CollectionConfig {
            test_command: Some("bun run test:collect".into()),
            ..Default::default()
        };
        let with_reports = oxplow_config::CollectionConfig {
            reports: vec![oxplow_config::ReportConfig {
                path: "coverage/lcov.info".into(),
                format: "lcov".into(),
            }],
            ..Default::default()
        };
        let no_profile = oxplow_config::CollectionConfig::default();
        for cfg in [&with_cmd, &with_reports, &no_profile] {
            let msg = report_nudge_message(cfg, "bun test");
            assert!(!msg.contains(".context/"), "leaked repo path: {msg}");
        }
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
    fn detect_test_run_ignores_read_only_commands_that_only_mention_a_pattern() {
        let extra = ["test:collect".to_string()];
        // grep/echo/cat that merely MENTION a pattern are not runs.
        assert!(!detect_test_run(
            "grep -n test:collect .oxplow/project.yaml",
            &extra
        ));
        assert!(!detect_test_run("echo run cargo test later", &[]));
        assert!(!detect_test_run("cat notes | grep nextest", &[]));
        // The real command still detects — compound, env-prefixed, and piped.
        assert!(detect_test_run("cd app && bun run test:collect", &extra));
        assert!(detect_test_run(
            "OXPLOW_TASK=tsk42 bun run test:collect 2>&1 | tail -5",
            &extra,
        ));
    }

    #[test]
    fn parse_task_token_reads_oxplow_task_prefix() {
        assert_eq!(
            parse_task_token("OXPLOW_TASK=tsk42 bun run test:collect"),
            Some(TaskId::new(42)),
        );
        // Bare number is accepted too.
        assert_eq!(
            parse_task_token("OXPLOW_TASK=42 cargo test"),
            Some(TaskId::new(42))
        );
        // Absent → None (falls back to the single-open auto rule).
        assert_eq!(parse_task_token("bun run test:collect"), None);
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
            /// Shared in-memory db handle — lets a test seed a second task/effort
            /// (e.g. the overlapping-efforts disentangle case).
            db: Database,
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
                Arc::new(oxplow_db::SqliteFactStore::new(db.clone())),
                nudges.clone(),
                efforts.clone(),
                Arc::new(SqliteThreadStore::new(db.clone())),
                snapshots,
                blobs,
                Arc::new(RwLock::new(cfg)),
                project_dir,
                EventBus::new(),
                Arc::new(oxplow_db::SqliteAttributionStore::new(db.clone())),
            );
            Harness {
                service,
                thread: thread.id,
                effort_id: effort.id.to_string(),
                efforts,
                nudges,
                tmp,
                db,
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
            // tsk270: ingest records ABSOLUTE coverage (instruments {1,2,4},
            // covers {1,2} → 2/3 ≈ 66.7%); the effort-relative DIFF (changed∩instr
            // = {2,4}, covered {2} → 50%, line 4 uncovered) is derived at READ.
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
                    assert_eq!(changed_lines, 3, "absolute instrumented");
                    assert_eq!(covered_lines, 2, "absolute covered");
                    assert!((summary_pct - 66.666).abs() < 0.01, "abs got {summary_pct}");
                }
                other => panic!("expected Stored, got {other:?}"),
            }
            // The DIFF is derived at read against the effort's changed lines.
            let rows = h
                .service
                .list_for_effort(&h.effort_id, Some("diff-coverage"))
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].provenance, "observed");
            assert!(
                (rows[0].metric_value.unwrap() - 50.0).abs() < 1e-6,
                "derived diff %"
            );
            let payload = rows[0].payload_json.as_deref().unwrap();
            let cov: DiffCovPayload = serde_json::from_str(payload).expect("payload parses");
            let foo = cov.files.iter().find(|f| f.path == "src/foo.rs").unwrap();
            assert_eq!(foo.uncovered, vec![4]);
        }

        #[tokio::test]
        async fn coverage_diff_is_unattributed_under_concurrency_then_claimable() {
            // tsk270: with two open efforts, a coverage run is observed but NOT
            // auto-attributed (no pollution) — neither effort's panel shows it
            // until the agent claims it. After a claim, the diff is DERIVED at
            // read for the claiming effort (late-claim works even post-close).
            use oxplow_db::SqliteAttributionStore;
            let h = build(Some(COBERTURA_50PCT)).await;
            let now = Timestamp::now();
            let eid1 = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            // Open a second effort so the thread is ambiguous.
            let task2 = SqliteTaskStore::new(h.db.clone())
                .insert(&Task {
                    id: TaskId::placeholder(),
                    thread_id: Some(h.thread),
                    parent_id: None,
                    title: "t2".into(),
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
            let _eff2 = h.efforts.start(task2, &h.thread, None).await.unwrap();

            // Observe coverage — two efforts open ⇒ unclaimed.
            assert!(matches!(
                h.service
                    .ingest_coverage(&h.thread, None, None, false)
                    .await
                    .unwrap(),
                CoverageIngest::Stored { .. }
            ));
            // No pollution: neither effort shows a diff-coverage observation yet.
            assert!(h
                .service
                .list_for_effort(&h.effort_id, Some("diff-coverage"))
                .await
                .unwrap()
                .is_empty());

            // The agent claims the run for eff1 (late-claim is the same path).
            let ledger = SqliteAttributionStore::new(h.db.clone());
            let runs = oxplow_db::SqliteFactStore::new(h.db.clone())
                .captures_in_window_by_trigger(
                    h.thread.value(),
                    "on-report",
                    Timestamp::from_unix_ms(0),
                    None,
                )
                .await
                .unwrap();
            let run_ref = format!("run:{}", runs[0].id);
            ledger
                .set_state(&eid1, "run", &run_ref, STATE_CLAIMED, None)
                .await
                .unwrap();

            // Now eff1's diff-coverage is derived at read (50%, line 4 uncovered).
            let rows = h
                .service
                .list_for_effort(&h.effort_id, Some("diff-coverage"))
                .await
                .unwrap();
            assert_eq!(rows.len(), 1, "claimed run now surfaces a derived diff");
            assert!((rows[0].metric_value.unwrap() - 50.0).abs() < 1e-6);
        }

        #[tokio::test]
        async fn ingest_coverage_mirrors_into_metric_substrate() {
            // git_init = true so a branch is present to capture.
            let h = build_full(Some(COBERTURA_50PCT), true, &[]).await;
            h.service
                .ingest_coverage(&h.thread, None, None, false)
                .await
                .unwrap();

            // The durable fact layer (epic tsk12; the legacy sample write is
            // gone, T-E2): one `oxplow.coverage` fact for the report's single
            // file, carrying the covered/instrumented counts so a module/repo
            // roll-up re-derives Σcovered/Σinstrumented rather than averaging
            // percentages. The capture carries the observed spine.
            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let measure = facts
                .get_measure("oxplow.coverage")
                .await
                .unwrap()
                .expect("coverage measure seeded by V43");
            let cov = facts.facts_for_measure(measure.id).await.unwrap();
            assert_eq!(cov.len(), 1, "one coverage fact per file");
            assert!(
                (cov[0].value - 66.666).abs() < 0.01,
                "value {}",
                cov[0].value
            );
            assert_eq!(cov[0].numerator, Some(2.0));
            assert_eq!(cov[0].denominator, Some(3.0));
            assert_eq!(cov[0].subject_kind.as_deref(), Some("file"));
            assert!(
                cov[0].subject_ref.as_deref().unwrap().starts_with("file:"),
                "subject_ref is file:<path>, got {:?}",
                cov[0].subject_ref
            );
            assert!(cov[0].branch.is_some(), "fact inherits the capture branch");
            assert_eq!(cov[0].provenance, "observed");
            assert_eq!(cov[0].source, "coverage-report");
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
                    None,
                )
                .await
                .unwrap();
            // The run CAPTURE carries the counts in its detail envelope (T-E2:
            // the legacy count samples are gone; asserted counts also become
            // status-sliced facts — see asserted_counts_without_report_…).
            let caps = oxplow_db::SqliteFactStore::new(h.db.clone())
                .captures_in_window_by_trigger(
                    h.thread.value(),
                    "on-report",
                    Timestamp::from_unix_ms(0),
                    None,
                )
                .await
                .unwrap();
            assert_eq!(caps.len(), 1, "one run capture");
            assert_eq!(caps[0].provenance, "observed");
            // Stamped with the stream's current snapshot (the code state) so the
            // panel can group runs into exact iterations (tsk259).
            assert!(
                caps[0].snapshot_id.is_some(),
                "the run capture carries the stream's current snapshot"
            );
            let envelope: serde_json::Value =
                serde_json::from_str(caps[0].detail_json.as_deref().unwrap()).unwrap();
            assert_eq!(envelope["kind"], "test-detail");
            assert_eq!(envelope["payload"]["passed"], 5);
            assert_eq!(envelope["payload"]["failed"], 1);
            assert_eq!(envelope["payload"]["total"], 6);
        }

        #[tokio::test]
        async fn report_less_test_run_records_a_run_capture_but_not_a_tests_zero() {
            // A report-less, count-less run (a bare `cargo test` the hook saw) is
            // a run RECORD, not a measurement — it must not read as "suite ran,
            // found 0 tests" and zero the semi-additive oxplow.tests.* timeline
            // via the tsk44 zero-fill.
            use oxplow_coverage::{TestCase, TestReport, TestStatus, TestSuite};
            let h = build(None).await;
            let report = TestReport {
                suites: vec![TestSuite {
                    name: "s".into(),
                    cases: vec![TestCase {
                        classname: "m".into(),
                        name: "t1".into(),
                        status: TestStatus::Passed,
                        time_ms: None,
                    }],
                }],
            };
            h.service
                .record_test_run(
                    &h.thread,
                    "bun run test:collect",
                    Some(0),
                    None,
                    None,
                    None,
                    None,
                    "observed",
                    "post-tool-bash",
                    Some(&report),
                    None,
                )
                .await
                .unwrap();
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
                    None,
                    None,
                )
                .await
                .unwrap();

            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let engine = crate::metric_engine::MetricEngine::new(facts.clone());
            let series = engine
                .series(
                    "oxplow.test_case",
                    crate::metric_engine::Aggregation::Count,
                    &Default::default(),
                    None,
                )
                .await
                .unwrap();
            assert_eq!(
                series.iter().map(|p| p.value).collect::<Vec<_>>(),
                vec![1.0],
                "the report-less run must not splice a value-0 point"
            );
            // The run record itself survives for the ledger/effort panel.
            let eid = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            let caps = facts.captures_for_effort(eid.value()).await.unwrap();
            let test_caps: Vec<_> = caps
                .iter()
                .filter(|c| {
                    c.detail_json
                        .as_deref()
                        .is_some_and(|d| d.contains("test-detail"))
                })
                .collect();
            assert_eq!(test_caps.len(), 2, "both runs recorded as captures");
        }

        #[tokio::test]
        async fn asserted_counts_without_report_become_status_sliced_facts() {
            // The MCP record_test_run path (a sub-agent's run): no report, but
            // real pass/fail counts — they must land as status-sliced facts so
            // the oxplow.tests.* specs read them, not ride only detail_json.
            let h = build(None).await;
            h.service
                .record_test_run(
                    &h.thread,
                    "cargo test -p sub",
                    None,
                    None,
                    Some(2),
                    Some(1),
                    Some(4),
                    "asserted",
                    "agent",
                    None,
                    None,
                )
                .await
                .unwrap();
            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let engine = crate::metric_engine::MetricEngine::new(facts.clone());
            let total = engine
                .series(
                    "oxplow.test_case",
                    crate::metric_engine::Aggregation::Count,
                    &Default::default(),
                    None,
                )
                .await
                .unwrap();
            assert_eq!(
                total.last().map(|p| p.value),
                Some(4.0),
                "2 passed + 1 failed + 1 skipped (total 4)"
            );
            let failed = engine
                .series(
                    "oxplow.test_case",
                    crate::metric_engine::Aggregation::Count,
                    &crate::metric_engine::FactFilter {
                        dim_eq: Some(("oxplow.status".into(), "failed".into())),
                        ..Default::default()
                    },
                    None,
                )
                .await
                .unwrap();
            assert_eq!(failed.last().map(|p| p.value), Some(1.0));
        }

        #[tokio::test]
        async fn record_test_run_returns_the_real_capture_id() {
            // The returned id is the capture id (the run identity the ledger
            // claims, T-E1) — not a placeholder 0.
            let h = build(None).await;
            let id = h
                .service
                .record_test_run(
                    &h.thread,
                    "cargo test --workspace",
                    Some(0),
                    None,
                    Some(3),
                    Some(0),
                    Some(3),
                    "observed",
                    "post-tool-bash",
                    None,
                    None,
                )
                .await
                .unwrap()
                .expect("a capture was recorded");
            let eid = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            let caps = oxplow_db::SqliteFactStore::new(h.db.clone())
                .captures_for_effort(eid.value())
                .await
                .unwrap();
            assert!(
                caps.iter().any(|c| c.id == id),
                "returned id {id} must be the recorded capture's id ({:?})",
                caps.iter().map(|c| c.id).collect::<Vec<_>>()
            );
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
                    None,
                )
                .await
                .unwrap();
            // The suite/case tree rides verbatim in the run CAPTURE's detail
            // envelope (T-E1/T-E2 — the legacy test-detail finding is gone).
            let caps = oxplow_db::SqliteFactStore::new(h.db.clone())
                .captures_in_window_by_trigger(
                    h.thread.value(),
                    "on-report",
                    Timestamp::from_unix_ms(0),
                    None,
                )
                .await
                .unwrap();
            assert_eq!(caps.len(), 1);
            let envelope: serde_json::Value =
                serde_json::from_str(caps[0].detail_json.as_deref().unwrap()).unwrap();
            assert_eq!(envelope["kind"], "test-detail");
            let payload = &envelope["payload"];
            assert_eq!(payload["suites"][0]["name"], "oxplow-app");
            assert_eq!(payload["suites"][0]["cases"][1]["status"], "failed");

            // The durable fact layer (epic tsk12): one `oxplow.test_case` fact
            // per case, status carried as the `oxplow.status` dim so Count()
            // sliced by status reconstructs the passed/failed headline.
            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let measure = facts
                .get_measure("oxplow.test_case")
                .await
                .unwrap()
                .expect("test_case measure seeded by V43");
            let cases = facts.facts_for_measure(measure.id).await.unwrap();
            assert_eq!(cases.len(), 2, "one fact per test case");
            assert!(cases.iter().all(|f| f.value == 1.0));
            let status_of = |sref: &str| -> String {
                cases
                    .iter()
                    .find(|f| f.subject_ref.as_deref() == Some(sref))
                    .and_then(|f| f.dims_json.as_deref())
                    .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                    .and_then(|v| v["oxplow.status"].as_str().map(String::from))
                    .unwrap_or_default()
            };
            assert_eq!(status_of("test:mod::t1"), "passed");
            assert_eq!(status_of("test:mod::t2"), "failed");

            // Keystone (T-B): the producer test specs re-aggregate these facts to
            // the baked counts through the engine — Count(oxplow.test_case) sliced
            // by status. The read-flip (tsk26) then serves them from the engine.
            for spec in crate::producer_metrics::builtin_producer_specs() {
                facts.upsert_spec(spec).await.unwrap();
            }
            let engine = crate::metric_engine::MetricEngine::new(facts.clone());
            for (key, expected) in [
                ("oxplow.tests.passed", 1.0),
                ("oxplow.tests.failed", 1.0),
                ("oxplow.tests.total", 2.0),
            ] {
                let spec = facts.get_spec(key).await.unwrap().unwrap();
                assert_eq!(
                    engine.headline_for_spec(&spec).await.unwrap(),
                    Some(expected),
                    "{key}: Count(oxplow.test_case) by status == baked count",
                );
            }
        }

        #[tokio::test]
        async fn test_headlines_report_the_latest_run_not_a_lifetime_sum() {
            // tsk42: `oxplow.test_case` is a SNAPSHOT of the suite state (a new
            // run replaces the previous one — semi-additive), so the tests.total
            // headline is the LATEST run's count, never the sum of every run
            // ever ("run a 100-test suite 10 times" must read 100, not 1000).
            use oxplow_coverage::{TestCase, TestReport, TestStatus, TestSuite};
            let h = build(None).await;
            let case = |name: &str, status: TestStatus| TestCase {
                classname: "mod".into(),
                name: name.into(),
                status,
                time_ms: None,
            };
            let report = |cases: Vec<TestCase>| TestReport {
                suites: vec![TestSuite {
                    name: "oxplow-app".into(),
                    cases,
                }],
            };
            for r in [
                report(vec![
                    case("t1", TestStatus::Passed),
                    case("t2", TestStatus::Passed),
                ]),
                report(vec![
                    case("t1", TestStatus::Passed),
                    case("t2", TestStatus::Failed),
                    case("t3", TestStatus::Passed),
                ]),
            ] {
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
                        Some(&r),
                        None,
                    )
                    .await
                    .unwrap();
            }

            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            for spec in crate::producer_metrics::builtin_producer_specs() {
                facts.upsert_spec(spec).await.unwrap();
            }
            let engine = crate::metric_engine::MetricEngine::new(facts.clone());
            for (key, expected) in [
                ("oxplow.tests.total", 3.0),
                ("oxplow.tests.passed", 2.0),
                ("oxplow.tests.failed", 1.0),
            ] {
                let spec = facts.get_spec(key).await.unwrap().unwrap();
                assert_eq!(
                    engine.headline_for_spec(&spec).await.unwrap(),
                    Some(expected),
                    "{key}: headline is the latest run's count, not a lifetime sum",
                );
            }
        }

        #[tokio::test]
        async fn record_test_run_stamps_test_case_capture_with_the_open_effort() {
            // tsk37: the on-report test producer stamps its fact-capture with the
            // owning effort (the harness opens one on the thread), so
            // `captures_for_effort` — the T-D fact-attribution read — attributes the
            // test facts. Same resolution the run auto-claim uses.
            use oxplow_coverage::{TestCase, TestReport, TestStatus, TestSuite};
            let h = build(None).await;
            let report = TestReport {
                suites: vec![TestSuite {
                    name: "oxplow-app".into(),
                    cases: vec![TestCase {
                        classname: "mod".into(),
                        name: "t1".into(),
                        status: TestStatus::Passed,
                        time_ms: Some(3),
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
                    None,
                )
                .await
                .unwrap();

            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let eid = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            let caps = facts.captures_for_effort(eid.value()).await.unwrap();
            let test_cap = caps
                .iter()
                .find(|c| c.producer == "tests")
                .expect("the test capture is attributed to the open effort");
            assert_eq!(test_cap.effort_id, Some(eid.value()));
            // Its facts are reachable through the fact-attribution read.
            let measure = facts
                .get_measure("oxplow.test_case")
                .await
                .unwrap()
                .unwrap();
            let scoped = facts
                .facts_for_captures(measure.id, vec![test_cap.id])
                .await
                .unwrap();
            assert_eq!(
                scoped.len(),
                1,
                "the one test case, attributed to the effort"
            );
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
                    None,
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
        /// Seed a headline gauge SPEC (+ its measure) and two captures straddling
        /// the effort start — a `baseline` before, `current` after — so with no
        /// claimed files the effort reads the repo-wide before→after (the `File`
        /// fallback). Operational `agent.*` keys get the operational category so
        /// they route to the `Window` family (no effort-stamped captures here →
        /// they no-op, which the skip-operational test relies on). Returns the
        /// measure id.
        async fn seed_gauge(
            h: &Harness,
            key: &str,
            direction: &str,
            warn_at: Option<f64>,
            fail_at: Option<f64>,
            baseline: f64,
            current: f64,
        ) -> i64 {
            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let measure_key = format!("{key}.m");
            let m = facts
                .upsert_measure(oxplow_db::NewMeasure::new(&measure_key, key))
                .await
                .unwrap();
            let mut s = oxplow_db::NewMetricSpec::base(key, "unsafe blocks", &measure_key, "sum");
            s.unit = Some("count".into());
            s.direction = direction.into();
            s.warn_at = warn_at;
            s.fail_at = fail_at;
            if crate::attribution::is_operational_metric_key(key) {
                s.display_kind = "event".into();
                s.category = Some("operational".into());
            } else {
                s.display_kind = "gauge".into();
                s.category = Some("custom".into());
            }
            facts.upsert_spec(s).await.unwrap();
            let start = effort_start(h, &h.effort_id).await;
            let before = Timestamp::from_unix_ms(start.unix_ms() - 60_000);
            let after = Timestamp::from_unix_ms(start.unix_ms() + 60_000);
            for (at, value) in [(before, baseline), (after, current)] {
                let mut cap = oxplow_db::NewMetricCapture::done(1, "test.gauge", "test");
                cap.captured_at = Some(at);
                cap.thread_id = Some(h.thread.value());
                facts
                    .record_facts(cap, vec![oxplow_db::NewFact::new(m, value)])
                    .await
                    .unwrap();
            }
            m
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

        // ---- effort_metric_deltas (tsk250) -------------------------------

        /// A per-file code-gauge SPEC (category custom → the `File` family) over a
        /// fresh measure, `sum` within a capture (per-file counts total the tree),
        /// semi-additive over time. Returns `(measure_id, fact_store)`; record its
        /// captures with [`seed_gauge_capture`].
        async fn seed_file_gauge(
            h: &Harness,
            key: &str,
            direction: &str,
            target: Option<f64>,
        ) -> (i64, oxplow_db::SqliteFactStore) {
            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let measure_key = format!("{key}.m");
            // Production parity (tsk43): the built-in code gauges seed a
            // path-grain measure (`subject_kind: file`) + a `static-quality`
            // spec — the classifier must still route them to the File family
            // (their snapshot-scan captures are never effort-stamped).
            let mut nm = oxplow_db::NewMeasure::new(&measure_key, key);
            nm.subject_kind = Some("file".into());
            let m = facts.upsert_measure(nm).await.unwrap();
            let mut s = oxplow_db::NewMetricSpec::base(key, key, &measure_key, "sum");
            s.unit = Some("count".into());
            s.direction = direction.into();
            s.display_kind = "findings".into();
            s.category = Some("static-quality".into());
            s.target = target;
            facts.upsert_spec(s).await.unwrap();
            (m, facts)
        }

        /// Record one gauge CAPTURE at `at` with sparse `file:<path>` facts (a file
        /// absent from a capture reads as 0), mimicking a snapshot scan. Gauge
        /// captures are NOT effort-stamped — the `File` family reads them by claimed
        /// files × time. Returns the capture id.
        async fn seed_gauge_capture(
            facts: &oxplow_db::SqliteFactStore,
            measure_id: i64,
            at: Timestamp,
            per_file: &[(&str, f64)],
        ) -> i64 {
            seed_gauge_capture_in_stream(facts, measure_id, 1, at, per_file).await
        }

        /// [`seed_gauge_capture`] against an explicit stream — the cross-worktree
        /// pollution fixture (tsk43).
        async fn seed_gauge_capture_in_stream(
            facts: &oxplow_db::SqliteFactStore,
            measure_id: i64,
            stream: i64,
            at: Timestamp,
            per_file: &[(&str, f64)],
        ) -> i64 {
            let mut cap = oxplow_db::NewMetricCapture::done(stream, "test.gauge", "test");
            cap.captured_at = Some(at);
            let rows: Vec<oxplow_db::NewFact> = per_file
                .iter()
                .map(|(path, v)| oxplow_db::NewFact {
                    subject_kind: Some("file".into()),
                    subject_ref: Some(format!("file:{path}")),
                    path: Some((*path).into()),
                    ..oxplow_db::NewFact::new(measure_id, *v)
                })
                .collect();
            facts.record_facts(cap, rows).await.unwrap()
        }

        async fn claim(h: &Harness, effort_id: &str, path: &str) {
            let eid = oxplow_domain::EffortId::try_from_str(effort_id).unwrap();
            h.efforts
                .record_file(
                    &eid,
                    path,
                    oxplow_db::EffortFileChange::Updated,
                    oxplow_db::FileRefVersion {
                        local_snapshot_id: 0,
                        closest_git_version: None,
                        git_version_exact: false,
                    },
                )
                .await
                .unwrap();
        }

        async fn effort_start(h: &Harness, effort_id: &str) -> Timestamp {
            let eid = oxplow_domain::EffortId::try_from_str(effort_id).unwrap();
            h.efforts
                .get_effort(&eid)
                .await
                .unwrap()
                .unwrap()
                .started_at
        }

        #[tokio::test]
        async fn effort_metric_deltas_attributes_gauge_by_claimed_files() {
            let h = build(None).await;
            let start = effort_start(&h, &h.effort_id).await;
            let before = Timestamp::from_unix_ms(start.unix_ms() - 60_000);
            let after = Timestamp::from_unix_ms(start.unix_ms() + 60_000);
            let (m, facts) =
                seed_file_gauge(&h, "oxplow.rust.unsafe_blocks", "lower-better", Some(0.0)).await;
            // The effort claims a.rs and b.rs. c.rs is changed elsewhere (NOT
            // claimed) — it must not leak into this effort's delta.
            claim(&h, &h.effort_id, "src/a.rs").await;
            claim(&h, &h.effort_id, "src/b.rs").await;
            // Baseline capture (before the effort): a.rs=2, c.rs=5.
            seed_gauge_capture(&facts, m, before, &[("src/a.rs", 2.0), ("src/c.rs", 5.0)]).await;
            // Current capture (during the effort): a.rs removed (absent ⇒ 0), b.rs=3
            // added, c.rs still 5.
            seed_gauge_capture(&facts, m, after, &[("src/b.rs", 3.0), ("src/c.rs", 5.0)]).await;

            let deltas = h.service.effort_metric_deltas(&h.effort_id).await;
            assert_eq!(deltas.len(), 1, "one metric touched the effort's files");
            let d = &deltas[0];
            assert_eq!(d.agg, "files");
            // a.rs 2→0, b.rs 0→3 ⇒ baseline 2, current 3, Δ +1. c.rs excluded.
            assert_eq!(d.baseline, Some(2.0));
            assert_eq!(d.current, 3.0);
            assert_eq!(d.delta, Some(1.0));
            assert!(d.changed);
            assert_eq!(d.attributed_files, Some(2));
        }

        #[tokio::test]
        async fn effort_metric_deltas_count_spec_counts_offenders_not_value_sum() {
            // A `count` spec (oxplow.high_complexity_fns' shape: count of facts
            // over a threshold) must COUNT offenders in the effort panel — not
            // sum their raw values, which contradicts the Metrics page and feeds
            // a value-sum into the count-calibrated crossing thresholds.
            let h = build(None).await;
            let start = effort_start(&h, &h.effort_id).await;
            let before = Timestamp::from_unix_ms(start.unix_ms() - 60_000);
            let after = Timestamp::from_unix_ms(start.unix_ms() + 60_000);
            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let mut nm = oxplow_db::NewMeasure::new("acme.cx", "cx");
            nm.subject_kind = Some("function".into());
            let m = facts.upsert_measure(nm).await.unwrap();
            let mut s =
                oxplow_db::NewMetricSpec::base("acme.hot_fns", "Hot fns", "acme.cx", "count");
            s.direction = "lower-better".into();
            s.display_kind = "findings".into();
            s.category = Some("static-quality".into());
            s.filter_json = Some("{\"min_value\":10.0}".into());
            s.warn_at = Some(3.0);
            s.fail_at = Some(6.0);
            facts.upsert_spec(s).await.unwrap();
            claim(&h, &h.effort_id, "src/a.rs").await;
            // Baseline: one offender (complexity 15). Current: two offenders
            // (12, 11) plus one function under the threshold (4).
            seed_gauge_capture(&facts, m, before, &[("src/a.rs", 15.0)]).await;
            seed_gauge_capture(
                &facts,
                m,
                after,
                &[("src/a.rs", 12.0), ("src/a.rs", 11.0), ("src/a.rs", 4.0)],
            )
            .await;

            let deltas = h.service.effort_metric_deltas(&h.effort_id).await;
            assert_eq!(deltas.len(), 1);
            let d = &deltas[0];
            // Offender COUNT 1 → 2 (Δ +1) — not Σ complexity 15 → 23 (Δ +8).
            assert_eq!(d.baseline, Some(1.0));
            assert_eq!(d.current, 2.0);
            assert_eq!(d.delta, Some(1.0));
            // The crossing badge reads the offender count (2 < warn 3 ⇒ none);
            // the value-sum (23) would spuriously cross the fail threshold.
            assert_eq!(d.crossing, None);
        }

        #[tokio::test]
        async fn effort_metric_deltas_disentangles_overlapping_efforts() {
            // Two efforts overlap in time on the same stream; each claims a
            // DIFFERENT file. The per-file attribution must keep their deltas
            // disjoint — the core concurrency guarantee.
            let h = build(None).await;
            let now = Timestamp::now();
            let task2 = SqliteTaskStore::new(h.db.clone())
                .insert(&Task {
                    id: TaskId::placeholder(),
                    thread_id: Some(h.thread),
                    parent_id: None,
                    title: "t2".into(),
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
            let eff2 = h.efforts.start(task2, &h.thread, None).await.unwrap();
            let eff2_id = eff2.id.to_string();

            let (m, facts) =
                seed_file_gauge(&h, "oxplow.rust.unsafe_blocks", "lower-better", Some(0.0)).await;
            claim(&h, &h.effort_id, "src/a.rs").await; // effort 1 → a.rs
            claim(&h, &eff2_id, "src/b.rs").await; // effort 2 → b.rs

            let s1 = effort_start(&h, &h.effort_id).await;
            let before = Timestamp::from_unix_ms(s1.unix_ms() - 60_000);
            let after = Timestamp::from_unix_ms(eff2.started_at.unix_ms() + 60_000);
            seed_gauge_capture(&facts, m, before, &[("src/a.rs", 2.0), ("src/b.rs", 4.0)]).await;
            seed_gauge_capture(&facts, m, after, &[("src/a.rs", 5.0), ("src/b.rs", 9.0)]).await;

            let d1 = h.service.effort_metric_deltas(&h.effort_id).await;
            assert_eq!(d1.len(), 1);
            assert_eq!(d1[0].baseline, Some(2.0)); // a.rs only
            assert_eq!(d1[0].current, 5.0);
            assert_eq!(d1[0].delta, Some(3.0));
            assert_eq!(d1[0].attributed_files, Some(1));

            let d2 = h.service.effort_metric_deltas(&eff2_id).await;
            assert_eq!(d2.len(), 1);
            assert_eq!(d2[0].baseline, Some(4.0)); // b.rs only
            assert_eq!(d2[0].current, 9.0);
            assert_eq!(d2[0].delta, Some(5.0));
            assert_eq!(d2[0].attributed_files, Some(1));
        }

        #[tokio::test]
        async fn effort_metric_deltas_ignores_other_streams_captures() {
            // tsk43: gauge captures are per-worktree scans. A LATER capture from
            // another stream (same repo-relative path, different worktree content)
            // must not become this effort's "current" — the effort reads only its
            // own stream's timeline.
            let h = build(None).await;
            let start = effort_start(&h, &h.effort_id).await;
            let before = Timestamp::from_unix_ms(start.unix_ms() - 60_000);
            let after = Timestamp::from_unix_ms(start.unix_ms() + 60_000);
            let later = Timestamp::from_unix_ms(start.unix_ms() + 120_000);
            let (m, facts) =
                seed_file_gauge(&h, "oxplow.rust.unsafe_blocks", "lower-better", None).await;
            claim(&h, &h.effort_id, "src/a.rs").await;
            seed_gauge_capture(&facts, m, before, &[("src/a.rs", 2.0)]).await;
            seed_gauge_capture(&facts, m, after, &[("src/a.rs", 3.0)]).await;
            // A second stream (worktree) whose scan covers the same path.
            let now = Timestamp::now();
            SqliteStreamStore::new(h.db.clone())
                .upsert(&Stream {
                    id: StreamId::new(2),
                    kind: StreamKind::Worktree,
                    title: "w".into(),
                    branch: "feat".into(),
                    branch_ref: "refs/heads/feat".into(),
                    branch_source: "main".into(),
                    worktree_path: "/tmp/other".into(),
                    working_pane: String::new(),
                    talking_pane: String::new(),
                    working_session_id: String::new(),
                    talking_session_id: String::new(),
                    custom_prompt: None,
                    created_at: now,
                    updated_at: now,
                    archived_at: None,
                })
                .await
                .unwrap();
            // Stream 2's worktree scans the same path — newer, different value.
            seed_gauge_capture_in_stream(&facts, m, 2, later, &[("src/a.rs", 100.0)]).await;

            let deltas = h.service.effort_metric_deltas(&h.effort_id).await;
            assert_eq!(deltas.len(), 1);
            assert_eq!(deltas[0].baseline, Some(2.0));
            assert_eq!(
                deltas[0].current, 3.0,
                "stream 2's later capture must not pollute stream 1's delta"
            );
        }

        #[tokio::test]
        async fn effort_metric_deltas_skips_closed_effort_with_only_post_close_captures() {
            // tsk43: a CLOSED effort whose window contains no gauge capture gets
            // no row — never a post-close capture's repo changes (the old
            // `.or_else(caps.last())` fallback), and never a fabricated
            // drop-to-zero against a pre-effort baseline.
            let h = build(None).await;
            let (m, facts) =
                seed_file_gauge(&h, "oxplow.rust.unsafe_blocks", "lower-better", None).await;
            claim(&h, &h.effort_id, "src/a.rs").await;
            // The effort closes with no capture at all in its window (e.g. the
            // gauge was first enabled after the close)…
            let eid = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            h.efforts.finish(&eid, None, None).await.unwrap();
            let end = h
                .efforts
                .get_effort(&eid)
                .await
                .unwrap()
                .unwrap()
                .ended_at
                .unwrap();
            // …and the only later capture lands AFTER the close.
            let post = Timestamp::from_unix_ms(end.unix_ms() + 60_000);
            seed_gauge_capture(&facts, m, post, &[("src/a.rs", 9.0)]).await;

            let deltas = h.service.effort_metric_deltas(&h.effort_id).await;
            assert!(
                deltas.is_empty(),
                "no in-window capture → no row, not a post-close delta: {deltas:?}"
            );
        }

        #[tokio::test]
        async fn effort_metric_deltas_attributes_analysis_by_own_capture() {
            // tsk37/T-D: analysis is a run-kind fact attributed by the CAPTURE's
            // stamped `effort_id` (set at ingest when the owning effort resolves),
            // NOT a time window and NOT claimed files. Under two overlapping
            // efforts, a capture stamped to e1 shows on e1 and must NOT pollute the
            // concurrent e2's analysis delta.
            let h = build(None).await;
            let now = Timestamp::now();
            let task2 = SqliteTaskStore::new(h.db.clone())
                .insert(&Task {
                    id: TaskId::placeholder(),
                    thread_id: Some(h.thread),
                    parent_id: None,
                    title: "t2".into(),
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
            let eff2 = h.efforts.start(task2, &h.thread, None).await.unwrap();
            let eff2_id = eff2.id.to_string();
            let eid1 = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            // A capture time inside BOTH (open) windows — after the later start
            // (eff2). Using `now` would land before eff2 started and dodge the bug.
            let at = Timestamp::from_unix_ms(eff2.started_at.unix_ms() + 1000);

            // Analysis spec (category static-quality → the Run family), counting
            // `error`-severity facts over the lint-hit measure.
            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let m = facts
                .upsert_measure(oxplow_db::NewMeasure::new("oxplow.lint_hit", "Lint hits"))
                .await
                .unwrap();
            let mut s = oxplow_db::NewMetricSpec::base(
                "oxplow.analysis.errors",
                "Analysis errors",
                "oxplow.lint_hit",
                "count",
            );
            s.direction = "lower-better".into();
            s.category = Some("static-quality".into());
            s.display_kind = "findings".into();
            s.filter_json = Some(r#"{"severity":"error"}"#.into());
            facts.upsert_spec(s).await.unwrap();

            // One analysis capture, stamped to e1 only, with three error facts (and
            // one warning, filtered out by the spec's severity predicate).
            let mut cap = oxplow_db::NewMetricCapture::done(1, "analysis", "analysis-report");
            cap.captured_at = Some(at);
            cap.thread_id = Some(h.thread.value());
            cap.trigger = Some("on-report".into());
            cap.effort_id = Some(eid1.value());
            let lint = |sev: &str| oxplow_db::NewFact {
                severity: Some(sev.into()),
                ..oxplow_db::NewFact::new(m, 1.0)
            };
            facts
                .record_facts(
                    cap,
                    vec![lint("error"), lint("error"), lint("error"), lint("warning")],
                )
                .await
                .unwrap();

            let d1 = h.service.effort_metric_deltas(&h.effort_id).await;
            let analysis = d1
                .iter()
                .find(|d| d.category.as_deref() == Some("static-quality"));
            assert!(
                analysis.is_some(),
                "the owning effort shows its analysis capture"
            );
            // Three error facts survive the severity filter (the warning is dropped).
            assert_eq!(analysis.unwrap().current, 3.0);
            let d2 = h.service.effort_metric_deltas(&eff2_id).await;
            assert!(
                !d2.iter()
                    .any(|d| d.category.as_deref() == Some("static-quality")),
                "a capture stamped to another effort must not pollute a concurrent effort"
            );
        }

        #[tokio::test]
        async fn effort_metric_deltas_shows_analysis_dropping_to_zero() {
            // tsk44: a CLEAN analysis run writes an EMPTY effort-stamped capture;
            // the stamped read zero-fills it, so the panel shows "3 → 0" instead
            // of a stuck 3 (or no row) after the agent fixes every lint.
            let h = build(None).await;
            let eid1 = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            let now = Timestamp::now();

            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let m = facts
                .upsert_measure(oxplow_db::NewMeasure::new("oxplow.lint_hit", "Lint hits"))
                .await
                .unwrap();
            let mut s = oxplow_db::NewMetricSpec::base(
                "oxplow.analysis.errors",
                "Analysis errors",
                "oxplow.lint_hit",
                "count",
            );
            s.direction = "lower-better".into();
            s.category = Some("static-quality".into());
            s.display_kind = "findings".into();
            s.filter_json = Some(r#"{"severity":"error"}"#.into());
            facts.upsert_spec(s).await.unwrap();

            // Run 1 (stamped to the effort): three errors.
            let mut cap1 = oxplow_db::NewMetricCapture::done(1, "analysis", "analysis-report");
            cap1.captured_at = Some(now);
            cap1.thread_id = Some(h.thread.value());
            cap1.effort_id = Some(eid1.value());
            let lint = |sev: &str| oxplow_db::NewFact {
                severity: Some(sev.into()),
                ..oxplow_db::NewFact::new(m, 1.0)
            };
            facts
                .record_facts(cap1, vec![lint("error"), lint("error"), lint("error")])
                .await
                .unwrap();
            // Run 2 (stamped, later): CLEAN — an empty capture, no facts.
            let mut cap2 = oxplow_db::NewMetricCapture::done(1, "analysis", "analysis-report");
            cap2.captured_at = Some(Timestamp::from_unix_ms(now.unix_ms() + 60_000));
            cap2.thread_id = Some(h.thread.value());
            cap2.effort_id = Some(eid1.value());
            facts.record_facts(cap2, vec![]).await.unwrap();

            let d1 = h.service.effort_metric_deltas(&h.effort_id).await;
            let analysis = d1
                .iter()
                .find(|d| d.category.as_deref() == Some("static-quality"))
                .expect("the clean run still yields a row");
            assert_eq!(analysis.baseline, Some(3.0));
            assert_eq!(analysis.current, 0.0, "the clean run reads as zero");
            assert_eq!(analysis.delta, Some(-3.0));
        }

        #[tokio::test]
        async fn effort_metric_deltas_sums_operational_flow() {
            let h = build(None).await;
            let start = effort_start(&h, &h.effort_id).await;
            let after = Timestamp::from_unix_ms(start.unix_ms() + 60_000);
            let eid = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            // An operational `sum` flow (tokens): summed over the facts of the
            // effort's OWN captures (`metric_capture.effort_id`, tsk37).
            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let m = facts
                .upsert_measure({
                    let mut mm = oxplow_db::NewMeasure::new("oxplow.tokens", "Tokens");
                    mm.temporal_semantics = "additive".into();
                    mm
                })
                .await
                .unwrap();
            let mut s = oxplow_db::NewMetricSpec::base(
                "agent.tokens.total",
                "Tokens",
                "oxplow.tokens",
                "sum",
            );
            s.category = Some("operational".into());
            s.display_kind = "event".into();
            facts.upsert_spec(s).await.unwrap();
            let mut cap = oxplow_db::NewMetricCapture::done(1, "tokens", "stop");
            cap.captured_at = Some(after);
            cap.thread_id = Some(1);
            cap.effort_id = Some(eid.value());
            facts
                .record_facts(
                    cap,
                    vec![
                        oxplow_db::NewFact::new(m, 1000.0),
                        oxplow_db::NewFact::new(m, 2000.0),
                    ],
                )
                .await
                .unwrap();

            let deltas = h.service.effort_metric_deltas(&h.effort_id).await;
            assert_eq!(deltas.len(), 1);
            let d = &deltas[0];
            assert_eq!(d.agg, "sum");
            assert_eq!(d.baseline, None);
            assert_eq!(d.current, 3000.0); // 1000 + 2000
            assert_eq!(d.delta, Some(3000.0));
        }

        #[tokio::test]
        async fn effort_metric_context_uses_file_attribution_and_agrees_with_panel() {
            // tsk253: the prompt now reports the effort's OWN contribution (the
            // claimed-file slice), matching the task-page panel — not the repo
            // total, which under overlap includes other efforts' changes.
            let h = build(None).await;
            let start = effort_start(&h, &h.effort_id).await;
            let before = Timestamp::from_unix_ms(start.unix_ms() - 60_000);
            let after = Timestamp::from_unix_ms(start.unix_ms() + 60_000);
            let (m, facts) =
                seed_file_gauge(&h, "oxplow.rust.unsafe_blocks", "lower-better", Some(0.0)).await;
            claim(&h, &h.effort_id, "src/a.rs").await;
            // Repo total jumps 10 → 13, but THIS effort's file (a.rs) only 2 → 5;
            // z.rs (8, unchanged, unclaimed) is another effort's churn.
            seed_gauge_capture(&facts, m, before, &[("src/a.rs", 2.0), ("src/z.rs", 8.0)]).await;
            seed_gauge_capture(&facts, m, after, &[("src/a.rs", 5.0), ("src/z.rs", 8.0)]).await;

            let ctx = h.service.effort_metric_context(&h.thread).await.unwrap();
            assert!(
                ctx.contains("2 → 5"),
                "file-attributed slice, not repo total: {ctx}"
            );
            assert!(
                !ctx.contains("10 → 13"),
                "must not report the repo total: {ctx}"
            );

            // The prompt agrees with the panel for the same effort.
            let panel = h.service.effort_metric_deltas(&h.effort_id).await;
            let d = panel
                .iter()
                .find(|d| d.key == "oxplow.rust.unsafe_blocks")
                .unwrap();
            assert_eq!(d.baseline, Some(2.0));
            assert_eq!(d.current, 5.0);
        }

        #[tokio::test]
        async fn run_attribution_disentangles_concurrent_efforts_and_flags_unclaimed() {
            // tsk262: test RUNS ride the kind-agnostic claim→reconcile engine.
            // Two efforts overlap on one thread; three test runs land in the
            // shared window. Each effort claims its own run; the third is
            // claimed by nobody. RunKind must keep each effort's residue to only
            // the truly-unattributed run — the other effort's run is deduped out.
            use crate::attribution::{reconcile_close, RunKind};
            use oxplow_db::{SqliteAttributionStore, STATE_CLAIMED};

            let h = build(None).await;
            let now = Timestamp::now();
            let task2 = SqliteTaskStore::new(h.db.clone())
                .insert(&Task {
                    id: TaskId::placeholder(),
                    thread_id: Some(h.thread),
                    parent_id: None,
                    title: "t2".into(),
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
            let eff2 = h.efforts.start(task2, &h.thread, None).await.unwrap();
            let eid1 = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();

            // Three observed run CAPTURES on the thread (the capture is the run,
            // T-E1) — after both efforts opened, so all three fall in both
            // open windows.
            let facts_store = oxplow_db::SqliteFactStore::new(h.db.clone());
            let seed_run = || async {
                let mut cap = oxplow_db::NewMetricCapture::done(1, "tests", "post-tool-bash");
                cap.thread_id = Some(h.thread.value());
                cap.trigger = Some("on-report".into());
                facts_store.record_facts(cap, vec![]).await.unwrap()
            };
            let r1 = seed_run().await;
            let r2 = seed_run().await;
            let r3 = seed_run().await;

            let ledger = SqliteAttributionStore::new(h.db.clone());
            ledger
                .set_state(&eid1, "run", &format!("run:{r1}"), STATE_CLAIMED, None)
                .await
                .unwrap();
            ledger
                .set_state(&eff2.id, "run", &format!("run:{r2}"), STATE_CLAIMED, None)
                .await
                .unwrap();

            let kind1 = RunKind::runs(h.efforts.as_ref(), &facts_store, &ledger);
            // eff1: observed {r1,r2,r3} − claimed {r1} − other-claimed {r2} = {r3}.
            assert_eq!(
                reconcile_close(&kind1, &eid1).await,
                vec![format!("run:{r3}")]
            );
            let kind2 = RunKind::runs(h.efforts.as_ref(), &facts_store, &ledger);
            // eff2: observed all − claimed {r2} − other-claimed {r1} = {r3}.
            assert_eq!(
                reconcile_close(&kind2, &eff2.id).await,
                vec![format!("run:{r3}")]
            );
        }

        #[tokio::test]
        async fn record_test_run_observes_with_no_open_effort() {
            // tsk269 observe-always: a run is recorded into the substrate even
            // with NO open effort — just left unattributed (no ledger claim). The
            // bug this fixes: collection used to drop it entirely.
            use oxplow_db::SqliteAttributionStore;
            let h = build(None).await;
            let eid1 = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            h.efforts.finish(&eid1, None, None).await.unwrap(); // close the only effort

            h.service
                .record_test_run(
                    &h.thread,
                    "cargo test",
                    Some(0),
                    None,
                    Some(5),
                    Some(0),
                    Some(5),
                    "observed",
                    "post-tool-bash",
                    None,
                    None,
                )
                .await
                .unwrap();

            // The run CAPTURE is in the substrate (observed)…
            let runs = oxplow_db::SqliteFactStore::new(h.db.clone())
                .captures_in_window_by_trigger(
                    h.thread.value(),
                    "on-report",
                    Timestamp::from_unix_ms(0),
                    None,
                )
                .await
                .unwrap();
            assert_eq!(runs.len(), 1, "run recorded despite no open effort");
            // …but nothing is claimed (no effort to attribute to).
            let ledger = SqliteAttributionStore::new(h.db.clone());
            assert!(
                ledger
                    .list_refs(&eid1, "run", STATE_CLAIMED)
                    .await
                    .unwrap()
                    .is_empty(),
                "no effort open ⇒ no claim, just an observed run"
            );
        }

        #[tokio::test]
        async fn run_residue_excludes_runs_dominated_by_a_nested_effort() {
            // tsk267 window-dominance: a run that falls inside a strictly-nested
            // sibling effort's window is that narrower effort's to own, so the
            // wider effort drops it from its residue; a run outside the nested
            // window stays the wider effort's. Windows are built in real time by
            // ordering start/finish so eff2 ⊂ eff1.
            use crate::attribution::{reconcile_close, RunKind};
            use oxplow_db::SqliteAttributionStore;

            let h = build(None).await; // eff1 (h.effort_id) opened first
            let now = Timestamp::now();
            let task2 = SqliteTaskStore::new(h.db.clone())
                .insert(&Task {
                    id: TaskId::placeholder(),
                    thread_id: Some(h.thread),
                    parent_id: None,
                    title: "t2".into(),
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
            let eid1 = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();

            let facts_store = oxplow_db::SqliteFactStore::new(h.db.clone());
            let mk_run = || async {
                let mut cap = oxplow_db::NewMetricCapture::done(1, "tests", "post-tool-bash");
                cap.thread_id = Some(h.thread.value());
                cap.trigger = Some("on-report".into());
                facts_store.record_facts(cap, vec![]).await.unwrap()
            };

            // eff2 opens after eff1; r_inner runs while eff2 is open; eff2 closes;
            // r_outer runs after; eff1 closes last ⇒ eff2 ⊂ eff1, r_inner ∈ eff2,
            // r_outer ∈ eff1 only. Small sleeps keep the timeline strictly ordered
            // past the microsecond truncation of canonical timestamps.
            let gap = || tokio::time::sleep(std::time::Duration::from_millis(3));
            let eff2 = h.efforts.start(task2, &h.thread, None).await.unwrap();
            gap().await;
            let r_inner = mk_run().await;
            gap().await;
            h.efforts.finish(&eff2.id, None, None).await.unwrap();
            gap().await;
            let r_outer = mk_run().await;
            gap().await;
            h.efforts.finish(&eid1, None, None).await.unwrap();

            let ledger = SqliteAttributionStore::new(h.db.clone());
            let kind = RunKind::runs(h.efforts.as_ref(), &facts_store, &ledger);
            // eff1 observes both, but r_inner is dominated by nested eff2 → only
            // r_outer is eff1's residue.
            let residue = reconcile_close(&kind, &eid1).await;
            assert_eq!(residue, vec![format!("run:{r_outer}")]);
            assert!(
                !residue.contains(&format!("run:{r_inner}")),
                "the nested effort's run is dominated away from eff1"
            );
        }

        #[tokio::test]
        async fn record_test_run_auto_attributes_when_single_open_effort() {
            // tsk263: a recorded test run is auto-attributed to the open effort
            // when it's unambiguous (the Harness has exactly one). The agent is
            // only asked in the concurrent case.
            use oxplow_db::{SqliteAttributionStore, STATE_CLAIMED};
            let h = build(None).await;
            h.service
                .record_test_run(
                    &h.thread,
                    "cargo test",
                    Some(0),
                    None,
                    Some(5),
                    Some(0),
                    Some(5),
                    "observed",
                    "post-tool-bash",
                    None,
                    None,
                )
                .await
                .unwrap();
            let ledger = SqliteAttributionStore::new(h.db.clone());
            let eid = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            let claimed = ledger.list_refs(&eid, "run", STATE_CLAIMED).await.unwrap();
            assert_eq!(claimed.len(), 1, "single open effort → run auto-attributed");
            assert!(claimed[0].starts_with("run:"), "ref is run:<id>");
            // The capture IS the run (T-E1, tsk48): the claimed id resolves to a
            // metric_capture carrying the verbatim payload in its detail envelope.
            let cid: i64 = claimed[0].strip_prefix("run:").unwrap().parse().unwrap();
            let cap = oxplow_db::SqliteFactStore::new(h.db.clone())
                .get_capture(cid)
                .await
                .unwrap()
                .expect("the claimed ref is a capture id");
            assert_eq!(cap.producer, "tests");
            assert_eq!(cap.trigger.as_deref(), Some("on-report"));
            let envelope: serde_json::Value =
                serde_json::from_str(cap.detail_json.as_deref().unwrap()).unwrap();
            assert_eq!(envelope["kind"], "test-detail");
            assert_eq!(envelope["payload"]["total"], 5);
        }

        #[tokio::test]
        async fn record_test_run_attributes_to_named_task_under_concurrent_efforts() {
            // tsk265: the agent-agnostic EXACT path. When the caller NAMES its
            // task (a dispatched sub-agent knows its own task id), the run is
            // claimed for THAT task's open effort even though two efforts are
            // open on the thread — `find_single` would punt (ambiguous), but the
            // named task resolves it exactly via the MCP contract, with no
            // visibility into which sub-agent ran it.
            use oxplow_db::{SqliteAttributionStore, STATE_CLAIMED};
            let h = build(None).await;
            let now = Timestamp::now();
            let task2 = SqliteTaskStore::new(h.db.clone())
                .insert(&Task {
                    id: TaskId::placeholder(),
                    thread_id: Some(h.thread),
                    parent_id: None,
                    title: "t2".into(),
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
            let eff2 = h.efforts.start(task2, &h.thread, None).await.unwrap();

            // Two efforts open ⇒ ambiguous for find_single. Name task2.
            h.service
                .record_test_run(
                    &h.thread,
                    "cargo test",
                    Some(0),
                    None,
                    Some(5),
                    Some(0),
                    Some(5),
                    "asserted",
                    "agent",
                    None,
                    Some(task2),
                )
                .await
                .unwrap();

            let ledger = SqliteAttributionStore::new(h.db.clone());
            let eid1 = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            // Claimed for the NAMED effort, never the other open one.
            assert_eq!(
                ledger
                    .list_refs(&eff2.id, "run", STATE_CLAIMED)
                    .await
                    .unwrap()
                    .len(),
                1,
                "named task → run claimed for its effort"
            );
            assert!(
                ledger
                    .list_refs(&eid1, "run", STATE_CLAIMED)
                    .await
                    .unwrap()
                    .is_empty(),
                "the other open effort is not credited"
            );
        }

        #[tokio::test]
        async fn record_test_run_named_task_without_open_effort_stays_unclaimed() {
            // tsk271: naming a task is EXACT-or-nothing. When the named task has
            // NO open effort, the run must NOT fall back to the thread's single
            // open effort (a DIFFERENT task) — that would be a wrong-exact claim
            // the design otherwise avoids. The run is still recorded
            // (observe-always); it's just left unclaimed for the agent to claim.
            use oxplow_db::{SqliteAttributionStore, STATE_CLAIMED};
            let h = build(None).await;
            let now = Timestamp::now();
            // task2 exists but never started an effort ⇒ find_open_for_task is
            // None. The harness's task1 effort is the ONLY open effort.
            let task2 = SqliteTaskStore::new(h.db.clone())
                .insert(&Task {
                    id: TaskId::placeholder(),
                    thread_id: Some(h.thread),
                    parent_id: None,
                    title: "t2".into(),
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

            h.service
                .record_test_run(
                    &h.thread,
                    "cargo test",
                    Some(0),
                    None,
                    Some(5),
                    Some(0),
                    Some(5),
                    "asserted",
                    "agent",
                    None,
                    Some(task2),
                )
                .await
                .unwrap();

            let ledger = SqliteAttributionStore::new(h.db.clone());
            let eid1 = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            assert!(
                ledger
                    .list_refs(&eid1, "run", STATE_CLAIMED)
                    .await
                    .unwrap()
                    .is_empty(),
                "named task with no open effort must not fall back to the single \
                 open effort of a different task"
            );
        }

        #[tokio::test]
        async fn ingest_coverage_writes_coverage_detail_finding_to_substrate() {
            let h = build_full(Some(COBERTURA_50PCT), true, &[]).await;
            h.service
                .ingest_coverage(&h.thread, None, None, false)
                .await
                .unwrap();
            // The per-file line-sets ride in the coverage CAPTURE's detail
            // envelope (T-E1/T-E2 — the legacy coverage-detail finding is gone).
            let caps = oxplow_db::SqliteFactStore::new(h.db.clone())
                .captures_in_window_by_trigger(
                    h.thread.value(),
                    "on-report",
                    Timestamp::from_unix_ms(0),
                    None,
                )
                .await
                .unwrap();
            assert_eq!(caps.len(), 1);
            let envelope: serde_json::Value =
                serde_json::from_str(caps[0].detail_json.as_deref().unwrap()).unwrap();
            assert_eq!(envelope["kind"], "coverage-detail");
            // tsk270: the stored detail is ABSOLUTE (per-file instrumented/covered
            // line-sets + whole-report absPct), not the effort-relative diff.
            let payload = envelope["payload"].clone();
            assert!(payload["files"].is_array(), "per-file line-sets kept");
            let foo = payload["files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["path"] == "src/foo.rs")
                .unwrap();
            assert_eq!(foo["instrumented"], serde_json::json!([1, 2, 4]));
            assert_eq!(foo["covered"], serde_json::json!([1, 2]));
            assert!((payload["absPct"].as_f64().unwrap() - 66.666).abs() < 0.01);
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
                    None,
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
        async fn ingest_coverage_observes_with_no_open_effort() {
            // tsk270 observe-always: coverage is recorded even with no open effort
            // (absolute), just left unattributed — no longer dropped.
            let h = build(Some(COBERTURA_50PCT)).await;
            h.efforts
                .finish(&EffortId::try_from_str(&h.effort_id).unwrap(), None, None)
                .await
                .unwrap();
            assert!(matches!(
                h.service
                    .ingest_coverage(&h.thread, None, None, true)
                    .await
                    .unwrap(),
                CoverageIngest::Stored { .. }
            ));
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
            // tsk270: observe records absolute coverage regardless (Stored)…
            assert!(matches!(
                h.service
                    .ingest_coverage(&h.thread, None, None, false)
                    .await
                    .unwrap(),
                CoverageIngest::Stored { .. }
            ));
            // …but the effort's DERIVED diff is empty (line 1 is unchanged), so no
            // diff-coverage observation surfaces for it.
            let rows = h
                .service
                .list_for_effort(&h.effort_id, Some("diff-coverage"))
                .await
                .unwrap();
            assert!(
                rows.is_empty(),
                "no changed instrumented lines → no diff observation"
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
            // The fired nudge also records an `oxplow.nudge` event FACT,
            // subject = the nudge kind (tsk216; the legacy sample is gone, T-E2).
            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let measure = facts
                .get_measure("oxplow.nudge")
                .await
                .unwrap()
                .expect("nudge measure seeded by V46");
            let fired = facts.facts_for_measure(measure.id).await.unwrap();
            assert_eq!(fired.len(), 1, "one nudge fact per fired nudge");
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
            // The policy rides the producer SPEC (T-E2: the legacy definition
            // write is gone) — seeded by seed_catalog.
            for spec in crate::producer_metrics::builtin_producer_specs() {
                oxplow_db::SqliteFactStore::new(h.db.clone())
                    .upsert_spec(spec)
                    .await
                    .unwrap();
            }
            let spec = oxplow_db::SqliteFactStore::new(h.db.clone())
                .get_spec("oxplow.coverage.abs_pct")
                .await
                .unwrap()
                .expect("coverage spec seeded");
            assert_eq!(spec.target, Some(80.0), "target in data");
            assert_eq!(spec.fail_at, Some(50.0), "fail floor in data");
            assert_eq!(spec.direction, "higher-better");
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
            let report = h
                .service
                .merge_fresh_test_reports(effort.started_at, &cfg, &registry);
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

            // Dual-written into the durable fact layer (epic tsk12): one fact on
            // the `oxplow.nudge` event measure so Sum() reconstructs the fired
            // count (the `agent.nudges.fired` spec).
            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let measure = facts
                .get_measure("oxplow.nudge")
                .await
                .unwrap()
                .expect("nudge measure seeded by V46");
            let nudge_facts = facts.facts_for_measure(measure.id).await.unwrap();
            assert_eq!(nudge_facts.len(), 1, "one nudge fact");
            assert_eq!(nudge_facts[0].value, 1.0);
            assert_eq!(
                nudge_facts[0].subject_ref.as_deref(),
                Some("report-less-run")
            );

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
                .merge_fresh_test_reports(effort.started_at, &cfg, &registry)
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

            // The analysis CAPTURE carries the verbatim payload in its detail
            // envelope (T-E1/T-E2 — the legacy samples + findings are gone).
            let caps = oxplow_db::SqliteFactStore::new(h.db.clone())
                .captures_in_window_by_trigger(
                    h.thread.value(),
                    "on-report",
                    Timestamp::from_unix_ms(0),
                    None,
                )
                .await
                .unwrap();
            assert_eq!(caps.len(), 1, "one analysis run capture");
            assert_eq!(caps[0].producer, "clippy");
            let envelope: serde_json::Value =
                serde_json::from_str(caps[0].detail_json.as_deref().unwrap()).unwrap();
            assert_eq!(envelope["kind"], "analysis-detail");
            assert_eq!(envelope["payload"]["errorCount"], 1);
            assert_eq!(envelope["payload"]["warningCount"], 1);

            // The durable fact layer (epic tsk12): one
            // `oxplow.lint_hit` fact per finding, reported severity/rule/detail
            // in the dedicated columns + the file location on the fact.
            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let measure = facts
                .get_measure("oxplow.lint_hit")
                .await
                .unwrap()
                .expect("lint_hit measure seeded by V43");
            let hits = facts.facts_for_measure(measure.id).await.unwrap();
            assert_eq!(hits.len(), 2, "one fact per lint hit");
            assert!(hits.iter().all(|f| f.value == 1.0), "each hit counts as 1");
            let err_hit = hits
                .iter()
                .find(|f| f.rule.as_deref() == Some("E0308"))
                .expect("the error hit landed as a fact");
            assert_eq!(err_hit.severity.as_deref(), Some("error"));
            assert_eq!(err_hit.detail.as_deref(), Some("boom"));
            assert_eq!(err_hit.path.as_deref(), Some("src/a.rs"));
            assert_eq!(err_hit.line, Some(10));
            assert_eq!(err_hit.subject_ref.as_deref(), Some("file:src/a.rs"));
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
                .merge_fresh_analysis(effort.started_at, &cfg, &registry)
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
