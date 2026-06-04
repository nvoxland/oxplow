//! Pluggable, cross-language collection: the registry seam that lets report
//! parsers be defined as plugins instead of hardcoded Rust `match` arms.
//!
//! The design is **two-layer** (see `.context/collection.md`):
//!
//! 1. **Container parse (host-owned).** The host reads report file(s) and
//!    exposes normalizer helpers (`parse_xml`, `parse_json`, …) — added in a
//!    later step. Scripts never touch the filesystem, which keeps an
//!    in-process parse deterministic and trustworthy as `observed`.
//! 2. **Field mapping (plugin-owned).** A *collector* maps the parsed value
//!    into a **typed output** for its kind — coverage line-sets or a test
//!    suite/case tree. The typed shapes are reused verbatim from
//!    [`oxplow_coverage`] so a plugin's output is exactly what oxplow stores.
//!
//! **There is never a formless observation.** Every collector declares a
//! [`CollectorKind`]; the genericity lives in this uniform
//! definition/registry mechanism over *typed* kinds, not in the data being a
//! blob. A future kind (perf, structure-map, …) is a new
//! [`CollectorKind`] + plugins that target it — not a new subsystem.
//!
//! This module is the registry + descriptor + typed contracts. The script
//! runtimes (jaq / Starlark / exec) and the host helpers land in later steps;
//! for now the only registered collectors are **builtin-rust** wrappers around
//! the existing [`oxplow_coverage`] parsers, so behavior is unchanged.

use std::collections::HashMap;
use std::sync::Arc;

use oxplow_coverage::{CoverageReport, TestReport};
use serde::{Deserialize, Serialize};

pub mod helpers;
pub mod runtime;
pub use helpers::HelperError;
pub use runtime::SandboxBudget;

/// The *type* of thing a collector observes. Each kind has a fixed,
/// host-side typed output contract (see [`CollectorOutput`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollectorKind {
    /// Per-file executed/instrumented line sets → diff coverage.
    Coverage,
    /// A suite/case tree of individual test outcomes.
    Test,
}

/// Which engine runs a collector's field-mapping step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CollectorRuntime {
    /// A first-party parser compiled into oxplow. Highest trust.
    BuiltinRust,
    /// jq via `jaq` — the primary script tier for JSON→JSON reshaping.
    Jaq,
    /// Starlark — the general script tier for imperative/odd formats.
    Starlark,
    /// An external process (JSON stdin→stdout). The escape hatch; lower-trust.
    Exec,
}

/// The typed result of running a collector. The variant is determined by the
/// collector's [`CollectorKind`] — a `Coverage` collector always yields
/// [`CollectorOutput::Coverage`], a `Test` collector always
/// [`CollectorOutput::Test`].
#[derive(Debug, Clone, PartialEq)]
pub enum CollectorOutput {
    Coverage(CoverageReport),
    Test(TestReport),
}

impl CollectorOutput {
    /// The kind this output corresponds to.
    pub fn kind(&self) -> CollectorKind {
        match self {
            CollectorOutput::Coverage(_) => CollectorKind::Coverage,
            CollectorOutput::Test(_) => CollectorKind::Test,
        }
    }

    /// Borrow the coverage report, if this is a coverage output.
    pub fn as_coverage(&self) -> Option<&CoverageReport> {
        match self {
            CollectorOutput::Coverage(r) => Some(r),
            _ => None,
        }
    }

    /// Borrow the test report, if this is a test output.
    pub fn as_test(&self) -> Option<&TestReport> {
        match self {
            CollectorOutput::Test(r) => Some(r),
            _ => None,
        }
    }
}

/// Errors surfaced while resolving or running a collector.
#[derive(Debug, thiserror::Error)]
pub enum CollectError {
    /// No collector is registered for the requested format string.
    #[error("no collector registered for format \"{0}\"")]
    UnknownFormat(String),
    /// A builtin-rust parser failed to parse its input.
    #[error("parse error: {0}")]
    Parse(String),
    /// The host failed to apply a collector's declared container parser to the
    /// raw report before handing it to the transform.
    #[error("container parse error: {0}")]
    Container(String),
    /// A script tier (jaq/starlark) failed to compile or run.
    #[error("runtime error: {0}")]
    Runtime(String),
    /// The transform produced output that doesn't match the kind's schema.
    #[error("output shape error: {0}")]
    Shape(String),
    /// An external-exec plugin failed to spawn or exited non-zero.
    #[error("exec error: {0}")]
    Exec(String),
    /// An in-process script exceeded its sandbox time budget.
    #[error("timed out")]
    Timeout,
}

impl From<HelperError> for CollectError {
    fn from(e: HelperError) -> Self {
        CollectError::Container(e.to_string())
    }
}

/// How the host pre-parses a raw report into the JSON value a transform
/// receives. Builtin-rust collectors ignore this (they take raw content);
/// external-exec also receives raw content on stdin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollectorInput {
    /// Raw report text, handed to the transform as a JSON string.
    #[default]
    Text,
    /// Parsed as JSON.
    Json,
    /// Parsed via [`helpers::parse_xml`] into the explicit element tree.
    Xml,
    /// Parsed via [`helpers::lcov_records`] into an array of records.
    Lcov,
    /// Split into an array of line strings.
    Lines,
}

impl CollectorInput {
    /// Apply this container parser to raw report `content`.
    fn parse(self, content: &str) -> Result<serde_json::Value, CollectError> {
        Ok(match self {
            CollectorInput::Text => serde_json::Value::String(content.to_string()),
            CollectorInput::Json => helpers::parse_json(content)?,
            CollectorInput::Xml => helpers::parse_xml(content)?,
            CollectorInput::Lcov => helpers::lcov_records(content),
            CollectorInput::Lines => helpers::lines(content),
        })
    }
}

/// The declarative, serde-friendly definition of a collector — the shape a
/// project lists in `oxplow.yaml` (parsed in a later step) and the shape
/// `crates/oxplow-plugin` ships bundled plugins as. `entry`/`args` are
/// runtime-specific: a jaq/Starlark script body (or path), or an exec argv.
/// Builtin-rust collectors are constructed in code and need no descriptor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectorDescriptor {
    pub name: String,
    pub kind: CollectorKind,
    pub formats: Vec<String>,
    pub runtime: CollectorRuntime,
    /// Script body or path (jaq/starlark), or the program for exec. Unused for
    /// builtin-rust.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// Extra arguments for the exec runtime.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

/// How a resolved collector actually runs.
#[derive(Clone)]
enum Runner {
    /// A compiled parser: report contents → typed output.
    Builtin(fn(&str) -> Result<CollectorOutput, CollectError>),
    /// A jq program (jaq). The host pre-parses content via `input`, runs the
    /// program, then deserializes the result into the collector's kind.
    Jaq {
        input: CollectorInput,
        program: String,
    },
    /// A Starlark `transform(input)` plugin. Same pre-parse + deserialize flow.
    Starlark {
        input: CollectorInput,
        script: String,
    },
    /// An external program (`argv`): raw content on stdin, kind JSON on stdout.
    Exec { argv: Vec<String> },
}

/// A *resolved, executable* collector: its identity + kind + the formats it
/// claims + a way to run it. The registry stores these (behind [`Arc`] so a
/// single collector can be indexed under several format names cheaply).
#[derive(Clone)]
pub struct Collector {
    name: String,
    kind: CollectorKind,
    runtime: CollectorRuntime,
    formats: Vec<String>,
    runner: Runner,
    budget: SandboxBudget,
}

impl Collector {
    fn new(
        name: impl Into<String>,
        kind: CollectorKind,
        runtime: CollectorRuntime,
        formats: impl IntoIterator<Item = impl Into<String>>,
        runner: Runner,
    ) -> Self {
        Collector {
            name: name.into(),
            kind,
            runtime,
            formats: formats.into_iter().map(Into::into).collect(),
            runner,
            budget: SandboxBudget::default(),
        }
    }

    /// Construct a builtin-rust collector from a parse function.
    pub fn builtin(
        name: impl Into<String>,
        kind: CollectorKind,
        formats: impl IntoIterator<Item = impl Into<String>>,
        run: fn(&str) -> Result<CollectorOutput, CollectError>,
    ) -> Self {
        Self::new(
            name,
            kind,
            CollectorRuntime::BuiltinRust,
            formats,
            Runner::Builtin(run),
        )
    }

    /// Construct a jaq (jq) collector: the host pre-parses content via `input`,
    /// then runs `program` and deserializes the result into `kind`.
    pub fn jaq(
        name: impl Into<String>,
        kind: CollectorKind,
        formats: impl IntoIterator<Item = impl Into<String>>,
        input: CollectorInput,
        program: impl Into<String>,
    ) -> Self {
        Self::new(
            name,
            kind,
            CollectorRuntime::Jaq,
            formats,
            Runner::Jaq {
                input,
                program: program.into(),
            },
        )
    }

    /// Construct a Starlark collector (`def transform(input): …`).
    pub fn starlark(
        name: impl Into<String>,
        kind: CollectorKind,
        formats: impl IntoIterator<Item = impl Into<String>>,
        input: CollectorInput,
        script: impl Into<String>,
    ) -> Self {
        Self::new(
            name,
            kind,
            CollectorRuntime::Starlark,
            formats,
            Runner::Starlark {
                input,
                script: script.into(),
            },
        )
    }

    /// Construct an external-exec collector. `argv[0]` is the program.
    pub fn exec(
        name: impl Into<String>,
        kind: CollectorKind,
        formats: impl IntoIterator<Item = impl Into<String>>,
        argv: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::new(
            name,
            kind,
            CollectorRuntime::Exec,
            formats,
            Runner::Exec {
                argv: argv.into_iter().map(Into::into).collect(),
            },
        )
    }

    /// Override the sandbox budget for the in-process script tiers.
    pub fn with_budget(mut self, budget: SandboxBudget) -> Self {
        self.budget = budget;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> CollectorKind {
        self.kind
    }

    pub fn runtime(&self) -> CollectorRuntime {
        self.runtime
    }

    /// The format names this collector claims (lower-cased on registration).
    pub fn formats(&self) -> &[String] {
        &self.formats
    }

    /// Run the collector against raw report `content`, producing typed output.
    /// In-process script tiers run under the sandbox budget; exec relies on the
    /// child process and is tagged lower-trust by the caller.
    pub fn run(&self, content: &str) -> Result<CollectorOutput, CollectError> {
        let kind = self.kind;
        match &self.runner {
            Runner::Builtin(f) => f(content),
            Runner::Jaq { input, program } => {
                let value = input.parse(content)?;
                let program = program.clone();
                let raw = runtime::run_sandboxed(&self.budget, move || {
                    runtime::run_jaq(&program, &value)
                })?;
                runtime::value_to_output(kind, raw)
            }
            Runner::Starlark { input, script } => {
                let value = input.parse(content)?;
                let script = script.clone();
                let raw = runtime::run_sandboxed(&self.budget, move || {
                    runtime::run_starlark(&script, &value)
                })?;
                runtime::value_to_output(kind, raw)
            }
            Runner::Exec { argv } => {
                let raw = runtime::run_exec(argv, content)?;
                runtime::value_to_output(kind, raw)
            }
        }
    }
}

impl std::fmt::Debug for Collector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collector")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("runtime", &self.runtime)
            .field("formats", &self.formats)
            .finish()
    }
}

/// Format string → collector. Replaces the old closed `enum CoverageFormat` +
/// `match` and the duplicated `KNOWN_REPORT_FORMATS` whitelist: a format is
/// valid iff a collector is registered for it.
///
/// **Registration order is precedence.** Builtins register first, bundled
/// plugins next, then project-local plugins — a later [`register`] for the
/// same format name overrides the earlier one, so a project can replace a
/// shipped parser by claiming its format.
///
/// [`register`]: CollectorRegistry::register
#[derive(Debug, Clone, Default)]
pub struct CollectorRegistry {
    /// Lower-cased format name → collector. Keyed by format (not collector
    /// name) because resolution is always by the `reports[].format` string.
    by_format: HashMap<String, Arc<Collector>>,
}

impl CollectorRegistry {
    /// An empty registry. Use [`with_builtins`] for the default set.
    ///
    /// [`with_builtins`]: CollectorRegistry::with_builtins
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry pre-loaded with the first-party builtin-rust collectors
    /// (cobertura / lcov / jacoco coverage + junit tests), preserving today's
    /// behavior and format names (`jacoco` aliases `jacoco-xml`).
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        reg.register(Collector::jaq(
            "builtin-cobertura",
            CollectorKind::Coverage,
            ["cobertura"],
            CollectorInput::Xml,
            include_str!("plugins/cobertura.jq"),
        ));
        reg.register(Collector::jaq(
            "builtin-lcov",
            CollectorKind::Coverage,
            ["lcov"],
            CollectorInput::Lcov,
            include_str!("plugins/lcov.jq"),
        ));
        reg.register(Collector::jaq(
            "builtin-jacoco",
            CollectorKind::Coverage,
            ["jacoco", "jacoco-xml"],
            CollectorInput::Xml,
            include_str!("plugins/jacoco.jq"),
        ));
        reg.register(Collector::jaq(
            "builtin-junit",
            CollectorKind::Test,
            ["junit"],
            CollectorInput::Xml,
            include_str!("plugins/junit.jq"),
        ));
        reg
    }

    /// Register a collector under each of its formats (lower-cased). A format
    /// already present is overridden — later registration wins.
    pub fn register(&mut self, collector: Collector) {
        let collector = Arc::new(collector);
        for fmt in collector.formats() {
            self.by_format
                .insert(fmt.trim().to_ascii_lowercase(), Arc::clone(&collector));
        }
    }

    /// Resolve the collector for a format string (case-insensitive), if any.
    pub fn resolve(&self, format: &str) -> Option<&Collector> {
        self.by_format
            .get(format.trim().to_ascii_lowercase().as_str())
            .map(|a| a.as_ref())
    }

    /// True if some collector claims `format`. Config validation uses this
    /// instead of a hardcoded whitelist.
    pub fn is_known(&self, format: &str) -> bool {
        self.by_format
            .contains_key(format.trim().to_ascii_lowercase().as_str())
    }

    /// All registered format names (lower-cased), unordered — for diagnostics
    /// and "unknown format, did you mean…" warnings.
    pub fn known_formats(&self) -> impl Iterator<Item = &str> {
        self.by_format.keys().map(|s| s.as_str())
    }

    /// Resolve and run in one step. Returns [`CollectError::UnknownFormat`]
    /// when nothing is registered for `format`.
    pub fn run(&self, format: &str, content: &str) -> Result<CollectorOutput, CollectError> {
        let collector = self
            .resolve(format)
            .ok_or_else(|| CollectError::UnknownFormat(format.to_string()))?;
        collector.run(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COBERTURA: &str = r#"<?xml version="1.0"?>
<coverage>
  <packages><package><classes>
    <class filename="src/a.rs"><lines>
      <line number="1" hits="1"/>
      <line number="2" hits="0"/>
    </lines></class>
  </classes></package></packages>
</coverage>"#;

    const JUNIT: &str = r#"<testsuites>
  <testsuite name="s"><testcase classname="m" name="t1"/></testsuite>
</testsuites>"#;

    #[test]
    fn builtins_resolve_known_coverage_and_test_formats() {
        let reg = CollectorRegistry::with_builtins();
        for fmt in ["cobertura", "lcov", "jacoco", "jacoco-xml"] {
            let c = reg.resolve(fmt).expect("coverage format registered");
            assert_eq!(c.kind(), CollectorKind::Coverage, "{fmt}");
            // The first-party parsers now ship as bundled jaq plugins.
            assert_eq!(c.runtime(), CollectorRuntime::Jaq);
        }
        let j = reg.resolve("junit").expect("junit registered");
        assert_eq!(j.kind(), CollectorKind::Test);
    }

    #[test]
    fn resolution_is_case_and_whitespace_insensitive() {
        let reg = CollectorRegistry::with_builtins();
        assert!(reg.resolve("  Cobertura  ").is_some());
        assert!(reg.is_known("JUNIT"));
    }

    #[test]
    fn unknown_format_resolves_to_none_and_errors() {
        let reg = CollectorRegistry::with_builtins();
        assert!(reg.resolve("clover").is_none());
        assert!(!reg.is_known("clover"));
        match reg.run("clover", "") {
            Err(CollectError::UnknownFormat(f)) => assert_eq!(f, "clover"),
            other => panic!("expected UnknownFormat, got {other:?}"),
        }
    }

    #[test]
    fn builtin_collector_runs_and_yields_typed_output() {
        let reg = CollectorRegistry::with_builtins();
        let out = reg.run("cobertura", COBERTURA).expect("parses");
        let cov = out.as_coverage().expect("coverage output");
        let f = cov.files.get("src/a.rs").expect("file present");
        assert!(f.instrumented.contains(&1) && f.instrumented.contains(&2));
        assert!(f.covered.contains(&1) && !f.covered.contains(&2));

        let out = reg.run("junit", JUNIT).expect("parses");
        let test = out.as_test().expect("test output");
        assert_eq!(test.suites.len(), 1);
        assert_eq!(test.suites[0].cases.len(), 1);
    }

    #[test]
    fn later_registration_overrides_format_by_name() {
        fn always_empty(_c: &str) -> Result<CollectorOutput, CollectError> {
            Ok(CollectorOutput::Coverage(CoverageReport::default()))
        }
        let mut reg = CollectorRegistry::with_builtins();
        reg.register(Collector::builtin(
            "project-lcov",
            CollectorKind::Coverage,
            ["lcov"],
            always_empty,
        ));
        let c = reg.resolve("lcov").expect("still registered");
        assert_eq!(c.name(), "project-lcov", "later registration wins");
    }

    #[test]
    fn descriptor_round_trips_serde() {
        let d = CollectorDescriptor {
            name: "clover".into(),
            kind: CollectorKind::Coverage,
            formats: vec!["clover".into()],
            runtime: CollectorRuntime::Jaq,
            entry: Some(".files".into()),
            args: vec![],
        };
        let json = serde_json::to_string(&d).expect("serialize");
        // kind/runtime serialize in the wire forms config will use.
        assert!(json.contains("\"coverage\""));
        assert!(json.contains("\"jaq\""));
        let back: CollectorDescriptor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, d);
    }

    #[test]
    fn jaq_collector_runs_end_to_end_with_xml_input() {
        // Host pre-parses XML → tree, jaq maps it to coverage output.
        let program = r#"{ files: { (.attrs.file): { instrumented: [1, 2], covered: [1] } } }"#;
        let c = Collector::jaq(
            "xcov",
            CollectorKind::Coverage,
            ["xcov"],
            CollectorInput::Xml,
            program,
        );
        let out = c.run(r#"<cov file="src/a.rs"/>"#).expect("runs");
        let cov = out.as_coverage().expect("coverage");
        let f = cov.files.get("src/a.rs").expect("file");
        assert_eq!(f.instrumented.len(), 2);
        assert!(f.covered.contains(&1));
    }

    #[test]
    fn starlark_collector_runs_end_to_end_with_xml_input() {
        let script = "def transform(input):\n    return {\"suites\": [{\"name\": input[\"tag\"], \"cases\": []}]}\n";
        let c = Collector::starlark(
            "xtest",
            CollectorKind::Test,
            ["xtest"],
            CollectorInput::Xml,
            script,
        );
        let out = c.run("<suite/>").expect("runs");
        assert_eq!(out.as_test().expect("test").suites[0].name, "suite");
    }

    #[test]
    fn jaq_collector_works_through_registry_override() {
        let program = "{ files: {} }";
        let mut reg = CollectorRegistry::with_builtins();
        reg.register(Collector::jaq(
            "jaq-lcov",
            CollectorKind::Coverage,
            ["lcov"],
            CollectorInput::Lcov,
            program,
        ));
        let out = reg
            .run("lcov", "SF:x\nDA:1,1\nend_of_record\n")
            .expect("runs");
        assert!(out.as_coverage().expect("coverage").files.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn exec_collector_round_trips_stdin_to_stdout() {
        // `cat` echoes the (already kind-shaped) JSON from stdin to stdout.
        let c = Collector::exec("e", CollectorKind::Test, ["xexec"], ["cat"]);
        let out = c
            .run(r#"{"suites":[{"name":"s","cases":[]}]}"#)
            .expect("runs");
        assert_eq!(out.as_test().expect("test").suites[0].name, "s");
    }

    // ---- golden: bundled jaq plugins reproduce the Rust parsers exactly ----

    const GOLD_COBERTURA: &str = r#"<?xml version="1.0"?>
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

    const GOLD_LCOV: &str = "TN:\nSF:src/foo.rs\nDA:1,3\nDA:2,0\nDA:5,1\nend_of_record\nSF:src/bar.rs\nDA:10,0\nend_of_record\n";

    const GOLD_JACOCO: &str = r#"<?xml version="1.0"?>
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

    const GOLD_JUNIT_NEXTEST: &str = r#"<?xml version="1.0"?>
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

    const GOLD_JUNIT_PYTEST: &str = r#"<testsuite name="pytest" tests="1">
  <testcase classname="tests.test_foo.TestBar" name="test_baz" time="0.01"/>
</testsuite>"#;

    // Committed expected values (the bundled jaq plugins are the only parser;
    // these golden fixtures pin their output — no live Rust-parser oracle).
    fn cov(files: &[(&str, &[u32], &[u32])]) -> CoverageReport {
        let mut report = CoverageReport::default();
        for (path, instrumented, covered) in files {
            report.files.insert(
                (*path).to_string(),
                oxplow_coverage::FileCoverage {
                    instrumented: instrumented.iter().copied().collect(),
                    covered: covered.iter().copied().collect(),
                },
            );
        }
        report
    }

    fn case(
        classname: &str,
        name: &str,
        status: oxplow_coverage::TestStatus,
        time_ms: Option<u64>,
    ) -> oxplow_coverage::TestCase {
        oxplow_coverage::TestCase {
            classname: classname.into(),
            name: name.into(),
            status,
            time_ms,
        }
    }

    #[test]
    fn builtin_cobertura_plugin_produces_expected_coverage() {
        let out = CollectorRegistry::with_builtins()
            .run("cobertura", GOLD_COBERTURA)
            .expect("plugin runs");
        let expected = cov(&[
            ("src/foo.rs", &[1, 2, 5], &[1, 5]),
            ("src/bar.rs", &[10], &[]),
        ]);
        assert_eq!(out.as_coverage().unwrap(), &expected);
    }

    #[test]
    fn builtin_lcov_plugin_produces_expected_coverage() {
        let out = CollectorRegistry::with_builtins()
            .run("lcov", GOLD_LCOV)
            .expect("plugin runs");
        let expected = cov(&[
            ("src/foo.rs", &[1, 2, 5], &[1, 5]),
            ("src/bar.rs", &[10], &[]),
        ]);
        assert_eq!(out.as_coverage().unwrap(), &expected);
    }

    #[test]
    fn builtin_jacoco_plugin_produces_expected_coverage() {
        let reg = CollectorRegistry::with_builtins();
        let expected = cov(&[
            ("com/example/Foo.java", &[1, 2], &[1]),
            ("Root.java", &[7], &[7]),
        ]);
        for fmt in ["jacoco", "jacoco-xml"] {
            let out = reg.run(fmt, GOLD_JACOCO).expect("plugin runs");
            assert_eq!(out.as_coverage().unwrap(), &expected, "format {fmt}");
        }
    }

    #[test]
    fn builtin_junit_plugin_produces_expected_tree() {
        use oxplow_coverage::{TestStatus, TestSuite};
        let reg = CollectorRegistry::with_builtins();

        let nextest = reg.run("junit", GOLD_JUNIT_NEXTEST).expect("plugin runs");
        let expected_nextest = TestReport {
            suites: vec![TestSuite {
                name: "oxplow-app".into(),
                cases: vec![
                    case(
                        "oxplow_app::collection",
                        "detect_test_run",
                        TestStatus::Passed,
                        Some(1),
                    ),
                    case(
                        "oxplow_app::collection",
                        "ingest_coverage",
                        TestStatus::Failed,
                        Some(50),
                    ),
                    case("oxplow_app::collection", "flaky", TestStatus::Skipped, None),
                ],
            }],
        };
        assert_eq!(nextest.as_test().unwrap(), &expected_nextest);

        let pytest = reg.run("junit", GOLD_JUNIT_PYTEST).expect("plugin runs");
        let expected_pytest = TestReport {
            suites: vec![TestSuite {
                name: "pytest".into(),
                cases: vec![case(
                    "tests.test_foo.TestBar",
                    "test_baz",
                    TestStatus::Passed,
                    Some(10),
                )],
            }],
        };
        assert_eq!(pytest.as_test().unwrap(), &expected_pytest);
    }

    #[test]
    fn builtin_plugins_surface_malformed_input_as_error() {
        let reg = CollectorRegistry::with_builtins();
        assert!(reg.run("cobertura", "<coverage><class").is_err());
        assert!(reg.run("junit", "<testsuites><testcase").is_err());
    }
}
