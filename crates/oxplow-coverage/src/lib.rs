//! Uniform data types for test/coverage results.
//!
//! This crate is **pure types** — the shapes a collector produces and oxplow
//! stores. Report *parsing* is no longer here: it moved to the pluggable
//! collector registry in `oxplow-collect-plugin` (the first-party
//! cobertura/lcov/jacoco/junit parsers ship as bundled jaq plugins). Keeping
//! these types in their own dependency-light crate lets both the plugin
//! runtime and the app/db layers share one definition of coverage line-sets
//! and the test suite/case tree.

use std::collections::{BTreeMap, BTreeSet};

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

/// A parsed coverage report: report-relative path → its line coverage. Paths
/// are exactly as they appear in the report; mapping them to repo-relative is
/// the caller's job.
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
/// emitted. `path` is verbatim from the report (the caller maps it to
/// repo-relative); `line`/`column` are 1-based and optional (some findings
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

#[cfg(test)]
mod tests {
    use super::*;

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
