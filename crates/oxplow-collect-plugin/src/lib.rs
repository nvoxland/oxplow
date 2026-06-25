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

use oxplow_coverage::{AnalysisReport, CoverageReport, TestReport};
use serde::{Deserialize, Serialize};

pub mod builtin_metrics;
pub mod helpers;
pub mod runtime;
pub use builtin_metrics::{builtin_metrics, BuiltinMetric};
pub use helpers::HelperError;
pub use runtime::{GaugeHost, SandboxBudget};

/// The *type* of thing a collector observes. Each kind has a fixed,
/// host-side typed output contract (see [`CollectorOutput`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollectorKind {
    /// Per-file executed/instrumented line sets → diff coverage.
    Coverage,
    /// A suite/case tree of individual test outcomes.
    Test,
    /// A flat list of linter/analyzer findings.
    Analysis,
    /// One or more scalar samples projected into the metric substrate
    /// (`metric_sample`). The author-able kind: any deterministically-computable
    /// number (LOC, unsafe-block count, bundle size, …).
    Gauge,
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

/// One scalar sample projected by a `gauge` collector. `subject` is an optional
/// `"kind:ref"` string (e.g. `"file:src/a.rs"`, `"module:apps/desktop"`) the
/// host splits onto `subject_kind`/`subject_ref`; `dims` are open author
/// dimensions carried onto the sample as `dims_json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GaugeSample {
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dims: Option<serde_json::Map<String, serde_json::Value>>,
}

/// The typed output of a `gauge` collector: ≥1 scalar sample to project into
/// `metric_sample`, plus optional located `findings` — the underlying items the
/// metric counted (e.g. each high-complexity function), persisted on the run so
/// a recording can be drilled into. Mirrors the JSON a gauge script returns —
/// `{ "samples": [ … ], "findings"?: [ { "path"?, "line"?, "message"?, … } ] }`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MetricReport {
    #[serde(default)]
    pub samples: Vec<GaugeSample>,
    #[serde(default)]
    pub findings: Vec<GaugeFinding>,
}

/// One located item a gauge counted — projected onto `metric_finding` on the
/// run. All fields optional so a script emits only what's meaningful (a
/// complexity finding: `path` + `line` + `message`=name + `value`=complexity).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GaugeFinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Optional `"kind:ref"` subject (split like `GaugeSample.subject`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
}

/// The typed result of running a collector. The variant is determined by the
/// collector's [`CollectorKind`] — a `Coverage` collector always yields
/// [`CollectorOutput::Coverage`], a `Test` collector always
/// [`CollectorOutput::Test`].
#[derive(Debug, Clone, PartialEq)]
pub enum CollectorOutput {
    Coverage(CoverageReport),
    Test(TestReport),
    Analysis(AnalysisReport),
    Gauge(MetricReport),
}

impl CollectorOutput {
    /// The kind this output corresponds to.
    pub fn kind(&self) -> CollectorKind {
        match self {
            CollectorOutput::Coverage(_) => CollectorKind::Coverage,
            CollectorOutput::Test(_) => CollectorKind::Test,
            CollectorOutput::Analysis(_) => CollectorKind::Analysis,
            CollectorOutput::Gauge(_) => CollectorKind::Gauge,
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

    /// Borrow the analysis report, if this is an analysis output.
    pub fn as_analysis(&self) -> Option<&AnalysisReport> {
        match self {
            CollectorOutput::Analysis(r) => Some(r),
            _ => None,
        }
    }

    /// Borrow the gauge report, if this is a gauge output.
    pub fn as_gauge(&self) -> Option<&MetricReport> {
        match self {
            CollectorOutput::Gauge(r) => Some(r),
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

    /// Run a gauge collector with a [`GaugeHost`] in scope so a Starlark script's
    /// `files(glob)` builtin can read the snapshot file map. The host moves into
    /// the sandbox worker by value. For non-Starlark runtimes the host is unused
    /// and this is equivalent to [`run`](Collector::run).
    pub fn run_gauge(
        &self,
        content: &str,
        host: GaugeHost,
    ) -> Result<CollectorOutput, CollectError> {
        let kind = self.kind;
        match &self.runner {
            Runner::Starlark { input, script } => {
                let value = input.parse(content)?;
                let script = script.clone();
                let raw = runtime::run_sandboxed(&self.budget, move || {
                    runtime::run_starlark_with_host(&script, &value, &host)
                })?;
                runtime::value_to_output(kind, raw)
            }
            // jaq / exec / builtin don't read the file-map host.
            _ => self.run(content),
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

    /// A registry pre-loaded with the first-party collectors, named under the
    /// reserved `oxplow.` namespace (`oxplow.cobertura` / `oxplow.lcov` /
    /// `oxplow.jacoco` coverage + `oxplow.junit` tests). Format names stay bare
    /// (`jacoco` aliases `jacoco-xml`); project plugins use their own
    /// `<vendor>.` name prefix.
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        reg.register(Collector::jaq(
            "oxplow.cobertura",
            CollectorKind::Coverage,
            ["cobertura"],
            CollectorInput::Xml,
            include_str!("plugins/cobertura.jq"),
        ));
        reg.register(Collector::jaq(
            "oxplow.lcov",
            CollectorKind::Coverage,
            ["lcov"],
            CollectorInput::Lcov,
            include_str!("plugins/lcov.jq"),
        ));
        reg.register(Collector::jaq(
            "oxplow.jacoco",
            CollectorKind::Coverage,
            ["jacoco", "jacoco-xml"],
            CollectorInput::Xml,
            include_str!("plugins/jacoco.jq"),
        ));
        reg.register(Collector::jaq(
            "oxplow.junit",
            CollectorKind::Test,
            ["junit"],
            CollectorInput::Xml,
            include_str!("plugins/junit.jq"),
        ));
        reg.register(Collector::jaq(
            "oxplow.clippy",
            CollectorKind::Analysis,
            ["clippy-json"],
            CollectorInput::Lines,
            include_str!("plugins/clippy.jq"),
        ));
        reg.register(Collector::jaq(
            "oxplow.eslint",
            CollectorKind::Analysis,
            ["eslint-json"],
            CollectorInput::Json,
            include_str!("plugins/eslint.jq"),
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
    fn jaq_analysis_collector_runs_end_to_end() {
        // A jaq analysis collector over JSON input → typed AnalysisReport.
        let program = r#"{ findings: [ .[] | { path: .file, line: .ln, severity: .lvl, rule: .lint, message: .msg } ] }"#;
        let c = Collector::jaq(
            "lint",
            CollectorKind::Analysis,
            ["lint"],
            CollectorInput::Json,
            program,
        );
        let out = c
            .run(r#"[{"file":"src/a.rs","ln":3,"lvl":"error","lint":"E1","msg":"boom"}]"#)
            .expect("runs");
        assert_eq!(out.kind(), CollectorKind::Analysis);
        let report = out.as_analysis().expect("analysis");
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].path, "src/a.rs");
        assert_eq!(
            report.findings[0].severity,
            oxplow_coverage::Severity::Error
        );
    }

    #[test]
    fn jaq_gauge_collector_runs_end_to_end() {
        // A jaq gauge over JSON input → typed MetricReport with one sample.
        let program =
            r#"{ samples: [ { value: (.lines | length), dims: { language: "rust" } } ] }"#;
        let c = Collector::jaq(
            "acme.loc",
            CollectorKind::Gauge,
            ["loc-json"],
            CollectorInput::Json,
            program,
        );
        let out = c.run(r#"{"lines":[1,2,3,4]}"#).expect("runs");
        assert_eq!(out.kind(), CollectorKind::Gauge);
        let report = out.as_gauge().expect("gauge");
        assert_eq!(report.samples.len(), 1);
        assert_eq!(report.samples[0].value, 4.0);
        assert_eq!(
            report.samples[0].dims.as_ref().unwrap()["language"],
            serde_json::json!("rust")
        );
    }

    #[test]
    fn starlark_gauge_collector_runs_end_to_end() {
        // A starlark gauge over raw text → counts via the regex_find host
        // builtin, projecting one subject-tagged sample.
        let script = r#"
def transform(input):
    n = len(regex_find(r"TODO", input))
    return {"samples": [{"value": n, "subject": "tree:."}]}
"#;
        let c = Collector::starlark(
            "acme.todos",
            CollectorKind::Gauge,
            ["todos"],
            CollectorInput::Text,
            script,
        );
        let out = c.run("a TODO here and a TODO there").expect("runs");
        let report = out.as_gauge().expect("gauge");
        assert_eq!(report.samples.len(), 1);
        assert_eq!(report.samples[0].value, 2.0);
        assert_eq!(report.samples[0].subject.as_deref(), Some("tree:."));
    }

    #[test]
    fn starlark_gauge_reads_snapshot_files_and_queries_ast() {
        // The headline P3 capability: a tree-derived gauge that walks the
        // snapshot file map via files() and counts AST nodes via ast_query().
        let script = r#"
def transform(input):
    n = 0
    for f in files("**/*.rs"):
        n += len(ast_query(f["text"], "rust", "(unsafe_block) @u"))
    return {"samples": [{"value": n, "subject": "tree:.", "dims": {"language": "rust"}}]}
"#;
        let c = Collector::starlark(
            "acme.unsafe_blocks",
            CollectorKind::Gauge,
            ["unsafe-blocks"],
            CollectorInput::Text,
            script,
        );
        let mut map = std::collections::HashMap::new();
        map.insert(
            "src/a.rs".to_string(),
            "fn a() { unsafe { x(); } }\nfn b() { unsafe { y(); } }".to_string(),
        );
        map.insert("src/b.rs".to_string(), "fn c() { let z = 1; }".to_string());
        // A non-Rust file the glob must skip.
        map.insert("README.md".to_string(), "unsafe { not code }".to_string());

        let out = c.run_gauge("", GaugeHost::new(map)).expect("runs");
        let report = out.as_gauge().expect("gauge");
        assert_eq!(report.samples.len(), 1);
        assert_eq!(report.samples[0].value, 2.0, "two unsafe blocks across .rs");
        assert_eq!(report.samples[0].subject.as_deref(), Some("tree:."));
    }

    #[test]
    fn files_builtin_is_empty_without_a_host() {
        // run() (no host) → files() sees no snapshot map and yields nothing.
        let script = r#"
def transform(input):
    return {"samples": [{"value": len(files("**/*"))}]}
"#;
        let c = Collector::starlark(
            "acme.count_files",
            CollectorKind::Gauge,
            ["count-files"],
            CollectorInput::Text,
            script,
        );
        let out = c.run("").expect("runs");
        assert_eq!(out.as_gauge().expect("gauge").samples[0].value, 0.0);
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
    fn builtin_plugins_skip_bad_fields_without_failing_the_report() {
        let reg = CollectorRegistry::with_builtins();
        // A non-numeric line number is skipped; the valid lines still land
        // (the old Rust parsers were field-tolerant — keep that).
        let cobertura = r#"<coverage><packages><package><classes>
          <class filename="src/a.rs"><lines>
            <line number="1" hits="1"/>
            <line number="oops" hits="1"/>
            <line number="2" hits="0"/>
          </lines></class>
        </classes></package></packages></coverage>"#;
        let out = reg.run("cobertura", cobertura).expect("plugin still runs");
        let f = out.as_coverage().unwrap().files.get("src/a.rs").unwrap();
        assert_eq!(
            f.instrumented.iter().copied().collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(f.covered.iter().copied().collect::<Vec<_>>(), vec![1]);

        // lcov: a garbage DA line is skipped, the rest survive.
        let lcov = "SF:src/a.rs\nDA:1,3\nDA:junk\nDA:2,0\nend_of_record\n";
        let out = reg.run("lcov", lcov).expect("plugin still runs");
        let f = out.as_coverage().unwrap().files.get("src/a.rs").unwrap();
        assert_eq!(
            f.instrumented.iter().copied().collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(f.covered.iter().copied().collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn builtin_plugins_surface_malformed_input_as_error() {
        let reg = CollectorRegistry::with_builtins();
        assert!(reg.run("cobertura", "<coverage><class").is_err());
        assert!(reg.run("junit", "<testsuites><testcase").is_err());
    }

    // ---- golden: bundled clippy / eslint analysis plugins ----

    use oxplow_coverage::{AnalysisFinding, AnalysisReport, Severity};

    fn finding(
        path: &str,
        line: Option<u32>,
        column: Option<u32>,
        severity: Severity,
        rule: Option<&str>,
        message: &str,
    ) -> AnalysisFinding {
        AnalysisFinding {
            path: path.into(),
            line,
            column,
            severity,
            rule: rule.map(Into::into),
            message: message.into(),
        }
    }

    // A realistic `cargo clippy --message-format=json` line stream: a
    // compiler-artifact line (skipped), a warning + an error with primary
    // spans, a message whose primary span is the second one, a span-less
    // summary ("N warnings emitted", skipped), and a plain non-JSON line.
    const GOLD_CLIPPY: &str = r#"{"reason":"compiler-artifact","target":{"name":"oxplow"}}
{"reason":"compiler-message","message":{"message":"unused variable: `y`","code":{"code":"unused_variables"},"level":"warning","spans":[{"file_name":"src/foo.rs","line_start":3,"column_start":9,"is_primary":true}]}}
{"reason":"compiler-message","message":{"message":"mismatched types","code":{"code":"E0308"},"level":"error","spans":[{"file_name":"src/bar.rs","line_start":10,"column_start":5,"is_primary":true}]}}
{"reason":"compiler-message","message":{"message":"needless return","code":{"code":"clippy::needless_return"},"level":"note","spans":[{"file_name":"a.rs","line_start":1,"column_start":1,"is_primary":false},{"file_name":"b.rs","line_start":2,"column_start":2,"is_primary":true}]}}
{"reason":"compiler-message","message":{"message":"1 warning emitted","code":null,"level":"warning","spans":[]}}
some plain text rustc emitted to the stream"#;

    #[test]
    fn builtin_clippy_plugin_produces_expected_findings() {
        let out = CollectorRegistry::with_builtins()
            .run("clippy-json", GOLD_CLIPPY)
            .expect("plugin runs");
        let expected = AnalysisReport {
            findings: vec![
                finding(
                    "src/foo.rs",
                    Some(3),
                    Some(9),
                    Severity::Warning,
                    Some("unused_variables"),
                    "unused variable: `y`",
                ),
                finding(
                    "src/bar.rs",
                    Some(10),
                    Some(5),
                    Severity::Error,
                    Some("E0308"),
                    "mismatched types",
                ),
                // Primary span (b.rs) is selected over the first (a.rs);
                // level "note" maps to Severity::Note.
                finding(
                    "b.rs",
                    Some(2),
                    Some(2),
                    Severity::Note,
                    Some("clippy::needless_return"),
                    "needless return",
                ),
            ],
        };
        assert_eq!(out.as_analysis().unwrap(), &expected);
    }

    const GOLD_ESLINT: &str = r#"[
      { "filePath": "src/a.js", "messages": [
        { "ruleId": "no-unused-vars", "severity": 2, "line": 1, "column": 7, "message": "x is unused" },
        { "ruleId": "eqeqeq", "severity": 1, "line": 5, "column": 3, "message": "use ===" }
      ] },
      { "filePath": "src/b.js", "messages": [
        { "ruleId": null, "severity": 2, "line": 2, "column": 1, "message": "Parsing error" }
      ] },
      { "filePath": "src/clean.js", "messages": [] }
    ]"#;

    #[test]
    fn builtin_eslint_plugin_produces_expected_findings() {
        let out = CollectorRegistry::with_builtins()
            .run("eslint-json", GOLD_ESLINT)
            .expect("plugin runs");
        let expected = AnalysisReport {
            findings: vec![
                finding(
                    "src/a.js",
                    Some(1),
                    Some(7),
                    Severity::Error,
                    Some("no-unused-vars"),
                    "x is unused",
                ),
                finding(
                    "src/a.js",
                    Some(5),
                    Some(3),
                    Severity::Warning,
                    Some("eqeqeq"),
                    "use ===",
                ),
                // ruleId null → a finding with no rule.
                finding(
                    "src/b.js",
                    Some(2),
                    Some(1),
                    Severity::Error,
                    None,
                    "Parsing error",
                ),
            ],
        };
        assert_eq!(out.as_analysis().unwrap(), &expected);
    }

    #[test]
    fn builtin_analysis_plugins_register_under_analysis_kind() {
        let reg = CollectorRegistry::with_builtins();
        for fmt in ["clippy-json", "eslint-json"] {
            let c = reg.resolve(fmt).expect("analysis format registered");
            assert_eq!(c.kind(), CollectorKind::Analysis, "{fmt}");
            assert_eq!(c.runtime(), CollectorRuntime::Jaq);
        }
    }
}
