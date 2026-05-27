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

use oxplow_config::OxplowConfig;
use oxplow_coverage::CoverageFormat;
use oxplow_db::observation_store::{NewEffortObservation, SqliteEffortObservationStore};
use oxplow_db::{
    SqliteSnapshotStore, SqliteTaskEffortStore, SqliteThreadStore, TaskEffort, TaskEffortStore,
};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{DomainError, ThreadId};

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

/// Does `command` look like a test run? Case-insensitive substring match
/// against the built-in patterns plus any caller-supplied extras.
pub fn detect_test_run(command: &str, extra_patterns: &[String]) -> bool {
    let lower = command.to_ascii_lowercase();
    DEFAULT_TEST_PATTERNS
        .iter()
        .map(|s| s.to_string())
        .chain(extra_patterns.iter().map(|s| s.to_ascii_lowercase()))
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
        }
    }

    /// Resolve the stream id that owns `thread` (the observation's hard
    /// scope + CASCADE anchor).
    async fn stream_id_for(&self, thread: &ThreadId) -> Result<Option<String>, DomainError> {
        Ok(self
            .threads
            .get(thread)
            .await?
            .map(|t| t.stream_id.as_str().to_string()))
    }

    fn collection_cfg(&self) -> oxplow_config::CollectionConfig {
        self.config
            .read()
            .map(|c| c.collection.clone())
            .unwrap_or_default()
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
        if let Some(p) = passed {
            payload.insert("passed".into(), json!(p));
        }
        if let Some(f) = failed {
            payload.insert("failed".into(), json!(f));
        }
        if let Some(t) = total {
            payload.insert("total".into(), json!(t));
        }
        let id = self
            .observations
            .record(NewEffortObservation {
                stream_id,
                effort_id: effort.id.as_str().to_string(),
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

    /// Ingest a coverage report and store a `diff-coverage` observation
    /// over the open effort's changed lines. oxplow parses the report
    /// itself, so the result is `observed`.
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
        let report_path = report_path_override.or(cfg.coverage_report_path);
        let format_str = format_override.or(cfg.coverage_format);
        let (Some(report_path), Some(format_str)) = (report_path, format_str) else {
            return Ok(CoverageIngest::NotConfigured);
        };
        let Some(format) = CoverageFormat::from_name(&format_str) else {
            return Ok(CoverageIngest::ParseError(format!(
                "unknown coverage format \"{format_str}\""
            )));
        };
        let abs = self.project_dir.join(&report_path);
        let Ok(content) = std::fs::read_to_string(&abs) else {
            return Ok(CoverageIngest::ReportMissing(report_path));
        };
        // Ride-along guard: a report whose mtime predates this effort is
        // from an earlier run the just-detected test command didn't
        // regenerate (e.g. `cargo test` when coverage comes from
        // `cargo cov`). Attributing it to this effort would be stale, so
        // skip. An explicit `ingest_coverage` (skip_if_stale = false)
        // bypasses this — the caller asked for it.
        if skip_if_stale && report_is_stale(&abs, effort.started_at) {
            return Ok(CoverageIngest::StaleReport(report_path));
        }
        let report = match oxplow_coverage::parse(format, &content) {
            Ok(r) => r,
            Err(e) => return Ok(CoverageIngest::ParseError(e.to_string())),
        };
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
                stream_id,
                effort_id: effort.id.as_str().to_string(),
                kind: "diff-coverage".into(),
                provenance: "observed".into(),
                source: "coverage-report".into(),
                metric_value: Some(summary_pct),
                payload_json: Some(payload.to_string()),
                local_snapshot_id: Some(version.local_snapshot_id),
                closest_git_version: version.closest_git_version,
                git_version_exact: version.git_version_exact,
            })
            .await?;
        self.emit(thread, &effort);
        Ok(CoverageIngest::Stored {
            observation_id: id,
            summary_pct,
            changed_lines: total_changed,
            covered_lines: total_covered,
        })
    }

    /// PostToolUse entry point: detect a test run, record it, and ride
    /// along to coverage. Best-effort — never fails the hook.
    pub async fn on_post_tool_use(
        &self,
        thread: &ThreadId,
        payload_json: &str,
    ) -> Result<(), DomainError> {
        let Some(bash) = parse_bash_post_tool(payload_json) else {
            return Ok(());
        };
        let cfg = self.collection_cfg();
        if !detect_test_run(&bash.command, &cfg.test_run_patterns) {
            return Ok(());
        }
        self.record_test_run(
            thread,
            &bash.command,
            bash.exit_code,
            None,
            None,
            None,
            None,
            "observed",
            "post-tool-bash",
        )
        .await?;
        // Coverage ride-along (only if a report is configured).
        if cfg.coverage_report_path.is_some() && cfg.coverage_format.is_some() {
            let _ = self.ingest_coverage(thread, None, None, true).await?;
        }
        Ok(())
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
            thread_id: thread.clone(),
            effort_id: effort.id.as_str().to_string(),
        });
    }
}

/// True when `path`'s mtime is at or before `effort_start` — i.e. the
/// report wasn't (re)generated during this effort. Conservative: if the
/// mtime can't be read, returns `false` (ingest rather than silently drop
/// a report on a platform that won't surface mtimes).
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

#[cfg(test)]
mod tests {
    use super::*;

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
            _tmp: tempfile::TempDir,
        }

        /// Build the fixture. `report_xml` Some → write it + configure the
        /// collection profile (cobertura/coverage.xml); None → leave
        /// collection unconfigured. The effort's start snapshot holds
        /// `src/foo.rs` as `a\nb\nc\n`; the working tree has `a\nB\nc\nd\n`
        /// (lines 2 changed, 4 added).
        async fn build(report_xml: Option<&str>) -> Harness {
            let tmp = tempfile::tempdir().unwrap();
            let project_dir = tmp.path().to_path_buf();
            std::fs::create_dir_all(project_dir.join(".oxplow/snapshots")).unwrap();
            let db = Database::in_memory();
            let now = Timestamp::now();

            let stream = Stream {
                id: StreamId::from("s-1"),
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
                id: ThreadId::from("b-1"),
                stream_id: stream.id.clone(),
                title: "x".into(),
                status: ThreadStatus::Active,
                sort_index: 0,
                pane_target: "working".into(),
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
                    thread_id: Some(thread.id.clone()),
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
            let snap_id = snapshots.create_snapshot(stream.id.clone()).await.unwrap();
            snapshots
                .capture(FileSnapshot {
                    id: 0,
                    stream_id: stream.id.clone(),
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

            let efforts = Arc::new(SqliteTaskEffortStore::new(db.clone()));
            let effort = efforts
                .start(task_id, &thread.id, Some(snap_id))
                .await
                .unwrap();

            // Current working-tree content: line 2 changed, line 4 added.
            std::fs::create_dir_all(project_dir.join("src")).unwrap();
            std::fs::write(project_dir.join("src/foo.rs"), "a\nB\nc\nd\n").unwrap();

            let mut cfg = oxplow_config::load_project_config(&project_dir).unwrap();
            if let Some(xml) = report_xml {
                std::fs::write(project_dir.join("coverage.xml"), xml).unwrap();
                cfg.collection.coverage_report_path = Some("coverage.xml".into());
                cfg.collection.coverage_format = Some("cobertura".into());
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
                effort_id: effort.id.as_str().to_string(),
                efforts,
                _tmp: tmp,
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
            assert_eq!(rows[0].stream_id, "s-1");
            assert!(rows[0]
                .payload_json
                .as_deref()
                .unwrap()
                .contains("cargo test"));
        }

        #[tokio::test]
        async fn ingest_coverage_no_open_effort_after_finish() {
            let h = build(Some(COBERTURA_50PCT)).await;
            h.efforts
                .finish(&EffortId::from(h.effort_id.clone()), None, None)
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
    }
}
