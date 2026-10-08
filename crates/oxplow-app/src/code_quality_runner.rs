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
//! script equivalent — cross-file token matching can't run in Starlark — so it
//! stays an inherent in-process feature.)

use std::collections::BTreeSet;
use std::path::Path;

use oxplow_code_dup::{detect_duplicates_scoped, DupOptions};
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
    /// Reading the tree failed.
    #[error("reading the tree failed: {0}")]
    Tree(String),
    /// A scan-row store operation failed.
    #[error("scan store failed: {0}")]
    Store(String),
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
    /// Wall-clock budget. `None` uses [`DEFAULT_SCAN_TIMEOUT`].
    pub timeout: Option<std::time::Duration>,
    /// Override the duplicate-detector tunables. `None` uses
    /// `DupOptions::default()` (min_lines 5).
    pub dup_options: Option<DupOptions>,
}

/// Duplicated blocks in `corpus` — `(path, text)` pairs, read by the
/// caller from whichever revision it means (`crate::trees::Trees::corpus`).
/// Files the metrics layer can't parse are dropped.
///
/// The whole corpus takes part as match targets (a copy of an unchanged
/// file is found), but a block is kept only when a side is in `scope`,
/// that side is reported first, and same-file pairs — almost always
/// shifted-by-one winnowing artifacts — are dropped. A whole-tree scan is
/// the built-in `oxplow.duplicate_lines` collector's (`duplicate_blocks`).
///
/// Runs on the blocking pool (tree-sitter is CPU-bound) under the
/// `opts.timeout` budget.
pub async fn scan_duplicates(
    corpus: Vec<(String, String)>,
    scope: BTreeSet<String>,
    opts: RunOptions,
) -> Result<Vec<CodeQualityFinding>, CodeQualityError> {
    let timeout = opts.timeout.unwrap_or(DEFAULT_SCAN_TIMEOUT);
    let dup_opts = opts.dup_options.unwrap_or_default();
    let task = tokio::task::spawn_blocking(move || {
        let inputs: Vec<(String, String)> = corpus
            .into_iter()
            .filter(|(p, _)| oxplow_code_metrics::is_supported_path(Path::new(p)))
            .collect();
        blocks_to_findings(detect_duplicates_scoped(inputs, &scope, dup_opts))
    });
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(findings)) => Ok(findings),
        Ok(Err(join_err)) => Err(CodeQualityError::Task(format!(
            "duplication task: {join_err}"
        ))),
        Err(_) => Err(CodeQualityError::Timeout(timeout)),
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

    const BODY: &str = r#"
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

    fn small() -> RunOptions {
        RunOptions {
            dup_options: Some(DupOptions {
                min_lines: 5,
                ..DupOptions::default()
            }),
            ..RunOptions::default()
        }
    }

    fn corpus(files: &[(&str, &str)]) -> Vec<(String, String)> {
        files
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect()
    }

    fn peer(f: &CodeQualityFinding) -> String {
        let extra: serde_json::Value =
            serde_json::from_str(f.extra_json.as_deref().expect("extra_json")).unwrap();
        extra["peerPath"].as_str().expect("peerPath").to_string()
    }

    /// Each duplicate is reported from both sides, the peer carried as
    /// flat `peerPath` / `peerStartLine` / `peerEndLine` keys (the
    /// panel reads them straight off `extra`). An unsupported file never
    /// takes part.
    #[tokio::test]
    async fn duplicates_are_paired_with_their_peers() {
        let findings = scan_duplicates(
            corpus(&[("a.rs", BODY), ("b.rs", BODY), ("README.md", BODY)]),
            BTreeSet::from(["a.rs".to_string(), "README.md".to_string()]),
            small(),
        )
        .await
        .unwrap();
        assert!(findings.len() >= 2, "{findings:?}");
        for f in &findings {
            assert_eq!(f.kind, "duplicate-block");
            assert_ne!(f.path, "README.md");
            let extra: serde_json::Value =
                serde_json::from_str(f.extra_json.as_deref().unwrap()).unwrap();
            assert!(extra["peerStartLine"].is_i64() && extra["peerEndLine"].is_i64());
            assert_ne!(peer(f), "README.md");
        }
    }

    #[tokio::test]
    async fn unique_files_have_no_duplicates() {
        let findings = scan_duplicates(
            corpus(&[
                ("a.rs", "fn add(a: i32, b: i32) -> i32 { a + b }"),
                ("b.rs", "fn unrelated() { println!(\"hi\"); }"),
            ]),
            BTreeSet::from(["a.rs".to_string(), "b.rs".to_string()]),
            RunOptions::default(),
        )
        .await
        .unwrap();
        assert!(findings.is_empty(), "{findings:?}");
    }

    /// Scoped: a changed file's copy of an unchanged peer is found, and
    /// the changed file anchors it.
    #[tokio::test]
    async fn a_scoped_scan_finds_copies_in_unchanged_peers() {
        let findings = scan_duplicates(
            corpus(&[("changed.rs", BODY), ("untouched.rs", BODY)]),
            BTreeSet::from(["changed.rs".to_string()]),
            small(),
        )
        .await
        .unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.path == "changed.rs" && peer(f) == "untouched.rs"),
            "{findings:?}"
        );
    }

    /// Scoped: two regions of one file are not reported.
    #[tokio::test]
    async fn a_scoped_scan_drops_same_file_matches() {
        let twice = format!("{BODY}\n{}", BODY.replace("helper", "helper_two"));
        let findings = scan_duplicates(
            corpus(&[("only.rs", &twice)]),
            BTreeSet::from(["only.rs".to_string()]),
            small(),
        )
        .await
        .unwrap();
        for f in &findings {
            assert_ne!(peer(f), f.path, "same-file pair leaked: {f:?}");
        }
    }
}
