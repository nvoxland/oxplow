//! Deterministic parsing of standard coverage interchange formats into a
//! uniform per-file `{ instrumented, covered }` line-set map.
//!
//! This is the **one place coverage numbers originate** in oxplow — the
//! agent never produces them (see `.context/collection.md`). The runtime
//! parses the report the project's test tooling already emitted, then
//! intersects these line sets with an effort's changed lines to derive
//! diff coverage. Keeping parsing here (not in the agent) is what makes a
//! `diff-coverage` observation `observed` rather than `asserted`.
//!
//! Three formats, because they cover almost every language's tooling:
//! **cobertura** XML, **lcov** (`.info`), and **jacoco** XML. Paths are
//! returned exactly as they appear in the report (report-relative); mapping
//! them to repo-relative paths is the caller's job.

use std::collections::{BTreeMap, BTreeSet};

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageFormat {
    Cobertura,
    Lcov,
    JacocoXml,
}

impl CoverageFormat {
    /// Parse the profile's `coverageFormat` string. Accepts `jacoco` as an
    /// alias for `jacoco-xml`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "cobertura" => Some(Self::Cobertura),
            "lcov" => Some(Self::Lcov),
            "jacoco" | "jacoco-xml" => Some(Self::JacocoXml),
            _ => None,
        }
    }
}

/// Per-file line coverage. `instrumented` is every line the report mentions;
/// `covered` is the subset that executed (`covered ⊆ instrumented`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileCoverage {
    pub instrumented: BTreeSet<u32>,
    pub covered: BTreeSet<u32>,
}

/// A parsed report: report-relative path → its line coverage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverageReport {
    pub files: BTreeMap<String, FileCoverage>,
}

#[derive(Debug, thiserror::Error)]
pub enum CoverageParseError {
    #[error("xml parse error: {0}")]
    Xml(String),
}

/// Parse `content` as the given format.
pub fn parse(format: CoverageFormat, content: &str) -> Result<CoverageReport, CoverageParseError> {
    match format {
        CoverageFormat::Cobertura => parse_cobertura(content),
        CoverageFormat::Lcov => Ok(parse_lcov(content)),
        CoverageFormat::JacocoXml => parse_jacoco(content),
    }
}

fn record_line(report: &mut CoverageReport, path: &str, number: u32, covered: bool) {
    let fc = report.files.entry(path.to_string()).or_default();
    fc.instrumented.insert(number);
    if covered {
        fc.covered.insert(number);
    }
}

fn attr_str(e: &BytesStart, key: &[u8]) -> Result<Option<String>, CoverageParseError> {
    for a in e.attributes() {
        let a = a.map_err(|err| CoverageParseError::Xml(err.to_string()))?;
        if a.key.as_ref() == key {
            let raw = std::str::from_utf8(&a.value)
                .map_err(|err| CoverageParseError::Xml(err.to_string()))?;
            let val = quick_xml::escape::unescape(raw)
                .map_err(|err| CoverageParseError::Xml(err.to_string()))?
                .into_owned();
            return Ok(Some(val));
        }
    }
    Ok(None)
}

fn attr_u32(e: &BytesStart, key: &[u8]) -> Result<Option<u32>, CoverageParseError> {
    Ok(attr_str(e, key)?.and_then(|s| s.trim().parse::<u32>().ok()))
}

fn attr_i64(e: &BytesStart, key: &[u8]) -> Result<i64, CoverageParseError> {
    Ok(attr_str(e, key)?
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0))
}

/// Cobertura: `<class filename="X">` … `<line number="N" hits="H"/>`. Line
/// elements (both class-level and method-level) are attributed to the
/// enclosing class's file; covered when `hits > 0`.
fn parse_cobertura(content: &str) -> Result<CoverageReport, CoverageParseError> {
    let mut report = CoverageReport::default();
    let mut reader = Reader::from_str(content);
    let mut current: Option<String> = None;
    loop {
        match reader
            .read_event()
            .map_err(|e| CoverageParseError::Xml(e.to_string()))?
        {
            Event::Start(e) | Event::Empty(e) => match e.name().as_ref() {
                b"class" => {
                    if let Some(f) = attr_str(&e, b"filename")? {
                        current = Some(f);
                    }
                }
                b"line" => {
                    if let Some(path) = current.clone() {
                        if let Some(number) = attr_u32(&e, b"number")? {
                            let hits = attr_i64(&e, b"hits")?;
                            record_line(&mut report, &path, number, hits > 0);
                        }
                    }
                }
                _ => {}
            },
            Event::End(e) if e.name().as_ref() == b"class" => current = None,
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(report)
}

/// JaCoCo: `<package name="P">` … `<sourcefile name="F">` …
/// `<line nr="N" ci="C" mi="M"/>`. Path is `P/F`; covered when covered
/// instructions `ci > 0`.
fn parse_jacoco(content: &str) -> Result<CoverageReport, CoverageParseError> {
    let mut report = CoverageReport::default();
    let mut reader = Reader::from_str(content);
    let mut package = String::new();
    let mut current: Option<String> = None;
    loop {
        match reader
            .read_event()
            .map_err(|e| CoverageParseError::Xml(e.to_string()))?
        {
            Event::Start(e) | Event::Empty(e) => match e.name().as_ref() {
                b"package" => package = attr_str(&e, b"name")?.unwrap_or_default(),
                b"sourcefile" => {
                    current = attr_str(&e, b"name")?.map(|name| {
                        if package.is_empty() {
                            name
                        } else {
                            format!("{}/{}", package.trim_end_matches('/'), name)
                        }
                    });
                }
                b"line" => {
                    if let Some(path) = current.clone() {
                        if let Some(number) = attr_u32(&e, b"nr")? {
                            let ci = attr_i64(&e, b"ci")?;
                            record_line(&mut report, &path, number, ci > 0);
                        }
                    }
                }
                _ => {}
            },
            Event::End(e) => match e.name().as_ref() {
                b"sourcefile" => current = None,
                b"package" => package.clear(),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(report)
}

/// LCOV: `SF:<path>` starts a file, `DA:<line>,<hits>[,…]` records a line
/// (covered when `hits > 0`), `end_of_record` ends it.
fn parse_lcov(content: &str) -> CoverageReport {
    let mut report = CoverageReport::default();
    let mut current: Option<String> = None;
    for line in content.lines() {
        let line = line.trim();
        if let Some(path) = line.strip_prefix("SF:") {
            let path = path.to_string();
            report.files.entry(path.clone()).or_default();
            current = Some(path);
        } else if let Some(rest) = line.strip_prefix("DA:") {
            if let Some(path) = current.clone() {
                let mut parts = rest.split(',');
                let number = parts.next().and_then(|s| s.trim().parse::<u32>().ok());
                let hits = parts
                    .next()
                    .and_then(|s| s.trim().parse::<i64>().ok())
                    .unwrap_or(0);
                if let Some(number) = number {
                    record_line(&mut report, &path, number, hits > 0);
                }
            }
        } else if line == "end_of_record" {
            current = None;
        }
    }
    report
}

// ---------------- JUnit test reports ----------------

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

/// A parsed JUnit report: suites → cases. Tech-agnostic — every framework
/// that emits JUnit XML (pytest, jest, go-junit-report, cargo-nextest, …)
/// lands here, so individual test results stay `observed`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TestReport {
    pub suites: Vec<TestSuite>,
}

fn junit_case(e: &BytesStart) -> Result<TestCase, CoverageParseError> {
    Ok(TestCase {
        classname: attr_str(e, b"classname")?.unwrap_or_default(),
        name: attr_str(e, b"name")?.unwrap_or_default(),
        status: TestStatus::Passed,
        time_ms: attr_str(e, b"time")?
            .and_then(|s| s.trim().parse::<f64>().ok())
            .map(|secs| (secs * 1000.0).round() as u64),
    })
}

/// Append a finished case to the current suite, lazily creating an
/// anonymous suite for reports with loose `<testcase>`s (the suite is
/// flushed into the report on `</testsuite>` or at EOF).
fn junit_push(suite: &mut Option<TestSuite>, c: TestCase) {
    suite
        .get_or_insert_with(|| TestSuite {
            name: String::new(),
            cases: Vec::new(),
        })
        .cases
        .push(c);
}

/// Parse JUnit XML into a uniform suite/case tree. Tolerant of both the
/// single-`<testsuite>` root and the `<testsuites>` wrapper. A
/// `<failure>` / `<error>` child marks a case Failed; `<skipped>` marks
/// it Skipped; otherwise Passed.
pub fn parse_junit(content: &str) -> Result<TestReport, CoverageParseError> {
    let mut reader = Reader::from_str(content);
    let mut report = TestReport::default();
    let mut suite: Option<TestSuite> = None;
    let mut case: Option<TestCase> = None;
    loop {
        match reader
            .read_event()
            .map_err(|e| CoverageParseError::Xml(e.to_string()))?
        {
            Event::Start(e) => match e.name().as_ref() {
                b"testsuite" => {
                    if let Some(s) = suite.take() {
                        report.suites.push(s);
                    }
                    suite = Some(TestSuite {
                        name: attr_str(&e, b"name")?.unwrap_or_default(),
                        cases: Vec::new(),
                    });
                }
                b"testcase" => case = Some(junit_case(&e)?),
                b"failure" | b"error" => {
                    if let Some(c) = case.as_mut() {
                        c.status = TestStatus::Failed;
                    }
                }
                b"skipped" => {
                    if let Some(c) = case.as_mut() {
                        if c.status == TestStatus::Passed {
                            c.status = TestStatus::Skipped;
                        }
                    }
                }
                _ => {}
            },
            Event::Empty(e) => match e.name().as_ref() {
                b"testcase" => junit_push(&mut suite, junit_case(&e)?),
                b"failure" | b"error" => {
                    if let Some(c) = case.as_mut() {
                        c.status = TestStatus::Failed;
                    }
                }
                b"skipped" => {
                    if let Some(c) = case.as_mut() {
                        if c.status == TestStatus::Passed {
                            c.status = TestStatus::Skipped;
                        }
                    }
                }
                _ => {}
            },
            Event::End(e) => match e.name().as_ref() {
                b"testcase" => {
                    if let Some(c) = case.take() {
                        junit_push(&mut suite, c);
                    }
                }
                b"testsuite" => {
                    if let Some(s) = suite.take() {
                        report.suites.push(s);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    if let Some(s) = suite.take() {
        report.suites.push(s);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(nums: &[u32]) -> BTreeSet<u32> {
        nums.iter().copied().collect()
    }

    #[test]
    fn from_name_accepts_aliases() {
        assert_eq!(
            CoverageFormat::from_name("cobertura"),
            Some(CoverageFormat::Cobertura)
        );
        assert_eq!(
            CoverageFormat::from_name("LCOV"),
            Some(CoverageFormat::Lcov)
        );
        assert_eq!(
            CoverageFormat::from_name("jacoco"),
            Some(CoverageFormat::JacocoXml)
        );
        assert_eq!(
            CoverageFormat::from_name("jacoco-xml"),
            Some(CoverageFormat::JacocoXml)
        );
        assert_eq!(CoverageFormat::from_name("clover"), None);
    }

    #[test]
    fn parses_cobertura() {
        let xml = r#"<?xml version="1.0"?>
<coverage>
  <packages>
    <package name="p">
      <classes>
        <class name="Foo" filename="src/foo.rs">
          <lines>
            <line number="1" hits="3"/>
            <line number="2" hits="0"/>
            <line number="5" hits="1"/>
          </lines>
        </class>
        <class name="Bar" filename="src/bar.rs">
          <lines>
            <line number="10" hits="0"/>
          </lines>
        </class>
      </classes>
    </package>
  </packages>
</coverage>"#;
        let report = parse(CoverageFormat::Cobertura, xml).unwrap();
        let foo = &report.files["src/foo.rs"];
        assert_eq!(foo.instrumented, lines(&[1, 2, 5]));
        assert_eq!(foo.covered, lines(&[1, 5]));
        let bar = &report.files["src/bar.rs"];
        assert_eq!(bar.instrumented, lines(&[10]));
        assert!(bar.covered.is_empty());
    }

    #[test]
    fn parses_lcov() {
        let info = "TN:\n\
            SF:src/foo.rs\n\
            DA:1,3\n\
            DA:2,0\n\
            DA:5,1\n\
            end_of_record\n\
            SF:src/bar.rs\n\
            DA:10,0\n\
            end_of_record\n";
        let report = parse(CoverageFormat::Lcov, info).unwrap();
        assert_eq!(report.files["src/foo.rs"].instrumented, lines(&[1, 2, 5]));
        assert_eq!(report.files["src/foo.rs"].covered, lines(&[1, 5]));
        assert_eq!(report.files["src/bar.rs"].covered, BTreeSet::new());
    }

    #[test]
    fn parses_jacoco_joins_package_and_sourcefile() {
        let xml = r#"<?xml version="1.0"?>
<report name="r">
  <package name="com/example">
    <sourcefile name="Foo.java">
      <line nr="1" mi="0" ci="4"/>
      <line nr="2" mi="3" ci="0"/>
    </sourcefile>
  </package>
  <package name="">
    <sourcefile name="Root.java">
      <line nr="7" mi="0" ci="1"/>
    </sourcefile>
  </package>
</report>"#;
        let report = parse(CoverageFormat::JacocoXml, xml).unwrap();
        let foo = &report.files["com/example/Foo.java"];
        assert_eq!(foo.instrumented, lines(&[1, 2]));
        assert_eq!(foo.covered, lines(&[1]));
        // Empty package name → bare sourcefile name.
        assert_eq!(report.files["Root.java"].covered, lines(&[7]));
    }

    #[test]
    fn malformed_xml_is_an_error_not_a_panic() {
        assert!(parse(CoverageFormat::Cobertura, "<coverage><class").is_err());
    }

    #[test]
    fn parses_junit_suites_and_statuses() {
        // nextest-shaped: testsuites wrapper, classname carries the module
        // path, mixed passed / failed / skipped.
        let xml = r#"<?xml version="1.0"?>
<testsuites>
  <testsuite name="oxplow-app" tests="3" failures="1" skipped="1" time="0.42">
    <testcase classname="oxplow_app::collection" name="detect_test_run" time="0.001"/>
    <testcase classname="oxplow_app::collection" name="ingest_coverage" time="0.05">
      <failure message="assert failed">left != right</failure>
    </testcase>
    <testcase classname="oxplow_app::collection" name="flaky">
      <skipped/>
    </testcase>
  </testsuite>
</testsuites>"#;
        let report = parse_junit(xml).unwrap();
        assert_eq!(report.suites.len(), 1);
        let s = &report.suites[0];
        assert_eq!(s.name, "oxplow-app");
        assert_eq!(s.cases.len(), 3);
        assert_eq!(s.cases[0].status, TestStatus::Passed);
        assert_eq!(s.cases[0].time_ms, Some(1));
        assert_eq!(s.cases[0].classname, "oxplow_app::collection");
        assert_eq!(s.cases[1].status, TestStatus::Failed);
        assert_eq!(s.cases[2].status, TestStatus::Skipped);
    }

    #[test]
    fn parses_junit_single_testsuite_root() {
        // pytest-shaped: bare <testsuite> root, no wrapper.
        let xml = r#"<testsuite name="pytest" tests="1">
  <testcase classname="tests.test_foo.TestBar" name="test_baz" time="0.01"/>
</testsuite>"#;
        let report = parse_junit(xml).unwrap();
        assert_eq!(report.suites.len(), 1);
        assert_eq!(
            report.suites[0].cases[0].classname,
            "tests.test_foo.TestBar"
        );
        assert_eq!(report.suites[0].cases[0].status, TestStatus::Passed);
    }

    #[test]
    fn malformed_junit_is_an_error_not_a_panic() {
        assert!(parse_junit("<testsuites><testcase").is_err());
    }
}
