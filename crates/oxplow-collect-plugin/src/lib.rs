//! Pluggable, cross-language collection: report parsers defined as scripts
//! (a bundled set, plus a project's own report collectors' — tsk863)
//! instead of hardcoded Rust `match` arms.
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
//! [`CollectorKind`]; the genericity lives in this uniform definition
//! mechanism over *typed* kinds, not in the data being a blob. A future
//! kind (perf, structure-map, …) is a new [`CollectorKind`] + parsers that
//! target it — not a new subsystem.
//!
//! A report collector (`.oxplow/project.yaml` `collectors:` with
//! `records:`) names its parser: a bundled one ([`Collector::bundled`],
//! `entry: oxplow:<name>`) or its own jaq / Starlark / exec program.

use oxplow_coverage::{AnalysisReport, CoverageReport, TestReport};
use serde::{Deserialize, Serialize};

pub mod ai;
pub mod builtin_metrics;
pub mod helpers;
pub mod runtime;
pub use ai::{AiHost, AiOracle};
pub use builtin_metrics::{builtin_metrics, BuiltinMetric};
pub use helpers::HelperError;
pub use runtime::{SandboxBudget, TreeHost};

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
}

/// Which engine runs a collector's field-mapping step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CollectorRuntime {
    /// jq via `jaq` — the primary script tier for JSON→JSON reshaping.
    Jaq,
    /// Starlark — the general script tier for imperative/odd formats.
    Starlark,
    /// An external process (JSON stdin→stdout). The escape hatch; lower-trust.
    Exec,
}

/// One durable atomic fact a collector records (epic tsk12; P7.B3) — a
/// per-subject measurement bound to a **measure**, the grain a metric spec
/// re-aggregates. `measure` is the (defined) measure key the fact lands on;
/// a fact on an undefined measure is a declare-to-collect violation the host
/// surfaces. `subject` is an optional `"kind:ref"` string;
/// `path`/`line` are the location at capture; `dims` are open author dimensions
/// carried onto the fact as `dims_json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectedFact {
    pub measure: String,
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<i64>,
    /// Reported rule/idiom this fact belongs to — populates the fact's `rule`
    /// column, which the engine reads as the `oxplow.rule` dimension (so a spec
    /// can `dim_eq` on it). The per-language idiom gauges tag each `oxplow.ast_hit`
    /// fact with the idiom slug here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    /// Ratio components (epic tsk12) — when the measure is a ratio base, a fact
    /// may carry its own `num`/`den` so a `ratio` metric re-derives Σnum/Σden
    /// exactly (coverage %, pass rate) instead of averaging pre-divided values.
    /// Both or neither; a bare `value` fact leaves them unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub den: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dims: Option<serde_json::Map<String, serde_json::Value>>,
}

/// A fact collector's output — `{"facts": [...]}` — as its facts.
pub fn facts_of(value: serde_json::Value) -> Result<Vec<CollectedFact>, CollectError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Output {
        #[serde(default)]
        facts: Vec<CollectedFact>,
    }
    serde_json::from_value::<Output>(value)
        .map(|o| o.facts)
        .map_err(|e| {
            CollectError::Shape(format!(
                "a fact collector returns {{\"facts\": [...]}}: {e}"
            ))
        })
}

/// Run a fact collector's Starlark `script` over `input` with `host`'s tree
/// in scope (`files()`, `source_files()`), under `budget`, and read its facts.
pub fn run_fact_starlark(
    script: &str,
    input: &serde_json::Value,
    host: TreeHost,
    budget: &SandboxBudget,
) -> Result<Vec<CollectedFact>, CollectError> {
    let (script, input) = (script.to_string(), input.clone());
    let raw = runtime::run_sandboxed(budget, move || {
        runtime::run_starlark_with_host(&script, &input, &host)
    })?;
    facts_of(raw)
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
}

impl CollectorOutput {
    /// The output with its file paths repo-relative to `root`, the
    /// checkout the run ran in (coverage files and analysis findings;
    /// see [`oxplow_coverage::repo_relative`]). A test report names no
    /// files.
    pub fn relative_to(self, root: &std::path::Path) -> Self {
        match self {
            Self::Coverage(r) => Self::Coverage(r.relative_to(root)),
            Self::Analysis(r) => Self::Analysis(r.relative_to(root)),
            Self::Test(r) => Self::Test(r),
        }
    }

    /// The kind this output corresponds to.
    pub fn kind(&self) -> CollectorKind {
        match self {
            CollectorOutput::Coverage(_) => CollectorKind::Coverage,
            CollectorOutput::Test(_) => CollectorKind::Test,
            CollectorOutput::Analysis(_) => CollectorKind::Analysis,
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

/// Parse a report's text in a collector's `report.format` (`text`,
/// `json`, `xml`, `lcov` or `lines`): what a collector gets as
/// `input.report`.
pub fn parse_report(format: &str, content: &str) -> Result<serde_json::Value, CollectError> {
    CollectorInput::named(format)
        .ok_or_else(|| CollectError::UnknownFormat(format.to_string()))?
        .parse(content)
}

impl CollectorInput {
    /// The container parser a `report.format` names (`text`, `json`, `xml`,
    /// `lcov`, `lines`).
    pub fn named(format: &str) -> Option<Self> {
        Some(match format {
            "text" => CollectorInput::Text,
            "json" => CollectorInput::Json,
            "xml" => CollectorInput::Xml,
            "lcov" => CollectorInput::Lcov,
            "lines" => CollectorInput::Lines,
            _ => return None,
        })
    }

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

/// How a collector runs.
#[derive(Clone)]
enum Runner {
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

/// The parsers oxplow ships, named by `entry: oxplow:<name>`: what each
/// records, how its report is pre-parsed, and its jq program. The config
/// side (`oxplow_config::collectors::BUNDLED_PARSERS`) names the same set;
/// a test holds them together.
pub const BUNDLED: &[(&str, CollectorKind, CollectorInput, &str)] = &[
    (
        "junit",
        CollectorKind::Test,
        CollectorInput::Xml,
        include_str!("plugins/junit.jq"),
    ),
    (
        "lcov",
        CollectorKind::Coverage,
        CollectorInput::Lcov,
        include_str!("plugins/lcov.jq"),
    ),
    (
        "cobertura",
        CollectorKind::Coverage,
        CollectorInput::Xml,
        include_str!("plugins/cobertura.jq"),
    ),
    (
        "jacoco",
        CollectorKind::Coverage,
        CollectorInput::Xml,
        include_str!("plugins/jacoco.jq"),
    ),
    (
        "clippy",
        CollectorKind::Analysis,
        CollectorInput::Lines,
        include_str!("plugins/clippy.jq"),
    ),
    (
        "eslint",
        CollectorKind::Analysis,
        CollectorInput::Json,
        include_str!("plugins/eslint.jq"),
    ),
];

/// An executable report parser: its name, kind and a way to run it.
#[derive(Clone)]
pub struct Collector {
    name: String,
    kind: CollectorKind,
    runtime: CollectorRuntime,
    runner: Runner,
    budget: SandboxBudget,
}

impl Collector {
    fn new(
        name: impl Into<String>,
        kind: CollectorKind,
        runtime: CollectorRuntime,
        runner: Runner,
    ) -> Self {
        Collector {
            name: name.into(),
            kind,
            runtime,
            runner,
            budget: SandboxBudget::default(),
        }
    }

    /// A bundled parser by its name (`junit`, `lcov`, …), named
    /// `oxplow.<name>`.
    pub fn bundled(name: &str) -> Option<Self> {
        let (name, kind, input, program) = BUNDLED.iter().find(|(n, ..)| *n == name)?;
        Some(Self::jaq(format!("oxplow.{name}"), *kind, *input, *program))
    }

    /// Construct a jaq (jq) collector: the host pre-parses content via `input`,
    /// then runs `program` and deserializes the result into `kind`.
    pub fn jaq(
        name: impl Into<String>,
        kind: CollectorKind,
        input: CollectorInput,
        program: impl Into<String>,
    ) -> Self {
        Self::new(
            name,
            kind,
            CollectorRuntime::Jaq,
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
        input: CollectorInput,
        script: impl Into<String>,
    ) -> Self {
        Self::new(
            name,
            kind,
            CollectorRuntime::Starlark,
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
        argv: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::new(
            name,
            kind,
            CollectorRuntime::Exec,
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

    /// Run the collector against raw report `content`, producing typed output.
    /// In-process script tiers run under the sandbox budget; exec relies on the
    /// child process and is tagged lower-trust by the caller.
    pub fn run(&self, content: &str) -> Result<CollectorOutput, CollectError> {
        let kind = self.kind;
        match &self.runner {
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
                let raw = runtime::run_exec(&self.budget, argv, content)?;
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
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the bundled parser `name` over `content`.
    fn bundled(name: &str, content: &str) -> Result<CollectorOutput, CollectError> {
        Collector::bundled(name)
            .unwrap_or_else(|| panic!("no bundled parser `{name}`"))
            .run(content)
    }

    /// tsk863: every bundled parser is a jaq program of its kind; an unknown
    /// name is none.
    #[test]
    fn the_bundled_parsers_are_named() {
        for (name, kind, _, _) in BUNDLED {
            let c = Collector::bundled(name).unwrap();
            assert_eq!(c.kind(), *kind, "{name}");
            assert_eq!(c.runtime(), CollectorRuntime::Jaq);
            assert_eq!(c.name(), format!("oxplow.{name}"));
        }
        assert!(Collector::bundled("clover").is_none());
    }

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

    // bun nests file-suite → describe-suite → testcase. The case must be
    // counted ONCE (under its immediate parent), not under both levels.
    const JUNIT_NESTED: &str = r#"<testsuites failures="1">
  <testsuite name="src/format.test.ts" file="src/format.test.ts">
    <testsuite name="describe block">
      <testcase classname="describe block" name="fails"><failure message="x"/></testcase>
    </testsuite>
  </testsuite>
</testsuites>"#;

    #[test]
    fn builtin_collector_runs_and_yields_typed_output() {
        let out = bundled("cobertura", COBERTURA).expect("parses");
        let cov = out.as_coverage().expect("coverage output");
        let f = cov.files.get("src/a.rs").expect("file present");
        assert!(f.instrumented.contains(&1) && f.instrumented.contains(&2));
        assert!(f.covered.contains(&1) && !f.covered.contains(&2));

        let out = bundled("junit", JUNIT).expect("parses");
        let test = out.as_test().expect("test output");
        assert_eq!(test.suites.len(), 1);
        assert_eq!(test.suites[0].cases.len(), 1);
    }

    #[test]
    fn nested_junit_counts_each_case_once() {
        // tsk361: a nested testcase must not be double-counted under both
        // its file-suite and its describe-suite.
        let out = bundled("junit", JUNIT_NESTED).expect("parses");
        let test = out.as_test().expect("test output");
        let total: usize = test.suites.iter().map(|s| s.cases.len()).sum();
        assert_eq!(
            total, 1,
            "nested testcase counted once, not per suite level"
        );
    }

    #[test]
    fn jaq_collector_runs_end_to_end_with_xml_input() {
        // Host pre-parses XML → tree, jaq maps it to coverage output.
        let program = r#"{ files: { (.attrs.file): { instrumented: [1, 2], covered: [1] } } }"#;
        let c = Collector::jaq(
            "xcov",
            CollectorKind::Coverage,
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

    /// A fact script's facts over a file map, with the default budget.
    fn facts(script: &str, map: std::collections::HashMap<String, String>) -> Vec<CollectedFact> {
        run_fact_starlark(
            script,
            &serde_json::json!({}),
            TreeHost::new(map),
            &SandboxBudget::default(),
        )
        .expect("runs")
    }

    #[test]
    fn a_jaq_fact_script_reads_its_report() {
        let raw = runtime::run_jaq(
            r#"{ facts: [ { measure: "acme.loc", value: (.report.lines | length), dims: { language: "rust" } } ] }"#,
            &serde_json::json!({ "report": { "lines": [1, 2, 3, 4] } }),
        )
        .expect("runs");
        let facts = facts_of(raw).expect("facts");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].value, 4.0);
        assert_eq!(
            facts[0].dims.as_ref().unwrap()["language"],
            serde_json::json!("rust")
        );
        // Anything but `facts` is refused.
        let err = facts_of(serde_json::json!({ "samples": [] })).unwrap_err();
        assert!(err.to_string().contains("facts"), "{err}");
    }

    #[test]
    fn a_fact_script_reads_snapshot_files_and_queries_ast() {
        // The headline capability: a tree script walks the snapshot file map
        // via files() and counts AST nodes via ast_query().
        let script = r#"
def transform(input):
    n = 0
    for f in files("**/*.rs"):
        n += len(ast_query(f["text"], "rust", "(unsafe_block) @u"))
    return {"facts": [{"measure": "acme.unsafe", "value": n, "subject": "tree:."}]}
"#;
        let mut map = std::collections::HashMap::new();
        map.insert(
            "src/a.rs".to_string(),
            "fn a() { unsafe { x(); } }\nfn b() { unsafe { y(); } }".to_string(),
        );
        map.insert("src/b.rs".to_string(), "fn c() { let z = 1; }".to_string());
        // A non-Rust file the glob must skip.
        map.insert("README.md".to_string(), "unsafe { not code }".to_string());
        let out = facts(script, map);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].value, 2.0, "two unsafe blocks across .rs");
        assert_eq!(out[0].subject.as_deref(), Some("tree:."));
    }

    #[test]
    fn source_files_and_markers_builtins_are_language_agnostic() {
        // A single script sweeps source_files() (each tagged with language) and
        // counts TODO/FIXME markers per file via the markers() capability — no
        // language named in the script.
        let script = r#"
def transform(input):
    by_lang = {}
    for f in source_files():
        lang = f["language"]
        c = len(markers(f["text"], lang))
        if c > 0:
            by_lang[lang] = by_lang.get(lang, 0) + c
    return {"facts": [{"measure": "acme.todo", "value": by_lang[lang], "dims": {"language": lang}} for lang in sorted(by_lang)]}
"#;
        let mut map = std::collections::HashMap::new();
        map.insert(
            "src/a.rs".to_string(),
            "// TODO one\nfn a() {}\n".to_string(),
        );
        map.insert(
            "src/b.ts".to_string(),
            "// FIXME two\n// TODO three\nexport const x = 1;\n".to_string(),
        );
        map.insert(
            "core.clj".to_string(),
            "; TODO four\n(defn f [])\n".to_string(),
        );
        // Non-source file (skipped by source_files) — its "TODO" must not count.
        map.insert("README.md".to_string(), "TODO not code\n".to_string());
        let by_lang: Vec<(String, f64)> = facts(script, map)
            .into_iter()
            .map(|f| {
                (
                    f.dims.unwrap()["language"].as_str().unwrap().to_string(),
                    f.value,
                )
            })
            .collect();
        assert_eq!(
            by_lang,
            vec![
                ("clojure".to_string(), 1.0),
                ("rust".to_string(), 1.0),
                ("typescript".to_string(), 2.0)
            ],
            "4 markers across rust/ts/clojure, README skipped"
        );
    }

    #[test]
    fn source_files_skips_generated_artifacts() {
        // tsk68: codegen output (a `generated/` path segment or a do-not-edit
        // header) must not enter the code-shape corpus — a 3k-line bindings
        // file read as one giant "function" dominates every tail metric.
        let script = r#"
def transform(input):
    return {"facts": [{"measure": "acme.files", "value": len(source_files())}]}
"#;
        let mut map = std::collections::HashMap::new();
        map.insert("src/a.rs".to_string(), "fn a() {}\n".to_string());
        map.insert(
            "src/generated/bindings.ts".to_string(),
            "export const x = 1;\n".to_string(),
        );
        map.insert(
            "src/api.ts".to_string(),
            "// This file has been generated by Tauri Specta. Do not edit this file manually.\nexport const y = 1;\n".to_string(),
        );
        assert_eq!(
            facts(script, map)[0].value,
            1.0,
            "only the hand-written file survives"
        );
    }

    #[test]
    fn files_builtin_is_empty_without_a_host() {
        // No host → files() sees no snapshot map and yields nothing.
        let raw = runtime::run_starlark(
            "def transform(input):\n    return {\"facts\": [{\"measure\": \"acme.n\", \"value\": len(files(\"**/*\"))}]}\n",
            &serde_json::json!({}),
        )
        .expect("runs");
        assert_eq!(facts_of(raw).unwrap()[0].value, 0.0);
    }

    #[test]
    fn starlark_collector_runs_end_to_end_with_xml_input() {
        let script = "def transform(input):\n    return {\"suites\": [{\"name\": input[\"tag\"], \"cases\": []}]}\n";
        let c = Collector::starlark("xtest", CollectorKind::Test, CollectorInput::Xml, script);
        let out = c.run("<suite/>").expect("runs");
        assert_eq!(out.as_test().expect("test").suites[0].name, "suite");
    }

    #[cfg(unix)]
    #[test]
    fn exec_collector_round_trips_stdin_to_stdout() {
        // `cat` echoes the (already kind-shaped) JSON from stdin to stdout.
        let c = Collector::exec("e", CollectorKind::Test, ["cat"]);
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
          <methods>
            <method name="a" signature="()V" line-rate="1.0" branch-rate="1.0">
              <lines><line number="1" hits="3"/></lines>
            </method>
            <method name="b" signature="()V" line-rate="0.0" branch-rate="0.0">
              <lines><line number="2" hits="0"/></lines>
            </method>
          </methods>
          <lines>
            <line number="1" hits="3" branch="true" condition-coverage="100% (2/2)"/>
            <line number="2" hits="0" branch="true" condition-coverage="0% (0/2)"/>
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

    const GOLD_LCOV: &str = "TN:\nSF:src/foo.rs\nFNF:2\nFNH:2\nDA:1,3\nDA:2,0\nDA:5,1\nBRF:4\nBRH:3\nend_of_record\nSF:src/bar.rs\nDA:10,0\nend_of_record\n";

    const GOLD_JACOCO: &str = r#"<?xml version="1.0"?>
<report name="r">
  <package name="com/example">
    <sourcefile name="Foo.java">
      <line nr="1" mi="0" ci="4"/>
      <line nr="2" mi="3" ci="0"/>
      <counter type="BRANCH" missed="1" covered="3"/>
      <counter type="METHOD" missed="0" covered="2"/>
      <counter type="LINE" missed="1" covered="1"/>
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
                    // Line-only helper; branch/function set per-file in the tests
                    // that exercise them.
                    ..Default::default()
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

    /// Set a file's branch/function counts on an expected report (line-only
    /// `cov` leaves them 0). `(bf, bh, ff, fh)`.
    fn set_bf(report: &mut CoverageReport, path: &str, bf: u32, bh: u32, ff: u32, fh: u32) {
        let f = report.files.get_mut(path).expect("file present");
        f.branches_found = bf;
        f.branches_hit = bh;
        f.functions_found = ff;
        f.functions_hit = fh;
    }

    #[test]
    fn builtin_cobertura_plugin_produces_expected_coverage() {
        let out = bundled("cobertura", GOLD_COBERTURA).expect("plugin runs");
        let mut expected = cov(&[
            ("src/foo.rs", &[1, 2, 5], &[1, 5]),
            ("src/bar.rs", &[10], &[]),
        ]);
        // Branch: (2/2)+(0/2) → 4 found / 2 hit. Methods: 2, one with line-rate>0.
        set_bf(&mut expected, "src/foo.rs", 4, 2, 2, 1);
        assert_eq!(out.as_coverage().unwrap(), &expected);
    }

    #[test]
    fn builtin_lcov_plugin_produces_expected_coverage() {
        let out = bundled("lcov", GOLD_LCOV).expect("plugin runs");
        let mut expected = cov(&[
            ("src/foo.rs", &[1, 2, 5], &[1, 5]),
            ("src/bar.rs", &[10], &[]),
        ]);
        // BRF:4 BRH:3 FNF:2 FNH:2 on the foo record; bar has none → 0.
        set_bf(&mut expected, "src/foo.rs", 4, 3, 2, 2);
        assert_eq!(out.as_coverage().unwrap(), &expected);
    }

    /// Per-file DA counts mirroring this repo's real whole-workspace `cargo cov`
    /// report: 196 SF records / ~64k DA lines, **heavily skewed** — the five
    /// biggest carry 4783 / 3866 / 2981 / 2165 / 2064 entries.
    ///
    /// The skew is the whole point. The cost being fixed was quadratic **per
    /// file**, so a uniform many-small-files report parses fine and hides the
    /// bug; one 4.8k-line file is what detonates it.
    fn workspace_lcov_sizes() -> Vec<usize> {
        let mut sizes = vec![4783, 3866, 2981, 2165, 2064];
        sizes.extend(std::iter::repeat_n(250, 191));
        sizes
    }

    fn lcov_with_sizes(sizes: &[usize]) -> String {
        let mut s = String::new();
        for (f, &lines) in sizes.iter().enumerate() {
            s.push_str(&format!("SF:src/generated/mod_{f}.rs\n"));
            for l in 1..=lines {
                s.push_str(&format!("DA:{l},{}\n", l % 3));
            }
            s.push_str("end_of_record\n");
        }
        s
    }

    #[test]
    fn lcov_plugin_parses_a_whole_workspace_report_without_timing_out() {
        // tsk88: the real report here is 5.1MB / 196 files / 64k DA lines, and the
        // lcov parse landed right at the 5s SandboxBudget — so it timed out
        // intermittently under load and coverage was silently NEVER ingested for
        // this project. Every overrun also detached a jq worker that kept burning
        // a core (Rust can't kill a thread), and the ride-along's retry spawned a
        // second: the budget bounded the wait, not the work.
        //
        // Deliberately asserts *completion under the real default budget* rather
        // than a wall-clock number — the failure mode was a timeout, and a
        // wall-clock assertion on a shared CI box is a flake generator.
        let sizes = workspace_lcov_sizes();
        let out = bundled("lcov", &lcov_with_sizes(&sizes))
            .expect("a whole-workspace report parses under the default budget");

        let parsed = out.as_coverage().unwrap();
        assert_eq!(parsed.files.len(), sizes.len(), "every record parsed");
        let biggest = parsed
            .files
            .get("src/generated/mod_0.rs")
            .expect("the 4783-line file present");
        assert_eq!(biggest.instrumented.len(), 4783);
        // Covered = hits > 0, i.e. every line except the multiples of 3.
        assert_eq!(biggest.covered.len(), 4783 - 4783 / 3);
    }

    /// The first measured ratio under `limit`, or every ratio seen if
    /// `attempts` all exceeded it.
    ///
    /// Retrying is sound *because* scheduler noise only ever ADDS time: a
    /// descheduled sample inflates one attempt, while a genuine quadratic
    /// regression sits near 16x on every attempt. Passing on any clean
    /// attempt therefore leaves the guard exactly as strict as a
    /// single-shot check while dropping its false-positive rate to ~0.
    fn first_ratio_under(
        limit: f64,
        attempts: usize,
        mut measure: impl FnMut() -> f64,
    ) -> Result<f64, Vec<f64>> {
        let mut seen = Vec::with_capacity(attempts);
        for _ in 0..attempts {
            let ratio = measure();
            if ratio < limit {
                return Ok(ratio);
            }
            seen.push(ratio);
        }
        Err(seen)
    }

    #[test]
    fn ratio_retry_passes_when_a_spike_is_followed_by_a_clean_sample() {
        let mut samples = [12.0, 4.1].into_iter();
        assert_eq!(
            first_ratio_under(8.0, 3, || samples.next().unwrap()),
            Ok(4.1)
        );
    }

    #[test]
    fn ratio_retry_fails_when_every_attempt_exceeds_the_limit() {
        let mut samples = [16.0, 15.8, 16.2].into_iter();
        assert_eq!(
            first_ratio_under(8.0, 3, || samples.next().unwrap()),
            Err(vec![16.0, 15.8, 16.2]),
        );
    }

    #[test]
    fn lcov_plugin_cost_stays_linear_in_lines_per_file() {
        // The original built each file's line lists with `.instrumented += [$n]`
        // inside a reduce — quadratic PER FILE. Doubling the lines in ONE file
        // quadrupled the work, so a single big generated file could blow any
        // budget on its own. Ratio-based rather than absolute: it's the SHAPE of
        // the curve that regressed, and a ratio survives a slow machine.
        // Min-of-3 per size: a parallel nextest run on a loaded box deschedules
        // a sample for whole scheduler quanta, and one spiked sample corrupts a
        // single-shot ratio (seen twice in real runs). Noise only ever ADDS
        // time, so the minimum estimates the true cost; the hypothesis gap
        // below (linear ~4x vs quadratic ~16x) is untouched.
        //
        // tsk175: min-of-3 was not enough. Under `cargo llvm-cov nextest` every
        // core is saturated by sibling test processes AND the binary is
        // instrumented, so all three samples of a size can be descheduled
        // together — which is how this test failed once in a full run and then
        // refused to reproduce in isolation. So retry the whole comparison via
        // `first_ratio_under`: a spike ruins one attempt, a real regression
        // ruins every one.
        let time_one_file = |lines: usize| {
            let content = lcov_with_sizes(&[lines]);
            (0..3)
                .map(|_| {
                    let started = std::time::Instant::now();
                    bundled("lcov", &content).expect("parses");
                    started.elapsed()
                })
                .min()
                .expect("three samples")
        };
        // Warm the compile path so it isn't charged to the first sample.
        let _ = time_one_file(1_000);

        // 4x the lines rather than 2x, to keep the two hypotheses far apart:
        // linear lands near 4x, quadratic near 16x, so the 8x line has ~2x
        // margin either side. (At 2x the measured gap was 2.0 vs 3.2 against a
        // 3.0 threshold — real, but too tight to trust on a loaded box.)
        let measure = || {
            let base = time_one_file(5_000).max(std::time::Duration::from_millis(1));
            let quadrupled = time_one_file(20_000);
            quadrupled.as_secs_f64() / base.as_secs_f64()
        };
        if let Err(ratios) = first_ratio_under(8.0, 3, measure) {
            panic!(
                "4x-ing one file's lines took {ratios:.1?}x across {} attempts; \
                 linear is ~4x and quadratic is ~16x — an accumulator copy is back",
                ratios.len(),
            );
        }
    }

    #[test]
    fn builtin_jacoco_plugin_produces_expected_coverage() {
        let mut expected = cov(&[
            ("com/example/Foo.java", &[1, 2], &[1]),
            ("Root.java", &[7], &[7]),
        ]);
        // Foo.java counters: BRANCH 3/4, METHOD 2/2. Root.java has none → 0.
        set_bf(&mut expected, "com/example/Foo.java", 4, 3, 2, 2);
        let out = bundled("jacoco", GOLD_JACOCO).expect("plugin runs");
        assert_eq!(out.as_coverage().unwrap(), &expected);
    }

    #[test]
    fn builtin_junit_plugin_produces_expected_tree() {
        use oxplow_coverage::{TestStatus, TestSuite};

        let nextest = bundled("junit", GOLD_JUNIT_NEXTEST).expect("plugin runs");
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

        let pytest = bundled("junit", GOLD_JUNIT_PYTEST).expect("plugin runs");
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
        // A non-numeric line number is skipped; the valid lines still land
        // (the old Rust parsers were field-tolerant — keep that).
        let cobertura = r#"<coverage><packages><package><classes>
          <class filename="src/a.rs"><lines>
            <line number="1" hits="1"/>
            <line number="oops" hits="1"/>
            <line number="2" hits="0"/>
          </lines></class>
        </classes></package></packages></coverage>"#;
        let out = bundled("cobertura", cobertura).expect("plugin still runs");
        let f = out.as_coverage().unwrap().files.get("src/a.rs").unwrap();
        assert_eq!(
            f.instrumented.iter().copied().collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(f.covered.iter().copied().collect::<Vec<_>>(), vec![1]);

        // lcov: a garbage DA line is skipped, the rest survive.
        let lcov = "SF:src/a.rs\nDA:1,3\nDA:junk\nDA:2,0\nend_of_record\n";
        let out = bundled("lcov", lcov).expect("plugin still runs");
        let f = out.as_coverage().unwrap().files.get("src/a.rs").unwrap();
        assert_eq!(
            f.instrumented.iter().copied().collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(f.covered.iter().copied().collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn builtin_plugins_surface_malformed_input_as_error() {
        assert!(bundled("cobertura", "<coverage><class").is_err());
        assert!(bundled("junit", "<testsuites><testcase").is_err());
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
        let out = bundled("clippy", GOLD_CLIPPY).expect("plugin runs");
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
        let out = bundled("eslint", GOLD_ESLINT).expect("plugin runs");
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
}
