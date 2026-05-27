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
}
