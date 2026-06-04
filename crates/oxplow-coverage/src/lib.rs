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

/// Per-file line coverage. `instrumented` is every line the report mentions;
/// `covered` is the subset that executed (`covered ⊆ instrumented`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileCoverage {
    pub instrumented: BTreeSet<u32>,
    pub covered: BTreeSet<u32>,
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
}
