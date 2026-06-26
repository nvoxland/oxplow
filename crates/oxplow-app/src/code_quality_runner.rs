//! Duplicate-block detection (`oxplow-code-dup`).
//!
//! Token-stream duplicate-block detection via `oxplow-code-dup`, surfaced in the
//! Change-analysis duplication card. The store + IPC refer to this by the
//! analysis-kind name `"duplication"`.
//!
//! (The former per-function metrics scan — complexity / length / parameter
//! count — was retired in tsk229: those signals now live in the metric
//! substrate as bundled, language-agnostic `oxplow.{high_complexity_fns,
//! long_functions, fn_count}` gauges, computed via the `code_metrics()` host
//! builtin across all languages (tsk314). Duplication has no
//! plugin equivalent — cross-file token matching can't run in Starlark — so it
//! stays an inherent in-process feature.)

use std::path::Path;
use std::sync::Arc;

use std::collections::BTreeSet;

use oxplow_code_dup::{detect_duplicates, detect_duplicates_scoped, DupOptions};
use oxplow_tree_source::{
    collect_corpus, AllFiles, DiskTreeSource, FileFilter, TreeError, TreeSource,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CodeQualityError {
    /// Surfaces a failure inside the spawn_blocking pool (panic or
    /// joining error).
    #[error("scan task failed: {0}")]
    Task(String),
    /// The scan exceeded the configured wall-clock budget.
    #[error("scan timed out after {0:?}")]
    Timeout(std::time::Duration),
    /// Tree source enumeration / read failed (git error, IO error,
    /// snapshot stub).
    #[error("tree source failed: {0}")]
    TreeSource(String),
    /// A scan-row store operation failed.
    #[error("scan store failed: {0}")]
    Store(String),
}

impl From<TreeError> for CodeQualityError {
    fn from(e: TreeError) -> Self {
        CodeQualityError::TreeSource(format!("{e}"))
    }
}

/// Default wall-clock budget for a single scan. Tunable via
/// `RunOptions::timeout`.
const DEFAULT_SCAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// One finding the renderer surfaces in the duplication card.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CodeQualityFinding {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    /// `"duplicate-block"` (the only kind produced now that the metrics scan is
    /// retired — see the module docs).
    pub kind: String,
    pub metric_value: f64,
    /// Free-form JSON for analysis-specific metadata. The store
    /// persists this as a string column.
    pub extra_json: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Subset of repo-relative paths. Empty = scan whole repo.
    pub files: Vec<String>,
    /// Wall-clock budget. `None` uses [`DEFAULT_SCAN_TIMEOUT`].
    pub timeout: Option<std::time::Duration>,
    /// Override the duplicate-detector tunables. `None` uses
    /// `DupOptions::default()` (production: min_lines=10).
    pub dup_options: Option<DupOptions>,
}

/// Duplicate-block scan against an arbitrary tree version.
///
/// `source` enumerates files and reads their content (Disk = working
/// tree, GitRef = a commit's tree, …); `filter` decides which paths
/// from the source make it into the corpus. Every pair of corpus
/// docs is matched, including same-file self-matches — this is the
/// "scan everything" mode used by the standalone Code Quality panel.
/// The change-analysis flow uses
/// [`run_duplication_scan_scoped`] instead so unchanged peers
/// participate as match targets without adding their own findings.
///
/// The whole scan runs on `spawn_blocking` because tree-sitter and
/// libgit2 are CPU/IO-bound; the trait objects are `Send + Sync` so
/// `Arc`-wrapping them lets us move references into the blocking
/// pool.
pub async fn run_duplication_scan_with(
    source: Arc<dyn TreeSource>,
    filter: Arc<dyn FileFilter>,
    workspace_filter: oxplow_fs_watch::WorkspaceFilter,
    timeout: Option<std::time::Duration>,
    dup_options: Option<DupOptions>,
) -> Result<Vec<CodeQualityFinding>, CodeQualityError> {
    let timeout = timeout.unwrap_or(DEFAULT_SCAN_TIMEOUT);
    let dup_opts = dup_options.unwrap_or_default();
    let filter: Arc<dyn FileFilter> = Arc::new(WorkspaceFileFilter::new(filter, workspace_filter));
    let task = tokio::task::spawn_blocking(move || -> Result<_, CodeQualityError> {
        let corpus = collect_corpus(source.as_ref(), filter.as_ref())?;
        // Drop entries the metrics layer can't parse — the detector
        // tolerates unsupported files but we'd rather not feed them
        // through tree-sitter at all.
        let inputs: Vec<(String, String)> = corpus
            .into_iter()
            .filter(|(p, _)| oxplow_code_metrics::is_supported_path(Path::new(p)))
            .collect();
        let blocks = detect_duplicates(inputs, dup_opts);
        Ok(blocks_to_findings(blocks))
    });
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(inner)) => inner,
        Ok(Err(join_err)) => Err(CodeQualityError::Task(format!(
            "duplication task: {join_err}"
        ))),
        Err(_) => Err(CodeQualityError::Timeout(timeout)),
    }
}

/// Scoped duplicate-block scan: corpus is the WHOLE tree (every
/// supported file the source enumerates), but a finding only
/// surfaces when at least one side's path is in `scope_filter`. The
/// scope-side is rotated to side A so the renderer's
/// "you're analyzing this file vs the peer over there" convention
/// holds. Same-path matches (a region of a file matching another
/// region of the SAME file) are dropped — those are almost always
/// shifted-by-one winnowing artifacts on long token streams.
///
/// This is what the change-analysis page wants: when a user changes
/// `foo.ts`, surface duplications between `foo.ts` and ANY existing
/// file in the repo, not just other changed files. Without this
/// mode the scan would miss copy-paste from an unchanged peer.
pub async fn run_duplication_scan_scoped(
    source: Arc<dyn TreeSource>,
    scope_filter: Arc<dyn FileFilter>,
    workspace_filter: oxplow_fs_watch::WorkspaceFilter,
    timeout: Option<std::time::Duration>,
    dup_options: Option<DupOptions>,
) -> Result<Vec<CodeQualityFinding>, CodeQualityError> {
    let timeout = timeout.unwrap_or(DEFAULT_SCAN_TIMEOUT);
    let dup_opts = dup_options.unwrap_or_default();
    let task = tokio::task::spawn_blocking(move || -> Result<_, CodeQualityError> {
        // The corpus deliberately uses AllFiles (modulo the workspace
        // ignore list) — the scope filter determines which findings
        // we keep, NOT which files we walk. A copy-paste from an
        // unchanged file only surfaces when that unchanged file is in
        // the corpus. Generated files are kept OUT of the corpus too:
        // their duplication is by construction (build output, vendored
        // code) and surfacing it is noise.
        let corpus_filter = WorkspaceFileFilter::new(Arc::new(AllFiles), workspace_filter);
        let corpus = collect_corpus(source.as_ref(), &corpus_filter)?;
        let inputs: Vec<(String, String)> = corpus
            .into_iter()
            .filter(|(p, _)| oxplow_code_metrics::is_supported_path(Path::new(p)))
            .collect();
        let scope: BTreeSet<String> = inputs
            .iter()
            .map(|(p, _)| p.clone())
            .filter(|p| scope_filter.keep(p))
            .collect();
        let blocks = detect_duplicates_scoped(inputs, &scope, dup_opts);
        Ok(blocks_to_findings(blocks))
    });
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(inner)) => inner,
        Ok(Err(join_err)) => Err(CodeQualityError::Task(format!(
            "duplication task: {join_err}"
        ))),
        Err(_) => Err(CodeQualityError::Timeout(timeout)),
    }
}

/// Backward-compat thin wrapper for callers that still pass a
/// project_dir: defaults to `DiskTreeSource` + `AllFiles`. New
/// callers should construct the source/filter explicitly via
/// [`run_duplication_scan_with`].
pub async fn run_duplication_scan(
    project_dir: &Path,
    opts: RunOptions,
    workspace_filter: oxplow_fs_watch::WorkspaceFilter,
) -> Result<Vec<CodeQualityFinding>, CodeQualityError> {
    let source: Arc<dyn TreeSource> = Arc::new(DiskTreeSource::new(project_dir.to_path_buf()));
    let filter: Arc<dyn FileFilter> = if opts.files.is_empty() {
        Arc::new(AllFiles)
    } else {
        Arc::new(oxplow_tree_source::ExplicitPaths::new(
            opts.files.iter().cloned(),
        ))
    };
    run_duplication_scan_with(
        source,
        filter,
        workspace_filter,
        opts.timeout,
        opts.dup_options,
    )
    .await
}

/// `FileFilter` adapter: keeps a path iff the wrapped inner filter
/// keeps it AND the `WorkspaceFilter` doesn't ignore it. Used to
/// fold the user's `generated` config into the duplication-scan
/// corpus without changing the `FileFilter` abstraction.
struct WorkspaceFileFilter {
    inner: Arc<dyn FileFilter>,
    workspace: oxplow_fs_watch::WorkspaceFilter,
}

impl WorkspaceFileFilter {
    fn new(inner: Arc<dyn FileFilter>, workspace: oxplow_fs_watch::WorkspaceFilter) -> Self {
        Self { inner, workspace }
    }
}

impl FileFilter for WorkspaceFileFilter {
    fn keep(&self, path: &str) -> bool {
        if !self.inner.keep(path) {
            return false;
        }
        // Code-quality only ever feeds file paths here.
        !self.workspace.ignore(Path::new(path), false)
    }
}

fn blocks_to_findings(blocks: Vec<oxplow_code_dup::DuplicateBlock>) -> Vec<CodeQualityFinding> {
    let mut out = Vec::with_capacity(blocks.len() * 2);
    for b in blocks {
        let extra_a = format!(
            r#"{{"peerPath":{:?},"peerStartLine":{},"peerEndLine":{}}}"#,
            b.b_path, b.b_start_line, b.b_end_line
        );
        out.push(CodeQualityFinding {
            path: b.a_path.clone(),
            start_line: b.a_start_line,
            end_line: b.a_end_line,
            kind: "duplicate-block".into(),
            metric_value: b.line_count as f64,
            extra_json: Some(extra_a),
        });
        let extra_b = format!(
            r#"{{"peerPath":{:?},"peerStartLine":{},"peerEndLine":{}}}"#,
            b.a_path, b.a_start_line, b.a_end_line
        );
        out.push(CodeQualityFinding {
            path: b.b_path,
            start_line: b.b_start_line,
            end_line: b.b_end_line,
            kind: "duplicate-block".into(),
            metric_value: b.line_count as f64,
            extra_json: Some(extra_b),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn duplication_scan_emits_paired_findings_for_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let body = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#;
        std::fs::write(dir.path().join("a.rs"), body).unwrap();
        std::fs::write(dir.path().join("b.rs"), body).unwrap();
        let opts = RunOptions {
            dup_options: Some(DupOptions {
                min_lines: 5,
                ..DupOptions::default()
            }),
            ..RunOptions::default()
        };
        let findings = run_duplication_scan(
            dir.path(),
            opts,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .await
        .unwrap();
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.kind == "duplicate-block")
            .collect();
        assert!(
            dups.len() >= 2,
            "expected at least one paired duplicate, got {:?}",
            findings
        );
        // Each finding's extra_json must carry the peer side as flat
        // keys (peerPath / peerStartLine / peerEndLine) — the panel
        // renderer reads them directly off `extra` without unwrapping
        // a nested object.
        for f in &dups {
            let raw = f.extra_json.as_deref().expect("extra_json present");
            let parsed: serde_json::Value =
                serde_json::from_str(raw).expect("extra_json parses as JSON");
            assert!(
                parsed.get("peerPath").and_then(|v| v.as_str()).is_some(),
                "expected peerPath in extra_json, got {raw}"
            );
            assert!(
                parsed
                    .get("peerStartLine")
                    .and_then(|v| v.as_i64())
                    .is_some(),
                "expected peerStartLine in extra_json, got {raw}"
            );
            assert!(
                parsed.get("peerEndLine").and_then(|v| v.as_i64()).is_some(),
                "expected peerEndLine in extra_json, got {raw}"
            );
        }
    }

    /// Integration: a multi-file fixture exercises the duplication scanner
    /// end-to-end (file walker + relative-path stripping + cross-doc dup
    /// matching), including a skipped dir and an unsupported file.
    #[tokio::test]
    async fn end_to_end_fixture_duplication_scan() {
        let dir = tempfile::tempdir().unwrap();
        // File A + File B: clones (same body, renamed identifiers).
        std::fs::write(
            dir.path().join("a.rs"),
            r#"
fn process(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.rs"),
            r#"
fn handle(values: Vec<i32>) -> Vec<i32> {
    let mut output = Vec::new();
    for v in values {
        if v > 0 {
            output.push(v * 2);
        } else if v < 0 {
            output.push(v * -1);
        } else {
            output.push(0);
        }
    }
    output
}
"#,
        )
        .unwrap();
        // Unsupported language — must not appear.
        std::fs::write(dir.path().join("README.md"), "# heading\nsome text\n").unwrap();
        // Nested skipped dir — must not be scanned.
        std::fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        std::fs::write(dir.path().join("target/debug/should_skip.rs"), "fn x() {}").unwrap();

        let filter = oxplow_fs_watch::WorkspaceFilter::with_user_entries(["target"]);
        let dup_opts = RunOptions {
            dup_options: Some(DupOptions {
                min_lines: 5,
                ..DupOptions::default()
            }),
            ..RunOptions::default()
        };
        let duplication = run_duplication_scan(dir.path(), dup_opts, filter)
            .await
            .unwrap();
        let dups: Vec<_> = duplication
            .iter()
            .filter(|f| f.kind == "duplicate-block")
            .collect();
        assert!(
            dups.len() >= 2,
            "expected paired duplicate, got {duplication:?}"
        );
        for f in &duplication {
            assert!(!f.path.starts_with('/'), "leaked absolute path: {}", f.path);
            assert!(
                !f.path.contains("target/"),
                "scanned skipped dir: {}",
                f.path
            );
        }
    }

    #[tokio::test]
    async fn duplication_scan_emits_nothing_for_unique_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "fn add(a: i32, b: i32) -> i32 { a + b }",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.rs"),
            "fn unrelated() { println!(\"hi\"); }",
        )
        .unwrap();
        let findings = run_duplication_scan(
            dir.path(),
            RunOptions::default(),
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .await
        .unwrap();
        assert!(findings.is_empty());
    }

    /// The dup scan must read content from the supplied tree source,
    /// not from disk. Set up a git repo whose committed `a.rs` and
    /// `b.rs` are intentional clones of each other, then mutate the
    /// disk versions to be unique. A scan against `HEAD` via
    /// `GitTreeSource` should still report the duplicates; a scan
    /// against `Disk` would not.
    /// The scoped runner walks the whole tree but only surfaces
    /// findings whose A side is in scope. Verifies the
    /// change-analysis "compare changed files against everything"
    /// semantic.
    #[tokio::test]
    async fn duplication_scan_scoped_finds_clones_in_unchanged_peers() {
        use oxplow_tree_source::{DiskTreeSource, ExplicitPaths};
        let dir = tempfile::tempdir().unwrap();
        let body = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#;
        std::fs::write(dir.path().join("changed.rs"), body).unwrap();
        std::fs::write(dir.path().join("untouched.rs"), body).unwrap();
        let source: Arc<dyn TreeSource> = Arc::new(DiskTreeSource::new(dir.path().to_path_buf()));
        // Scope = only the changed file. The peer (untouched.rs) is
        // NOT in scope but must still participate as a match
        // target.
        let scope: Arc<dyn FileFilter> =
            Arc::new(ExplicitPaths::new(vec!["changed.rs".to_string()]));
        let findings = run_duplication_scan_scoped(
            source,
            scope,
            oxplow_fs_watch::WorkspaceFilter::default(),
            None,
            Some(DupOptions {
                min_lines: 5,
                ..DupOptions::default()
            }),
        )
        .await
        .unwrap();
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.kind == "duplicate-block")
            .collect();
        assert!(
            !dups.is_empty(),
            "expected dup findings between changed and untouched, got {findings:?}",
        );
        // Every finding's anchor (path) is the scope file; the peer
        // (extra.peerPath) is the unchanged file. The flat
        // findings list emits one record per side, so the scope
        // file shows up at least once.
        assert!(
            findings.iter().any(|f| f.path == "changed.rs"),
            "expected changed.rs to anchor at least one finding"
        );
    }

    /// Same-file pairs (file matched against itself, two regions
    /// in one file) must be dropped by the scoped runner.
    #[tokio::test]
    async fn duplication_scan_scoped_drops_same_file_self_match() {
        use oxplow_tree_source::{DiskTreeSource, ExplicitPaths};
        let dir = tempfile::tempdir().unwrap();
        let body_with_repeat = r#"
fn case_a(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 { out.push(item * 2); }
        else if item < 0 { out.push(item * -1); }
        else { out.push(0); }
    }
    out
}

fn case_b(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 { out.push(item * 2); }
        else if item < 0 { out.push(item * -1); }
        else { out.push(0); }
    }
    out
}
"#;
        std::fs::write(dir.path().join("only.rs"), body_with_repeat).unwrap();
        let source: Arc<dyn TreeSource> = Arc::new(DiskTreeSource::new(dir.path().to_path_buf()));
        let scope: Arc<dyn FileFilter> = Arc::new(ExplicitPaths::new(vec!["only.rs".to_string()]));
        let findings = run_duplication_scan_scoped(
            source,
            scope,
            oxplow_fs_watch::WorkspaceFilter::default(),
            None,
            None,
        )
        .await
        .unwrap();
        // Even if the engine surfaces in-file matches, the scoped
        // runner's same-path filter must drop them.
        for f in &findings {
            let raw = f.extra_json.as_deref().unwrap_or("{}");
            let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
            let peer = parsed
                .get("peerPath")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assert_ne!(peer, f.path, "same-file pair leaked: {f:?}");
        }
    }

    #[tokio::test]
    async fn duplication_scan_reads_from_tree_source_not_disk() {
        use oxplow_tree_source::{AllFiles, GitTreeSource};
        use std::process::Command;
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(path)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} failed: {:?}", out);
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        let body = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#;
        std::fs::write(path.join("a.rs"), body).unwrap();
        std::fs::write(path.join("b.rs"), body).unwrap();
        run(&["add", "a.rs", "b.rs"]);
        run(&["commit", "-q", "-m", "first"]);
        // After commit: stomp the disk versions so they're no longer
        // duplicates. Any scan that secretly reads disk would now
        // emit zero findings.
        std::fs::write(path.join("a.rs"), "fn unique_a() {}").unwrap();
        std::fs::write(path.join("b.rs"), "fn unique_b() {}").unwrap();

        let source: Arc<dyn TreeSource> = Arc::new(GitTreeSource::new(path, "HEAD"));
        let filter: Arc<dyn FileFilter> = Arc::new(AllFiles);
        let findings = run_duplication_scan_with(
            source,
            filter,
            oxplow_fs_watch::WorkspaceFilter::default(),
            None,
            Some(DupOptions {
                min_lines: 5,
                ..DupOptions::default()
            }),
        )
        .await
        .unwrap();
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.kind == "duplicate-block")
            .collect();
        assert!(
            dups.len() >= 2,
            "expected dup findings from HEAD content, got {findings:?}"
        );
    }
}
