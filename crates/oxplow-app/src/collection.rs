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

use oxplow_domain::refs::build::work_item_ref;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde_json::json;
use similar::{ChangeTag, TextDiff};

use oxplow_collect_plugin::{
    Collector, CollectorInput, CollectorKind, CollectorOutput, CollectorRuntime,
};
use oxplow_config::collectors::{CollectorSpec, Records, RunKind, Trigger};
use oxplow_config::OxplowConfig;
use oxplow_db::agent_nudge_store::{NewAgentNudge, SqliteAgentNudgeStore};
use oxplow_db::{
    Effort, EffortStore, SqliteAttributionStore, SqliteEffortStore, SqliteSnapshotStore,
    SqliteThreadStore, STATE_CLAIMED,
};
use oxplow_db::{NewFact, NewMetricCapture, SqliteFactStore};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{DomainError, EffortId, TaskId, ThreadId};

use crate::file_ref_version;
use crate::metric_engine::threshold_state;

/// Built-in command substrings that count as a test run. The `testing:`
/// block's `runPatterns` extends (never replaces) this list.
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
/// `testing:` block's `analysisPatterns` extends (never replaces) this
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

/// True when the command runs `git revert` AND lets it commit (tsk77) —
/// `--no-commit`/`-n` stages the inverse without landing anything, so there
/// is no revert commit to attribute waste from. Same global-option skipping
/// as [`detect_git_commit`]; `git revert` never carries the word "commit",
/// so the waste leg needs its own detector.
pub fn detect_git_revert(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    if lower.contains("--no-commit") || lower.split_whitespace().any(|t| t == "-n") {
        return false;
    }
    let toks: Vec<&str> = lower.split_whitespace().collect();
    for (i, t) in toks.iter().enumerate() {
        if *t != "git" && !t.ends_with("/git") {
            continue;
        }
        let mut j = i + 1;
        while let Some(tok) = toks.get(j) {
            if *tok == "-c" || *tok == "--config" {
                j += 2;
                continue;
            }
            if tok.starts_with('-') {
                j += 1;
                continue;
            }
            if *tok == "revert" {
                return true;
            }
            break;
        }
    }
    false
}

/// Full-length shas from the stock `git revert` trailer lines
/// (`This reverts commit <sha>.`), in body order. Prose that merely mentions
/// reverting doesn't match — only the exact trailer shape counts, which is
/// what both `git revert` and a hand-written faithful trailer produce.
fn parse_reverted_shas(body: &str) -> Vec<String> {
    let mut shas = Vec::new();
    for line in body.lines() {
        let Some(rest) = line.trim().strip_prefix("This reverts commit ") else {
            continue;
        };
        let sha: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        if sha.len() == 40 {
            shas.push(sha);
        }
    }
    shas
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
/// a hardcoded 50/80 ramp. oxplow-analytics' `coverage-target` advisory
/// uses the same 80%.
pub const COVERAGE_TARGET_PCT: f64 = 80.0;
pub const COVERAGE_FAIL_PCT: f64 = 50.0;

/// What recording a coverage report came to, so a caller can say why
/// nothing landed.
#[derive(Debug, Clone, PartialEq)]
pub enum CoverageIngest {
    /// The thread has no stream to record in.
    NoStream,
    /// Nothing in the report was instrumented (or the coverage metric is
    /// off).
    NoChangedCoverage,
    Stored {
        observation_id: i64,
        summary_pct: f64,
        changed_lines: usize,
        covered_lines: usize,
    },
}

/// What a by-hand run of a report collector recorded
/// ([`CollectionService::sync_report_collector`]).
#[derive(Debug, Clone, PartialEq)]
pub enum ReportSync {
    /// A test run: its capture (the `run:<id>` agents claim), or none when
    /// the report held no cases.
    Tests {
        run: Option<i64>,
    },
    Coverage(CoverageIngest),
    Analysis(AnalysisIngest),
}

impl ReportSync {
    /// As `collector.sync` answers: `status` (`stored`, or why nothing
    /// landed) with what was recorded.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            ReportSync::Tests { run: Some(run) } => {
                json!({ "status": "stored", "records": "tests", "run": format!("run:{run}") })
            }
            ReportSync::Tests { run: None } => {
                json!({ "status": "no_cases", "records": "tests" })
            }
            ReportSync::Coverage(CoverageIngest::NoStream)
            | ReportSync::Analysis(AnalysisIngest::NotRecorded) => {
                json!({ "status": "no_stream" })
            }
            ReportSync::Coverage(CoverageIngest::NoChangedCoverage) => {
                json!({ "status": "no_coverage", "records": "coverage" })
            }
            ReportSync::Coverage(CoverageIngest::Stored {
                observation_id,
                summary_pct,
                changed_lines,
                covered_lines,
            }) => json!({
                "status": "stored",
                "records": "coverage",
                "run": format!("run:{observation_id}"),
                "summaryPct": summary_pct,
                "instrumentedLines": changed_lines,
                "coveredLines": covered_lines,
            }),
            ReportSync::Analysis(AnalysisIngest::Stored {
                observation_id,
                error_count,
                warning_count,
                info_count,
                note_count,
                findings,
            }) => json!({
                "status": "stored",
                "records": "analysis",
                "run": format!("run:{observation_id}"),
                "errorCount": error_count,
                "warningCount": warning_count,
                "infoCount": info_count,
                "noteCount": note_count,
                "findings": findings,
            }),
        }
    }
}

/// What recording an analysis report came to. Mirrors [`CoverageIngest`].
#[derive(Debug, Clone, PartialEq)]
pub enum AnalysisIngest {
    /// Nothing was recorded (no stream).
    NotRecorded,
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
    /// Durable fact layer (epic tsk12): coverage/test/analysis producers
    /// dual-write atomic facts here beside the legacy samples/findings. The
    /// aggregation engine reads these; the samples are the rebuildable cache.
    facts: Arc<SqliteFactStore>,
    nudges: Arc<SqliteAgentNudgeStore>,
    efforts: Arc<SqliteEffortStore>,
    /// Read-only, and only for run attribution: an effort's task text names the
    /// files being worked on before snapshot capture has claimed any (tsk185).
    tasks: Arc<oxplow_db::SqliteTaskStore>,
    threads: Arc<SqliteThreadStore>,
    snapshots: Arc<SqliteSnapshotStore>,
    /// Each stream's snapshot taker: a run's coverage is pinned to a take
    /// of the code it measured (tsk883).
    captures: crate::snapshot_capture_registry::SnapshotCaptureRegistry,
    content: crate::snapshot_content::SnapshotContent,
    vcs: Arc<dyn oxplow_domain::vcs::Vcs>,
    config: Arc<RwLock<OxplowConfig>>,
    project_dir: PathBuf,
    /// This machine's program approvals (`exec_consent`).
    approvals: Arc<crate::exec_consent::ApprovalStore>,
    /// Where a report collector's run is recorded (`collector_run`,
    /// `collector.synced`) and its health kept (tsk863). Without one (unit
    /// harnesses) a report is still read, its run unrecorded.
    run_log: Option<crate::collector_runner::RunLog>,
    /// Kind-agnostic attribution ledger (tsk262/263) — runs (test/coverage/
    /// analysis) record their claim state here. A run is auto-attributed to the
    /// open effort at record time only when unambiguous (`find_single_open_for_thread`);
    /// the concurrent case is resolved by the close reconcile + the agent's claim.
    attribution: Arc<SqliteAttributionStore>,
    /// The metric-ancestry resolver (tsk102) for this service's own
    /// `tree_state_series` call — built in `new` from the stores it already
    /// holds, so the wiring can't be forgotten at a call site. Same rule,
    /// same answers as the engine's resolver (both are pure over the same DB).
    metric_visibility: Arc<crate::metric_visibility::VisibilityResolver>,
    /// Validates the `test.*` events a capture logs with it.
    vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
}

/// The `agent.tool.finished` event a collection reacts to (P3.6): its id
/// keys every capture and nudge the reaction writes, so a redelivered
/// event records nothing twice, and its anchors (turn, and the effort open
/// when the command ran) carry into what it records.
#[derive(Debug, Clone)]
pub struct RunCause {
    pub event_id: String,
    /// The event's place in the log: a report collector's run records it
    /// (`last_event_id`) and dedupes on it.
    pub seq: i64,
    pub anchors: oxplow_domain::Anchors,
    /// When the run finished (the event's time): what report freshness is
    /// judged against, however late the event is delivered.
    pub at: oxplow_domain::Timestamp,
}

impl CollectionService {
    /// The agent turn a run on `thread` was made in (tsk483): its causing
    /// tool event's — however late that event is delivered, and none when
    /// the event had none — else, for a run reported by command, the
    /// thread's open turn.
    async fn turn_of(&self, thread: &ThreadId, cause: Option<&RunCause>) -> Option<i64> {
        if let Some(cause) = cause {
            return cause.anchors.turn_id;
        }
        let thread = *thread;
        self.facts
            .database()
            .read(move |tx| oxplow_db::agent_stores::open_turn_ids_tx(tx, thread))
            .await
            .ok()
            .and_then(|open| open.first().map(|t| t.value()))
    }

    /// The branch the project checkout has checked out (`None` when
    /// detached or unreadable).
    async fn current_branch(&self) -> Option<String> {
        self.vcs
            .head(&self.project_dir)
            .await
            .ok()
            .and_then(|h| h.branch)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        facts: Arc<SqliteFactStore>,
        nudges: Arc<SqliteAgentNudgeStore>,
        efforts: Arc<SqliteEffortStore>,
        tasks: Arc<oxplow_db::SqliteTaskStore>,
        threads: Arc<SqliteThreadStore>,
        snapshots: Arc<SqliteSnapshotStore>,
        captures: crate::snapshot_capture_registry::SnapshotCaptureRegistry,
        content: crate::snapshot_content::SnapshotContent,
        vcs: Arc<dyn oxplow_domain::vcs::Vcs>,
        config: Arc<RwLock<OxplowConfig>>,
        project_dir: PathBuf,
        attribution: Arc<SqliteAttributionStore>,
    ) -> Self {
        let metric_visibility = Arc::new(crate::metric_visibility::VisibilityResolver::new(
            (*snapshots).clone(),
            vcs.revision_graph(&project_dir),
        ));
        Self {
            facts,
            nudges,
            efforts,
            tasks,
            threads,
            snapshots,
            captures,
            content,
            vcs,
            config,
            project_dir,
            approvals: Arc::new(crate::exec_consent::ApprovalStore::disabled()),
            run_log: None,
            attribution,
            metric_visibility,
            vocabulary: oxplow_domain::vocabulary::VocabularyHandle::core(),
        }
    }

    /// The registry the `test.*` events are validated against.
    pub fn with_vocabulary(
        mut self,
        vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    ) -> Self {
        self.vocabulary = vocabulary;
        self
    }

    /// The program approvals an `exec` report collector is checked against.
    pub fn with_approvals(mut self, approvals: Arc<crate::exec_consent::ApprovalStore>) -> Self {
        self.approvals = approvals;
        self
    }

    /// Where report collectors' runs are recorded.
    pub fn with_run_log(mut self, log: crate::collector_runner::RunLog) -> Self {
        self.run_log = Some(log);
        self
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

    /// The project's `testing:` block, as it is now (hot-reloaded).
    fn testing_cfg(&self) -> oxplow_config::TestingConfig {
        self.config
            .read()
            .map(|c| c.testing.clone())
            .unwrap_or_default()
    }

    /// The project's report collectors (`collectors:` with `records:`), as
    /// they are now.
    fn report_collectors(&self) -> Vec<CollectorSpec> {
        self.config
            .read()
            .map(|c| {
                c.collectors
                    .iter()
                    .filter(|s| s.records.is_some())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether the project declares report collector `id`.
    pub fn is_report_collector(&self, id: &str) -> bool {
        self.report_collectors().iter().any(|s| s.id == id)
    }

    /// The parser a report collector names: a bundled one, or its own jaq /
    /// Starlark script or program — a program only once a person approved
    /// it on this machine, as it is now (tsk331).
    fn report_parser(&self, spec: &CollectorSpec) -> Result<Collector, ParserProblem> {
        let records = spec
            .records
            .ok_or_else(|| ParserProblem::Broken(format!("`{}` records nothing", spec.id)))?;
        if let Some(name) = spec.bundled_parser() {
            return Collector::bundled(name).ok_or_else(|| {
                ParserProblem::Broken(format!("no bundled parser `{name}` in this oxplow"))
            });
        }
        let kind = match records {
            Records::Tests => CollectorKind::Test,
            Records::Coverage => CollectorKind::Coverage,
            Records::Analysis => CollectorKind::Analysis,
        };
        let entry = spec.entry.as_deref().unwrap_or_default();
        let abs = self.project_dir.join(entry);
        if spec.runtime == oxplow_config::collectors::CollectorRuntime::Exec {
            use crate::exec_consent::{may_run, needs_approval, ProgramKind};
            if !may_run(
                &self.approvals,
                &self.project_dir,
                ProgramKind::Collector,
                &spec.id,
                entry,
                &[],
            ) {
                return Err(ParserProblem::NeedsApproval(needs_approval(
                    ProgramKind::Collector,
                    &spec.id,
                    entry,
                )));
            }
            return Ok(Collector::exec(
                spec.id.clone(),
                kind,
                [abs.to_string_lossy().into_owned()],
            ));
        }
        // The host reads the script; the script never touches the files.
        let script = std::fs::read_to_string(&abs)
            .map_err(|e| ParserProblem::Broken(format!("entry `{entry}`: {e}")))?;
        let format = spec.report.as_ref().map_or("text", |r| r.format.as_str());
        let input = CollectorInput::named(format).ok_or_else(|| {
            ParserProblem::Broken(format!("report format `{format}` isn't one oxplow reads"))
        })?;
        Ok(match spec.runtime {
            oxplow_config::collectors::CollectorRuntime::Starlark => {
                Collector::starlark(spec.id.clone(), kind, input, script)
            }
            _ => Collector::jaq(spec.id.clone(), kind, input, script),
        })
    }

    /// Read one report collector's report (tsk863): when `window` is given,
    /// only a report written inside it (the run's own). A disabled
    /// collector, an unapproved program or a report that isn't there runs
    /// nothing; anything that ran is recorded as the collector's run
    /// (`collector_run`, `collector.synced`) and counts toward its health —
    /// the third failure in a row disables it (P7.C2).
    async fn read_report(
        &self,
        spec: &CollectorSpec,
        window: Option<FreshWindow>,
        how: &ReportRun<'_>,
    ) -> ReportRead {
        let Some(report) = spec.report.as_ref() else {
            return ReportRead::Missing(String::new());
        };
        let abs = self.project_dir.join(&report.path);
        if window.is_some_and(|w| !w.holds(&abs)) {
            return ReportRead::NotFresh;
        }
        let Ok(content) = std::fs::read_to_string(&abs) else {
            return ReportRead::Missing(report.path.clone());
        };
        let key = crate::collector_runner::plugin_key(oxplow_config::collectors::PROJECT, &spec.id);
        if let Some(log) = &self.run_log {
            match log.health().disabled_reason(&key).await {
                Ok(Some(reason)) => {
                    return ReportRead::Disabled(format!(
                        "collector `{}` is disabled: {reason}. A person can enable it again \
                         (`plugin.enable`, Settings → Extensions).",
                        spec.id
                    ))
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(collector = %spec.id, error = %e, "reading its health failed")
                }
            }
        }
        let started = std::time::Instant::now();
        let parsed = match self.report_parser(spec) {
            Err(ParserProblem::NeedsApproval(reason)) => {
                if let Some(log) = &self.run_log {
                    if let Err(e) = log
                        .record_needs_approval(
                            oxplow_config::collectors::PROJECT,
                            &spec.id,
                            how.cause.map(|c| c.seq),
                            reason.clone(),
                        )
                        .await
                    {
                        tracing::warn!(collector = %spec.id, error = %e, "recording its run failed");
                    }
                }
                return ReportRead::NeedsApproval(reason);
            }
            Err(ParserProblem::Broken(e)) => Err(e),
            Ok(parser) => {
                // A whole-workspace report is seconds of script work: off the
                // async workers.
                let exec =
                    (parser.runtime() == CollectorRuntime::Exec).then(|| parser.name().to_string());
                let label = parser
                    .name()
                    .strip_prefix("oxplow.")
                    .unwrap_or(parser.name())
                    .to_string();
                // Paths as the checkout the report came from names them.
                let root = self.project_dir.clone();
                tokio::task::spawn_blocking(move || {
                    parser.run(&content).map(|output| output.relative_to(&root))
                })
                .await
                .map_err(|e| format!("parser task failed: {e}"))
                .and_then(|r| r.map_err(|e| e.to_string()))
                .map(|output| (output, exec, label))
            }
        };
        let elapsed = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let error = parsed.as_ref().err().cloned();
        self.record_report_run(&spec.id, how, &key, elapsed, error)
            .await;
        match parsed {
            Ok((output, exec, label)) => ReportRead::Parsed {
                output,
                exec,
                label,
            },
            Err(e) => ReportRead::Failed(format!("{} ({}): {e}", report.path, spec.id)),
        }
    }

    /// A report collector's run, recorded once per cause: its
    /// `collector_run` row and `collector.synced`, then its health. A
    /// redelivered run's record is already there, and its health already
    /// counted.
    async fn record_report_run(
        &self,
        id: &str,
        how: &ReportRun<'_>,
        key: &oxplow_db::PluginKey,
        elapsed_ms: i64,
        error: Option<String>,
    ) {
        let Some(log) = &self.run_log else {
            return;
        };
        let recorded = log
            .record_with(
                crate::collector_runner::RunRecord {
                    owner: oxplow_config::collectors::PROJECT,
                    id,
                    trigger: how.trigger,
                    source: how.source,
                    cause: how
                        .cause
                        .map(|c| (oxplow_domain::EventId(c.event_id.clone()), c.seq)),
                    status: if error.is_some() { "error" } else { "ok" },
                    entities: Default::default(),
                    facts: 0,
                    elapsed_ms,
                    error: error.clone(),
                },
                None,
            )
            .await;
        match recorded {
            Ok(false) => {}
            Ok(true) => {
                let health = log.health();
                let counted = match &error {
                    None => {
                        health
                            .succeeded(
                                key,
                                Some(std::time::Duration::from_millis(elapsed_ms.max(0) as u64)),
                            )
                            .await
                    }
                    Some(e) => health.failed(key, e).await.map(|_| ()),
                };
                if let Err(e) = counted {
                    tracing::warn!(collector = %id, error = %e, "recording its health failed");
                }
            }
            Err(e) => tracing::warn!(collector = %id, error = %e, "recording its run failed"),
        }
    }

    /// What a detected run's report collectors read (tsk863): each
    /// collector whose `on_run` is `run` and whose report this run wrote
    /// (inside `window`), merged by what it records. A collector reads only
    /// after its own kind of run.
    async fn read_run_reports(
        &self,
        run: RunKind,
        window: FreshWindow,
        cause: Option<&RunCause>,
    ) -> RunReports {
        let source = oxplow_domain::Actor::System.source();
        let how = ReportRun {
            trigger: "on",
            source: &source,
            cause,
        };
        let mut out = RunReports::default();
        for spec in self.report_collectors() {
            if spec.trigger != (Trigger::OnRun { run }) {
                continue;
            }
            match self.read_report(&spec, Some(window), &how).await {
                ReportRead::Parsed {
                    output,
                    exec,
                    label,
                } => out.add(output, exec, label),
                ReportRead::Failed(e) => {
                    tracing::warn!(collector = %spec.id, error = %e, "report collector failed");
                    if spec.records == Some(Records::Coverage) {
                        out.coverage_errors.push(e);
                    }
                }
                ReportRead::Disabled(m) | ReportRead::NeedsApproval(m) => {
                    tracing::warn!(collector = %spec.id, "{m}")
                }
                ReportRead::NotFresh | ReportRead::Missing(_) => {}
            }
        }
        out
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
        self.record_test_run_caused(
            thread,
            command,
            exit_code,
            (duration_ms, passed, failed, total),
            provenance,
            source,
            report,
            task,
            None,
        )
        .await
    }

    /// [`Self::record_test_run`] for a run the collection reactor saw
    /// (`cause`): the capture is keyed by the event, so a redelivery records
    /// nothing new, and the effort the command ran in owns it. Every run
    /// logs `test.run.recorded` in the capture's transaction.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_test_run_caused(
        &self,
        thread: &ThreadId,
        command: &str,
        exit_code: Option<i64>,
        (duration_ms, passed, failed, total): (Option<i64>, Option<i64>, Option<i64>, Option<i64>),
        provenance: &str,
        source: &str,
        report: Option<&oxplow_coverage::TestReport>,
        task: Option<TaskId>,
        cause: Option<&RunCause>,
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
        let owning = self
            .resolve_owner(thread, task, anchored_effort(cause), Some(command))
            .await;
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
                // A run with per-case results records them change-only
                // against each test's summary (tsk733, `record_test_run`);
                // asserted counts have no case identity and record plainly.
                let mut facts = Vec::new();
                let mut cases: Option<(i64, Option<i64>, Vec<oxplow_db::TestCaseResult>)> = None;
                // Stop-collecting gate (tsk31): only emit `oxplow.test_case` facts
                // when an enabled metric consumes that measure. When every
                // `oxplow.tests.*` metric is disabled the facts are skipped and the
                // run falls through to the record-only `test-run` producer below —
                // so the effort-review run record + detail survive, but no metric
                // facts are written and the pruned metric stays empty.
                let tests_active = self
                    .facts
                    .measure_has_active_spec("oxplow.test_case")
                    .await
                    .unwrap_or(true);
                if tests_active {
                    if let Some(r) = report {
                        let Some(measure) = self.facts.get_measure("oxplow.test_case").await?
                        else {
                            return Ok::<Option<i64>, DomainError>(None);
                        };
                        // Per-test DURATION (tsk46), on the same subject as the
                        // status, when its metric is on.
                        let duration = if self
                            .facts
                            .measure_has_active_spec("oxplow.test_duration")
                            .await
                            .unwrap_or(false)
                        {
                            self.facts.get_measure("oxplow.test_duration").await?
                        } else {
                            None
                        };
                        use oxplow_coverage::TestStatus::*;
                        let mut results = Vec::new();
                        for suite in &r.suites {
                            for case in &suite.cases {
                                let status = match case.status {
                                    Passed => "passed",
                                    Failed => "failed",
                                    Skipped => "skipped",
                                };
                                results.push(oxplow_db::TestCaseResult {
                                    subject: format!("test:{}::{}", case.classname, case.name),
                                    status: status.into(),
                                    time_ms: duration
                                        .as_ref()
                                        .and(case.time_ms.map(|ms| ms as f64)),
                                    dims_json: serde_json::to_string(&json!({
                                        "oxplow.status": status,
                                        "oxplow.test_suite": suite.name,
                                    }))
                                    .ok(),
                                });
                            }
                        }
                        cases = Some((measure.id, duration.map(|d| d.id), results));
                    } else if counted {
                        let Some(measure) = self.facts.get_measure("oxplow.test_case").await?
                        else {
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
                }
                let branch = self.current_branch().await;
                let snapshot_id = self
                    .snapshots
                    .latest_snapshot_id_for_stream(oxplow_domain::StreamId::new(stream_val))
                    .await
                    .ok()
                    .flatten();
                // `tests` = a measurement (report or asserted counts — a zero
                // here is a real "found 0"); `test-run` = a run record only,
                // invisible to the tests metric timeline. A measured run whose
                // tests metrics are all disabled records as a run record too
                // (tsk31) — the run is remembered, but nothing feeds a pruned
                // metric.
                let producer = if (report.is_some() || counted) && tests_active {
                    "tests"
                } else {
                    "test-run"
                };
                // A test result is about a CODE STATE, not a branch name (tsk95),
                // so stamp the commit the run tested — the fold needs it to be
                // ancestry-aware (tsk97), and it is NOT backfillable after the
                // fact. A snapshot carrying its own commit reads exact; otherwise
                // this falls back to HEAD with `vcs_rev_exact = false`, which
                // is the normal case: the agent edits, then runs tests, so the
                // tree is dirty and the commit is only the CLOSEST one.
                let version = crate::file_ref_version::resolve(
                    &self.snapshots,
                    &*self.vcs,
                    &self.project_dir,
                    snapshot_id.unwrap_or(0),
                )
                .await
                .ok();
                let mut capture = NewMetricCapture::done(stream_val, producer, source.to_string());
                capture.provenance = provenance.to_string();
                capture.thread_id = Some(thread.value());
                capture.trigger = Some("on-report".into());
                capture.branch = branch;
                capture.snapshot_id = snapshot_id;
                capture.closest_vcs_rev = version.as_ref().and_then(|v| v.closest_vcs_rev.clone());
                capture.vcs_rev_exact = version.as_ref().map(|v| v.vcs_rev_exact).unwrap_or(false);
                capture.effort_id = owning_val;
                let turn = self.turn_of(thread, cause).await;
                capture.turn_id = turn;
                capture.detail_json = Self::capture_detail(
                    "test-detail",
                    &serde_json::Value::Object(payload.clone()),
                );
                capture.idempotency_key = cause.map(|c| format!("test-run:{}", c.event_id));
                let log = self.test_run_event(
                    thread,
                    stream_val,
                    owning_val,
                    turn,
                    cause,
                    oxplow_domain::events::schema::TestRunRecordedV1 {
                        run: String::new(), // filled with the capture id
                        command: command.to_string(),
                        exit_code,
                        passed: passed.map(|v| v.max(0) as u64),
                        failed: failed.map(|v| v.max(0) as u64),
                        skipped: skipped.map(|v| v.max(0) as u64),
                        report_parsed: report.is_some(),
                        source: source.to_string(),
                    },
                );
                let id = match cases {
                    Some((measure, duration, results)) => {
                        self.facts
                            .record_test_run(capture, results, measure, duration, Some(log))
                            .await?
                    }
                    None => {
                        self.facts
                            .record_facts_logged(capture, facts, Some(log))
                            .await?
                    }
                };
                Ok(Some(id))
            }
            .await;
            match dual {
                Ok(id) => capture_id = id,
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
        }
        Ok(capture_id)
    }

    /// The `test.run.recorded` a run capture logs with it: anchored to the
    /// cause's turn and effort when the reactor saw the command, else to the
    /// thread, its stream and the owning effort.
    fn test_run_event(
        &self,
        thread: &ThreadId,
        stream_val: i64,
        owning: Option<i64>,
        turn: Option<i64>,
        cause: Option<&RunCause>,
        payload: oxplow_domain::events::schema::TestRunRecordedV1,
    ) -> oxplow_db::fact_store::CaptureEvent {
        use oxplow_domain::events::schema::TestRunRecorded;
        let anchors = match cause {
            Some(c) => oxplow_domain::Anchors {
                effort_id: owning.map(EffortId::new).or(c.anchors.effort_id),
                ..c.anchors.clone()
            },
            None => oxplow_domain::Anchors {
                stream_id: Some(oxplow_domain::StreamId::new(stream_val)),
                thread_id: Some(*thread),
                effort_id: owning.map(EffortId::new),
                turn_id: turn,
                ..oxplow_domain::Anchors::default()
            },
        };
        let (cause_id, dedupe) = match cause {
            Some(c) => (
                Some(oxplow_domain::EventId(c.event_id.clone())),
                Some(format!("test-run:{}", c.event_id)),
            ),
            None => (None, None),
        };
        oxplow_db::fact_store::CaptureEvent {
            vocabulary: self.vocabulary.clone(),
            build: Box::new(move |capture_id| {
                let run = format!("run:{capture_id}");
                let env = oxplow_domain::Envelope::typed::<TestRunRecorded>(
                    oxplow_domain::refs::build::system_source("collection"),
                    &oxplow_domain::events::schema::TestRunRecordedV1 {
                        run: run.clone(),
                        ..payload.clone()
                    },
                )
                .with_anchors(anchors.clone())
                .with_subject([run])
                .with_dedupe_key_opt(dedupe.clone());
                match &cause_id {
                    Some(c) => env.with_cause(c.clone()),
                    None => env,
                }
            }),
        }
    }

    /// The effort a just-produced run/capture belongs to, by the SAME
    /// exact-or-nothing (a named task's open effort) / single-open (unnamed)
    /// resolution the run auto-claim uses (tsk271). A named task is
    /// exact-or-nothing — never the single-open thread guess, which could claim a
    /// DIFFERENT task's effort. Used both to claim the run in the ledger AND to
    /// stamp `metric_capture.effort_id`, so the fact-attribution read
    /// (`captures_for_effort`, T-D) attributes the producer's facts (tsk37).
    /// The effort a run belongs to: the one its event was anchored to (so a
    /// late delivery keeps the effort it ran in, open or closed since), else
    /// — a live call with no event — the thread's single open effort.
    async fn run_effort(
        &self,
        thread: &ThreadId,
        cause: Option<&RunCause>,
    ) -> Result<Option<Effort>, DomainError> {
        match anchored_effort(cause) {
            Some(id) => self.efforts.get_effort(&id).await,
            None if cause.is_some() => Ok(None),
            None => self.efforts.find_single_open_for_thread(thread).await,
        }
    }

    async fn resolve_owning_effort(
        &self,
        thread: &ThreadId,
        task: Option<TaskId>,
    ) -> Option<Effort> {
        self.resolve_owning_effort_for_command(thread, task, None)
            .await
    }

    /// [`resolve_owning_effort`], plus the run's command when there is one.
    ///
    /// Order of precedence:
    /// 1. a named task — EXACT-or-nothing, unchanged;
    /// 2. exactly one open effort — the existing AUTO rule;
    /// 3. **several open, but the command names exactly one of them** — attribute
    ///    by target overlap (tsk169).
    ///
    /// (3) is what turns the common concurrent-effort case from "unattributed,
    /// reconcile later" into a correct answer at record time: `cargo test -p
    /// oxplow-git symlink` belongs to whichever open effort is working in
    /// `crates/oxplow-git/`. It never *guesses* — [`unique_best_by_targets`]
    /// requires a strict maximum, so ties and misses fall through to the
    /// unclaimed path exactly as before. The invariant is preserved: less
    /// exact, never wrong-exact.
    async fn resolve_owning_effort_for_command(
        &self,
        thread: &ThreadId,
        task: Option<TaskId>,
        command: Option<&str>,
    ) -> Option<Effort> {
        self.resolve_owner(thread, task, None, command).await
    }

    /// [`Self::resolve_owning_effort_for_command`] with the effort the
    /// command ran in (`anchored`, the tool event's effort anchor — the
    /// thread's single open effort then). It ranks after a named task and
    /// before the thread's open efforts now, so a run the reactor records
    /// after the effort closed still belongs to it.
    async fn resolve_owner(
        &self,
        thread: &ThreadId,
        task: Option<TaskId>,
        anchored: Option<EffortId>,
        command: Option<&str>,
    ) -> Option<Effort> {
        if task.is_none() {
            if let Some(id) = anchored {
                if let Ok(Some(e)) = self.efforts.get_effort(&id).await {
                    return Some(e);
                }
            }
        }
        if let Some(tid) = task {
            return self
                .efforts
                .find_open_for_work_item(&work_item_ref(tid))
                .await
                .ok()
                .flatten();
        }
        if let Some(single) = self
            .efforts
            .find_single_open_for_thread(thread)
            .await
            .ok()
            .flatten()
        {
            return Some(single);
        }
        let targets = crate::attribution::run_targets(command?);
        if targets.is_empty() {
            return None;
        }
        let open = self.efforts.list_open_for_thread(thread).await.ok()?;
        // Same shared decision the per-file auto-claim makes (tsk186) — one
        // implementation, so a run claim and a file claim can never disagree
        // about which effort owns the work.
        crate::attribution::resolve_by_targets(&self.efforts, &self.tasks, open, &targets).await
    }

    /// Claim `run:<id>` for an effort in the unified run ledger (best-effort — a
    /// ledger write error never fails the host path).
    async fn claim_run(&self, effort: &Effort, run_id: i64) {
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

    /// Run report collector `id` by hand (`collector.sync`, tsk863): read
    /// its report now, whenever it was written, and record what it parsed
    /// in `thread` — a test run, a coverage capture or a static-analysis
    /// capture — like a detected run's, with `collector.sync project/<id>`
    /// as the run's command. The run is recorded as the collector's.
    pub async fn sync_report_collector(
        &self,
        thread: &ThreadId,
        id: &str,
        source: &str,
    ) -> Result<ReportSync, crate::collector_runner::RunCollectorError> {
        use crate::collector_runner::RunCollectorError;
        let spec = self
            .report_collectors()
            .into_iter()
            .find(|s| s.id == id)
            .ok_or(RunCollectorError::NotFound)?;
        let how = ReportRun {
            trigger: "manual",
            source,
            cause: None,
        };
        let mut reads = RunReports::default();
        match self.read_report(&spec, None, &how).await {
            ReportRead::Parsed {
                output,
                exec,
                label,
            } => reads.add(output, exec, label),
            ReportRead::Failed(e) => return Err(RunCollectorError::Failed(e)),
            ReportRead::Disabled(m) => return Err(RunCollectorError::Disabled(m)),
            ReportRead::NeedsApproval(m) => return Err(RunCollectorError::NeedsApproval(m)),
            ReportRead::Missing(path) => {
                return Err(RunCollectorError::Failed(format!(
                    "collector `{id}`: report `{path}` isn't there; run what writes it first"
                )))
            }
            ReportRead::NotFresh => unreachable!("a by-hand run reads the report as it is"),
        }
        let command = format!("collector.sync project/{id}");
        let storage = RunCollectorError::Storage;
        Ok(match spec.records {
            Some(Records::Tests) => {
                let Some((report, source)) = reads.tests() else {
                    return Ok(ReportSync::Tests { run: None });
                };
                let run = self
                    .record_test_run(
                        thread,
                        &command,
                        None,
                        None,
                        None,
                        None,
                        None,
                        "observed",
                        &source,
                        Some(report),
                        None,
                    )
                    .await
                    .map_err(storage)?;
                ReportSync::Tests { run }
            }
            Some(Records::Coverage) => {
                let Some(stream_id) = self.stream_id_for(thread).await.map_err(storage)? else {
                    return Ok(ReportSync::Coverage(CoverageIngest::NoStream));
                };
                let Some((report, source)) = reads.coverage() else {
                    return Ok(ReportSync::Coverage(CoverageIngest::NoChangedCoverage));
                };
                ReportSync::Coverage(
                    self.observe_coverage(thread, &stream_id, report, &source, None)
                        .await
                        .map_err(storage)?,
                )
            }
            Some(Records::Analysis) | None => {
                let Some((report, source)) = reads.analysis() else {
                    return Ok(ReportSync::Analysis(AnalysisIngest::NotRecorded));
                };
                let (mut error_count, mut warning_count, mut info_count, mut note_count) =
                    (0u64, 0, 0, 0);
                for f in &report.findings {
                    use oxplow_coverage::Severity::*;
                    match f.severity {
                        Error => error_count += 1,
                        Warning => warning_count += 1,
                        Info => info_count += 1,
                        Note => note_count += 1,
                    }
                }
                // No baseline gate: findings are ABSOLUTE (current-file),
                // not diff-relative like coverage (tsk86).
                ReportSync::Analysis(
                    match self
                        .record_static_analysis(
                            thread,
                            &command,
                            Some(report),
                            &reads.analyzers,
                            &source,
                        )
                        .await
                        .map_err(storage)?
                    {
                        Some(observation_id) => AnalysisIngest::Stored {
                            observation_id,
                            error_count,
                            warning_count,
                            info_count,
                            note_count,
                            findings: report.findings.len(),
                        },
                        None => AnalysisIngest::NotRecorded,
                    },
                )
            }
        })
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
        vcs_rev: Option<&str>,
        snapshot_id: Option<i64>,
        detail_json: Option<&str>,
    ) -> Option<String> {
        let detail = detail_json?;
        let identity = format!(
            "{producer}|{}|{}|{detail}",
            vcs_rev.unwrap_or(""),
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
        vcs_rev: Option<String>,
        vcs_rev_exact: bool,
        detail: Option<serde_json::Value>,
        turn: Option<i64>,
    ) -> Option<i64> {
        let stream_val = oxplow_domain::StreamId::try_from_str(stream_id).map(|s| s.value())?;
        let branch = self.current_branch().await;
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
            // Stop-collecting gate (tsk31): skip the analysis capture entirely when
            // no enabled metric consumes `oxplow.lint_hit` (both `oxplow.analysis.*`
            // disabled). The code-quality findings store is written elsewhere and
            // is unaffected.
            if !self
                .facts
                .measure_has_active_spec("oxplow.lint_hit")
                .await
                .unwrap_or(true)
            {
                return Ok::<Option<i64>, DomainError>(None);
            }
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
            capture.closest_vcs_rev = vcs_rev.clone();
            capture.vcs_rev_exact = vcs_rev_exact;
            capture.branch = branch.clone();
            capture.effort_id = owning_val;
            capture.turn_id = turn;
            capture.detail_json = capture_detail_json;
            capture.idempotency_key = Self::ingest_idempotency_key(
                &analyzer,
                vcs_rev.as_deref(),
                snapshot_id,
                capture.detail_json.as_deref(),
            );
            let id = self.facts.record_facts(capture, facts).await?;
            Ok(Some(id))
        }
        .await;
        match dual {
            Ok(capture_id) => capture_id,
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
    /// The effort-relative diff-coverage is derived from these line-sets with
    /// the effort's evidence ([`diff_coverage_for_effort`]). Attribution rides the unified
    /// `"run"` ledger (auto-claimed when unambiguous, else reconciled/claimed).
    /// The id of a coverage sub-measure (`oxplow.coverage.branch`/`.function`,
    /// tsk123) IFF an enabled spec consumes it — else `None` (the per-measure
    /// stop-collecting gate). Lookup errors read as "not active" so a
    /// branch/function hiccup never sinks the line-coverage capture.
    async fn active_coverage_measure(&self, key: &str) -> Option<i64> {
        if !self
            .facts
            .measure_has_active_spec(key)
            .await
            .unwrap_or(false)
        {
            return None;
        }
        self.facts
            .get_measure(key)
            .await
            .ok()
            .flatten()
            .map(|m| m.id)
    }

    async fn observe_coverage(
        &self,
        thread: &ThreadId,
        stream_id: &str,
        report: &oxplow_coverage::CoverageReport,
        source: &str,
        cause: Option<&RunCause>,
    ) -> Result<CoverageIngest, DomainError> {
        // Stop-collecting gate (tsk31): with the coverage metric disabled, no
        // enabled spec consumes `oxplow.coverage` — record nothing (no capture, no
        // diff-coverage). Reads/effort-review see it turned off.
        if !self
            .facts
            .measure_has_active_spec("oxplow.coverage")
            .await
            .unwrap_or(true)
        {
            return Ok(CoverageIngest::NoChangedCoverage);
        }
        let Some((abs_pct, total_cov, total_instr, payload)) = coverage_abs_payload(report) else {
            return Ok(CoverageIngest::NoChangedCoverage);
        };

        // The owning effort stamps the coverage capture AND receives the ledger
        // claim below — the capture IS the run now (T-E1, tsk48).
        let attribute_to = self
            .resolve_owner(thread, None, anchored_effort(cause), None)
            .await;
        let owning_val = attribute_to.as_ref().map(|e| e.id.value());
        let turn = self.turn_of(thread, cause).await;
        // Pin to a take of the code the report measured (tsk883),
        // independent of any effort (observe-always).
        let pin = self
            .measured_snapshot(
                thread,
                stream_id,
                (turn, attribute_to.as_ref().map(|e| e.id)),
                cause,
            )
            .await;
        let version = match pin {
            Some(p) => {
                file_ref_version::resolve(&self.snapshots, &*self.vcs, &self.project_dir, p).await?
            }
            None => file_ref_version::ResolvedFileVersion {
                local_snapshot_id: 0,
                closest_vcs_rev: None,
                vcs_rev_exact: false,
            },
        };

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
                // Branch/function coverage (tsk123) ride the SAME capture as extra
                // per-file facts on their own measures — but only when an enabled
                // spec consumes them (the stop-collecting gate, per measure), and
                // only for files whose report actually carried the counts
                // (`*_found > 0` — a line-only report leaves them 0, which must not
                // read as "0% branch coverage").
                let branch_measure = self.active_coverage_measure("oxplow.coverage.branch").await;
                let function_measure = self
                    .active_coverage_measure("oxplow.coverage.function")
                    .await;
                let mut facts = Vec::new();
                for (path, fc) in &report.files {
                    let ratio_fact = |measure_id: i64, hit: u32, found: u32| NewFact {
                        numerator: Some(hit as f64),
                        denominator: Some(found as f64),
                        subject_kind: Some("file".into()),
                        subject_ref: Some(format!("file:{path}")),
                        path: Some(path.clone()),
                        ..NewFact::new(measure_id, hit as f64 / found as f64 * 100.0)
                    };
                    if let Some(bm) = branch_measure {
                        if fc.branches_found > 0 {
                            facts.push(ratio_fact(bm, fc.branches_hit, fc.branches_found));
                        }
                    }
                    if let Some(fm) = function_measure {
                        if fc.functions_found > 0 {
                            facts.push(ratio_fact(fm, fc.functions_hit, fc.functions_found));
                        }
                    }
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
                let branch = self.current_branch().await;
                let snapshot_id =
                    (version.local_snapshot_id != 0).then_some(version.local_snapshot_id);
                let mut capture = NewMetricCapture::done(stream_val, "coverage", source);
                capture.thread_id = Some(thread.value());
                capture.trigger = Some("on-report".into());
                capture.snapshot_id = snapshot_id;
                capture.closest_vcs_rev = version.closest_vcs_rev.clone();
                capture.vcs_rev_exact = version.vcs_rev_exact;
                capture.basis_ref = version.closest_vcs_rev.clone();
                capture.branch = branch;
                capture.effort_id = owning_val;
                capture.turn_id = turn;
                capture.detail_json = Self::capture_detail("coverage-detail", &payload);
                capture.idempotency_key = Self::ingest_idempotency_key(
                    "coverage",
                    version.closest_vcs_rev.as_deref(),
                    snapshot_id,
                    capture.detail_json.as_deref(),
                );
                let log = self.coverage_event(
                    thread,
                    stream_val,
                    (owning_val, turn),
                    cause,
                    abs_pct,
                    source,
                );
                let id = self
                    .facts
                    .record_facts_logged(capture, facts, Some(log))
                    .await?;
                Ok(Some(id))
            }
            .await;
            match dual {
                Ok(id) => capture_id = id,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to write the coverage capture")
                }
            }
        }
        // ATTRIBUTE via the unified run ledger (the capture id is the ref), then
        // refresh the panel for the effort it landed on (if any).
        if let (Some(cid), Some(effort)) = (capture_id, attribute_to.as_ref()) {
            self.claim_run(effort, cid).await;
        }

        Ok(CoverageIngest::Stored {
            observation_id: 0,
            summary_pct: abs_pct,
            changed_lines: total_instr,
            covered_lines: total_cov,
        })
    }

    /// The snapshot a run's coverage measured (tsk883): a take of the
    /// stream's worktree now, as the run's report is recorded. The take is
    /// the code the run measured only while nothing changed after the run
    /// ended, so a run delivered late (a file in the take written after
    /// the run's `cause.at`) gets no pin — no diff rather than a wrong one.
    /// An explicit ingest (no cause) is of the code as it stands.
    async fn measured_snapshot(
        &self,
        thread: &ThreadId,
        stream_id: &str,
        (turn, effort): (Option<i64>, Option<EffortId>),
        cause: Option<&RunCause>,
    ) -> Option<i64> {
        let stream = oxplow_domain::StreamId::try_from_str(stream_id)?;
        let capture = self.captures.get(&stream)?;
        capture.await_initial_ready().await;
        let taken = capture
            .request_snapshot(crate::snapshot_capture::TakeRequest {
                trigger: oxplow_domain::snapshot::SnapshotTrigger::Coverage,
                thread_id: Some(*thread),
                turn_id: turn,
                effort_id: effort,
                budget: None,
            })
            .await;
        let id = match taken {
            Ok(id) => id?,
            Err(e) => {
                tracing::warn!(error = %e, "coverage: the measured snapshot failed");
                return None;
            }
        };
        if let Some(cause) = cause {
            let ended = cause.at.unix_ms();
            let tree = self.snapshots.tree_at(id).await.ok()?;
            if tree
                .values()
                .any(|entry| entry.mtime_ms.is_some_and(|m| m > ended))
            {
                return None;
            }
        }
        Some(id)
    }

    /// Derive the effort-relative **diff-coverage** from a run's stored ABSOLUTE
    /// per-file line-sets (tsk270): the lines that changed between the effort's
    /// start snapshot and `measured`, the snapshot the run's capture is pinned
    /// to — the code the run measured, never a working tree (tsk862), so it
    /// holds for a worktree stream and doesn't drift after the run. Computed
    /// when the evidence is (`effort_evidence`), so a run claimed after the
    /// effort closed still produces a diff. Returns `(summary_pct,
    /// diff_payload)`, or `None` when either snapshot is unknown, a side's
    /// bytes are gone, or no changed instrumented lines overlap.
    /// `diff_payload` is the shape the panel renders.
    async fn diff_coverage_for_effort(
        &self,
        effort: &Effort,
        measured: Option<i64>,
        abs_payload: &serde_json::Value,
    ) -> Result<Option<(f64, serde_json::Value)>, DomainError> {
        let (Some(start), Some(measured)) = (effort.start_snapshot_id, measured) else {
            return Ok(None);
        };
        let start_tree = self.snapshots.tree_at(start).await?;
        let measured_tree = self.snapshots.tree_at(measured).await?;
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
            let Some(changed) = self.changed_lines_between(path, &start_tree, &measured_tree)
            else {
                return Ok(None);
            };
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

    /// One coverage ride-along attempt: resolve the thread's stream and record
    /// the absolute report. `Ok` when the thread has no stream — nothing to do.
    async fn try_observe_coverage(
        &self,
        thread: &ThreadId,
        report: &oxplow_coverage::CoverageReport,
        source: &str,
        cause: Option<&RunCause>,
    ) -> Result<(), DomainError> {
        if let Some(stream_id) = self.stream_id_for(thread).await? {
            self.observe_coverage(thread, &stream_id, report, source, cause)
                .await?;
        }
        Ok(())
    }

    /// The `test.coverage.recorded` a coverage capture logs with it.
    fn coverage_event(
        &self,
        thread: &ThreadId,
        stream_val: i64,
        (owning, turn): (Option<i64>, Option<i64>),
        cause: Option<&RunCause>,
        lines_pct: f64,
        source: &str,
    ) -> oxplow_db::fact_store::CaptureEvent {
        use oxplow_domain::events::schema::{TestCoverageRecorded, TestCoverageRecordedV1};
        let anchors = match cause {
            Some(c) => oxplow_domain::Anchors {
                effort_id: owning.map(EffortId::new).or(c.anchors.effort_id),
                ..c.anchors.clone()
            },
            None => oxplow_domain::Anchors {
                stream_id: Some(oxplow_domain::StreamId::new(stream_val)),
                thread_id: Some(*thread),
                effort_id: owning.map(EffortId::new),
                turn_id: turn,
                ..oxplow_domain::Anchors::default()
            },
        };
        let subject: Vec<String> = anchors
            .effort_id
            .map(oxplow_domain::refs::build::effort_ref)
            .into_iter()
            .collect();
        let cause_id = cause.map(|c| oxplow_domain::EventId(c.event_id.clone()));
        let dedupe = cause.map(|c| format!("coverage:{}", c.event_id));
        let source = source.to_string();
        oxplow_db::fact_store::CaptureEvent {
            vocabulary: self.vocabulary.clone(),
            build: Box::new(move |capture_id| {
                let env = oxplow_domain::Envelope::typed::<TestCoverageRecorded>(
                    oxplow_domain::refs::build::system_source("collection"),
                    &TestCoverageRecordedV1 {
                        capture: capture_id,
                        lines_pct: Some(lines_pct),
                        branches_pct: None,
                        source: source.clone(),
                    },
                )
                .with_anchors(anchors.clone())
                .with_subject(subject.clone())
                .with_dedupe_key_opt(dedupe.clone());
                match &cause_id {
                    Some(c) => env.with_cause(c.clone()),
                    None => env,
                }
            }),
        }
    }

    /// The coverage leg with one retry (tsk79). The detached collection task
    /// runs right after a test command — DB contention or a snapshot-lookup
    /// hiccup is transient, and without a retry one swallowed error means the
    /// run's coverage simply never exists (fatal when it was the effort's LAST
    /// run before close). When both attempts lose, the miss is made durable
    /// via [`Self::record_coverage_failure`].
    async fn coverage_ride_along_with_retry(
        &self,
        thread: &ThreadId,
        report: &oxplow_coverage::CoverageReport,
        source: &str,
        cause: Option<&RunCause>,
    ) {
        let first = match self
            .try_observe_coverage(thread, report, source, cause)
            .await
        {
            Ok(()) => return,
            Err(e) => e,
        };
        tracing::warn!(error = %first, "coverage ride-along failed; retrying once");
        tokio::time::sleep(COVERAGE_RETRY_DELAY).await;
        if let Err(e) = self
            .try_observe_coverage(thread, report, source, cause)
            .await
        {
            tracing::warn!(error = %e, "coverage ride-along failed after retry");
            self.record_coverage_failure(thread, &format!("{first}; retry: {e}"), cause)
                .await;
        }
    }

    /// Durable visibility for a dropped coverage leg (tsk79): a facts-empty
    /// `status = failed` coverage capture carrying the error — the same
    /// convention as gauge failures — so the miss is queryable in the
    /// substrate instead of living only in a tty warn. Best-effort.
    async fn record_coverage_failure(
        &self,
        thread: &ThreadId,
        error: &str,
        cause: Option<&RunCause>,
    ) {
        let stream_val = match self.stream_id_for(thread).await {
            Ok(Some(sid)) => match oxplow_domain::StreamId::try_from_str(&sid) {
                Some(s) => s.value(),
                None => return,
            },
            // Can't resolve even the stream — the warn above is all we have.
            _ => return,
        };
        let mut capture = NewMetricCapture::done(stream_val, "coverage", "coverage-report");
        capture.status = "failed".into();
        capture.error = Some(error.to_string());
        capture.thread_id = Some(thread.value());
        capture.trigger = Some("on-report".into());
        capture.turn_id = self.turn_of(thread, cause).await;
        capture.idempotency_key = cause.map(|c| format!("coverage-failure:{}", c.event_id));
        if let Err(e) = self.facts.record_facts(capture, Vec::new()).await {
            tracing::warn!(error = %e, "coverage failure record write failed");
        }
    }

    /// The revert/waste signal (tsk77): read HEAD's `This reverts commit
    /// <sha>` trailers; for each reverted commit that falls inside exactly ONE
    /// closed effort of this thread's stream (the ambiguity stance — 0 or >1
    /// candidates → no attribution), emit an `oxplow.token_waste` fact carrying
    /// the effort's whole token spend as a numerator-only ratio row
    /// (num = spend, den = 0; the close-side row carried num 0 / den = spend),
    /// so Σn/Σd across the measure is the wasted share. Idempotent per effort:
    /// a second revert touching the same effort is the same waste. V1
    /// granularity is deliberately coarse — one reverted commit flags the
    /// effort's FULL spend, over-counting multi-commit efforts where only part
    /// was rolled back.
    async fn record_token_waste_for_reverts(&self, thread: &ThreadId) -> Result<(), DomainError> {
        if !self
            .facts
            .measure_has_active_spec("oxplow.token_waste")
            .await
            .unwrap_or(true)
        {
            return Ok(());
        }
        let Some(waste_measure) = self.facts.get_measure("oxplow.token_waste").await? else {
            return Ok(());
        };
        let Some(effort_tokens_measure) = self.facts.get_measure("oxplow.effort_tokens").await?
        else {
            return Ok(());
        };
        // HEAD is the just-landed (revert) commit; its body carries the trailers.
        let Some(head_sha) = self
            .vcs
            .head(&self.project_dir)
            .await
            .ok()
            .and_then(|h| h.revision)
        else {
            return Ok(());
        };
        let Ok(Some(head)) = self.vcs.revision(&self.project_dir, &head_sha).await else {
            return Ok(());
        };
        let shas = parse_reverted_shas(&head.body);
        if shas.is_empty() {
            return Ok(());
        }
        let Some(thread_row) = self.threads.get(thread).await? else {
            return Ok(());
        };
        let stream_val = thread_row.stream_id.value();
        for sha in shas {
            let Ok(Some(reverted)) = self.vcs.revision(&self.project_dir, &sha).await else {
                continue;
            };
            // git timestamps are SECONDS-granular — the commit happened
            // somewhere inside [secs, secs+1), so the containment window must
            // span that whole second or a commit made in the same wall-second
            // an effort started would truncate to before it.
            let at_start = oxplow_domain::Timestamp::from_unix_ms(reverted.info.time * 1000);
            let at_end = oxplow_domain::Timestamp::from_unix_ms(reverted.info.time * 1000 + 999);
            // Closed efforts whose window contains the reverted commit, scoped
            // to this stream (commits are per-worktree = per-stream).
            let mut owners = Vec::new();
            for e in self.efforts.list_in_window(at_start, at_end).await? {
                if e.ended_at.is_none() {
                    continue; // still open — its spend isn't final (and it's us)
                }
                match self.threads.get(&e.thread_id).await {
                    Ok(Some(t)) if t.stream_id.value() == stream_val => owners.push(e),
                    _ => {}
                }
            }
            let [owner] = owners.as_slice() else {
                continue; // 0 or ambiguous → no attribution
            };
            let owner_ref = owner.id.to_string();
            let spend: f64 = self
                .facts
                .facts_for_measure_in_stream(effort_tokens_measure.id, stream_val)
                .await?
                .iter()
                .filter(|f| f.subject_ref.as_deref() == Some(owner_ref.as_str()))
                .map(|f| f.value)
                .sum();
            if spend <= 0.0 {
                continue; // unmetered effort — no waste to count
            }
            let mut capture = NewMetricCapture::done(stream_val, "revert-detect", "git");
            capture.thread_id = Some(thread.value());
            capture.trigger = Some("on-commit".into());
            capture.basis_ref = Some(head_sha.clone());
            capture.idempotency_key = Some(format!("token-waste:{}", owner.id.value()));
            let fact = NewFact {
                subject_kind: Some("effort".into()),
                subject_ref: Some(owner_ref),
                numerator: Some(spend),
                denominator: Some(0.0),
                ..NewFact::new(waste_measure.id, spend)
            };
            self.facts.record_facts(capture, vec![fact]).await?;
        }
        Ok(())
    }

    /// PostToolUse entry point: detect a test and/or static-analysis run,
    /// record it, and ride along to coverage / findings. The collection
    /// reactor runs it (P3.6) with `cause`, the `agent.tool.finished`
    /// event — every capture and nudge
    /// is keyed by it and anchored to its turn, and the effort the command
    /// ran in owns what it records. The nudge is persisted (the hook
    /// response picks it up); the returned text is for callers that want it.
    pub async fn on_post_tool_use(
        &self,
        thread: &ThreadId,
        payload_json: &str,
        cause: Option<&RunCause>,
    ) -> Result<Option<String>, DomainError> {
        let Some(bash) = parse_bash_post_tool(payload_json) else {
            return Ok(None);
        };
        let cfg = self.testing_cfg();
        // The project's OWN configured commands count as test-run patterns
        // without having to be restated in `runPatterns` — if you declared
        // it as the way to run tests, running it is a test run. This is what
        // makes `fastCommand` (tsk171) detectable when its script name
        // doesn't happen to contain a built-in pattern like `cargo test`.
        let mut test_patterns = cfg.run_patterns.clone();
        test_patterns.extend(cfg.command.clone());
        test_patterns.extend(cfg.fast_command.clone());
        let is_test = detect_test_run(&bash.command, &test_patterns);
        let is_analysis = detect_analysis_run(&bash.command, &cfg.analysis_patterns);
        let is_commit = detect_git_commit(&bash.command);
        let is_revert = detect_git_revert(&bash.command);
        if !is_test && !is_analysis && !is_commit && !is_revert {
            return Ok(None);
        }
        // OBSERVE-ALWAYS (tsk269): tests + analysis are recorded regardless of how
        // many efforts are open — attribution is deferred to the ledger, never a
        // precondition for recording. The single open effort (if any) is resolved
        // here only for the effort-RELATIVE advisories (coverage, nudges),
        // which legitimately no-op when ambiguous.
        let effort_opt = self.run_effort(thread, cause).await?;

        // Wasted-token leg (tsk77): any LANDED commit — including `git revert`,
        // which never says "commit" — may carry "This reverts commit <sha>"
        // trailers pointing into a closed effort's window. Best-effort, before
        // the pure-commit early return below.
        if (is_commit || is_revert) && !matches!(bash.exit_code, Some(c) if c != 0) {
            if let Err(e) = self.record_token_waste_for_reverts(thread).await {
                tracing::warn!(error = %e, "token-waste leg failed");
            }
        }
        // A commit gets no advisory of its own (tsk250). Attribution nudges
        // exist to sort out which changes belong to which effort while several
        // run concurrently; by commit time one actor has already decided what
        // to include, and second-guessing that is oxplow getting in the way.
        //
        // A pure commit/revert (not also a test/analysis run) is done.
        if (is_commit || is_revert) && !is_test && !is_analysis {
            return Ok(None);
        }
        // Reports this run could have written: judged at the run's own
        // time, so a redelivery (a crash before the checkpoint, a retried
        // dead letter, a pump backlog) sees what the first delivery saw.
        let window = cause.map_or_else(FreshWindow::ending_now, |c| FreshWindow::around(c.at));

        // Static-analysis ride-along (OBSERVE-ALWAYS): when an analyzer ran,
        // record a static-analysis observation — command-only (the ran-record)
        // when none of its report collectors read a report it wrote, or
        // carrying their merged findings.
        if is_analysis {
            let reads = self
                .read_run_reports(RunKind::Analysis, window, cause)
                .await;
            let (report, source) = match reads.analysis() {
                Some((r, source)) => (Some(r.clone()), source),
                None => (None, "analysis-report".to_string()),
            };
            let analyzers = reads.analyzers.clone();
            // Leg isolation (tsk79): one leg's transient error must not kill
            // the legs after it — the test-run + coverage recording below is
            // independent of whether this analysis write landed.
            if let Err(e) = self
                .record_static_analysis_caused(
                    thread,
                    &bash.command,
                    report.as_ref(),
                    &analyzers,
                    &source,
                    cause,
                )
                .await
            {
                tracing::warn!(error = %e, "analysis ride-along failed");
            }
        }

        // A pure analysis run (no test patterns matched) is done — the
        // test-run / coverage / nudge path below is test-specific.
        if !is_test {
            return Ok(None);
        }
        // The test run's report collectors, each reading the report it wrote
        // (each test stack writes its own; the freshness window leaves out
        // stale ones from prior runs or other stacks), merged by kind.
        let reads = self.read_run_reports(RunKind::Test, window, cause).await;
        // Trust tier rides in `source`: "post-tool-bash" for the plain hook /
        // in-process parsers, "plugin-exec:<ids>" when a lower-trust program
        // parser produced the suites (mirrors the coverage path).
        let (report, source) = match reads.tests() {
            Some((r, source)) => (Some(r.clone()), source),
            None => (None, "post-tool-bash".to_string()),
        };
        // Leg isolation (tsk79): a failed test-run write still lets the
        // coverage below record — the report on disk is real either way.
        if let Err(e) = self
            .record_test_run_caused(
                thread,
                &bash.command,
                bash.exit_code,
                (None, None, None, None),
                "observed",
                &source,
                report.as_ref(),
                // Exact attribution when the agent prefixed `OXPLOW_TASK=<id>`
                // (find_open_for_task — survives concurrent efforts); then the
                // effort the command ran in; otherwise the single-open auto
                // rule attributes it (tsk265/tsk271).
                parse_task_token(&bash.command),
                cause,
            )
            .await
        {
            tracing::warn!(error = %e, "test-run ride-along failed");
        }
        // Coverage ride-along (OBSERVE-ALWAYS, tsk270): record the ABSOLUTE
        // report regardless of effort; the effort-relative diff is derived with
        // the effort's evidence (it lands in v_effort_observation,
        // where oxplow-analytics' coverage-target advisory reads it).
        // A transient error here used to silently drop the run's coverage
        // (tsk79) — now it retries once and, when both attempts (or the parse
        // of a fresh report) lose, records a durable `failed` capture.
        let coverage = reads.coverage();
        if let Some((merged, source)) = &coverage {
            // The label says whether a lower-trust program parser produced it.
            self.coverage_ride_along_with_retry(thread, merged, source, cause)
                .await;
        } else if !reads.coverage_errors.is_empty() {
            self.record_coverage_failure(thread, &reads.coverage_errors.join("; "), cause)
                .await;
        }
        // A run delivered after its freshness window can't be judged: the
        // agent has moved on, and its reports may since have been rewritten.
        // Record it (above), advise nothing.
        if cause.is_some_and(|c| {
            oxplow_domain::Timestamp::now().unix_ms() - c.at.unix_ms() > REPORT_FRESH_WINDOW_MS
        }) {
            return Ok(None);
        }
        // Nudges below are effort-RELATIVE (key/dedup per effort), so they only
        // run with a single open effort. The runs above are already recorded.
        let Some(effort) = effort_opt else {
            // No single open effort — so this test run may have landed
            // unattributed. Say so NOW, while a one-token fix is available on the
            // next command, rather than leaving it for the closing EFFORT REVIEW
            // to reconcile in bulk long after the context is gone (tsk170).
            //
            // Only when the run is genuinely unattributed: an `OXPLOW_TASK=`
            // token or a target-overlap match (tsk169) resolves most of these
            // silently, and nagging about a run that WAS attributed would train
            // the agent to ignore the nudge.
            if is_test && parse_task_token(&bash.command).is_none() {
                let resolved = self
                    .resolve_owner(thread, None, anchored_effort(cause), Some(&bash.command))
                    .await;
                if resolved.is_none() {
                    let open = self
                        .efforts
                        .list_open_for_thread(thread)
                        .await
                        .unwrap_or_default();
                    if open.len() > 1 {
                        let msg = unattributed_run_message(&bash.command, &open);
                        self.persist_nudge(
                            thread,
                            None,
                            "unattributed-run",
                            &msg,
                            &bash.command,
                            cause,
                        )
                        .await;
                        return Ok(Some(msg));
                    }
                }
            }
            return Ok(None);
        };
        // Nudge: the agent ran tests but this run regenerated no report
        // oxplow could parse for the effort (so the effort gets a
        // command-only test-run and no coverage). Steer it to the
        // report-emitting command — at most once per effort. This is
        // tool-agnostic: it keys only on "test run detected" + "no fresh
        // report", never on which tool ran; the command it names comes
        // from the project's own config.
        let produced_report = report.is_some() || coverage.is_some();
        if !produced_report && self.mark_nudged(&effort.id).await {
            let msg =
                report_nudge_message(&cfg, !self.report_collectors().is_empty(), &bash.command);
            self.persist_nudge(
                thread,
                Some(&effort),
                "report-less-run",
                &msg,
                &bash.command,
                cause,
            )
            .await;
            return Ok(Some(msg));
        }
        Ok(None)
    }

    /// Persist a fired nudge (best-effort; the renderer re-reads
    /// `v_agent_nudge` when it lands). Called only AFTER the
    /// one-shot dedup gates pass, so a deduped/non-fired nudge is never
    /// stored. Never fails the hook: a persistence error is logged and
    /// swallowed.
    pub(crate) async fn persist_nudge(
        &self,
        thread: &ThreadId,
        effort: Option<&Effort>,
        kind: &str,
        message: &str,
        trigger: &str,
        cause: Option<&RunCause>,
    ) {
        let new = NewAgentNudge {
            thread_id: thread.to_string(),
            effort_id: effort.map(|e| e.id.to_string()),
            kind: kind.to_string(),
            message: message.to_string(),
            trigger: Some(trigger.to_string()),
            turn_id: cause.and_then(|c| c.anchors.turn_id),
            cause: cause.map(|c| c.event_id.clone()),
        };
        match self.nudges.record(new).await {
            // Already fired for this cause (a redelivered event).
            Ok(None) => {}
            Ok(Some(_)) => {
                // Project the fired nudge into the metric substrate (tsk216):
                // `agent.nudges.fired` is an agent-activity signal — the agent
                // drifted off-task often enough to be corrected.
                self.project_nudge_metric(thread, effort.map(|e| e.id), kind, cause)
                    .await;
            }
            Err(err) => tracing::warn!(?err, "persisting agent nudge failed"),
        }
    }

    /// Project one `agent.nudges.fired` event sample into the unified
    /// substrate. The nudge `kind` is the subject (so the explorer can break
    /// down which guardrail fired). Event kind → run-less. Best-effort: a
    /// metric write error is logged and never fails the hook. Lower is
    /// better — fewer nudges means the agent stayed on task.
    async fn project_nudge_metric(
        &self,
        thread: &ThreadId,
        effort: Option<EffortId>,
        kind: &str,
        cause: Option<&RunCause>,
    ) {
        let stream_val = match self.threads.get(thread).await {
            Ok(Some(t)) => t.stream_id.value(),
            _ => return,
        };
        let branch = self.current_branch().await;
        let result = async {
            // Stop-collecting gate (tsk31): skip when the `agent.nudges.fired`
            // metric is disabled (nothing consumes `oxplow.nudge`). The nudge
            // itself still fires + persists — only the analytics fact is skipped.
            if !self
                .facts
                .measure_has_active_spec("oxplow.nudge")
                .await
                .unwrap_or(true)
            {
                return Ok::<(), DomainError>(());
            }
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
                // The nudge's effort, else the one its run was anchored to;
                // only a live call with neither resolves one now.
                let owning = match effort.or_else(|| anchored_effort(cause)) {
                    Some(id) => Some(id),
                    None if cause.is_some() => None,
                    None => self.resolve_owning_effort(thread, None).await.map(|e| e.id),
                };
                let owning_val = owning.map(|e| e.value());
                let mut capture = NewMetricCapture::done(stream_val, "nudges", "nudges");
                capture.thread_id = Some(thread.value());
                capture.trigger = Some("continuous".into());
                capture.branch = branch;
                capture.effort_id = owning_val;
                capture.turn_id = self.turn_of(thread, cause).await;
                self.facts.record_facts(capture, vec![fact]).await?;
            }
            Ok::<(), DomainError>(())
        }
        .await;
        if let Err(e) = result {
            tracing::warn!(error = %e, "failed to project nudge into metric substrate")
        }
    }

    /// Record that `effort` has been nudged about a report-less run.
    /// `true` the first time (caller should nudge), `false` afterwards —
    /// durably (`effort_once_mark`), so a restart doesn't repeat it. A
    /// failed write reads as "already nudged": don't nag on an error.
    async fn mark_nudged(&self, effort: &EffortId) -> bool {
        self.nudges
            .claim_once(effort.value(), "report-less-run")
            .await
            .unwrap_or(false)
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
        self.record_static_analysis_caused(thread, command, report, analyzers, source, None)
            .await
    }

    /// [`Self::record_static_analysis`] for a run the reactor saw: the effort
    /// the command ran in owns it. (A redelivery is already safe: only a
    /// parsed report writes a capture, keyed by its content.)
    async fn record_static_analysis_caused(
        &self,
        thread: &ThreadId,
        command: &str,
        report: Option<&oxplow_coverage::AnalysisReport>,
        analyzers: &[String],
        source: &str,
        cause: Option<&RunCause>,
    ) -> Result<Option<i64>, DomainError> {
        let Some(stream_id) = self.stream_id_for(thread).await? else {
            return Ok(None);
        };
        // The run's effort — for the snapshot pin + panel refresh only;
        // attribution rides the ledger (auto-claimed below when unambiguous).
        let effort = self.run_effort(thread, cause).await?;
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
        let (local_snapshot_id, closest_vcs_rev, vcs_rev_exact) = match pin {
            Some(p) => {
                let v =
                    file_ref_version::resolve(&self.snapshots, &*self.vcs, &self.project_dir, p)
                        .await?;
                (
                    Some(v.local_snapshot_id),
                    v.closest_vcs_rev,
                    v.vcs_rev_exact,
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
                closest_vcs_rev.clone(),
                vcs_rev_exact,
                Some(serde_json::Value::Object(payload.clone())),
                self.turn_of(thread, cause).await,
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
            closest_vcs_rev,
            vcs_rev_exact,
        );
        // Attribute the run via the unified ledger.
        if let Some(rid) = run_id {
            let owner = self
                .resolve_owner(thread, None, anchored_effort(cause), Some(command))
                .await;
            if let Some(effort) = owner.as_ref() {
                self.claim_run(effort, rid).await;
            }
        }
        Ok(Some(0))
    }

    /// Reconstruct the effort-review observations for `effort_id` from the
    /// **metric substrate** (tsk215): the coverage/test/analysis headline
    /// samples that fall in the effort's time window + their verbatim
    /// `*-detail` finding payloads, shaped as `EffortObservation` rows so the
    /// effort panel renders off the model. Computed only for the
    /// `effort_evidence` asset, which stores the rows
    /// (`v_effort_observation`); readers read those (tsk862).
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
                            .diff_coverage_for_effort(eff, c.snapshot_id, &payload)
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
                kind: obs_kind.to_string(),
                provenance: c.provenance.clone(),
                source: c.source.clone(),
                metric_value,
                payload_json,
                local_snapshot_id: c.snapshot_id,
                created_at: c.captured_at,
            });
        }
        out
    }

    /// Roll every metric up over a single effort for the task/effort page — the
    /// structured sibling of the oxplow-analytics `metric-deltas` advisory
    /// (which builds the agent-prompt text). Reads the spec catalog and, per
    /// family, aggregates the effort's own facts (epic tsk12, T-D; see metrics.md):
    /// - **per-file gauges** (`File`): Σ over the effort's *claimed* files
    ///   (`effort_file`) of `(current − baseline)` fact value — the slice this
    ///   effort actually moved, even on a branch shared with another effort. With
    ///   no claimed files it falls back to the repo-wide before→after.
    /// - **run + operational** (`Run`/`Window`): before→after (or `sum` flow) over
    ///   the facts of the effort's OWN captures (`metric_capture.effort_id`,
    ///   stamped at ingest — tsk37), so overlapping efforts stay disjoint.
    /// - **coverage** (`Coverage`): effort-relative diff derived from each run's
    ///   detail payload (line-sets aren't facts — a documented special case).
    ///
    /// Returns only metrics the effort moved/touched, grouped code-health →
    /// coverage → tests → operational, then by title.
    /// Recompute `effort_id`'s metric deltas and observations and store them
    /// (`v_effort_metric_delta`, `v_effort_observation`), so lenses can read
    /// what only the engine can compute.
    pub async fn refresh_effort_evidence(
        &self,
        effort_id: &str,
        store: &oxplow_db::SqliteEffortEvidenceStore,
    ) -> Result<(), DomainError> {
        let row_id = EffortId::try_from_str(effort_id)
            .ok_or_else(|| DomainError::Invalid(format!("not an effort id: {effort_id}")))?
            .value();
        let deltas = self.effort_metric_deltas(effort_id).await;
        let observations = self.effort_observations_from_metrics(effort_id, None).await;
        store.replace_metric_deltas(row_id, deltas).await?;
        store.replace_observations(row_id, observations).await
    }

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
            // Entity metrics (tsk322) measure the project's data, not an
            // effort's work; they have no effort-scoped delta.
            if spec.entity_json.is_some() {
                continue;
            }
            // One classifier (in `attribution.rs`, beside the write-side
            // `AttributionKind` each family maps to) decides the family; this match
            // is the only place each family's read computation is named (tsk274).
            let row = match classify_effort_attribution(spec) {
                EffortAttributionFamily::File => {
                    self.file_delta_from_facts(spec, &effort, &claimed, stream, &mut fact_cache)
                        .await
                }
                // Coverage stays effort-relative + on the legacy detail payload
                // (line-sets aren't in facts yet) — derive the diff from it via
                // the spec's legacy definition (tsk270, T-D scope guard).
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
        effort: &Effort,
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
        // Load this measure's history once per `effort_metric_deltas` call and
        // reuse it across every spec sharing the measure (tsk17) — bounded to
        // the effort's stream SQL-side when known (tsk75): the delta is
        // per-worktree by definition, so other streams' rows are pure load.
        let facts = match fact_cache.get(&measure.id) {
            Some(cached) => cached.clone(),
            None => {
                let loaded = std::sync::Arc::new(match stream {
                    Some(s) => self
                        .facts
                        .facts_for_measure_in_stream(measure.id, s)
                        .await
                        .ok()?,
                    None => self.facts.facts_for_measure(measure.id).await.ok()?,
                });
                fact_cache.insert(measure.id, loaded.clone());
                loaded
            }
        };
        let kept: Vec<&oxplow_db::FactRow> = facts
            .iter()
            .filter(|f| filter.matches(f))
            .filter(|f| stream.is_none_or(|s| f.stream_id == s))
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
                if stream.is_none_or(|s| c.stream_id == s)
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
        // Repo total AS OF a capture.
        //
        // For a `complete` measure a capture restates the whole population, so the
        // repo total in it is just its own kept facts. For a **per-path** measure a
        // capture is a DELTA — summing its own facts would call "the 8 files this
        // commit touched" the repo, which is the bug tsk41 fixes in the large. The
        // repo total as of a capture is the folded tree state at that point, which is
        // exactly what `tree_state_series` yields per capture, so we index it by
        // capture id.
        let per_path = measure.capture_scope == "per-path";
        let tree_totals: std::collections::HashMap<i64, f64> = if per_path {
            let cap_list = self
                .facts
                .captures_for_producers(
                    kept.iter()
                        .map(|f| f.producer.clone())
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect(),
                )
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|c| stream.is_none_or(|s| c.stream_id == s))
                .collect::<Vec<_>>();
            let mut scanned: std::collections::HashMap<i64, Vec<String>> = Default::default();
            if let Ok(rows) = self
                .facts
                .scanned_paths_for_captures(cap_list.iter().map(|c| c.id).collect())
                .await
            {
                for (cid, path) in rows {
                    scanned.entry(cid).or_default().push(path);
                }
            }
            let visibility = self.metric_visibility.for_captures(&cap_list).await;
            crate::metric_engine::tree_state_series(
                &cap_list,
                &facts,
                &scanned,
                &crate::metric_engine::FoldRead {
                    agg,
                    filter: &filter,
                    group_by: None,
                    visibility: &visibility,
                    // This branch IS the per-path arm (`per_path` guard above).
                    scope: crate::metric_engine::CaptureScope::PerPath,
                },
            )
            .into_iter()
            .map(|p| (p.capture_id, p.value))
            .collect()
        } else {
            Default::default()
        };
        let repo_total = |cap: Option<i64>| -> f64 {
            match cap {
                Some(c) if per_path => tree_totals.get(&c).copied().unwrap_or(0.0),
                Some(c) => kept
                    .iter()
                    .filter(|f| f.capture_id == c)
                    .map(contribution)
                    .sum(),
                None => 0.0,
            }
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
                // Which producers emit this metric's slice, answered as cheaply
                // as the filter allows. All three branches agree; they differ
                // only in how much of the measure's history they have to touch,
                // and on a 900k-fact measure that gap was 38% of backend CPU
                // (tsk239).
                producers = if filter.is_unconstrained() {
                    // Nothing is filtered out, so "producers of the matching
                    // slices" IS "producers of the measure" — memoized (tsk153),
                    // no scan at all.
                    self.facts
                        .producers_for_measure(measure.id)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .collect()
                } else if filter.slice_key_only() {
                    // The predicate reads only (rule, severity, dims_json), and
                    // every fact of a slice agrees on those — so the slice key
                    // decides it and no representative row is needed. Same
                    // scan, ~half the cost: 4 columns through the sorter, no
                    // join-back.
                    self.facts
                        .distinct_slice_keys(measure.id)
                        .await
                        .unwrap_or_default()
                        .iter()
                        .filter(|k| filter.matches_slice(k))
                        .map(|k| k.producer.clone())
                        .collect()
                } else {
                    // The predicate reads a column outside the slice key
                    // (`value`, or a `package`/`branch`/`subject`/`model` dim),
                    // so it needs a real fact: one representative — the lowest-id
                    // member — per slice (tsk75).
                    self.facts
                        .representative_facts_by_slice(measure.id)
                        .await
                        .unwrap_or_default()
                        .iter()
                        .filter(|f| filter.matches(f))
                        .map(|f| f.producer.clone())
                        .collect()
                };
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
                        vcs_rev: c.closest_vcs_rev.clone(),
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
    /// capture's ABSOLUTE per-file line-sets (`detail_json`) against the
    /// snapshot it measured, then before→after over the derived sequence.
    async fn coverage_delta_for_spec(
        &self,
        spec: &oxplow_db::MetricSpec,
        effort: &Effort,
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
                if let Ok(Some((pct, _))) = self
                    .diff_coverage_for_effort(effort, c.snapshot_id, &abs)
                    .await
                {
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

    /// The measured side's changed line numbers (1-based) for `path` between
    /// `start` and `measured`. Empty when the run's tree doesn't have it, it
    /// is unchanged, or a side is too big to have kept its bytes; a path new
    /// since the start is all changed. `None` when a side's bytes should be
    /// there and aren't (collected): no honest diff exists.
    fn changed_lines_between(
        &self,
        path: &str,
        start: &oxplow_db::SnapshotTree,
        measured: &oxplow_db::SnapshotTree,
    ) -> Option<BTreeSet<u32>> {
        let Some(now) = measured.get(path) else {
            return Some(BTreeSet::new());
        };
        let before = start.get(path);
        if before.is_some_and(|b| b.identity() == now.identity()) {
            return Some(BTreeSet::new());
        }
        // `Some(None)`: no bytes kept by design (oversize); `None`: gone.
        let text = |entry: &oxplow_db::TreeEntry| -> Option<Option<String>> {
            match entry.content_ref() {
                None => Some(None),
                Some(r) => self
                    .content
                    .read_ref(&r)
                    .ok()
                    .map(|bytes| Some(String::from_utf8_lossy(&bytes).into_owned())),
            }
        };
        let Some(new) = text(now)? else {
            return Some(BTreeSet::new());
        };
        let old = match before {
            Some(b) => match text(b)? {
                Some(old) => old,
                None => return Some(BTreeSet::new()),
            },
            None => String::new(),
        };
        if old == new {
            return Some(BTreeSet::new());
        }
        Some(diff_new_side_lines(&old, &new))
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
/// The nudge shown when a test run lands with SEVERAL efforts open and nothing
/// resolves which one owns it (tsk170).
///
/// Timing is the whole point. The closing EFFORT REVIEW already reports these,
/// but by then they arrive in bulk, detached from what the agent was doing, and
/// fixing them means hand-mapping run ids to efforts. Here it costs one token on
/// the next command. It names the candidate tasks so the right id doesn't have
/// to be looked up.
fn unattributed_run_message(command: &str, open: &[Effort]) -> String {
    let ids: Vec<String> = open
        .iter()
        .map(|e| oxplow_domain::refs::build::work_item_label(&e.work_item))
        .collect();
    format!(
        "`{cmd}` was recorded but NOT attributed to an effort — {n} efforts are open \
         ({list}) and the command doesn't name which one it's for. Prefix the run with \
         `OXPLOW_TASK=<task id>` (e.g. `OXPLOW_TASK={first} {cmd}`) to pin it. Otherwise \
         it stays unattributed until you claim it at close.",
        cmd = command.trim(),
        n = open.len(),
        list = ids.join(", "),
        first = ids.first().map(String::as_str).unwrap_or("tskNN"),
    )
}

fn report_nudge_message(
    cfg: &oxplow_config::TestingConfig,
    has_report_collectors: bool,
    command: &str,
) -> String {
    let cmd = command.trim();
    if let Some(tc) = cfg
        .command
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        match cfg
            .fast_command
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(fast) => format!(
                "Tests ran (`{cmd}`) but produced no report, so this run won't appear in the \
                 effort's Tests panel — only report-emitting runs do. While iterating (red and \
                 green, filtered runs) use `{fast}`; close with `{tc}`. Both in the foreground."
            ),
            None => format!(
                "Tests ran (`{cmd}`) but produced no report, so this run won't appear in the \
                 effort's Tests panel — only report-emitting runs do. Run EVERY test invocation \
                 (including failing/red-phase and single-test runs, not just the final green \
                 one) via `{tc}` in the foreground so they all show."
            ),
        }
    } else if has_report_collectors {
        format!(
            "Tests ran (`{cmd}`) but wrote none of the reports this project's report collectors \
             read, so this effort has no parsed tests/coverage. Re-run via the command that \
             writes them, and set `testing.command` in .oxplow/project.yaml to make it one step."
        )
    } else {
        format!(
            "Tests ran (`{cmd}`) but this project reads no test reports, so oxplow can't \
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

/// How far back a report's mtime can be and still count as "fresh" for passive
/// ingestion (tsk269). Replaces the effort-start floor so collection works at
/// 0/N open efforts (observe-always). A report regenerated by the command that
/// just ran is seconds old; stale reports from prior runs/other stacks fall
/// outside the window and are skipped — same router/anti-replay intent as the
/// old effort-start floor, minus the effort dependency. 10 minutes is generous
/// for a slow suite that finishes writing well after its command started.
const REPORT_FRESH_WINDOW_MS: i64 = 10 * 60 * 1000;

/// Pause before the coverage leg's single retry (tsk79) — long enough for a
/// DB-contention blip to clear, short enough that the detached task still
/// finishes well within a test run's shadow.
const COVERAGE_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

/// Build the ABSOLUTE coverage payload from a report (tsk270): whole-report %
/// plus per-file instrumented/covered line-sets, stored verbatim so the
/// effort-relative diff can be derived with the effort's evidence. Returns
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

/// The effort a tool event was anchored to (the thread's single open effort
/// when the command ran).
fn anchored_effort(cause: Option<&RunCause>) -> Option<EffortId> {
    cause.and_then(|c| c.anchors.effort_id)
}

/// How far past a run's event a report's mtime may be and still be its
/// report: filesystem timestamp granularity and clock skew.
const REPORT_FRESH_SLACK_MS: i64 = 60 * 1000;

/// The report mtimes a run could have produced: after `from`, no later than
/// `to`. A report outside it belongs to an earlier run (or a later one).
/// Why a report collector's parser couldn't be made.
enum ParserProblem {
    /// An `exec` parser nobody on this machine approved as it is now.
    NeedsApproval(String),
    /// Its script can't be read or its format isn't one oxplow reads.
    Broken(String),
}

/// How a report collector is being run: what ran it (`on` / `manual`), as
/// whom (an event source), and for which detected run.
struct ReportRun<'a> {
    trigger: &'static str,
    source: &'a str,
    cause: Option<&'a RunCause>,
}

/// What reading one report collector came to.
#[derive(Debug)]
enum ReportRead {
    /// Its report wasn't written by this run: nothing ran.
    NotFresh,
    /// Its report isn't there: nothing ran.
    Missing(String),
    /// Its failures disabled it until a person enables it: nothing ran.
    Disabled(String),
    /// An `exec` parser nobody approved: recorded, nothing ran.
    NeedsApproval(String),
    /// It ran and parsed. `exec` names a program parser (a lower-trust
    /// source); `label` is the parser (`clippy`, or the collector's id).
    Parsed {
        output: CollectorOutput,
        exec: Option<String>,
        label: String,
    },
    /// It ran and failed: recorded, counted toward disabling it.
    Failed(String),
}

/// A detected run's report collectors' outputs, merged by kind, with the
/// program parsers among them (their output is tagged lower-trust).
#[derive(Debug, Default)]
struct RunReports {
    tests: Option<oxplow_coverage::TestReport>,
    coverage: Option<oxplow_coverage::CoverageReport>,
    analysis: Option<oxplow_coverage::AnalysisReport>,
    /// The bundled analyzers whose findings joined (`clippy`, `eslint`),
    /// or the collector's id for its own parser: the UI's "which analyzer
    /// ran" label.
    analyzers: Vec<String>,
    /// Program parsers by kind.
    exec_tests: Vec<String>,
    exec_coverage: Vec<String>,
    exec_analysis: Vec<String>,
    /// Coverage reports this run wrote that failed to parse (tsk79: a lost
    /// coverage run is recorded, not only logged).
    coverage_errors: Vec<String>,
}

impl RunReports {
    fn add(&mut self, output: CollectorOutput, exec: Option<String>, label: String) {
        match output {
            CollectorOutput::Test(parsed) => {
                self.tests
                    .get_or_insert_with(Default::default)
                    .suites
                    .extend(parsed.suites);
                self.exec_tests.extend(exec);
            }
            CollectorOutput::Coverage(parsed) => {
                self.coverage
                    .get_or_insert_with(Default::default)
                    .merge(parsed);
                self.exec_coverage.extend(exec);
            }
            CollectorOutput::Analysis(parsed) => {
                self.analysis
                    .get_or_insert_with(Default::default)
                    .findings
                    .extend(parsed.findings);
                if !self.analyzers.contains(&label) {
                    self.analyzers.push(label);
                }
                self.exec_analysis.extend(exec);
            }
        }
    }

    /// The merged test tree, unless no case was in it.
    fn tests(&self) -> Option<(&oxplow_coverage::TestReport, String)> {
        let t = self.tests.as_ref()?;
        if t.suites.iter().all(|s| s.cases.is_empty()) {
            return None;
        }
        Some((t, trust("post-tool-bash", &self.exec_tests)))
    }

    fn coverage(&self) -> Option<(&oxplow_coverage::CoverageReport, String)> {
        Some((
            self.coverage.as_ref()?,
            trust("coverage-report", &self.exec_coverage),
        ))
    }

    fn analysis(&self) -> Option<(&oxplow_coverage::AnalysisReport, String)> {
        Some((
            self.analysis.as_ref()?,
            trust("analysis-report", &self.exec_analysis),
        ))
    }
}

/// A run's `source`: `observed` from oxplow's own parse, or flagged
/// `plugin-exec:<ids>` when a program parser contributed ([[tsk162]]).
fn trust(observed: &str, exec: &[String]) -> String {
    if exec.is_empty() {
        observed.to_string()
    } else {
        format!("plugin-exec:{}", exec.join(","))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FreshWindow {
    from_ms: i64,
    to_ms: i64,
}

impl FreshWindow {
    /// Around a run that finished at `at`.
    fn around(at: oxplow_domain::Timestamp) -> Self {
        Self {
            from_ms: at.unix_ms() - REPORT_FRESH_WINDOW_MS,
            to_ms: at.unix_ms() + REPORT_FRESH_SLACK_MS,
        }
    }

    /// Around a run that finished just now (an explicit ingest).
    fn ending_now() -> Self {
        Self::around(oxplow_domain::Timestamp::now())
    }

    /// Whether `path` was written inside the window. An unreadable mtime
    /// counts as fresh: the parse decides.
    fn holds(&self, path: &std::path::Path) -> bool {
        let Ok(mtime) = std::fs::metadata(path).and_then(|m| m.modified()) else {
            return true;
        };
        let Ok(since) = mtime.duration_since(std::time::UNIX_EPOCH) else {
            return true;
        };
        let ms = since.as_millis() as i64;
        ms > self.from_ms && ms <= self.to_ms
    }
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

/// What the collection tests declare (tsk863).
#[cfg(test)]
pub(crate) mod test_support {
    use oxplow_config::collectors::CollectorSpec;

    /// A project report collector: `records` (`tests` / `coverage` /
    /// `analysis`) read from `path` by `entry` (`oxplow:<parser>` or a
    /// script) after a `run` (`test` / `analysis`).
    pub(crate) fn report_collector(
        id: &str,
        records: &str,
        entry: &str,
        path: &str,
        run: &str,
    ) -> CollectorSpec {
        let yaml = format!(
            "- {{ id: {id}, records: {records}, entry: \"{entry}\", report: {{ path: \"{path}\" }}, trigger: {{ on_run: {run} }} }}"
        );
        let value: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let (mut specs, errors) = oxplow_config::collectors::parse_collectors(
            oxplow_config::collectors::PROJECT,
            &value,
            &|_| true,
        );
        assert_eq!(errors, Vec::<String>::new());
        specs.remove(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob_store::BlobStore;

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

    fn testing(command: Option<&str>, fast: Option<&str>) -> oxplow_config::TestingConfig {
        oxplow_config::TestingConfig {
            command: command.map(str::to_string),
            fast_command: fast.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn report_nudge_names_configured_test_command() {
        let msg = report_nudge_message(
            &testing(Some("bun run test:collect"), None),
            true,
            "bun test",
        );
        // Echoes the project's own command — tool-agnostic, no built-in
        // tool→command knowledge in the hook.
        assert!(msg.contains("bun run test:collect"), "{msg}");
        assert!(msg.contains("bun test"), "{msg}");
        assert!(!msg.contains("/oxplow:configure"), "{msg}");
    }

    /// With a fast command configured, the nudge offers it for iterating
    /// and keeps the full one for the closing run.
    #[test]
    fn report_nudge_offers_the_fast_command_for_iterating() {
        let msg = report_nudge_message(
            &testing(Some("bun run test:collect"), Some("bun run test:fast")),
            true,
            "cargo test -p x",
        );
        assert!(msg.contains("`bun run test:fast`"), "{msg}");
        assert!(msg.contains("`bun run test:collect`"), "{msg}");
        assert!(!msg.contains("EVERY"), "{msg}");
    }

    #[test]
    fn report_nudge_routes_to_configure_without_profile() {
        // Nothing reads reports → route to the agent-driven configure flow
        // (which adapts to any tool).
        let msg = report_nudge_message(&testing(None, None), false, "pytest -q tests/");
        assert!(msg.contains("/oxplow:configure"), "{msg}");
        assert!(msg.contains("pytest -q tests/"), "{msg}");
        let with_collectors = report_nudge_message(&testing(None, None), true, "pytest");
        assert!(
            with_collectors.contains("testing.command"),
            "{with_collectors}"
        );
    }

    #[test]
    fn report_nudge_has_no_repo_specific_context_path() {
        // The nudge ships in oxplow-app and fires in every downstream
        // project — it must never point at this repo's own `.context/`
        // docs, which don't exist in a user's project.
        for (cfg, collectors) in [
            (testing(Some("bun run test:collect"), None), true),
            (testing(None, None), true),
            (testing(None, None), false),
        ] {
            let msg = report_nudge_message(&cfg, collectors, "bun test");
            assert!(!msg.contains(".context/"), "leaked repo path: {msg}");
        }
    }

    /// tsk863: the parsers a report collector may name are the ones oxplow
    /// ships, each recording what it parses and pre-parsing its report the
    /// way its program expects.
    #[test]
    fn the_bundled_parsers_agree_with_what_config_accepts() {
        let config: Vec<(String, Records, String)> = oxplow_config::collectors::BUNDLED_PARSERS
            .iter()
            .map(|(n, r, f)| (n.to_string(), *r, f.to_string()))
            .collect();
        let shipped: Vec<(String, Records, String)> = oxplow_collect_plugin::BUNDLED
            .iter()
            .map(|(n, kind, input, _)| {
                let records = match kind {
                    CollectorKind::Test => Records::Tests,
                    CollectorKind::Coverage => Records::Coverage,
                    CollectorKind::Analysis => Records::Analysis,
                };
                let format = ["text", "json", "xml", "lcov", "lines"]
                    .into_iter()
                    .find(|f| CollectorInput::named(f) == Some(*input))
                    .unwrap();
                (n.to_string(), records, format.to_string())
            })
            .collect();
        assert_eq!(config, shipped);
    }

    #[test]
    fn a_program_parser_keeps_its_lower_trust_label() {
        assert_eq!(trust("coverage-report", &[]), "coverage-report");
        assert_eq!(
            trust("coverage-report", &["tests.parse".into(), "x".into()]),
            "plugin-exec:tests.parse,x"
        );
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
    fn detect_git_revert_matches_revert_commands() {
        // tsk77: `git revert` creates its commit without the word "commit"
        // in the command line, so the waste leg needs its own detector.
        assert!(detect_git_revert("git revert abc1234"));
        assert!(detect_git_revert("git -c user.email=t@t revert HEAD~1"));
        // --no-commit stages the inverse without committing — nothing landed.
        assert!(!detect_git_revert("git revert --no-commit abc1234"));
        assert!(!detect_git_revert("git revert -n abc1234"));
        assert!(!detect_git_revert("git commit -m \"revert the thing\""));
        assert!(!detect_git_revert("git status"));
    }

    #[test]
    fn parse_reverted_shas_reads_the_git_trailer() {
        // The stock `git revert` body line. Multi-revert bodies carry one
        // trailer per reverted commit.
        let body = "This reverts commit 0123456789abcdef0123456789abcdef01234567.\n\
                    \n\
                    This reverts commit fedcba9876543210fedcba9876543210fedcba98.";
        assert_eq!(
            parse_reverted_shas(body),
            vec![
                "0123456789abcdef0123456789abcdef01234567".to_string(),
                "fedcba9876543210fedcba9876543210fedcba98".to_string(),
            ]
        );
        assert!(parse_reverted_shas("fix: ordinary commit body").is_empty());
        // Prose mentioning a revert without the trailer shape doesn't count.
        assert!(parse_reverted_shas("this reverts the earlier approach").is_empty());
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
    fn a_report_is_fresh_only_inside_its_runs_window() {
        use oxplow_domain::Timestamp;
        let f = tempfile::NamedTempFile::new().unwrap();
        let hour = 60 * 60 * 1000;
        let written = Timestamp::now().unix_ms() - hour;
        f.as_file()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_millis(written as u64))
            .unwrap();
        // A run that finished just after the report was written, judged an
        // hour later (a redelivery): still its report.
        assert!(FreshWindow::around(Timestamp::from_unix_ms(written + 1_000)).holds(f.path()));
        // A run long after it: an old report.
        assert!(!FreshWindow::ending_now().holds(f.path()));
        // A run long before it: a later run's report.
        assert!(!FreshWindow::around(Timestamp::from_unix_ms(written - hour)).holds(f.path()));
    }

    /// End-to-end exercises of the orchestration: a real in-memory DB +
    /// tempdir project, with stream/thread/task/effort/snapshot/blob rows
    /// built through the public store APIs.
    mod integration {
        use super::*;
        use oxplow_db::{Database, SqliteSnapshotStore, SqliteStreamStore, SqliteTaskStore};
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
            efforts: Arc<SqliteEffortStore>,
            nudges: Arc<SqliteAgentNudgeStore>,
            tmp: tempfile::TempDir,
            /// Shared in-memory db handle — lets a test seed a second task/effort
            /// (e.g. the overlapping-efforts disentangle case).
            db: Database,
        }

        /// Build the fixture. `report_xml` Some → write it + configure the
        /// `tests.coverage` report collector (cobertura/coverage.xml); None →
        /// no report collector. The effort's start snapshot holds
        /// `src/foo.rs` as `a\nb\nc\n`; the working tree has
        /// `a\nB\nc\nd\n` (lines 2 changed, 4 added), in no snapshot yet:
        /// a coverage run takes the snapshot of what it measured.
        async fn build(report_xml: Option<&str>) -> Harness {
            build_full(report_xml, false).await
        }

        use super::super::test_support::report_collector;

        /// Declare `specs` as the project's report collectors.
        fn declare(h: &Harness, specs: Vec<CollectorSpec>) {
            h.service.config.write().unwrap().collectors = specs;
        }

        /// What a run of `kind` that ended just now reads.
        async fn run_reads(h: &Harness, kind: RunKind) -> RunReports {
            h.service
                .read_run_reports(kind, FreshWindow::ending_now(), None)
                .await
        }

        /// Run the harness's coverage collector by hand (`collector.sync`).
        async fn ingest_coverage(h: &Harness) -> CoverageIngest {
            match h
                .service
                .sync_report_collector(&h.thread, "tests.coverage", "human")
                .await
                .unwrap()
            {
                ReportSync::Coverage(c) => c,
                other => panic!("{other:?}"),
            }
        }

        /// Like [`build`], plus `git_init`: `git init` the project and lay
        /// down a base commit, for tests that need a real repo (HEAD with a
        /// parent, commit detail, revert trailers).
        async fn build_full(report_xml: Option<&str>, git_init: bool) -> Harness {
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
                acp_agent: None,
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

            // The stream's snapshot taker, as boot registers one.
            let blobs = BlobStore::new(project_dir.join(".oxplow/snapshots"));
            let snapshots = Arc::new(SqliteSnapshotStore::new(db.clone()));
            let capture = Arc::new(
                crate::snapshot_capture::SnapshotCaptureService::new(
                    snapshots.clone(),
                    blobs.clone(),
                    project_dir.clone(),
                    Arc::new(crate::vcs::GitProvider),
                    stream.id,
                    1_000_000,
                    oxplow_fs_watch::WorkspaceFilter::default(),
                )
                .with_settle_duration(std::time::Duration::ZERO)
                .with_predrain_delay(std::time::Duration::ZERO),
            );
            let captures = crate::snapshot_capture_registry::SnapshotCaptureRegistry::new(
                crate::snapshot_capture_registry::SnapshotCaptureRegistryConfig {
                    vcs: Arc::new(crate::vcs::GitProvider),
                    snapshot_store: snapshots.clone(),
                    blobs: blobs.clone(),
                    max_file_bytes: 1_000_000,
                    workspace_filter: oxplow_fs_watch::WorkspaceFilter::default(),
                    open_turn_probe: None,
                },
            );
            captures.insert_for_test(stream.id, capture.clone());

            // The effort's start snapshot: src/foo.rs as `a b c`.
            let foo = project_dir.join("src/foo.rs");
            std::fs::create_dir_all(project_dir.join("src")).unwrap();
            std::fs::write(&foo, "a\nb\nc\n").unwrap();
            capture.mark_dirty(foo.clone(), oxplow_fs_watch::WatchEventKind::Other);
            let snap_id = capture
                .request_snapshot(oxplow_domain::snapshot::SnapshotTrigger::EffortStart)
                .await
                .unwrap()
                .expect("start snapshot");

            let efforts = Arc::new(SqliteEffortStore::new(db.clone()));
            let effort = efforts
                .start(&work_item_ref(task_id), &thread.id, Some(snap_id))
                .await
                .unwrap();

            // The agent's edit, on disk only — line 2 changed, line 4 added.
            // A coverage run takes the snapshot it measured (tsk883).
            std::fs::write(&foo, NEW_FOO).unwrap();
            capture.mark_dirty(foo, oxplow_fs_watch::WatchEventKind::Other);
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
                cfg.collectors
                    .push(super::super::test_support::report_collector(
                        "tests.coverage",
                        "coverage",
                        "oxplow:cobertura",
                        "coverage.xml",
                        "test",
                    ));
            }

            let nudges = Arc::new(SqliteAgentNudgeStore::new(db.clone()));
            // Seed the producer specs the way boot's `seed_catalog` does — the
            // producers gate their collection on `measure_has_active_spec` (tsk31),
            // so without an active spec over each measure the gate stays closed.
            let facts = Arc::new(oxplow_db::SqliteFactStore::new(db.clone()));
            for spec in crate::producer_metrics::builtin_producer_specs() {
                facts.upsert_spec(spec).await.unwrap();
            }
            let service = CollectionService::new(
                facts,
                nudges.clone(),
                efforts.clone(),
                Arc::new(SqliteTaskStore::new(db.clone())),
                Arc::new(SqliteThreadStore::new(db.clone())),
                snapshots,
                captures,
                crate::snapshot_content::SnapshotContent::new(
                    blobs,
                    oxplow_domain::vcs::Vcs::object_store(&crate::vcs::GitProvider, &project_dir),
                ),
                Arc::new(crate::vcs::GitProvider),
                Arc::new(RwLock::new(cfg)),
                project_dir,
                Arc::new(oxplow_db::SqliteAttributionStore::new(db.clone())),
            )
            .with_run_log(crate::collector_runner::RunLog {
                db: db.clone(),
                vocabulary: oxplow_domain::vocabulary::VocabularyHandle::core(),
                layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            });
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
            let outcome = ingest_coverage(&h).await;
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
                .effort_observations_from_metrics(&h.effort_id, Some("diff-coverage"))
                .await;
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

        /// The harness's measured `src/foo.rs` (start: `a b c`).
        const NEW_FOO: &str = "a\nB\nc\nd\n";

        /// The derived diff-coverage % of the harness's effort, if any.
        async fn diff_pct(h: &Harness) -> Option<f64> {
            let rows = h
                .service
                .effort_observations_from_metrics(&h.effort_id, Some("diff-coverage"))
                .await;
            assert!(rows.len() <= 1, "{rows:?}");
            rows.first().and_then(|r| r.metric_value)
        }

        /// tsk862: the diff is of the code the run measured — the snapshot
        /// its capture is pinned to — so an edit after the run moves nothing.
        #[tokio::test]
        async fn diff_coverage_ignores_edits_after_the_run() {
            let h = build(Some(COBERTURA_50PCT)).await;
            ingest_coverage(&h).await;
            std::fs::write(h.tmp.path().join("src/foo.rs"), "x\n").unwrap();
            let pct = diff_pct(&h).await.expect("a diff");
            assert!((pct - 50.0).abs() < 1e-6, "got {pct}");
        }

        /// tsk862: the diff reads snapshots, never a working tree — a
        /// worktree stream's files aren't under the project directory.
        #[tokio::test]
        async fn a_worktree_streams_diff_uses_its_snapshots() {
            let h = build(Some(COBERTURA_50PCT)).await;
            ingest_coverage(&h).await;
            std::fs::remove_file(h.tmp.path().join("src/foo.rs")).unwrap();
            let pct = diff_pct(&h).await.expect("a diff from snapshots alone");
            assert!((pct - 50.0).abs() < 1e-6, "got {pct}");
        }

        /// tsk883: a run delivered after an edit landed can't know what it
        /// measured — its coverage is recorded, with no snapshot and so no
        /// diff rather than a wrong one.
        #[tokio::test]
        async fn a_run_delivered_after_an_edit_gets_no_diff() {
            let h = build(Some(COBERTURA_50PCT)).await;
            let eid = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            let cause = RunCause {
                event_id: "evt-before-the-edit".into(),
                seq: 0,
                anchors: oxplow_domain::Anchors {
                    effort_id: Some(eid),
                    ..Default::default()
                },
                // The run ended before the harness's edit landed.
                at: Timestamp::from_unix_ms(Timestamp::now().unix_ms() - 30_000),
            };
            h.service
                .on_post_tool_use(&h.thread, &bash_payload("bun test", 0), Some(&cause))
                .await
                .unwrap();
            let pinned: Vec<Option<i64>> =
                h.db.transaction(|tx| {
                    let mut stmt = tx
                        .prepare(
                            "SELECT snapshot_id FROM metric_capture WHERE producer = 'coverage'",
                        )
                        .map_err(oxplow_db::map_sql_err)?;
                    let rows = stmt
                        .query_map([], |r| r.get(0))
                        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                        .map_err(oxplow_db::map_sql_err)?;
                    Ok(rows)
                })
                .await
                .unwrap();
            assert_eq!(pinned, vec![None], "recorded, unpinned");
            assert_eq!(diff_pct(&h).await, None);
        }

        /// tsk884: `cargo llvm-cov` names files by absolute path. They're
        /// read repo-relative, so they diff against the snapshots and their
        /// facts name repo files; a file outside the project is dropped.
        #[tokio::test]
        async fn an_absolute_report_path_is_read_repo_relative() {
            let h = build(Some(COBERTURA_50PCT)).await;
            let root = h.tmp.path().to_string_lossy().into_owned();
            let absolute = COBERTURA_50PCT.replace(
                r#"filename="src/foo.rs""#,
                &format!(r#"filename="{root}/src/foo.rs""#),
            );
            let outside = r#"<class name="Dep" filename="/elsewhere/dep.rs"><lines>
    <line number="1" hits="1"/></lines></class>
</classes>"#;
            let absolute = absolute.replace("</classes>", outside);
            std::fs::write(h.tmp.path().join("coverage.xml"), absolute).unwrap();
            ingest_coverage(&h).await;

            let pct = diff_pct(&h).await.expect("a diff");
            assert!((pct - 50.0).abs() < 1e-6, "got {pct}");
            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let measure = facts.get_measure("oxplow.coverage").await.unwrap().unwrap();
            let refs: Vec<_> = facts
                .facts_for_measure(measure.id)
                .await
                .unwrap()
                .into_iter()
                .map(|f| f.subject_ref)
                .collect();
            assert_eq!(refs, vec![Some("file:src/foo.rs".to_string())]);
        }

        /// tsk862: a baseline whose bytes are gone (collected) can't be
        /// diffed — no row, not "every line changed".
        #[tokio::test]
        async fn an_expired_baseline_gives_no_diff() {
            let h = build(Some(COBERTURA_50PCT)).await;
            ingest_coverage(&h).await;
            let keep = std::collections::HashSet::from([BlobStore::hash(NEW_FOO.as_bytes())]);
            BlobStore::new(h.tmp.path().join(".oxplow/snapshots"))
                .gc(&keep)
                .unwrap();
            assert_eq!(diff_pct(&h).await, None);
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
            let _eff2 = h
                .efforts
                .start(&work_item_ref(task2), &h.thread, None)
                .await
                .unwrap();

            // Observe coverage — two efforts open ⇒ unclaimed.
            assert!(matches!(
                ingest_coverage(&h).await,
                CoverageIngest::Stored { .. }
            ));
            // No pollution: neither effort shows a diff-coverage observation yet.
            assert!(h
                .service
                .effort_observations_from_metrics(&h.effort_id, Some("diff-coverage"))
                .await
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
                .effort_observations_from_metrics(&h.effort_id, Some("diff-coverage"))
                .await;
            assert_eq!(rows.len(), 1, "claimed run now surfaces a derived diff");
            assert!((rows[0].metric_value.unwrap() - 50.0).abs() < 1e-6);
        }

        #[tokio::test]
        async fn ingest_coverage_mirrors_into_metric_substrate() {
            // git_init = true so a branch is present to capture.
            let h = build_full(Some(COBERTURA_50PCT), true).await;
            ingest_coverage(&h).await;

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

        #[tokio::test]
        async fn ingest_coverage_records_branch_and_function_facts() {
            // tsk123: a report carrying branch (condition-coverage) + function
            // (methods) data lands per-file facts on oxplow.coverage.branch /
            // .function beside the line facts, num/den = hit/found so the headline
            // reads Σhit/Σfound. Branch 3/4 → 75%, function 1/2 → 50%.
            const COBERTURA_BF: &str = r#"<?xml version="1.0"?>
<coverage><packages><package name="p"><classes>
  <class name="Foo" filename="src/foo.rs">
    <methods>
      <method name="a" signature="()V" line-rate="1.0"/>
      <method name="b" signature="()V" line-rate="0.0"/>
    </methods>
    <lines>
      <line number="1" hits="3" branch="true" condition-coverage="100% (2/2)"/>
      <line number="2" hits="1" branch="true" condition-coverage="50% (1/2)"/>
      <line number="4" hits="0"/>
    </lines>
  </class>
</classes></package></packages></coverage>"#;
            let h = build_full(Some(COBERTURA_BF), true).await;
            ingest_coverage(&h).await;

            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let bm = facts
                .get_measure("oxplow.coverage.branch")
                .await
                .unwrap()
                .expect("branch measure seeded by V68");
            let br = facts.facts_for_measure(bm.id).await.unwrap();
            assert_eq!(br.len(), 1, "one branch fact for the file");
            assert_eq!(br[0].numerator, Some(3.0));
            assert_eq!(br[0].denominator, Some(4.0));
            assert!(
                (br[0].value - 75.0).abs() < 0.01,
                "branch value {}",
                br[0].value
            );
            assert_eq!(br[0].subject_kind.as_deref(), Some("file"));

            let fm = facts
                .get_measure("oxplow.coverage.function")
                .await
                .unwrap()
                .expect("function measure seeded by V68");
            let fnf = facts.facts_for_measure(fm.id).await.unwrap();
            assert_eq!(fnf.len(), 1, "one function fact for the file");
            assert_eq!(fnf[0].numerator, Some(1.0));
            assert_eq!(fnf[0].denominator, Some(2.0));
            assert!(
                (fnf[0].value - 50.0).abs() < 0.01,
                "function value {}",
                fnf[0].value
            );
        }

        #[tokio::test]
        async fn merge_fresh_coverage_preserves_branch_and_function_counts() {
            // tsk160: the passive ride-along is the PRIMARY ingestion path, and
            // it merges per-file coverage from every fresh report. It used to
            // copy only the line sets, so branch/function counters arrived as 0
            // and observe_coverage's `*_found > 0` gate meant
            // oxplow.coverage.branch/.function never got a fact here — while the
            // explicit ingest_coverage path (which passes the parse straight
            // through) worked. Same report as
            // `ingest_coverage_records_branch_and_function_facts`: branch 3/4,
            // function 1/2.
            const COBERTURA_BF: &str = r#"<?xml version="1.0"?>
<coverage><packages><package name="p"><classes>
  <class name="Foo" filename="src/foo.rs">
    <methods>
      <method name="a" signature="()V" line-rate="1.0"/>
      <method name="b" signature="()V" line-rate="0.0"/>
    </methods>
    <lines>
      <line number="1" hits="3" branch="true" condition-coverage="100% (2/2)"/>
      <line number="2" hits="1" branch="true" condition-coverage="50% (1/2)"/>
      <line number="4" hits="0"/>
    </lines>
  </class>
</classes></package></packages></coverage>"#;
            let h = build(None).await;
            std::fs::write(h.tmp.path().join("cov.xml"), COBERTURA_BF).unwrap();
            declare(
                &h,
                vec![report_collector(
                    "r0",
                    "coverage",
                    "oxplow:cobertura",
                    "cov.xml",
                    "test",
                )],
            );
            // Floor at the epoch so the just-written report is always fresh
            // (same approach as the merge_fresh_test_reports tests).
            let (merged, _errors) = {
                let reads = run_reads(&h, RunKind::Test).await;
                (
                    reads.coverage().map(|(r, s)| (r.clone(), s)),
                    reads.coverage_errors.clone(),
                )
            };
            let (report, _source) = merged.expect("fresh cobertura report should merge");
            let fc = report
                .files
                .get("src/foo.rs")
                .expect("src/foo.rs in the merged report");

            assert_eq!(
                fc.branches_found, 4,
                "branch denominator survives the merge"
            );
            assert_eq!(fc.branches_hit, 3, "branch numerator survives the merge");
            assert_eq!(
                fc.functions_found, 2,
                "function denominator survives the merge"
            );
            assert_eq!(fc.functions_hit, 1, "function numerator survives the merge");
        }

        #[tokio::test]
        async fn merge_fresh_coverage_sums_counts_across_reports() {
            // Two reports covering the same file (a polyglot repo reporting from
            // more than one toolchain, which 0.5 explicitly supports) must SUM
            // their branch/function counters, matching how the line sets union.
            const A: &str = r#"<?xml version="1.0"?>
<coverage><packages><package name="p"><classes>
  <class name="Foo" filename="src/foo.rs">
    <methods><method name="a" signature="()V" line-rate="1.0"/></methods>
    <lines><line number="1" hits="1" branch="true" condition-coverage="50% (1/2)"/></lines>
  </class>
</classes></package></packages></coverage>"#;
            const B: &str = r#"<?xml version="1.0"?>
<coverage><packages><package name="p"><classes>
  <class name="Foo" filename="src/foo.rs">
    <methods><method name="b" signature="()V" line-rate="0.0"/></methods>
    <lines><line number="2" hits="0" branch="true" condition-coverage="0% (0/2)"/></lines>
  </class>
</classes></package></packages></coverage>"#;
            let h = build(None).await;
            std::fs::write(h.tmp.path().join("a.xml"), A).unwrap();
            std::fs::write(h.tmp.path().join("b.xml"), B).unwrap();
            declare(
                &h,
                vec![
                    report_collector("r0", "coverage", "oxplow:cobertura", "a.xml", "test"),
                    report_collector("r1", "coverage", "oxplow:cobertura", "b.xml", "test"),
                ],
            );
            let (merged, _errors) = {
                let reads = run_reads(&h, RunKind::Test).await;
                (
                    reads.coverage().map(|(r, s)| (r.clone(), s)),
                    reads.coverage_errors.clone(),
                )
            };
            let (report, _source) = merged.expect("both reports should merge");
            let fc = report.files.get("src/foo.rs").expect("merged file");

            assert_eq!(fc.branches_found, 4, "2 + 2");
            assert_eq!(fc.branches_hit, 1, "1 + 0");
            assert_eq!(fc.functions_found, 2, "1 + 1");
            assert_eq!(fc.functions_hit, 1, "1 + 0");
            assert_eq!(fc.instrumented.len(), 2, "line sets still union");
        }

        #[tokio::test]
        async fn reverting_a_closed_efforts_commit_records_token_waste() {
            // tsk77: `git revert` of a commit made inside a CLOSED, token-
            // metered effort → one `oxplow.token_waste` fact (value/num = the
            // effort's spend, den = 0 — the numerator side of the wasted
            // ratio), idempotent per effort.
            let h = build_full(None, true).await;
            let dir = h.tmp.path();

            // A commit inside the (still open) effort's window.
            std::fs::write(dir.join("work.txt"), "v1\n").unwrap();
            git_in(dir, &["add", "work.txt"]);
            git_commit(dir, "bad work");
            let bad_sha = String::from_utf8(
                std::process::Command::new("git")
                    .current_dir(dir)
                    .args(["rev-parse", "HEAD"])
                    .output()
                    .unwrap()
                    .stdout,
            )
            .unwrap()
            .trim()
            .to_string();

            // Give the effort a token spend, then close it.
            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let effort = h
                .efforts
                .find_open_for_thread(&h.thread)
                .await
                .unwrap()
                .expect("open effort");
            let etm = facts
                .get_measure("oxplow.effort_tokens")
                .await
                .unwrap()
                .unwrap();
            let mut cap = oxplow_db::NewMetricCapture::done(1, "effort-lifecycle", "test");
            cap.effort_id = Some(effort.id.value());
            facts
                .record_facts(
                    cap,
                    vec![oxplow_db::NewFact {
                        subject_kind: Some("effort".into()),
                        subject_ref: Some(effort.id.to_string()),
                        ..oxplow_db::NewFact::new(etm.id, 5000.0)
                    }],
                )
                .await
                .unwrap();
            h.efforts.finish(&effort.id, None, None).await.unwrap();

            // The revert lands as a fresh commit with the trailer.
            git_in(
                dir,
                &[
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "user.name=t",
                    "revert",
                    "--no-edit",
                    &bad_sha,
                ],
            );
            h.service
                .on_post_tool_use(
                    &h.thread,
                    &bash_payload(&format!("git revert {bad_sha}"), 0),
                    None,
                )
                .await
                .unwrap();

            let waste_measure = facts
                .get_measure("oxplow.token_waste")
                .await
                .unwrap()
                .expect("token_waste measure seeded by V61");
            let waste = facts.facts_for_measure(waste_measure.id).await.unwrap();
            assert_eq!(waste.len(), 1, "one waste fact for the reverted effort");
            assert_eq!(waste[0].value, 5000.0, "the effort's full spend");
            assert_eq!(waste[0].numerator, Some(5000.0));
            assert_eq!(waste[0].denominator, Some(0.0), "numerator-only ratio row");
            assert_eq!(
                waste[0].subject_ref.as_deref(),
                Some(effort.id.to_string().as_str())
            );

            // Re-firing (same revert seen again) is idempotent per effort.
            h.service
                .on_post_tool_use(
                    &h.thread,
                    &bash_payload(&format!("git revert {bad_sha}"), 0),
                    None,
                )
                .await
                .unwrap();
            let again = facts.facts_for_measure(waste_measure.id).await.unwrap();
            assert_eq!(again.len(), 1, "one waste fact per effort, ever");
        }

        #[tokio::test]
        async fn coverage_parse_failure_records_a_failed_capture() {
            // tsk79: a FRESH but malformed coverage report used to vanish with
            // only a tty warn — the run's coverage just never existed. Now the
            // miss is durable: a facts-empty `status=failed` coverage capture
            // (the gauge-failure convention) lands in the substrate.
            let h = build(Some("<coverage this is not xml")).await;
            let out = h
                .service
                .on_post_tool_use(&h.thread, &bash_payload("bun test --watch false", 0), None)
                .await
                .unwrap();
            // The failure is recorded, not returned — the hook never fails.
            // (A nudge may still fire for the report-less run; either is fine.)
            let _ = out;
            let caps = oxplow_db::SqliteFactStore::new(h.db.clone())
                .captures_in_window_by_trigger(
                    h.thread.value(),
                    "on-report",
                    Timestamp::from_unix_ms(0),
                    None,
                )
                .await
                .unwrap();
            let failed: Vec<_> = caps
                .iter()
                .filter(|c| c.producer == "coverage" && c.status == "failed")
                .collect();
            assert_eq!(failed.len(), 1, "one failed coverage capture: {caps:?}");
            assert!(
                !failed[0].error.as_deref().unwrap_or_default().is_empty(),
                "carries the parse error"
            );
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
                .effort_observations_from_metrics(&h.effort_id, Some("test-run"))
                .await;
            assert_eq!(rows.len(), 1);
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
        async fn test_run_capture_stamps_the_commit_it_tested() {
            // tsk95: a test result is about a CODE STATE, not a branch name, so
            // the run capture must record which commit it tested. Without it the
            // per-subject fold can never be ancestry-aware (tsk97) — and it is
            // NOT backfillable: which commit a past run tested is unrecoverable.
            // Gauge captures got this for free via the snapshot-driven
            // `CollectorRunContext`; test runs went unstamped for 125 captures.
            let h = build_full(None, true).await;
            h.service
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
                .unwrap();
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
            assert!(
                caps[0]
                    .closest_vcs_rev
                    .as_deref()
                    .is_some_and(|v| !v.is_empty()),
                "the run capture must carry the commit it tested, got {:?}",
                caps[0].closest_vcs_rev
            );
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
                        closest_vcs_rev: None,
                        vcs_rev_exact: false,
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
        async fn refreshing_effort_evidence_stores_deltas_for_lenses() {
            let h = build(None).await;
            let start = effort_start(&h, &h.effort_id).await;
            let before = Timestamp::from_unix_ms(start.unix_ms() - 60_000);
            let after = Timestamp::from_unix_ms(start.unix_ms() + 60_000);
            let (m, facts) =
                seed_file_gauge(&h, "oxplow.rust.unsafe_blocks", "lower-better", Some(0.0)).await;
            claim(&h, &h.effort_id, "src/a.rs").await;
            seed_gauge_capture(&facts, m, before, &[("src/a.rs", 2.0)]).await;
            seed_gauge_capture(&facts, m, after, &[("src/a.rs", 5.0)]).await;

            let store = oxplow_db::SqliteEffortEvidenceStore::new(h.db.clone());
            h.service
                .refresh_effort_evidence(&h.effort_id, &store)
                .await
                .unwrap();

            let out = crate::sql_gateway::SqlGateway::new(h.db.clone())
                .query_sql(
                    "SELECT key, baseline, current, delta FROM v_effort_metric_delta",
                    vec![],
                    None,
                )
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&out.rows).unwrap(),
                serde_json::json!([["oxplow.rust.unsafe_blocks", 2.0, 5.0, 3.0]])
            );
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
            let eff2 = h
                .efforts
                .start(&work_item_ref(task2), &h.thread, None)
                .await
                .unwrap();
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
            let eff2 = h
                .efforts
                .start(&work_item_ref(task2), &h.thread, None)
                .await
                .unwrap();
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
        async fn clean_only_effort_discovers_producers_from_global_history() {
            // tsk239/tsk242: when the effort's ONLY run was clean, its own
            // captures carry no matching facts, so the producer set that drives
            // the zero-fill has to come from the measure's global history. Two
            // branches serve that now — an unconstrained filter short-circuits
            // to the memoized `producers_for_measure`, a constrained one walks
            // one representative fact per slice — and BOTH must agree with the
            // filter. The risk this pins down is the short-circuit leaking into
            // the constrained case and zero-filling a metric whose slice this
            // producer never emits.
            let h = build(None).await;
            let eid = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            let now = Timestamp::now();

            let facts = oxplow_db::SqliteFactStore::new(h.db.clone());
            let m = facts
                .upsert_measure(oxplow_db::NewMeasure::new("oxplow.lint_hit", "Lint hits"))
                .await
                .unwrap();
            let spec = |key: &str, title: &str, filter: Option<&str>| {
                let mut s = oxplow_db::NewMetricSpec::base(key, title, "oxplow.lint_hit", "count");
                s.direction = "lower-better".into();
                s.category = Some("static-quality".into());
                s.display_kind = "findings".into();
                s.filter_json = filter.map(str::to_string);
                s
            };
            facts
                .upsert_spec(spec("oxplow.lint.all", "All lint hits", None))
                .await
                .unwrap();
            facts
                .upsert_spec(spec(
                    "oxplow.lint.errors",
                    "Lint errors",
                    Some(r#"{"severity":"error"}"#),
                ))
                .await
                .unwrap();
            facts
                .upsert_spec(spec(
                    "oxplow.lint.warnings",
                    "Lint warnings",
                    Some(r#"{"severity":"warning"}"#),
                ))
                .await
                .unwrap();

            // Global history, NOT stamped to the effort: producer `analysis`
            // has only ever emitted WARNINGS.
            let mut past = oxplow_db::NewMetricCapture::done(1, "analysis", "analysis-report");
            past.captured_at = Some(Timestamp::from_unix_ms(now.unix_ms() - 600_000));
            facts
                .record_facts(
                    past,
                    vec![
                        oxplow_db::NewFact {
                            severity: Some("warning".into()),
                            ..oxplow_db::NewFact::new(m, 1.0)
                        },
                        oxplow_db::NewFact {
                            severity: Some("warning".into()),
                            ..oxplow_db::NewFact::new(m, 1.0)
                        },
                    ],
                )
                .await
                .unwrap();
            // The effort's own — and only — run: CLEAN, no facts at all.
            let mut clean = oxplow_db::NewMetricCapture::done(1, "analysis", "analysis-report");
            clean.captured_at = Some(now);
            clean.thread_id = Some(h.thread.value());
            clean.effort_id = Some(eid.value());
            facts.record_facts(clean, vec![]).await.unwrap();

            let rows = h.service.effort_metric_deltas(&h.effort_id).await;
            let row = |title: &str| rows.iter().find(|d| d.title == title);

            // Unconstrained: every fact counts, so the producer is discovered
            // and its clean capture reads as an explicit 0.
            assert_eq!(
                row("All lint hits").map(|d| d.current),
                Some(0.0),
                "the clean run zero-fills for an unfiltered spec"
            );
            // Constrained and matching: warnings ARE this producer's slice.
            assert_eq!(
                row("Lint warnings").map(|d| d.current),
                Some(0.0),
                "the clean run zero-fills for a filter the producer's slice matches"
            );
            // Constrained and NOT matching: `analysis` has never emitted an
            // error, so "errors dropped to 0" would be a fabricated reading.
            assert!(
                row("Lint errors").is_none(),
                "no zero-fill for a filter this producer's slice never matched"
            );
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
        async fn a_nudge_fact_takes_the_effort_its_run_was_anchored_to() {
            // A late delivery: the run's event was anchored to the effort;
            // judged now, no single effort is open (it closed, or others
            // opened) — the `oxplow.nudge` fact still belongs to it.
            let h = build(None).await;
            let eid = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            for spec in crate::producer_metrics::builtin_producer_specs() {
                oxplow_db::SqliteFactStore::new(h.db.clone())
                    .upsert_spec(spec)
                    .await
                    .unwrap();
            }
            let cause = RunCause {
                event_id: "evt-anchored".into(),
                seq: 0,
                anchors: oxplow_domain::Anchors {
                    effort_id: Some(eid),
                    ..Default::default()
                },
                at: Timestamp::now(),
            };
            h.efforts.finish(&eid, None, None).await.unwrap();
            h.service
                .persist_nudge(&h.thread, None, "report-less-run", "m", "cmd", Some(&cause))
                .await;
            let owner: Option<i64> =
                h.db.read(|c| {
                    c.query_row(
                        "SELECT effort_id FROM metric_capture WHERE producer = 'nudges'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap();
            assert_eq!(owner, Some(eid.value()));
        }

        #[tokio::test]
        async fn an_unattributable_run_nudges_at_the_moment_it_happens() {
            // tsk170: with several efforts open and nothing naming an owner, the
            // run IS recorded (observe-always) but lands unattributed. Say so
            // now, while `OXPLOW_TASK=` is a one-token fix on the next command —
            // the closing EFFORT REVIEW reports the same thing in bulk, detached
            // from what the agent was doing, needing run ids mapped by hand.
            let h = build(None).await;
            let now = Timestamp::now();
            let task2 = SqliteTaskStore::new(h.db.clone())
                .insert(&Task {
                    id: TaskId::placeholder(),
                    thread_id: Some(h.thread),
                    parent_id: None,
                    title: "second".into(),
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
            h.efforts
                .start(&work_item_ref(task2), &h.thread, None)
                .await
                .unwrap();

            // Names no crate and no path, so target overlap can't resolve it.
            let msg = h
                .service
                .on_post_tool_use(&h.thread, &bash_payload("cargo test --workspace", 0), None)
                .await
                .unwrap()
                .expect("an unattributable run must nudge");
            assert!(msg.contains("NOT attributed"), "{msg}");
            assert!(msg.contains("OXPLOW_TASK="), "names the fix: {msg}");
            assert!(
                msg.contains(&task2.to_string()),
                "lists the candidate tasks so the id needn't be looked up: {msg}"
            );
        }

        #[tokio::test]
        async fn an_attributable_run_does_not_nudge() {
            // The nudge must stay quiet whenever something DID resolve the owner,
            // or it trains the agent to ignore it. Here the command names a crate
            // that exactly one open effort is working in (tsk169), so attribution
            // succeeds silently even though two efforts are open.
            let h = build(None).await;
            let now = Timestamp::now();
            let task2 = SqliteTaskStore::new(h.db.clone())
                .insert(&Task {
                    id: TaskId::placeholder(),
                    thread_id: Some(h.thread),
                    parent_id: None,
                    title: "second".into(),
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
            let eff2 = h
                .efforts
                .start(&work_item_ref(task2), &h.thread, None)
                .await
                .unwrap();
            let eff1 = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            for (eid, path) in [
                (eff1, "crates/oxplow-git/src/smart_merge.rs"),
                (eff2.id, "crates/oxplow-config/src/lib.rs"),
            ] {
                h.efforts
                    .record_file(
                        &eid,
                        path,
                        oxplow_db::EffortFileChange::Updated,
                        oxplow_db::FileRefVersion {
                            local_snapshot_id: 0,
                            closest_vcs_rev: None,
                            vcs_rev_exact: false,
                        },
                    )
                    .await
                    .unwrap();
            }

            let msg = h
                .service
                .on_post_tool_use(
                    &h.thread,
                    &bash_payload("cargo test -p oxplow-git", 0),
                    None,
                )
                .await
                .unwrap();
            assert!(
                msg.is_none(),
                "target overlap resolved the owner — no nudge: {msg:?}"
            );
        }

        #[tokio::test]
        async fn an_efforts_first_run_attributes_from_its_task_text() {
            // tsk185: the exact miss this was filed for. Two efforts open,
            // NEITHER has claimed a file yet — snapshot capture is asynchronous,
            // so an effort seconds old has nothing attributed, which is exactly
            // when its first red-phase run happens. Scoring on files alone had
            // an empty list and declined; the task text names the crate.
            let h = build(None).await;
            let now = Timestamp::now();
            let store = SqliteTaskStore::new(h.db.clone());
            let mk = |title: &str, description: &str| {
                let store = store.clone();
                let (title, description) = (title.to_string(), description.to_string());
                async move {
                    store
                        .insert(&Task {
                            id: TaskId::placeholder(),
                            thread_id: Some(h.thread),
                            parent_id: None,
                            title,
                            description,
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
                        .unwrap()
                }
            };
            let task_cfg = mk("config work", "Fix `[[crates/oxplow-config/src/lib.rs]]`.").await;
            let task_git = mk(
                "git work",
                "Fix `[[crates/oxplow-git/src/smart_merge.rs]]`.",
            )
            .await;
            let eff_cfg = h
                .efforts
                .start(&work_item_ref(task_cfg), &h.thread, None)
                .await
                .unwrap();
            let eff_git = h
                .efforts
                .start(&work_item_ref(task_git), &h.thread, None)
                .await
                .unwrap();

            // Precondition: neither effort has claimed anything.
            for id in [eff_cfg.id, eff_git.id] {
                assert!(
                    h.efforts.list_files(&id).await.unwrap().is_empty(),
                    "fixture must have no attributed files yet"
                );
            }

            let owner = |cmd: &'static str| {
                let svc = h.service.clone();
                let thread = h.thread;
                async move {
                    svc.resolve_owning_effort_for_command(&thread, None, Some(cmd))
                        .await
                        .map(|e| e.id)
                }
            };
            assert_eq!(
                owner("cargo test -p oxplow-config gauge").await,
                Some(eff_cfg.id),
                "resolved from the task's text alone"
            );
            assert_eq!(
                owner("cargo test -p oxplow-git symlink").await,
                Some(eff_git.id)
            );
            // Still declines when nothing names an owner.
            assert_eq!(owner("bun run test:collect").await, None);
        }

        #[tokio::test]
        async fn concurrent_efforts_attribute_by_what_the_command_names() {
            // tsk169: two efforts open at once, each working in a different
            // crate. Before this, `find_single_open_for_thread` declined and BOTH
            // runs landed unattributed, to be reconciled by hand at close. The
            // command names its target, so each run can be attributed at record
            // time instead.
            let h = build(None).await;
            let now = Timestamp::now();
            let task2 = SqliteTaskStore::new(h.db.clone())
                .insert(&Task {
                    id: TaskId::placeholder(),
                    thread_id: Some(h.thread),
                    parent_id: None,
                    title: "second".into(),
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
            let eff2 = h
                .efforts
                .start(&work_item_ref(task2), &h.thread, None)
                .await
                .unwrap();
            let eff1 = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();

            // Each effort declares the file it is working on — the signal the
            // command gets matched against.
            let claim = |eid: oxplow_domain::EffortId, path: &'static str| {
                let efforts = h.efforts.clone();
                async move {
                    efforts
                        .record_file(
                            &eid,
                            path,
                            oxplow_db::EffortFileChange::Updated,
                            oxplow_db::FileRefVersion {
                                local_snapshot_id: 0,
                                closest_vcs_rev: None,
                                vcs_rev_exact: false,
                            },
                        )
                        .await
                        .unwrap();
                }
            };
            claim(eff1, "crates/oxplow-git/src/smart_merge.rs").await;
            claim(eff2.id, "crates/oxplow-config/src/lib.rs").await;

            // Sanity: with two open, the old single-open rule genuinely declines.
            assert!(
                h.efforts
                    .find_single_open_for_thread(&h.thread)
                    .await
                    .unwrap()
                    .is_none(),
                "two efforts open — the single-open rule must decline"
            );

            let owner = |cmd: &'static str| {
                let svc = h.service.clone();
                let thread = h.thread;
                async move {
                    svc.resolve_owning_effort_for_command(&thread, None, Some(cmd))
                        .await
                        .map(|e| e.id)
                }
            };
            assert_eq!(
                owner("cargo test -p oxplow-git symlink").await,
                Some(eff1),
                "the git run belongs to the effort working in crates/oxplow-git"
            );
            assert_eq!(
                owner("cargo test -p oxplow-config gauge").await,
                Some(eff2.id),
                "the config run belongs to the other effort"
            );
            // A whole-suite run names nothing, so it still declines rather than
            // guessing — the agent claims it at close.
            assert_eq!(owner("bun run test:collect").await, None);
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
            let eff2 = h
                .efforts
                .start(&work_item_ref(task2), &h.thread, None)
                .await
                .unwrap();
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
            let eff2 = h
                .efforts
                .start(&work_item_ref(task2), &h.thread, None)
                .await
                .unwrap();
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
            let eff2 = h
                .efforts
                .start(&work_item_ref(task2), &h.thread, None)
                .await
                .unwrap();

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
            let h = build_full(Some(COBERTURA_50PCT), true).await;
            ingest_coverage(&h).await;
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
            let junit = match Collector::bundled("junit")
                .unwrap()
                .run(
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
                .effort_observations_from_metrics(&h.effort_id, Some("test-run"))
                .await;
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
                ingest_coverage(&h).await,
                CoverageIngest::Stored { .. }
            ));
        }

        const JUNIT_ONE: &str = r#"<testsuites><testsuite name="s"><testcase classname="c" name="t1"/></testsuite></testsuites>"#;

        /// The rows `collector_run` has for project collector `id`:
        /// `(status, error)`.
        async fn collector_run(h: &Harness, id: &str) -> Option<(String, Option<String>)> {
            let id = id.to_string();
            h.db.read(move |c| {
                use rusqlite::OptionalExtension;
                c.query_row(
                    "SELECT status, error FROM collector_run WHERE owner = 'project' AND id = ?1",
                    [&id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
        }

        /// tsk863: a test run reads its `on_run: test` collectors, an
        /// analyzer run its `on_run: analysis` ones — never the other's.
        #[tokio::test]
        async fn a_run_reads_only_its_on_run_collectors() {
            let h = build(None).await;
            std::fs::write(h.tmp.path().join("tests.xml"), JUNIT_ONE).unwrap();
            std::fs::write(h.tmp.path().join("clippy.json"), CLIPPY_JSON).unwrap();
            declare(
                &h,
                vec![
                    report_collector("tests.junit", "tests", "oxplow:junit", "tests.xml", "test"),
                    report_collector(
                        "lint.clippy",
                        "analysis",
                        "oxplow:clippy",
                        "clippy.json",
                        "analysis",
                    ),
                ],
            );
            let test = run_reads(&h, RunKind::Test).await;
            assert!(test.tests().is_some());
            assert!(test.analysis().is_none());
            assert_eq!(collector_run(&h, "lint.clippy").await, None, "not run");
            let analysis = run_reads(&h, RunKind::Analysis).await;
            assert!(analysis.analysis().is_some());
            assert!(analysis.tests().is_none());
        }

        /// tsk863: what a report collector read is its run: a
        /// `collector_run` row and a `collector.synced`, like any
        /// collector's — caused by the detected run, once per run.
        #[tokio::test]
        async fn a_report_collector_run_is_recorded() {
            let h = build(None).await;
            std::fs::write(h.tmp.path().join("tests.xml"), JUNIT_ONE).unwrap();
            declare(
                &h,
                vec![report_collector(
                    "tests.junit",
                    "tests",
                    "oxplow:junit",
                    "tests.xml",
                    "test",
                )],
            );
            let cause = RunCause {
                event_id: oxplow_domain::EventId::generate().to_string(),
                seq: 42,
                anchors: Default::default(),
                at: Timestamp::now(),
            };
            for _ in 0..2 {
                h.service
                    .read_run_reports(RunKind::Test, FreshWindow::around(cause.at), Some(&cause))
                    .await;
            }
            assert_eq!(
                collector_run(&h, "tests.junit").await,
                Some(("ok".into(), None))
            );
            let synced: Vec<serde_json::Value> =
                h.db.read(|c| {
                    let mut stmt = c
                        .prepare("SELECT payload FROM event_log WHERE type = 'collector.synced'")
                        .map_err(oxplow_db::map_sql_err)?;
                    let rows = stmt
                        .query_map([], |r| r.get::<_, String>(0))
                        .map_err(oxplow_db::map_sql_err)?
                        .map(|r| serde_json::from_str(&r.unwrap()).unwrap())
                        .collect();
                    Ok(rows)
                })
                .await
                .unwrap();
            assert_eq!(synced.len(), 1, "a redelivered run records nothing more");
            assert_eq!(synced[0]["collector"], "collector:project/tests.junit");
            assert_eq!(synced[0]["trigger"], "on");
            assert_eq!(synced[0]["status"], "ok");
        }

        /// tsk863: a program parser runs only once a person approved it on
        /// this machine, like any project collector's program; until then
        /// its run says why, and nothing is read.
        #[cfg(unix)]
        #[tokio::test]
        async fn an_exec_parser_waits_for_the_collector_approval() {
            use std::os::unix::fs::PermissionsExt;
            let h = build(None).await;
            let dir = h.tmp.path();
            std::fs::create_dir_all(dir.join("tools")).unwrap();
            let program = dir.join("tools/parse.sh");
            std::fs::write(
                &program,
                "#!/bin/sh\ncat >/dev/null\necho '{\"suites\":[{\"name\":\"s\",\"cases\":[{\"classname\":\"c\",\"name\":\"t\",\"status\":\"passed\"}]}]}'\n",
            )
            .unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::write(dir.join("out.txt"), "whatever the tool wrote").unwrap();
            let value: serde_yaml::Value = serde_yaml::from_str(
                "- { id: tests.parse, records: tests, runtime: exec, entry: tools/parse.sh, report: { path: out.txt }, trigger: { on_run: test } }",
            )
            .unwrap();
            let (specs, errors) = oxplow_config::collectors::parse_collectors(
                oxplow_config::collectors::PROJECT,
                &value,
                &|_| true,
            );
            assert_eq!(errors, Vec::<String>::new());
            declare(&h, specs);
            let approvals = Arc::new(crate::exec_consent::ApprovalStore::for_tests(dir));
            let service = h.service.clone().with_approvals(approvals.clone());
            let read = |service: CollectionService| async move {
                service
                    .read_run_reports(RunKind::Test, FreshWindow::ending_now(), None)
                    .await
            };

            assert!(read(service.clone()).await.tests().is_none());
            let (status, error) = collector_run(&h, "tests.parse").await.unwrap();
            assert_eq!(status, "needs_approval");
            assert!(error.unwrap().contains("approval"));

            let cfg = h.service.config.read().unwrap().clone();
            let version = crate::exec_consent::version_of(
                &approvals,
                dir,
                &cfg,
                crate::exec_consent::ProgramKind::Collector,
                "tests.parse",
            );
            crate::exec_consent::approve_program(
                &approvals,
                dir,
                &cfg,
                &[],
                crate::exec_consent::ProgramKind::Collector,
                "tests.parse",
                &version,
            )
            .unwrap();
            let reads = read(service).await;
            let (tests, source) = reads.tests().expect("the approved program parsed");
            assert_eq!(tests.suites[0].cases.len(), 1);
            assert_eq!(
                source, "plugin-exec:tests.parse",
                "a program's output is lower-trust"
            );
            assert_eq!(collector_run(&h, "tests.parse").await.unwrap().0, "ok");
        }

        /// tsk863: a report collector is held to the plugin failure policy
        /// (P7.C2): its third failed parse in a row disables it, and a
        /// disabled one reads nothing until a person enables it.
        #[tokio::test]
        async fn three_failed_parses_disable_the_collector() {
            let h = build(None).await;
            std::fs::write(h.tmp.path().join("tests.xml"), "<testsuites><testcase").unwrap();
            declare(
                &h,
                vec![report_collector(
                    "tests.junit",
                    "tests",
                    "oxplow:junit",
                    "tests.xml",
                    "test",
                )],
            );
            let health = crate::plugin_health::PluginHealth::new(
                h.db.clone(),
                oxplow_domain::vocabulary::VocabularyHandle::core(),
            );
            let key = crate::collector_runner::plugin_key("project", "tests.junit");
            for n in 1..=3 {
                assert_eq!(
                    health.disabled_reason(&key).await.unwrap(),
                    None,
                    "before failure {n}"
                );
                run_reads(&h, RunKind::Test).await;
                assert_eq!(collector_run(&h, "tests.junit").await.unwrap().0, "error");
            }
            assert!(health.disabled_reason(&key).await.unwrap().is_some());
            // A good report now isn't read: the collector is off.
            std::fs::write(h.tmp.path().join("tests.xml"), JUNIT_ONE).unwrap();
            assert!(run_reads(&h, RunKind::Test).await.tests().is_none());
            assert!(matches!(
                h.service
                    .sync_report_collector(&h.thread, "tests.junit", "human")
                    .await,
                Err(crate::collector_runner::RunCollectorError::Disabled(_))
            ));
        }

        /// tsk863: `collector.sync` runs a collector the project declares.
        #[tokio::test]
        async fn an_undeclared_report_collector_is_not_found() {
            let h = build(None).await;
            assert!(matches!(
                h.service
                    .sync_report_collector(&h.thread, "tests.coverage", "human")
                    .await,
                Err(crate::collector_runner::RunCollectorError::NotFound)
            ));
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
                ingest_coverage(&h).await,
                CoverageIngest::Stored { .. }
            ));
            // …but the effort's DERIVED diff is empty (line 1 is unchanged), so no
            // diff-coverage observation surfaces for it.
            let rows = h
                .service
                .effort_observations_from_metrics(&h.effort_id, Some("diff-coverage"))
                .await;
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
        async fn on_post_tool_use_nudges_on_report_less_test_run() {
            // A detected test command with no fresh report → nudge returned.
            let h = build(None).await;
            // Configure a test command so the nudge names it.
            {
                let mut cfg = h.service.config.write().unwrap();
                cfg.testing.command = Some("bun run test:collect".into());
            }
            let result = h
                .service
                .on_post_tool_use(&h.thread, &bash_payload("bun test --watch false", 0), None)
                .await
                .unwrap();
            let nudge = result.expect("nudge returned for report-less run");
            assert!(
                nudge.contains("bun run test:collect"),
                "nudge should name the configured test command; got: {nudge}"
            );
            // A report-LESS run produces no parseable counts, so the substrate
            // records no test sample → the effort panel shows no test-run row
            // for it (tsk215). The report-less *nudge* is what surfaces the run
            // to the agent; the command-only "ran-record" marker is retired.
            let obs = h
                .service
                .effort_observations_from_metrics(&h.effort_id, Some("test-run"))
                .await;
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
            let h = build_full(Some(COBERTURA_50PCT), true).await;
            ingest_coverage(&h).await;
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

        #[tokio::test]
        async fn on_post_tool_use_no_nudge_when_report_produced() {
            // A detected test command that regenerated a fresh JUnit report →
            // no nudge. We use merge_fresh_test_reports directly: this
            // exercises exactly the branch that suppresses the nudge
            // (produced_report is true).
            let h = build(None).await;
            std::fs::write(
                h.tmp.path().join("tests.xml"),
                r#"<testsuites><testsuite name="suite"><testcase classname="c" name="t1"/></testsuite></testsuites>"#,
            )
            .unwrap();
            declare(
                &h,
                vec![report_collector(
                    "r0",
                    "tests",
                    "oxplow:junit",
                    "tests.xml",
                    "test",
                )],
            );
            // Just written, so inside a window ending now.
            let report = run_reads(&h, RunKind::Test)
                .await
                .tests()
                .map(|(r, s)| (r.clone(), s));
            assert!(
                report.is_some(),
                "fresh JUnit report should be merged (effort start = epoch)"
            );
            // When produced_report is true, mark_nudged is never called.
            // Directly verify: set the nudge flag manually, confirm it was
            // only set once the test asks for it (i.e. nudge didn't fire).
            let nudged = h
                .service
                .nudges
                .has_fired(901, "report-less-run")
                .await
                .unwrap();
            assert!(
                !nudged,
                "nudge should not have fired when a report was produced"
            );
        }

        #[tokio::test]
        async fn a_run_delivered_after_its_window_fires_no_nudge() {
            // A redelivery (crash before the checkpoint, a retried dead
            // letter, a backlog) of a report-less test run judged now would
            // nudge; judged at its own time it can't be judged at all.
            let h = build(None).await;
            let eid = oxplow_domain::EffortId::try_from_str(&h.effort_id).unwrap();
            let old =
                Timestamp::from_unix_ms(Timestamp::now().unix_ms() - 2 * REPORT_FRESH_WINDOW_MS);
            let cause = RunCause {
                event_id: "evt-late".into(),
                seq: 0,
                anchors: oxplow_domain::Anchors {
                    effort_id: Some(eid),
                    ..Default::default()
                },
                at: old,
            };
            let result = h
                .service
                .on_post_tool_use(
                    &h.thread,
                    &bash_payload("bun test --watch false", 0),
                    Some(&cause),
                )
                .await
                .unwrap();
            assert_eq!(result, None);
            assert!(!h
                .service
                .nudges
                .has_fired(eid.value(), "report-less-run")
                .await
                .unwrap());
        }

        #[tokio::test]
        async fn on_post_tool_use_nudge_fires_once_per_effort() {
            // The nudge is at most once per effort — second no-report run
            // returns None.
            let h = build(None).await;
            {
                let mut cfg = h.service.config.write().unwrap();
                cfg.testing.command = Some("bun run test:collect".into());
            }
            let payload = bash_payload("bun test", 0);
            let first = h
                .service
                .on_post_tool_use(&h.thread, &payload, None)
                .await
                .unwrap();
            assert!(first.is_some(), "first report-less run should nudge");
            let second = h
                .service
                .on_post_tool_use(&h.thread, &payload, None)
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
                cfg.testing.command = Some("bun run test:collect".into());
            }
            let payload = bash_payload("bun test --watch false", 0);
            h.service
                .on_post_tool_use(&h.thread, &payload, None)
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
                .on_post_tool_use(&h.thread, &payload, None)
                .await
                .unwrap();
            assert!(second.is_none(), "second run deduped");
            let rows = h.nudges.list_for_effort(&h.effort_id).await.unwrap();
            assert_eq!(rows.len(), 1, "deduped nudge must not double-store");
        }

        #[tokio::test]
        async fn on_post_tool_use_no_nudge_for_non_test_command() {
            // A non-test Bash command → no nudge, no observation.
            let h = build(None).await;
            let result = h
                .service
                .on_post_tool_use(&h.thread, &bash_payload("cargo build", 0), None)
                .await
                .unwrap();
            assert!(result.is_none());
            let obs = h
                .service
                .effort_observations_from_metrics(&h.effort_id, Some("test-run"))
                .await;
            assert!(obs.is_empty());
        }

        #[tokio::test]
        async fn on_post_tool_use_routes_to_configure_when_no_collection_profile() {
            // No collection block → nudge routes to /oxplow:configure.
            let h = build(None).await;
            // No test command and no report collectors by default.
            let result = h
                .service
                .on_post_tool_use(&h.thread, &bash_payload("bun test", 0), None)
                .await
                .unwrap();
            let nudge = result.expect("nudge returned even without report collectors");
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
            // Just written, so inside a window ending now.
            declare(
                &h,
                vec![
                    report_collector("r0", "tests", "oxplow:junit", "rust.xml", "test"),
                    report_collector("r1", "tests", "oxplow:junit", "front.xml", "test"),
                ],
            );
            let reads = run_reads(&h, RunKind::Test).await;
            let (merged, source) = reads.tests().expect("both fresh reports merged");
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
                .effort_observations_from_metrics(&h.effort_id, Some("static-analysis"))
                .await;
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
                .effort_observations_from_metrics(&h.effort_id, Some("static-analysis"))
                .await;
            assert!(
                rows.is_empty(),
                "no substrate row for a report-less analyzer run"
            );
        }

        #[tokio::test]
        async fn merge_fresh_analysis_unions_findings_from_reports() {
            let h = build(None).await;
            std::fs::write(h.tmp.path().join("clippy.json"), CLIPPY_JSON).unwrap();
            // Just written, so inside a window ending now.
            declare(
                &h,
                vec![report_collector(
                    "r0",
                    "analysis",
                    "oxplow:clippy",
                    "clippy.json",
                    "analysis",
                )],
            );
            let reads = run_reads(&h, RunKind::Analysis).await;
            let (merged, source) = reads.analysis().expect("fresh clippy report merged");
            assert_eq!(merged.findings.len(), 2);
            assert_eq!(source, "analysis-report");
            assert_eq!(reads.analyzers, vec!["clippy".to_string()]);
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
            // End-to-end TS path: the bundled eslint parser → store, through
            // the real service entry point (not just the golden parser test).
            let h = build(None).await;
            std::fs::write(h.tmp.path().join("eslint.json"), ESLINT_JSON).unwrap();
            declare(
                &h,
                vec![report_collector(
                    "lint.eslint",
                    "analysis",
                    "oxplow:eslint",
                    "eslint.json",
                    "analysis",
                )],
            );
            let outcome = match h
                .service
                .sync_report_collector(&h.thread, "lint.eslint", "human")
                .await
                .unwrap()
            {
                ReportSync::Analysis(a) => a,
                other => panic!("{other:?}"),
            };
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
                .effort_observations_from_metrics(&h.effort_id, Some("static-analysis"))
                .await;
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
                .start(&open.work_item, &h.thread, None)
                .await
                .unwrap();
            assert!(no_base.start_snapshot_id.is_none());

            std::fs::write(h.tmp.path().join("eslint.json"), ESLINT_JSON).unwrap();
            declare(
                &h,
                vec![report_collector(
                    "lint.eslint",
                    "analysis",
                    "oxplow:eslint",
                    "eslint.json",
                    "analysis",
                )],
            );
            let outcome = match h
                .service
                .sync_report_collector(&h.thread, "lint.eslint", "human")
                .await
                .unwrap()
            {
                ReportSync::Analysis(a) => a,
                other => panic!("{other:?}"),
            };
            match outcome {
                AnalysisIngest::Stored { findings, .. } => assert_eq!(findings, 3),
                other => panic!("expected Stored with no baseline, got {other:?}"),
            }
            // The observation landed, pinned to no local snapshot.
            let rows = h
                .service
                .effort_observations_from_metrics(&no_base.id.to_string(), Some("static-analysis"))
                .await;
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].local_snapshot_id, None);
        }

        /// tsk863: a by-hand run of a collector whose report isn't there
        /// says so; nothing ran, so nothing is recorded.
        #[tokio::test]
        async fn a_missing_report_is_named() {
            let h = build(None).await;
            declare(
                &h,
                vec![report_collector(
                    "lint.eslint",
                    "analysis",
                    "oxplow:eslint",
                    "nope.json",
                    "analysis",
                )],
            );
            let err = h
                .service
                .sync_report_collector(&h.thread, "lint.eslint", "human")
                .await
                .unwrap_err();
            assert!(
                matches!(&err, crate::collector_runner::RunCollectorError::Failed(m) if m.contains("nope.json")),
                "{err:?}"
            );
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
                    None,
                )
                .await
                .unwrap();
            // No test patterns matched → no test nudge.
            assert!(result.is_none());
            assert!(h
                .service
                .effort_observations_from_metrics(&h.effort_id, Some("static-analysis"))
                .await
                .is_empty());
            assert!(h
                .service
                .effort_observations_from_metrics(&h.effort_id, Some("test-run"))
                .await
                .is_empty());
        }
    }
}
