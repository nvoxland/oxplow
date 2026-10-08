//! Uniform data types for test/coverage results.
//!
//! This crate is **pure types** — the shapes a report parser produces and
//! oxplow stores. Parsing is `oxplow-script`'s: the bundled
//! junit/lcov/cobertura/jacoco/clippy/eslint jq programs and a project's
//! own report collectors' scripts. Keeping
//! these types in their own dependency-light crate lets both the script
//! runtime (`oxplow-script`) and the app/db layers share one definition of coverage line-sets
//! and the test suite/case tree.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;

/// Per-file coverage. `instrumented` is every line the report mentions;
/// `covered` is the subset that executed (`covered ⊆ instrumented`).
///
/// Branch and function coverage are **counts**, not line-sets: a single line can
/// hold several branches, and functions are named entities rather than lines, so
/// neither maps onto a `BTreeSet<u32>` of line numbers. `*_found == 0` means the
/// report carried no branch/function data for this file (many line-only reports),
/// so aggregators skip it — a 0/0 file contributes nothing to the ratio rather
/// than reading as "0% covered".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileCoverage {
    pub instrumented: BTreeSet<u32>,
    pub covered: BTreeSet<u32>,
    pub branches_found: u32,
    pub branches_hit: u32,
    pub functions_found: u32,
    pub functions_hit: u32,
}

/// A parsed coverage report: path → its line coverage. A parser writes paths
/// as the report has them (often absolute: `cargo llvm-cov`);
/// [`CoverageReport::relative_to`] maps them to repo-relative.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverageReport {
    pub files: BTreeMap<String, FileCoverage>,
}

/// Outcome of a single test case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TestStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestCase {
    /// JUnit `classname` — the grouping path (Rust module path, pytest
    /// file·class, jest describe path). The UI builds its tree by
    /// splitting this on `::` / `.`.
    pub classname: String,
    pub name: String,
    pub status: TestStatus,
    #[serde(rename = "timeMs", skip_serializing_if = "Option::is_none")]
    pub time_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestSuite {
    pub name: String,
    pub cases: Vec<TestCase>,
}

impl CoverageReport {
    /// The report with every path repo-relative to `root` (the checkout
    /// the run ran in), files outside it dropped. Two spellings of one
    /// file merge as [`Self::merge`] does.
    pub fn relative_to(self, root: &Path) -> Self {
        let mut files: BTreeMap<String, FileCoverage> = BTreeMap::new();
        for (path, fc) in self.files {
            let Some(path) = repo_relative(&path, root) else {
                continue;
            };
            files.entry(path).or_default().absorb(fc);
        }
        Self { files }
    }

    /// Fold `other` in: line sets union and the branch/function counters
    /// sum (tsk160) — two toolchains reporting the same file each
    /// contribute their own.
    pub fn merge(&mut self, other: CoverageReport) {
        for (path, fc) in other.files {
            self.files.entry(path).or_default().absorb(fc);
        }
    }
}

impl FileCoverage {
    fn absorb(&mut self, other: FileCoverage) {
        self.instrumented.extend(other.instrumented);
        self.covered.extend(other.covered);
        self.branches_found = self.branches_found.saturating_add(other.branches_found);
        self.branches_hit = self.branches_hit.saturating_add(other.branches_hit);
        self.functions_found = self.functions_found.saturating_add(other.functions_found);
        self.functions_hit = self.functions_hit.saturating_add(other.functions_hit);
    }
}

/// `path`, as a report wrote it, repo-relative to `root`: an absolute path
/// under `root` (as given, or with symlinks resolved) loses the prefix; a
/// relative one is already relative to the checkout the run ran in and
/// loses only a leading `./`. `None` for a path outside `root` — not a
/// file of this project. An empty path (a project-level finding) stays
/// empty.
pub fn repo_relative(path: &str, root: &Path) -> Option<String> {
    let p = Path::new(path);
    if !p.is_absolute() {
        let rel = path.trim_start_matches("./");
        return (!rel.split('/').any(|part| part == "..")).then(|| rel.to_string());
    }
    let under = |p: &Path, r: &Path| -> Option<String> {
        p.strip_prefix(r).ok()?.to_str().map(str::to_string)
    };
    if let Some(rel) = under(p, root) {
        return Some(rel);
    }
    let canonical_root = root.canonicalize().ok()?;
    under(p, &canonical_root).or_else(|| under(&p.canonicalize().ok()?, &canonical_root))
}

/// A parsed JUnit-style report: suites → cases. Tech-agnostic — every
/// framework whose results a collector maps here (pytest, jest,
/// go-junit-report, cargo-nextest, …) lands in this shape, so individual test
/// results stay `observed`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TestReport {
    pub suites: Vec<TestSuite>,
}

/// Severity of a single static-analysis finding, in descending order of
/// concern. Maps from a linter's native levels (clippy `error`/`warning`,
/// eslint `2`/`1`, …) so findings stay tech-agnostic and `observed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
    Note,
}

/// One static-analysis finding — a single diagnostic a linter/analyzer
/// emitted. `path` is as the report wrote it until
/// [`AnalysisReport::relative_to`] maps it to repo-relative; `line`/`column` are 1-based and optional (some findings
/// are file- or project-level); `rule` is the lint name (clippy `code.code`,
/// eslint `ruleId`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnalysisFinding {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    pub message: String,
}

/// A parsed static-analysis report: a flat list of findings. Tech-agnostic —
/// every analyzer whose output a collector maps here (clippy, eslint, ruff,
/// golangci-lint, …) lands in this shape, so the result stays `observed`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AnalysisReport {
    pub findings: Vec<AnalysisFinding>,
}

impl AnalysisReport {
    /// The report with every finding's path repo-relative to `root` (see
    /// [`repo_relative`]); a finding in a file outside it is dropped.
    pub fn relative_to(self, root: &Path) -> Self {
        let findings = self
            .findings
            .into_iter()
            .filter_map(|mut f| {
                f.path = repo_relative(&f.path, root)?;
                Some(f)
            })
            .collect();
        Self { findings }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_paths_become_repo_relative() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let abs = format!("{}/src/a.rs", root.display());
        assert_eq!(repo_relative(&abs, root).as_deref(), Some("src/a.rs"));
        assert_eq!(
            repo_relative("./src/a.rs", root).as_deref(),
            Some("src/a.rs")
        );
        assert_eq!(repo_relative("src/a.rs", root).as_deref(), Some("src/a.rs"));
        assert_eq!(repo_relative("", root).as_deref(), Some(""));
        assert_eq!(repo_relative("/elsewhere/a.rs", root), None);
        assert_eq!(repo_relative("../a.rs", root), None);
        // Through the root's resolved spelling (macOS: /var → /private/var).
        let canonical = format!("{}/src/b.rs", root.canonicalize().unwrap().display());
        assert_eq!(repo_relative(&canonical, root).as_deref(), Some("src/b.rs"));
    }

    #[test]
    fn two_spellings_of_one_file_merge() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = |lines: &[u32], branches: u32| FileCoverage {
            instrumented: lines.iter().copied().collect(),
            covered: lines.iter().copied().collect(),
            branches_found: branches,
            ..Default::default()
        };
        let report = CoverageReport {
            files: BTreeMap::from([
                (format!("{}/src/a.rs", root.display()), file(&[1], 2)),
                ("src/a.rs".to_string(), file(&[2], 3)),
                ("/elsewhere/dep.rs".to_string(), file(&[1], 0)),
            ]),
        }
        .relative_to(root);
        assert_eq!(report.files.keys().collect::<Vec<_>>(), vec!["src/a.rs"]);
        let a = &report.files["src/a.rs"];
        assert_eq!(a.instrumented, BTreeSet::from([1, 2]));
        assert_eq!(a.branches_found, 5);
    }

    #[test]
    fn test_report_serializes_in_the_ui_wire_shape() {
        let report = TestReport {
            suites: vec![TestSuite {
                name: "s".into(),
                cases: vec![
                    TestCase {
                        classname: "m".into(),
                        name: "a".into(),
                        status: TestStatus::Passed,
                        time_ms: Some(12),
                    },
                    TestCase {
                        classname: "m".into(),
                        name: "b".into(),
                        status: TestStatus::Skipped,
                        time_ms: None,
                    },
                ],
            }],
        };
        let json = serde_json::to_value(&report).expect("serialize");
        assert_eq!(json["suites"][0]["cases"][0]["status"], "passed");
        assert_eq!(json["suites"][0]["cases"][0]["timeMs"], 12);
        // `time_ms` is omitted when absent.
        assert!(json["suites"][0]["cases"][1].get("timeMs").is_none());
    }

    #[test]
    fn analysis_report_serializes_in_the_ui_wire_shape() {
        let report = AnalysisReport {
            findings: vec![
                AnalysisFinding {
                    path: "src/a.rs".into(),
                    line: Some(12),
                    column: Some(5),
                    severity: Severity::Warning,
                    rule: Some("clippy::needless_return".into()),
                    message: "unneeded return".into(),
                },
                AnalysisFinding {
                    path: "src/b.rs".into(),
                    line: None,
                    column: None,
                    severity: Severity::Error,
                    rule: None,
                    message: "file-level problem".into(),
                },
            ],
        };
        let json = serde_json::to_value(&report).expect("serialize");
        assert_eq!(json["findings"][0]["severity"], "warning");
        assert_eq!(json["findings"][0]["line"], 12);
        assert_eq!(json["findings"][0]["rule"], "clippy::needless_return");
        assert_eq!(json["findings"][1]["severity"], "error");
        // Optional line/column/rule are omitted when absent.
        assert!(json["findings"][1].get("line").is_none());
        assert!(json["findings"][1].get("column").is_none());
        assert!(json["findings"][1].get("rule").is_none());
    }
}
