//! Layer-2 **transform runtimes** — the engines that run a collector's
//! field-mapping step and produce the kind's typed output.
//!
//! Three tiers, in trust/preference order:
//! - [`run_jaq`] — jq via `jaq` (pure Rust). Primary: JSON→JSON reshaping.
//! - [`run_starlark`] — Starlark (pure Rust). General/imperative tier.
//! - [`run_exec`] — an external process (JSON on stdin, JSON on stdout). The
//!   escape hatch; lower-trust (it can do I/O), tagged so the UI can mark it.
//!
//! The two in-process tiers are deterministic and do no I/O, so their output
//! is `observed`-eligible. They are run under a [`SandboxBudget`] (a wall-clock
//! timeout) so a malformed or runaway script is isolated and surfaced as an
//! error rather than hanging or crashing collection.
//!
//! Every engine takes an already-parsed JSON input value (the host applied the
//! collector's declared container parser first — see `CollectorInput`) and
//! returns a JSON value, which the caller deserializes into the kind's typed
//! output via [`value_to_output`].

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::mpsc;
use std::time::Duration;

use oxplow_coverage::{
    AnalysisFinding, AnalysisReport, CoverageReport, FileCoverage, Severity, TestCase, TestReport,
    TestStatus, TestSuite,
};
use serde::Deserialize;
use serde_json::Value;

use crate::{CollectError, CollectorKind, CollectorOutput, MetricReport};

/// Resource limits for an in-process script run. Currently a wall-clock
/// timeout enforced by running the engine on a worker thread; the caller
/// returns promptly on overrun even if the worker is still unwinding.
#[derive(Debug, Clone, Copy)]
pub struct SandboxBudget {
    pub timeout: Duration,
}

impl Default for SandboxBudget {
    fn default() -> Self {
        SandboxBudget {
            timeout: Duration::from_secs(5),
        }
    }
}

impl SandboxBudget {
    /// A budget with a custom timeout (used by tests to exercise overrun).
    pub fn with_timeout(timeout: Duration) -> Self {
        SandboxBudget { timeout }
    }
}

/// Run `f` on a worker thread, returning its result or [`CollectError::Timeout`]
/// if it does not finish within `budget.timeout`. On timeout the worker is
/// detached (it finishes on its own); the caller is never blocked past the
/// deadline.
pub fn run_sandboxed<F>(budget: &SandboxBudget, f: F) -> Result<Value, CollectError>
where
    F: FnOnce() -> Result<Value, CollectError> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(budget.timeout) {
        Ok(result) => result,
        Err(_) => Err(CollectError::Timeout),
    }
}

/// Run a jq program (`jaq`) against `input`, returning its single output value.
/// More or fewer than one output is an error — a collector maps to exactly one
/// report object.
pub fn run_jaq(program: &str, input: &Value) -> Result<Value, CollectError> {
    use jaq_core::load::{Arena, File, Loader};
    use jaq_core::{data, Compiler, Ctx, Vars};
    use jaq_json::Val;

    let input_bytes =
        serde_json::to_vec(input).map_err(|e| CollectError::Runtime(e.to_string()))?;
    let input_val = jaq_json::read::parse_single(input_bytes.as_slice())
        .map_err(|e| CollectError::Runtime(format!("jaq input parse: {e}")))?;

    let program_file = File {
        code: program,
        path: (),
    };
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    // Drop jaq-std's impure natives so a collection transform stays
    // deterministic and does no I/O — its output is trusted as `observed`.
    // We keep the full `funs()` (not `base_funs()`) because jaq-std's *defs*
    // depend on pure extras like `split_`/`matches`/`pow`; we only filter the
    // ones that read host state. A `Fun` is `(name, arity, impl)`, so `f.0` is
    // the name. None of these are referenced by jaq-std defs, so removing them
    // doesn't break the library — a plugin calling them just fails to compile.
    const IMPURE_JAQ_FUNS: &[&str] = &["env", "now", "localtime", "input", "inputs"];
    let funs = jaq_core::funs()
        .chain(jaq_std::funs().filter(|f| !IMPURE_JAQ_FUNS.contains(&f.0)))
        .chain(jaq_json::funs());

    let loader = Loader::new(defs);
    let arena = Arena::default();
    let modules = loader
        .load(&arena, program_file)
        .map_err(|e| CollectError::Runtime(format!("jaq compile: {e:?}")))?;
    let filter = Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|e| CollectError::Runtime(format!("jaq compile: {e:?}")))?;

    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
    let mut outputs = filter.id.run((ctx, input_val)).map(jaq_core::unwrap_valr);

    let first = outputs
        .next()
        .ok_or_else(|| CollectError::Runtime("jaq produced no output".into()))?
        .map_err(|e| CollectError::Runtime(format!("jaq run: {e}")))?;
    if outputs.next().is_some() {
        return Err(CollectError::Runtime(
            "jaq produced more than one output (expected a single report object)".into(),
        ));
    }
    // jaq_json::Val's Display is its JSON serialization.
    serde_json::from_str(&first.to_string()).map_err(|e| CollectError::Shape(e.to_string()))
}

/// Native Starlark builtins exposing the layer-1 container-parse helpers, so a
/// Starlark plugin can parse raw text itself (e.g. `input: text` + bespoke
/// logic) instead of relying only on the host pre-parse. Each returns a real
/// Starlark value — starlark implements `AllocValue` for `serde_json::Value`,
/// so `heap.alloc(...)` does the conversion. (jaq can't call host functions,
/// which is why the bundled parsers pre-parse via `CollectorInput` instead.)
#[starlark::starlark_module]
fn collect_helpers(builder: &mut starlark::environment::GlobalsBuilder) {
    fn parse_xml<'v>(
        content: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        Ok(heap.alloc(crate::helpers::parse_xml(content).map_err(helper_anyhow)?))
    }
    fn parse_json<'v>(
        content: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        Ok(heap.alloc(crate::helpers::parse_json(content).map_err(helper_anyhow)?))
    }
    fn lcov_records<'v>(
        content: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        Ok(heap.alloc(crate::helpers::lcov_records(content)))
    }
    fn lines<'v>(
        content: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        Ok(heap.alloc(crate::helpers::lines(content)))
    }
    fn regex_find<'v>(
        pattern: &str,
        text: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        Ok(heap.alloc(crate::helpers::regex_find(pattern, text).map_err(helper_anyhow)?))
    }
    fn xpath<'v>(
        content: &str,
        expr: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        Ok(heap.alloc(crate::helpers::xpath(content, expr).map_err(helper_anyhow)?))
    }
    /// Parse `text` as `language` and run a tree-sitter S-expression `query`,
    /// returning a flat list of `{capture, text, start_row, start_col, end_row,
    /// end_col}` matches. Pure (text passed inline → deterministic, `observed`).
    fn ast_query<'v>(
        text: &str,
        language: &str,
        query: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let matches = oxplow_code_metrics::ast_query(text, language, query)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let arr: Vec<serde_json::Value> = matches
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "capture": m.capture,
                    "text": m.text,
                    "start_row": m.start_row,
                    "start_col": m.start_col,
                    "end_row": m.end_row,
                    "end_col": m.end_col,
                })
            })
            .collect();
        Ok(heap.alloc(serde_json::Value::Array(arr)))
    }
    /// Per-function code metrics for `text` in `language` — a flat list of
    /// `{name, complexity, length, parameter_count, start_line, end_line,
    /// visibility}`. Backed by `oxplow_code_metrics`' tree-sitter walker (the
    /// cyclomatic-complexity / length / param computations that the in-process
    /// code-quality producer used), exposed generically so a bundled or
    /// user-authored metric can project them. Pure → `observed`.
    fn code_metrics<'v>(
        text: &str,
        language: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let lang = oxplow_code_metrics::language_from_name(language)
            .ok_or_else(|| anyhow::anyhow!("unknown language \"{language}\""))?;
        let fns = oxplow_code_metrics::analyze_with_language("", text, lang);
        let arr: Vec<serde_json::Value> = fns
            .into_iter()
            .map(|m| {
                let visibility = match m.visibility {
                    oxplow_code_metrics::Visibility::Public => "public",
                    oxplow_code_metrics::Visibility::Private => "private",
                    oxplow_code_metrics::Visibility::Unknown => "unknown",
                };
                serde_json::json!({
                    "name": m.name,
                    "complexity": m.complexity,
                    "length": m.length,
                    "parameter_count": m.parameter_count,
                    "start_line": m.start_line,
                    "end_line": m.end_line,
                    "visibility": visibility,
                })
            })
            .collect();
        Ok(heap.alloc(serde_json::Value::Array(arr)))
    }
    /// Return the snapshot files matching `glob` as `[{path, text}]`, read from
    /// the per-run [`GaugeHost`] injected via `Evaluator::extra`. When no host is
    /// present (e.g. a report-derived run) or the host has no files, returns an
    /// empty list. A malformed glob is an error.
    fn files<'v>(
        glob: &str,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let matcher = globset::Glob::new(glob)
            .map_err(|e| anyhow::anyhow!("bad glob \"{glob}\": {e}"))?
            .compile_matcher();
        let arr: Vec<serde_json::Value> =
            match eval.extra.and_then(|e| e.downcast_ref::<GaugeHost>()) {
                Some(host) => {
                    let mut entries: Vec<(&String, &String)> = host
                        .files
                        .iter()
                        .filter(|(path, _)| matcher.is_match(path.as_str()))
                        .collect();
                    // Deterministic order so a gauge's output is reproducible.
                    entries.sort_by(|a, b| a.0.cmp(b.0));
                    entries
                        .into_iter()
                        .map(|(path, text)| serde_json::json!({ "path": path, "text": text }))
                        .collect()
                }
                None => Vec::new(),
            };
        Ok(eval.heap().alloc(serde_json::Value::Array(arr)))
    }
}

/// Per-run host state for a Starlark **gauge**, injected via `Evaluator::extra`
/// and read by the `files(glob)` builtin. Owns its file map (path → content) so
/// it carries no borrow lifetime across the Starlark boundary and is
/// `Send + 'static` (it can move into the sandbox worker thread).
#[derive(Debug, Default, starlark::any::ProvidesStaticType)]
pub struct GaugeHost {
    files: std::collections::HashMap<String, String>,
}

impl GaugeHost {
    /// A host exposing `files` (repo-relative path → UTF-8 content) to
    /// `files(glob)`.
    pub fn new(files: std::collections::HashMap<String, String>) -> Self {
        Self { files }
    }
}

fn helper_anyhow(e: crate::HelperError) -> anyhow::Error {
    anyhow::anyhow!(e.to_string())
}

/// Run a Starlark plugin against `input`. The plugin must define
/// `def transform(input): … return <object>`; the host appends a call that
/// JSON-encodes the result, so the return value crosses back as JSON. The
/// `json` stdlib extension and the container-parse helpers
/// (`parse_xml`/`parse_json`/`lcov_records`/`lines`/`regex_find`/`xpath`) are
/// available to the script.
pub fn run_starlark(script: &str, input: &Value) -> Result<Value, CollectError> {
    run_starlark_inner(script, input, None)
}

/// Like [`run_starlark`] but with a [`GaugeHost`] in scope, so the script's
/// `files(glob)` builtin can read the snapshot file map. Used by gauge
/// collectors; the host moves in by value so this stays `Send` for the sandbox.
pub fn run_starlark_with_host(
    script: &str,
    input: &Value,
    host: &GaugeHost,
) -> Result<Value, CollectError> {
    run_starlark_inner(script, input, Some(host))
}

fn run_starlark_inner(
    script: &str,
    input: &Value,
    host: Option<&GaugeHost>,
) -> Result<Value, CollectError> {
    use starlark::environment::{GlobalsBuilder, LibraryExtension, Module};
    use starlark::eval::Evaluator;
    use starlark::syntax::{AstModule, Dialect};

    // Embed the input as a JSON document, then as a JSON string *literal*
    // (serializing a String yields a quoted, escaped form that is also a valid
    // Starlark string literal), so the script gets it via json.decode.
    let input_doc =
        serde_json::to_string(input).map_err(|e| CollectError::Runtime(e.to_string()))?;
    let input_literal =
        serde_json::to_string(&input_doc).map_err(|e| CollectError::Runtime(e.to_string()))?;
    let source = format!("{script}\njson.encode(transform(json.decode({input_literal})))\n");

    let ast = AstModule::parse("plugin.star", source, &Dialect::Standard)
        .map_err(|e| CollectError::Runtime(format!("starlark parse: {e}")))?;
    let globals = GlobalsBuilder::extended_by(&[LibraryExtension::Json])
        .with(collect_helpers)
        .build();

    Module::with_temp_heap(|module| {
        let mut eval = Evaluator::new(&module);
        if let Some(host) = host {
            eval.extra = Some(host);
        }
        let result = eval
            .eval_module(ast, &globals)
            .map_err(|e| CollectError::Runtime(format!("starlark run: {e}")))?;
        let json = result.unpack_str().ok_or_else(|| {
            CollectError::Shape("starlark transform did not return a JSON string".into())
        })?;
        serde_json::from_str(json).map_err(|e| CollectError::Shape(e.to_string()))
    })
}

/// Run an external program (the escape hatch). `argv[0]` is the program; the
/// raw report `content` is written to its stdin and its stdout is parsed as the
/// kind's JSON output. Unlike the in-process tiers this can do I/O, so callers
/// tag it lower-trust.
pub fn run_exec(argv: &[String], content: &str) -> Result<Value, CollectError> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let (program, args) = argv
        .split_first()
        .ok_or_else(|| CollectError::Exec("empty exec argv".into()))?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| CollectError::Exec(format!("spawn {program}: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(content.as_bytes())
            .map_err(|e| CollectError::Exec(e.to_string()))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| CollectError::Exec(e.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CollectError::Exec(format!(
            "{program} exited with {}: {}",
            output.status,
            stderr.trim()
        )));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| CollectError::Shape(e.to_string()))
}

// ---- typed-output deserialization (one place: JSON value → kind output) ----

/// Convert a script's raw JSON output into the typed [`CollectorOutput`] for
/// `kind`. The accepted shapes mirror what oxplow already stores:
/// `{ "files": { "<path>": { "instrumented": [..], "covered": [..] } } }` for
/// coverage and `{ "suites": [ { "name", "cases": [ { "classname", "name",
/// "status", "timeMs"? } ] } ] }` for tests.
pub fn value_to_output(kind: CollectorKind, value: Value) -> Result<CollectorOutput, CollectError> {
    match kind {
        CollectorKind::Coverage => {
            let parsed: CoverageJson =
                serde_json::from_value(value).map_err(|e| CollectError::Shape(e.to_string()))?;
            Ok(CollectorOutput::Coverage(parsed.into()))
        }
        CollectorKind::Test => {
            let parsed: TestReportJson =
                serde_json::from_value(value).map_err(|e| CollectError::Shape(e.to_string()))?;
            Ok(CollectorOutput::Test(parsed.into()))
        }
        CollectorKind::Analysis => {
            let parsed: AnalysisReportJson =
                serde_json::from_value(value).map_err(|e| CollectError::Shape(e.to_string()))?;
            Ok(CollectorOutput::Analysis(parsed.into()))
        }
        CollectorKind::Gauge => {
            // `MetricReport` is its own serde-friendly shape, so it deserializes
            // directly (no separate `*Json` mirror): `samples` defaults to empty,
            // each sample's `subject`/`dims` are optional.
            let report: MetricReport =
                serde_json::from_value(value).map_err(|e| CollectError::Shape(e.to_string()))?;
            Ok(CollectorOutput::Gauge(report))
        }
    }
}

#[derive(Deserialize)]
struct CoverageJson {
    #[serde(default)]
    files: BTreeMap<String, FileCoverageJson>,
}

#[derive(Deserialize)]
struct FileCoverageJson {
    #[serde(default)]
    instrumented: Vec<u32>,
    #[serde(default)]
    covered: Vec<u32>,
}

impl From<CoverageJson> for CoverageReport {
    fn from(j: CoverageJson) -> Self {
        let files = j
            .files
            .into_iter()
            .map(|(path, fc)| {
                (
                    path,
                    FileCoverage {
                        instrumented: fc.instrumented.into_iter().collect::<BTreeSet<u32>>(),
                        covered: fc.covered.into_iter().collect::<BTreeSet<u32>>(),
                    },
                )
            })
            .collect();
        CoverageReport { files }
    }
}

#[derive(Deserialize)]
struct TestReportJson {
    #[serde(default)]
    suites: Vec<TestSuiteJson>,
}

#[derive(Deserialize)]
struct TestSuiteJson {
    #[serde(default)]
    name: String,
    #[serde(default)]
    cases: Vec<TestCaseJson>,
}

#[derive(Deserialize)]
struct TestCaseJson {
    #[serde(default)]
    classname: String,
    #[serde(default)]
    name: String,
    status: TestStatusJson,
    #[serde(default, rename = "timeMs")]
    time_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum TestStatusJson {
    Passed,
    Failed,
    Skipped,
}

impl From<TestStatusJson> for TestStatus {
    fn from(s: TestStatusJson) -> Self {
        match s {
            TestStatusJson::Passed => TestStatus::Passed,
            TestStatusJson::Failed => TestStatus::Failed,
            TestStatusJson::Skipped => TestStatus::Skipped,
        }
    }
}

#[derive(Deserialize)]
struct AnalysisReportJson {
    #[serde(default)]
    findings: Vec<AnalysisFindingJson>,
}

#[derive(Deserialize)]
struct AnalysisFindingJson {
    #[serde(default)]
    path: String,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    column: Option<u32>,
    severity: SeverityJson,
    #[serde(default)]
    rule: Option<String>,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SeverityJson {
    Error,
    Warning,
    Info,
    Note,
}

impl From<SeverityJson> for Severity {
    fn from(s: SeverityJson) -> Self {
        match s {
            SeverityJson::Error => Severity::Error,
            SeverityJson::Warning => Severity::Warning,
            SeverityJson::Info => Severity::Info,
            SeverityJson::Note => Severity::Note,
        }
    }
}

impl From<AnalysisReportJson> for AnalysisReport {
    fn from(j: AnalysisReportJson) -> Self {
        let findings = j
            .findings
            .into_iter()
            .map(|f| AnalysisFinding {
                path: f.path,
                line: f.line,
                column: f.column,
                severity: f.severity.into(),
                rule: f.rule,
                message: f.message,
            })
            .collect();
        AnalysisReport { findings }
    }
}

impl From<TestReportJson> for TestReport {
    fn from(j: TestReportJson) -> Self {
        let suites = j
            .suites
            .into_iter()
            .map(|s| TestSuite {
                name: s.name,
                cases: s
                    .cases
                    .into_iter()
                    .map(|c| TestCase {
                        classname: c.classname,
                        name: c.name,
                        status: c.status.into(),
                        time_ms: c.time_ms,
                    })
                    .collect(),
            })
            .collect();
        TestReport { suites }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jaq_maps_input_to_coverage_output() {
        // Reshape a list of {path,total,hit} into the coverage output shape.
        let input = json!([
            { "path": "src/a.rs", "lines": [1, 2], "covered": [1] }
        ]);
        let program = r#"{ files: ( reduce .[] as $f ({}; .[$f.path] = { instrumented: $f.lines, covered: $f.covered }) ) }"#;
        let out = run_jaq(program, &input).expect("jaq runs");
        let typed = value_to_output(CollectorKind::Coverage, out).expect("typed");
        let cov = typed.as_coverage().expect("coverage");
        let f = cov.files.get("src/a.rs").expect("file");
        assert!(f.instrumented.contains(&1) && f.instrumented.contains(&2));
        assert!(f.covered.contains(&1) && !f.covered.contains(&2));
    }

    #[test]
    fn jaq_errors_on_malformed_program() {
        assert!(matches!(
            run_jaq(".[", &json!({})),
            Err(CollectError::Runtime(_))
        ));
    }

    #[test]
    fn jaq_impure_builtins_are_unavailable() {
        // Determinism/trust guarantee: env/now/input must not resolve, so a
        // plugin can't read the host environment or clock (which would make
        // `observed` output non-deterministic/leaky).
        for prog in ["env", "now", "input"] {
            assert!(
                matches!(run_jaq(prog, &json!({})), Err(CollectError::Runtime(_))),
                "expected `{prog}` to be unavailable"
            );
        }
        // A pure builtin still works.
        assert_eq!(run_jaq("length", &json!([1, 2, 3])).unwrap(), json!(3));
    }

    #[test]
    fn starlark_maps_input_to_test_output() {
        let input = json!({ "names": ["t1", "t2"] });
        let script = r#"
def transform(input):
    cases = [{"classname": "m", "name": n, "status": "passed"} for n in input["names"]]
    return {"suites": [{"name": "s", "cases": cases}]}
"#;
        let out = run_starlark(script, &input).expect("starlark runs");
        let typed = value_to_output(CollectorKind::Test, out).expect("typed");
        let report = typed.as_test().expect("test");
        assert_eq!(report.suites.len(), 1);
        assert_eq!(report.suites[0].cases.len(), 2);
        assert_eq!(report.suites[0].cases[1].name, "t2");
        assert_eq!(report.suites[0].cases[0].status, TestStatus::Passed);
    }

    #[test]
    fn starlark_can_self_parse_via_host_helpers() {
        // input is raw text; the script calls the parse_xml host builtin.
        let input = json!("<cov file=\"src/a.rs\"/>");
        let script = r#"
def transform(input):
    doc = parse_xml(input)
    path = doc["attrs"]["file"]
    return {"files": {path: {"instrumented": [1, 2], "covered": [1]}}}
"#;
        let out = run_starlark(script, &input).expect("starlark runs");
        let typed = value_to_output(CollectorKind::Coverage, out).expect("typed");
        let cov = typed.as_coverage().expect("coverage");
        let f = cov.files.get("src/a.rs").expect("file");
        assert_eq!(f.instrumented.len(), 2);
        assert!(f.covered.contains(&1));
    }

    #[test]
    fn starlark_helpers_cover_lines_and_regex() {
        let input = json!("1,3\n2,0\n");
        // lines + regex_find host builtins, mapped into coverage.
        let script = r#"
def transform(input):
    instrumented = []
    covered = []
    for row in regex_find(r"(\d+),(\d+)", input):
        n = int(row[1])
        instrumented.append(n)
        if int(row[2]) > 0:
            covered.append(n)
    return {"files": {"f": {"instrumented": instrumented, "covered": covered}}}
"#;
        let out = run_starlark(script, &input).expect("starlark runs");
        let cov = value_to_output(CollectorKind::Coverage, out)
            .expect("typed")
            .as_coverage()
            .expect("coverage")
            .clone();
        let f = cov.files.get("f").expect("file");
        assert_eq!(
            f.instrumented.iter().copied().collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(f.covered.iter().copied().collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn starlark_errors_on_malformed_script() {
        assert!(matches!(
            run_starlark("def transform(", &json!({})),
            Err(CollectError::Runtime(_))
        ));
    }

    #[test]
    fn sandbox_times_out_on_slow_script() {
        // A genuinely slow jaq run with a tiny budget must surface Timeout
        // promptly rather than block the caller.
        let budget = SandboxBudget::with_timeout(Duration::from_millis(5));
        let program = "reduce range(50000000) as $x (0; . + $x)".to_string();
        let input = json!(null);
        let result = run_sandboxed(&budget, move || run_jaq(&program, &input));
        assert!(matches!(result, Err(CollectError::Timeout)));
    }

    #[test]
    fn sandbox_returns_result_when_within_budget() {
        let budget = SandboxBudget::default();
        let program = "{ files: {} }".to_string();
        let input = json!(null);
        let result = run_sandboxed(&budget, move || run_jaq(&program, &input));
        assert!(result.is_ok());
    }

    #[test]
    fn jaq_maps_input_to_analysis_output() {
        // Reshape a list of raw diagnostics into the analysis output shape.
        let input = json!([
            { "file": "src/a.rs", "ln": 12, "col": 5, "lvl": "warning", "lint": "needless_return", "msg": "x" }
        ]);
        let program = r#"{ findings: [ .[] | { path: .file, line: .ln, column: .col, severity: .lvl, rule: .lint, message: .msg } ] }"#;
        let out = run_jaq(program, &input).expect("jaq runs");
        let typed = value_to_output(CollectorKind::Analysis, out).expect("typed");
        let report = typed.as_analysis().expect("analysis");
        assert_eq!(report.findings.len(), 1);
        let f = &report.findings[0];
        assert_eq!(f.path, "src/a.rs");
        assert_eq!(f.line, Some(12));
        assert_eq!(f.column, Some(5));
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.rule.as_deref(), Some("needless_return"));
    }

    #[test]
    fn analysis_json_deserializes_with_defaults() {
        // Optional line/column/rule absent → None; missing findings → empty.
        let v = json!({ "findings": [ { "path": "x", "severity": "error", "message": "boom" } ] });
        let out = value_to_output(CollectorKind::Analysis, v).expect("typed");
        let report = out.as_analysis().unwrap();
        assert_eq!(report.findings.len(), 1);
        let f = &report.findings[0];
        assert_eq!(f.severity, Severity::Error);
        assert!(f.line.is_none() && f.column.is_none() && f.rule.is_none());

        let empty = value_to_output(CollectorKind::Analysis, json!({})).expect("typed");
        assert!(empty.as_analysis().unwrap().findings.is_empty());
    }

    #[test]
    fn gauge_report_deserializes_samples_with_defaults() {
        // Two samples: one fully populated, one bare value. Integer values
        // coerce to f64; missing subject/dims → None.
        let v = json!({ "samples": [
            { "value": 3.0, "subject": "file:src/a.rs", "dims": { "language": "rust" } },
            { "value": 1 }
        ]});
        let out = value_to_output(CollectorKind::Gauge, v).expect("typed");
        let report = out.as_gauge().expect("gauge");
        assert_eq!(report.samples.len(), 2);
        assert_eq!(report.samples[0].value, 3.0);
        assert_eq!(report.samples[0].subject.as_deref(), Some("file:src/a.rs"));
        assert_eq!(
            report.samples[0].dims.as_ref().unwrap()["language"],
            json!("rust")
        );
        assert_eq!(report.samples[1].value, 1.0);
        assert!(report.samples[1].subject.is_none() && report.samples[1].dims.is_none());

        // Missing `samples` → empty report, not an error.
        let empty = value_to_output(CollectorKind::Gauge, json!({})).expect("typed");
        assert!(empty.as_gauge().unwrap().samples.is_empty());
    }

    #[test]
    fn coverage_json_deserializes_with_defaults() {
        let v = json!({ "files": { "x": { "instrumented": [1, 2, 3] } } });
        let out = value_to_output(CollectorKind::Coverage, v).expect("typed");
        let cov = out.as_coverage().unwrap();
        let f = cov.files.get("x").unwrap();
        assert_eq!(f.instrumented.len(), 3);
        assert!(f.covered.is_empty());
    }
}
