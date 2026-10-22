//! Golden tests for oxplow-foundation's code-metric collectors: each
//! script, read from the foundation as it ships, run over a fixture corpus.

use oxplow_script::TreeHost;
use std::collections::HashMap;

/// The script oxplow-foundation's collector `id` runs.
fn script(id: &str) -> &'static str {
    let foundation = crate::bundled_extensions::find("oxplow-foundation").expect("bundled");
    let manifest: serde_yaml::Value =
        serde_yaml::from_str(foundation.file("extension.yaml").expect("manifest")).unwrap();
    let entry = manifest["collectors"]
        .as_sequence()
        .expect("collectors")
        .iter()
        .find(|c| c["id"].as_str() == Some(id))
        .and_then(|c| c["entry"].as_str())
        .unwrap_or_else(|| panic!("no foundation collector {id}"));
    foundation.file(entry).expect("its script ships")
}

/// Run a foundation collector's script by id over a fixture file map and
/// return its facts.
fn report_over(key: &str, files: HashMap<String, String>) -> Vec<oxplow_script::CollectedFact> {
    let hosts = oxplow_script::runtime::Hosts {
        tree: Some(TreeHost::new(files)),
        ..oxplow_script::runtime::Hosts::default()
    };
    oxplow_script::runtime::run_starlark_hosted(script(key), &serde_json::json!({}), &hosts)
        .and_then(oxplow_script::facts_of)
        .expect("script runs")
}

/// Run a foundation idiom collector and return its repo total — the sum of its
/// per-file facts — asserting each is a `oxplow:oxplow/file?path=<path>` fact on its path.
fn run_over(key: &str, files: HashMap<String, String>) -> f64 {
    let facts = report_over(key, files);
    for f in &facts {
        let path = f.path.as_deref().unwrap_or_default();
        assert_eq!(
            f.subject.as_deref(),
            Some(oxplow_domain::uri::build::file_uri(path).as_str()),
            "{key}: a per-file fact"
        );
    }
    facts.iter().map(|f| f.value).sum()
}

/// Every `oxplow.*` FACT the script emits, as `(measure, value,
/// language)`.
fn facts_over(key: &str, files: HashMap<String, String>) -> Vec<(String, f64, Option<String>)> {
    report_over(key, files)
        .iter()
        .map(|fc| {
            // The conformed dimension key (V43) — the scripts emit it
            // namespaced so `oxplow.language` group_by/dim_eq matches.
            let lang = fc
                .dims
                .as_ref()
                .and_then(|d| d.get("oxplow.language"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            (fc.measure.clone(), fc.value, lang)
        })
        .collect()
}

/// The `(path, value)` of every per-file fact, in emit order — the
/// attribution grain the effort rollup reads.
fn per_file_over(key: &str, files: HashMap<String, String>) -> Vec<(String, f64)> {
    report_over(key, files)
        .iter()
        .filter_map(|f| f.path.clone().map(|p| (p, f.value)))
        .collect()
}

fn corpus() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert(
        "src/a.rs".to_string(),
        r#"
// TODO: clean this up
fn a() {
unsafe { foo(); }
let x = maybe().unwrap();
let y = maybe().expect("nope");
if x { panic!("boom"); }
}
fn b() {
unsafe { bar(); }
todo!();
std::panic!("path-qualified macro counts too");
}
"#
        .to_string(),
    );
    m.insert(
        "src/b.rs".to_string(),
        "// FIXME later\nfn c() { let s = \"a TODO in a string is ignored\"; }\n".to_string(),
    );
    // A non-Rust file the glob must skip.
    m.insert(
        "README.md".to_string(),
        "unsafe { not code } panic!()".to_string(),
    );
    m
}

#[test]
fn rust_unsafe_blocks_golden() {
    assert_eq!(run_over("oxplow.rust.unsafe_blocks", corpus()), 2.0);
}

#[test]
fn per_file_breakdown_attributes_to_paths() {
    // unsafe_blocks: src/a.rs has 2 unsafe blocks, src/b.rs has 0 (omitted —
    // sparse), README.md is skipped by the glob → one file:* sample.
    assert_eq!(
        per_file_over("oxplow.rust.unsafe_blocks", corpus()),
        vec![("src/a.rs".to_string(), 2.0)]
    );
}

#[test]
fn per_language_gauges_emit_rule_tagged_ast_hit_facts() {
    // tsk30: each per-language idiom script emits per-file `oxplow.ast_hit`
    // facts tagged with its rule (so the Sum(ast_hit)-by-rule spec is the
    // headline).
    let facts = report_over("oxplow.rust.unsafe_blocks", corpus());
    assert!(!facts.is_empty(), "emits ast_hit facts");
    assert!(
        facts
            .iter()
            .all(|f| f.measure == "oxplow.ast_hit" && f.rule.as_deref() == Some("unsafe_block")),
        "every fact is on oxplow.ast_hit tagged rule=unsafe_block"
    );
    assert_eq!(facts.iter().map(|f| f.value).sum::<f64>(), 2.0);
}

#[test]
fn rust_unwrap_expect_golden() {
    assert_eq!(run_over("oxplow.rust.unwrap_expect_calls", corpus()), 2.0);
}

#[test]
fn rust_panic_macros_golden() {
    // panic! + todo! + path-qualified std::panic! = 3 (the scoped form is
    // counted via the scoped_identifier pattern).
    assert_eq!(run_over("oxplow.rust.panic_macros", corpus()), 3.0);
}

fn ts_corpus() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert(
        "src/a.ts".to_string(),
        r#"
// @ts-ignore
function f(x: any): any {
console.log(x);
window.console.error(x);
const y = x!.foo;
return y;
}
const g = (a: any) => a!;
"#
        .to_string(),
    );
    m.insert(
        "src/b.tsx".to_string(),
        "// @ts-expect-error\nexport const C = () => { console.warn('x'); return null; };\n"
            .to_string(),
    );
    // Non-TS file the globs must skip.
    m.insert(
        "notes.md".to_string(),
        "any! console.log @ts-ignore".to_string(),
    );
    m
}

#[test]
fn ts_any_usage_golden() {
    // a.ts: `x: any`, `: any` return, `a: any` = 3.
    assert_eq!(run_over("oxplow.ts.any_usage", ts_corpus()), 3.0);
}

#[test]
fn ts_non_null_assertions_golden() {
    // a.ts: `x!`, `a!`; = 2 (the tsx file has none).
    assert_eq!(run_over("oxplow.ts.non_null_assertions", ts_corpus()), 2.0);
}

#[test]
fn ts_console_calls_golden() {
    // console.log + namespaced window.console.error (a.ts) + console.warn
    // (b.tsx) = 3.
    assert_eq!(run_over("oxplow.ts.console_calls", ts_corpus()), 3.0);
}

#[test]
fn ts_ts_ignore_golden() {
    // @ts-ignore (a.ts) + @ts-expect-error (b.tsx) = 2; the markdown is skipped.
    assert_eq!(run_over("oxplow.ts.ts_ignore", ts_corpus()), 2.0);
}

fn clj_corpus() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert(
        "src/core.clj".to_string(),
        // The last form binds a local literally named `defn` and references
        // it — neither is a definition, so the head-anchored query must NOT
        // count them (the old `(sym_lit)` query would have).
        ";; TODO: refactor\n(defn add [a b] (+ a b))\n(defn- helper [] :ok)\n(def x 1)\n(let [defn 1] defn)\n"
            .to_string(),
    );
    m.insert(
        "src/util.cljs".to_string(),
        "(defn greet [n] (str \"hi \" n)) ; FIXME i18n\n".to_string(),
    );
    m
}

#[test]
fn doc_coverage_golden() {
    // tsk125: one per-file RATIO fact — documented public ÷ public. a() is a
    // documented public fn, b() an undocumented public fn, c() is private
    // (excluded). → 1/2 = 50%.
    let mut files = HashMap::new();
    files.insert(
        "src/a.rs".to_string(),
        "/// documented\npub fn a() {}\npub fn b() {}\nfn c() {}\n".to_string(),
    );
    let facts = report_over("oxplow.doc_coverage", files);
    assert_eq!(facts.len(), 1, "one per-file doc-coverage fact");
    let f = &facts[0];
    assert_eq!(f.measure, "oxplow.doc_coverage");
    assert_eq!(f.num, Some(1.0), "1 documented public");
    assert_eq!(f.den, Some(2.0), "2 public");
    assert!((f.value - 50.0).abs() < 0.01, "value {}", f.value);
    assert_eq!(f.path.as_deref(), Some("src/a.rs"));
}

#[test]
fn clojure_defn_count_golden() {
    // add (defn) + helper (defn-) in core.clj + greet (defn) in util.cljs = 3.
    // The `(let [defn 1] defn)` form's two `defn` symbols are NOT defs (not in
    // head position) and must not inflate the count.
    assert_eq!(run_over("oxplow.clojure.defn_count", clj_corpus()), 3.0);
}

fn metrics_corpus() -> HashMap<String, String> {
    // `complex`: 11 `if` branches → cyclomatic complexity 12 (> 10).
    let mut complex = String::from("fn complex(x: i32) -> i32 {\n");
    for i in 0..11 {
        complex.push_str(&format!("    if x == {i} {{ return {i}; }}\n"));
    }
    complex.push_str("    0\n}\n");
    // `big`: 65-statement body → length > 60. Low complexity.
    let mut big = String::from("fn big() {\n");
    for i in 0..65 {
        big.push_str(&format!("    let v{i} = {i};\n"));
    }
    big.push_str("}\n");
    let mut m = HashMap::new();
    m.insert("src/c.rs".to_string(), format!("{complex}{big}"));
    m
}

/// A mixed-language corpus exercising the language-agnostic code metrics:
/// a high-complexity + long Rust fn, a TS function with a TODO, and a
/// Clojure def with a FIXME. A non-source file is skipped by `source_files`.
fn mixed_corpus() -> HashMap<String, String> {
    let mut m = metrics_corpus(); // src/c.rs: `complex` (cc 12) + `big` (long)
    m.insert(
        "src/a.ts".to_string(),
        "// TODO wire this up\nfunction f(x: number) { return x; }\n".to_string(),
    );
    m.insert(
        "src/core.clj".to_string(),
        "; FIXME naming\n(defn g [] :ok)\n".to_string(),
    );
    m.insert("README.md".to_string(), "TODO not code\n".to_string());
    m
}

#[test]
fn unified_high_complexity_fns_across_languages() {
    // Only the Rust `complex` (cc 12) exceeds 10 across the whole corpus. The
    // code gauges are facts-only now (unbaked, T-C3b) — the metric total is a
    // count-over-threshold of the per-function `oxplow.complexity` facts.
    let complexity = facts_over("oxplow.high_complexity_fns", mixed_corpus());
    assert_eq!(complexity.iter().filter(|(_, v, _)| *v > 10.0).count(), 1);
}

#[test]
fn unified_long_functions_across_languages() {
    // Only the Rust `big` (>60 lines) is long — count-over-threshold of the
    // per-function `oxplow.fn_length` facts (facts-only gauge, T-C3b).
    let lengths = facts_over("oxplow.long_functions", mixed_corpus());
    assert_eq!(lengths.iter().filter(|(_, v, _)| *v > 60.0).count(), 1);
}

#[test]
fn code_gauges_emit_measure_bound_facts_for_every_item() {
    // The inverted substrate (epic tsk12): each code gauge emits a durable
    // per-item FACT on its measure for EVERY function/marker — not just the
    // offenders the baked count reports — so a spec can re-threshold. 4
    // functions across the corpus (rust complex+big, ts f, clj g); 2 markers.
    let complexity = facts_over("oxplow.high_complexity_fns", mixed_corpus());
    assert_eq!(complexity.len(), 4, "one complexity fact per function");
    assert!(complexity.iter().all(|(m, _, _)| m == "oxplow.complexity"));
    // Every fact is language-tagged, and one function (rust `complex`) is >10.
    assert!(complexity.iter().all(|(_, _, lang)| lang.is_some()));
    assert_eq!(
        complexity.iter().filter(|(_, v, _)| *v > 10.0).count(),
        1,
        "the baked high_complexity count is recoverable from the facts"
    );

    let lengths = facts_over("oxplow.long_functions", mixed_corpus());
    assert_eq!(lengths.len(), 4, "one fn_length fact per function");
    assert!(lengths.iter().all(|(m, _, _)| m == "oxplow.fn_length"));
    assert_eq!(lengths.iter().filter(|(_, v, _)| *v > 60.0).count(), 1);

    let params = facts_over("oxplow.fn_count", mixed_corpus());
    assert_eq!(params.len(), 4, "one parameter_count fact per function");
    assert!(params.iter().all(|(m, _, _)| m == "oxplow.parameter_count"));

    let todos = facts_over("oxplow.todos", mixed_corpus());
    assert_eq!(
        todos.len(),
        2,
        "one todo fact per marker (ts TODO + clj FIXME)"
    );
    assert!(todos
        .iter()
        .all(|(m, v, _)| m == "oxplow.todo" && *v == 1.0));
}

#[test]
fn unified_fn_count_across_languages() {
    // rust complex + big (2) + ts f (1) + clojure g (1) = 4. README skipped.
    // Facts-only gauge (T-C3b): the total is a count of `oxplow.parameter_count`
    // facts (one per function).
    assert_eq!(facts_over("oxplow.fn_count", mixed_corpus()).len(), 4);
}

#[test]
fn unified_todos_across_languages() {
    // TS TODO + Clojure FIXME = 2 (comment-scoped); README's "TODO" is not a
    // source file → skipped by source_files(). Facts-only gauge (T-C3b): the
    // total is a count of per-marker `oxplow.todo` facts.
    assert_eq!(facts_over("oxplow.todos", mixed_corpus()).len(), 2);
}

fn cs_corpus() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert(
        "src/Service.cs".to_string(),
        r#"
namespace Acme {
class Service {
    public void Run(int x) {
        try { Work(); } catch (System.Exception) { }
        var r = FetchAsync().Result;
        _task.Wait();
        System.Action w = _task.Wait; // method-group ref, NOT a blocking call
    }
    async void Background() { await Task.Delay(1); }
}
}
"#
        .to_string(),
    );
    m.insert(
        "src/Util.cs".to_string(),
        "class Util {\n    static void Noop() { try { } catch { } }\n}\n".to_string(),
    );
    // A non-C# file the glob must skip.
    m.insert(
        "README.md".to_string(),
        ".Result .Wait() catch { }".to_string(),
    );
    m
}

#[test]
fn csharp_empty_catch_golden() {
    // Service.cs: `catch (System.Exception) { }` (1); Util.cs: `catch { }`
    // (1) = 2. The non-empty catch (if any) and the markdown are excluded.
    assert_eq!(run_over("oxplow.csharp.empty_catch", cs_corpus()), 2.0);
}

#[test]
fn csharp_blocking_async_calls_golden() {
    // `.Result` + invoked `.Wait()` in Service.cs = 2; the non-invoked
    // `.Wait` method-group reference and the markdown are NOT counted.
    assert_eq!(
        run_over("oxplow.csharp.blocking_async_calls", cs_corpus()),
        2.0
    );
}

/// Two files sharing a ten-line function body, and one that shares
/// nothing.
fn dup_corpus() -> HashMap<String, String> {
    let body = "pub fn compute(input: &[i64]) -> i64 {\n\
        \x20   let mut total = 0;\n\
        \x20   for value in input {\n\
        \x20       if *value > 0 {\n\
        \x20           total += *value;\n\
        \x20       } else {\n\
        \x20           total -= *value;\n\
        \x20       }\n\
        \x20   }\n\
        \x20   total * 2 + 1\n\
        }\n";
    HashMap::from([
        ("src/a.rs".to_string(), body.to_string()),
        ("src/b.rs".to_string(), format!("// a copy\n{body}")),
        ("src/c.rs".to_string(), "fn other() {}\n".to_string()),
    ])
}

#[test]
fn duplicate_lines_restates_both_sides_of_every_duplicate_block() {
    let files = dup_corpus();
    let mut want: Vec<(String, f64, Option<String>, Option<i64>)> =
        oxplow_code_dup::detect_duplicates(files.clone(), oxplow_code_dup::DupOptions::default())
            .into_iter()
            .flat_map(|b| {
                [
                    (b.a_path, b.a_start_line, b.a_end_line),
                    (b.b_path, b.b_start_line, b.b_end_line),
                ]
                .map(|(path, start, end)| {
                    (
                        "oxplow.duplicate_lines".to_string(),
                        b.line_count as f64,
                        Some(oxplow_domain::uri::build::block_uri(
                            &path,
                            &format!("{start}-{end}"),
                        )),
                        Some(start as i64),
                    )
                })
            })
            .collect();
    assert!(!want.is_empty(), "the corpus has a duplicate");
    let mut got: Vec<(String, f64, Option<String>, Option<i64>)> =
        report_over("oxplow.duplicate_lines", files)
            .into_iter()
            .map(|f| (f.measure, f.value, f.subject, f.line))
            .collect();
    want.sort_by(|a, b| a.2.cmp(&b.2));
    got.sort_by(|a, b| a.2.cmp(&b.2));
    assert_eq!(got, want);
}

// --- an effort's test outcome, from the runs it recorded -------------------

/// One test run an effort recorded, as the run recorder writes it: a
/// `tests` capture (a `test-run` one is a record without measurements) with
/// its detail and an `oxplow.test_case` fact per case it reported, or per
/// asserted failure when it reported counts only.
pub(crate) struct SeedRun {
    pub at_ms: i64,
    pub producer: &'static str,
    /// `(name, status)` of each case the run's report listed.
    pub cases: Vec<(&'static str, &'static str)>,
    /// Failures asserted as a count, with no report.
    pub asserted_failed: i64,
}

impl SeedRun {
    pub fn report(at_ms: i64, cases: &[(&'static str, &'static str)]) -> Self {
        Self {
            at_ms,
            producer: "tests",
            cases: cases.to_vec(),
            asserted_failed: 0,
        }
    }

    pub fn asserted(at_ms: i64, failed: i64) -> Self {
        Self {
            at_ms,
            producer: "tests",
            cases: Vec::new(),
            asserted_failed: failed,
        }
    }
}

/// Record `runs` for `effort` the way a run recorder does.
pub(crate) async fn seed_test_runs(
    facts: &oxplow_db::SqliteFactStore,
    stream: i64,
    thread: i64,
    effort: i64,
    branch: &str,
    runs: &[SeedRun],
) {
    let case = facts
        .get_measure("oxplow.test_case")
        .await
        .unwrap()
        .expect("the test_case measure");
    for run in runs {
        let failed = run
            .cases
            .iter()
            .filter(|(_, status)| *status == "failed")
            .count() as i64
            + run.asserted_failed;
        let suites: Vec<_> = if run.cases.is_empty() {
            Vec::new()
        } else {
            vec![serde_json::json!({
                "name": "suite",
                "cases": run.cases.iter().map(|(name, status)| serde_json::json!({
                    "classname": "mod", "name": name, "status": status
                })).collect::<Vec<_>>()
            })]
        };
        let mut capture = oxplow_db::NewMetricCapture::done(stream, run.producer, "hook");
        capture.thread_id = Some(thread);
        capture.effort_id = Some(effort);
        capture.branch = Some(branch.to_string());
        capture.captured_at = Some(oxplow_domain::Timestamp::from_unix_ms(run.at_ms));
        capture.detail_json = Some(
            serde_json::json!({
                "kind": "test-detail",
                "payload": { "command": "cargo test", "failed": failed, "suites": suites }
            })
            .to_string(),
        );
        let mut rows: Vec<oxplow_db::NewFact> = run
            .cases
            .iter()
            .map(|(name, status)| oxplow_db::NewFact {
                subject: Some(oxplow_domain::uri::build::test_uri("mod", name)),
                dims_json: Some(format!(r#"{{"oxplow.status":"{status}"}}"#)),
                ..oxplow_db::NewFact::new(case.id, 1.0)
            })
            .collect();
        for _ in 0..run.asserted_failed {
            rows.push(oxplow_db::NewFact {
                dims_json: Some(r#"{"oxplow.status":"failed"}"#.to_string()),
                ..oxplow_db::NewFact::new(case.id, 1.0)
            });
        }
        // A record-only run measures nothing.
        if run.producer != "tests" {
            rows.clear();
        }
        facts.record_facts(capture, rows).await.unwrap();
    }
}

/// `(measure, subject, dims, value, numerator, denominator)`.
pub(crate) type OutcomeFact = (
    String,
    Option<String>,
    Option<String>,
    f64,
    Option<f64>,
    Option<f64>,
);

/// What an effort's outcome facts say, in a form to compare:
/// `(measure, subject, dims, value, numerator, denominator)`, sorted.
pub(crate) async fn outcome_facts(facts: &oxplow_db::SqliteFactStore) -> Vec<OutcomeFact> {
    let mut out = Vec::new();
    for key in ["oxplow.effort_test_outcome", "oxplow.effort_time_to_green"] {
        let measure = facts.get_measure(key).await.unwrap().expect("measure");
        for f in facts.facts_for_measure(measure.id).await.unwrap() {
            out.push((
                key.to_string(),
                f.subject,
                f.dims_json,
                f.value,
                f.numerator,
                f.denominator,
            ));
        }
    }
    out.sort_by(|a, b| (&a.0, &a.2).cmp(&(&b.0, &b.2)));
    out
}

/// A red run, a record-only run (ignored), a red-then-recovering run and a
/// green one: the outcome facts both the old producer and the collector
/// must write for it, for the effort ref `effort`.
pub(crate) fn expected_outcome(
    effort: &str,
    stats: [i64; 4],
    time_to_green: Option<i64>,
) -> Vec<OutcomeFact> {
    let subject = Some(effort.to_string());
    let mut want: Vec<_> = ["at_close", "distinct_failed", "peak", "red_runs"]
        .into_iter()
        .zip([stats[0], stats[2], stats[1], stats[3]])
        .map(|(stat, value)| {
            (
                "oxplow.effort_test_outcome".to_string(),
                subject.clone(),
                Some(format!(r#"{{"oxplow.tests_stat":"{stat}"}}"#)),
                value as f64,
                Some(value as f64),
                Some(1.0),
            )
        })
        .collect();
    if let Some(ms) = time_to_green {
        want.push((
            "oxplow.effort_time_to_green".to_string(),
            subject,
            None,
            ms as f64,
            Some(ms as f64),
            Some(1.0),
        ));
    }
    want.sort_by(|a, b| (&a.0, &a.2).cmp(&(&b.0, &b.2)));
    want
}

/// The runs the equivalence tests share: red twice (a and b, then a alone), a
/// record-only run that must not count, and a green one 60.25 s after the
/// first red.
pub(crate) fn red_then_green_runs() -> Vec<SeedRun> {
    let mut record_only =
        SeedRun::report(20_000, &[("a", "failed"), ("b", "failed"), ("c", "failed")]);
    record_only.producer = "test-run";
    vec![
        SeedRun::report(1_000, &[("a", "failed"), ("b", "failed"), ("c", "passed")]),
        record_only,
        SeedRun::report(31_500, &[("a", "failed"), ("b", "passed")]),
        SeedRun::report(61_250, &[("a", "passed")]),
    ]
}

/// An asserted-count run that fails three, then one that recovers.
pub(crate) fn asserted_runs() -> Vec<SeedRun> {
    vec![SeedRun::asserted(5_000, 3), SeedRun::asserted(9_000, 0)]
}

/// Run the test-outcome collector's script over a seeded effort's rows, as its
/// `input` yields them — `(run, failed, cases, at_ms, case)` — and read the
/// facts back as `(measure, value, stat)`.
fn outcome_over(rows: &[(i64, i64, i64, i64, Option<&str>)]) -> Vec<(String, f64, Option<String>)> {
    let rows: Vec<serde_json::Value> = rows
        .iter()
        .map(|(run, failed, cases, at_ms, case)| {
            serde_json::json!({
                "run": run, "failed": failed, "cases": cases, "at_ms": at_ms,
                "case": case, "effort": "eff1"
            })
        })
        .collect();
    oxplow_script::runtime::run_starlark_hosted(
        script("oxplow.effort_test_outcome"),
        &serde_json::json!({ "rows": rows }),
        &oxplow_script::runtime::Hosts::default(),
    )
    .and_then(oxplow_script::facts_of)
    .expect("script runs")
    .into_iter()
    .map(|f| {
        assert_eq!(f.subject.as_deref(), Some("oxplow:oxplow/effort?id=eff1"));
        assert_eq!(f.den, Some(1.0), "non-additive, denominator 1");
        assert_eq!(f.num, Some(f.value));
        let stat = f
            .dims
            .as_ref()
            .and_then(|d| d.get("oxplow.tests_stat"))
            .and_then(|v| v.as_str())
            .map(String::from);
        (f.measure, f.value, stat)
    })
    .collect()
}

fn outcome(stats: [f64; 4]) -> Vec<(String, f64, Option<String>)> {
    ["at_close", "peak", "distinct_failed", "red_runs"]
        .into_iter()
        .zip(stats)
        .map(|(stat, v)| {
            (
                "oxplow.effort_test_outcome".to_string(),
                v,
                Some(stat.to_string()),
            )
        })
        .collect()
}

fn time_to_green(ms: f64) -> (String, f64, Option<String>) {
    ("oxplow.effort_time_to_green".to_string(), ms, None)
}

#[test]
fn no_runs_means_no_facts() {
    assert_eq!(outcome_over(&[]), vec![]);
}

#[test]
fn a_single_green_run_is_all_zeros() {
    assert_eq!(outcome_over(&[(1, 0, 3, 100, None)]), outcome([0.0; 4]));
}

#[test]
fn red_then_green_closes_clean_but_went_red() {
    let mut want = outcome([0.0, 2.0, 2.0, 1.0]);
    want.push(time_to_green(300.0));
    assert_eq!(
        outcome_over(&[
            (1, 2, 2, 100, Some("oxplow:oxplow/test?class=mod&name=a")),
            (1, 2, 2, 100, Some("oxplow:oxplow/test?class=mod&name=b")),
            (2, 0, 2, 400, None),
        ]),
        want
    );
}

#[test]
fn the_same_test_red_twice_is_one_distinct_test() {
    assert_eq!(
        outcome_over(&[
            (1, 1, 1, 100, Some("oxplow:oxplow/test?class=mod&name=a")),
            (2, 1, 1, 200, Some("oxplow:oxplow/test?class=mod&name=a")),
        ]),
        outcome([1.0, 1.0, 1.0, 2.0])
    );
    assert_eq!(
        outcome_over(&[
            (1, 1, 1, 100, Some("oxplow:oxplow/test?class=mod&name=a")),
            (2, 1, 1, 200, Some("oxplow:oxplow/test?class=mod&name=b")),
        ]),
        outcome([1.0, 1.0, 2.0, 2.0])
    );
}

#[test]
fn asserted_failures_have_no_identity_so_distinct_falls_back_to_the_peak() {
    // Three failures asserted as a count, no cases listed.
    assert_eq!(
        outcome_over(&[(1, 3, 0, 100, None)]),
        outcome([3.0, 3.0, 3.0, 1.0])
    );
    // One known and one anonymous in a run is two, not one.
    let want = outcome_over(&[
        (1, 2, 1, 100, Some("oxplow:oxplow/test?class=mod&name=a")),
        (2, 0, 1, 200, None),
    ]);
    assert_eq!(
        want[1].1, 1.0,
        "a report that lists its failures needs no fallback"
    );
}

#[test]
fn a_run_with_no_case_rows_still_counts() {
    // Run 1 failed a; run 2 a green repeat that listed no failed case.
    let mut want = outcome([0.0, 1.0, 1.0, 1.0]);
    want.push(time_to_green(5.0));
    assert_eq!(
        outcome_over(&[
            (1, 1, 2, 100, Some("oxplow:oxplow/test?class=mod&name=a")),
            (2, 0, 0, 105, None)
        ]),
        want
    );
}

#[test]
fn time_to_green_is_the_first_red_to_the_first_green_after_it() {
    // green @0, red @100, red @250, green @400 → 300, whatever follows.
    let mut want = outcome([1.0, 1.0, 2.0, 3.0]);
    want.push(time_to_green(300.0));
    assert_eq!(
        outcome_over(&[
            (1, 0, 1, 0, None),
            (2, 1, 1, 100, Some("oxplow:oxplow/test?class=mod&name=a")),
            (3, 1, 1, 250, Some("oxplow:oxplow/test?class=mod&name=a")),
            (4, 0, 1, 400, None),
            (5, 1, 1, 500, Some("oxplow:oxplow/test?class=mod&name=b")),
        ]),
        want
    );
}

#[test]
fn time_to_green_needs_a_red_then_a_green() {
    let only_outcome = |rows: &[(i64, i64, i64, i64, Option<&str>)]| {
        outcome_over(rows)
            .into_iter()
            .filter(|(m, ..)| m == "oxplow.effort_time_to_green")
            .count()
    };
    assert_eq!(
        only_outcome(&[(1, 0, 1, 5, None), (2, 0, 1, 9, None)]),
        0,
        "never red"
    );
    assert_eq!(
        only_outcome(&[
            (1, 1, 1, 5, Some("oxplow:oxplow/test?class=mod&name=a")),
            (2, 1, 1, 9, Some("oxplow:oxplow/test?class=mod&name=a"))
        ]),
        0,
        "never recovered"
    );
}
