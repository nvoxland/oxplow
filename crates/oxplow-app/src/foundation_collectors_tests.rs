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
/// per-file facts — asserting each is a `file:<path>` fact on its path.
fn run_over(key: &str, files: HashMap<String, String>) -> f64 {
    let facts = report_over(key, files);
    for f in &facts {
        let path = f.path.as_deref().unwrap_or_default();
        assert_eq!(
            f.subject.as_deref(),
            Some(format!("file:{path}").as_str()),
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
                        Some(format!("block:{path}:{start}-{end}")),
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
