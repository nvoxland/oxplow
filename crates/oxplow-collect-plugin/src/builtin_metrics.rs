//! Bundled built-in metric catalog (epic tsk213, P3b). Every code/language
//! metric oxplow ships is authored through the **public** capability surface
//! (`files()` / `ast_query()` + the gauge `MetricReport` shape) and embedded
//! here — never a privileged Rust path. A project enables one with
//! `metrics: - use: oxplow.<lang>.<name>`; the runner resolves it at `built-in`
//! scope and runs it from the embedded script below (no project-disk file).
//!
//! These are the reference implementations a user copies. Each is exercised by a
//! golden test over a fixture corpus (see the tests module).

use crate::{Collector, CollectorInput, CollectorKind};

/// One bundled metric: its catalog metadata + the embedded script that computes
/// it. `key` is reserved under the `oxplow.` namespace.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinMetric {
    pub key: &'static str,
    pub kind: &'static str,
    pub title: &'static str,
    /// One-line description of what the metric measures (shown atop the Metric
    /// Detail page).
    pub description: &'static str,
    pub unit: &'static str,
    pub direction: &'static str,
    pub grain: &'static str,
    pub language: &'static str,
    pub dimensions: &'static [&'static str],
    pub target: Option<f64>,
    pub trigger: &'static str,
    pub runtime: &'static str,
    pub input: &'static str,
    pub script: &'static str,
}

impl BuiltinMetric {
    fn collector_input(&self) -> CollectorInput {
        match self.input {
            "json" => CollectorInput::Json,
            "xml" => CollectorInput::Xml,
            "lcov" => CollectorInput::Lcov,
            "lines" => CollectorInput::Lines,
            _ => CollectorInput::Text,
        }
    }

    /// Build the gauge collector for this metric from its embedded script. The
    /// runner calls this for `built-in`-scoped metrics instead of reading a
    /// project-disk `entryFile`.
    pub fn collector(&self) -> Collector {
        let input = self.collector_input();
        match self.runtime {
            "jaq" => Collector::jaq(
                self.key,
                CollectorKind::Gauge,
                [self.key],
                input,
                self.script,
            ),
            // starlark (default)
            _ => Collector::starlark(
                self.key,
                CollectorKind::Gauge,
                [self.key],
                input,
                self.script,
            ),
        }
    }
}

const RUST: &[BuiltinMetric] = &[
    BuiltinMetric {
        key: "oxplow.rust.unsafe_blocks",
        kind: "gauge",
        title: "unsafe blocks",
        description: "Count of `unsafe` blocks in the codebase.",
        unit: "count",
        direction: "lower-better",
        grain: "tree",
        language: "rust",
        dimensions: &["language", "git_version"],
        target: Some(0.0),
        trigger: "on-snapshot",
        runtime: "starlark",
        input: "text",
        script: include_str!("plugins/metrics/rust/unsafe_blocks.star"),
    },
    BuiltinMetric {
        key: "oxplow.rust.unwrap_expect_calls",
        kind: "gauge",
        title: "unwrap / expect calls",
        description: "Calls to `.unwrap()` / `.expect()` that can panic at runtime.",
        unit: "count",
        direction: "lower-better",
        grain: "tree",
        language: "rust",
        dimensions: &["language", "git_version"],
        target: None,
        trigger: "on-snapshot",
        runtime: "starlark",
        input: "text",
        script: include_str!("plugins/metrics/rust/unwrap_expect_calls.star"),
    },
    BuiltinMetric {
        key: "oxplow.rust.panic_macros",
        kind: "gauge",
        title: "panic-family macros",
        description: "Uses of `panic!` / `unreachable!` / `todo!` and similar panic macros.",
        unit: "count",
        direction: "lower-better",
        grain: "tree",
        language: "rust",
        dimensions: &["language", "git_version"],
        target: None,
        trigger: "on-snapshot",
        runtime: "starlark",
        input: "text",
        script: include_str!("plugins/metrics/rust/panic_macros.star"),
    },
];

/// Language-agnostic code metrics (tsk314): one metric per concept, driven by
/// the per-language capability layer (`source_files()` + `code_metrics()` /
/// `markers()`). `language: ""` → no single language (the seeded definition's
/// language is NULL; samples carry the per-file `language` dim). These replace
/// the old per-language todo/complexity/fn-count/long-function gauges.
const CODE: &[BuiltinMetric] = &[
    code_gauge(
        "oxplow.todos",
        "TODO / FIXME markers",
        "TODO/FIXME/HACK/XXX/BUG markers in comments, across all languages.",
        "lower-better",
        include_str!("plugins/metrics/code/todos.star"),
    ),
    code_gauge(
        "oxplow.fn_count",
        "function count",
        "Total functions / methods defined, across all languages.",
        "neutral",
        include_str!("plugins/metrics/code/fn_count.star"),
    ),
    code_gauge(
        "oxplow.high_complexity_fns",
        "high-complexity functions",
        "Functions whose cyclomatic complexity exceeds the threshold, across all languages.",
        "lower-better",
        include_str!("plugins/metrics/code/high_complexity_fns.star"),
    ),
    code_gauge(
        "oxplow.long_functions",
        "long functions (>60 lines)",
        "Functions longer than 60 lines, across all languages.",
        "lower-better",
        include_str!("plugins/metrics/code/long_functions.star"),
    ),
];

/// A language-agnostic tree gauge (the unified code metrics). Like `ast_gauge`
/// but `language: ""` (no single language — it sweeps `source_files()` itself).
const fn code_gauge(
    key: &'static str,
    title: &'static str,
    description: &'static str,
    direction: &'static str,
    script: &'static str,
) -> BuiltinMetric {
    BuiltinMetric {
        key,
        kind: "gauge",
        title,
        description,
        unit: "count",
        direction,
        grain: "tree",
        language: "",
        dimensions: &["language", "git_version"],
        target: None,
        trigger: "on-snapshot",
        runtime: "starlark",
        input: "text",
        script,
    }
}

/// A `gauge`/`tree`/`on-snapshot`/`starlark`/`text` metric (the common shape for
/// a tree-derived AST scan), so each per-language entry stays terse.
const fn ast_gauge(
    key: &'static str,
    title: &'static str,
    description: &'static str,
    direction: &'static str,
    language: &'static str,
    target: Option<f64>,
    script: &'static str,
) -> BuiltinMetric {
    BuiltinMetric {
        key,
        kind: "gauge",
        title,
        description,
        unit: "count",
        direction,
        grain: "tree",
        language,
        dimensions: &["language", "git_version"],
        target,
        trigger: "on-snapshot",
        runtime: "starlark",
        input: "text",
        script,
    }
}

const TS: &[BuiltinMetric] = &[
    ast_gauge(
        "oxplow.ts.any_usage",
        "any usage",
        "Uses of the `any` type.",
        "lower-better",
        "typescript",
        None,
        include_str!("plugins/metrics/ts/any_usage.star"),
    ),
    ast_gauge(
        "oxplow.ts.non_null_assertions",
        "non-null assertions",
        "Non-null assertions (`!`).",
        "lower-better",
        "typescript",
        None,
        include_str!("plugins/metrics/ts/non_null_assertions.star"),
    ),
    ast_gauge(
        "oxplow.ts.console_calls",
        "console.* calls",
        "Calls to `console.*`.",
        "lower-better",
        "typescript",
        None,
        include_str!("plugins/metrics/ts/console_calls.star"),
    ),
    ast_gauge(
        "oxplow.ts.ts_ignore",
        "ts-ignore / ts-expect-error",
        "`@ts-ignore` / `@ts-expect-error` suppressions.",
        "lower-better",
        "typescript",
        None,
        include_str!("plugins/metrics/ts/ts_ignore.star"),
    ),
];

const CLOJURE: &[BuiltinMetric] = &[ast_gauge(
    "oxplow.clojure.defn_count",
    "defn count",
    "Number of `defn` definitions.",
    "neutral",
    "clojure",
    None,
    include_str!("plugins/metrics/clojure/defn_count.star"),
)];

const CSHARP: &[BuiltinMetric] = &[
    ast_gauge(
        "oxplow.csharp.empty_catch",
        "empty catch blocks",
        "Empty `catch` blocks that swallow exceptions.",
        "lower-better",
        "csharp",
        None,
        include_str!("plugins/metrics/csharp/empty_catch.star"),
    ),
    ast_gauge(
        "oxplow.csharp.blocking_async_calls",
        "blocking async calls (.Result / .Wait())",
        "Blocking calls on async code (`.Result` / `.Wait()`).",
        "lower-better",
        "csharp",
        None,
        include_str!("plugins/metrics/csharp/blocking_async_calls.star"),
    ),
];

/// Every bundled built-in metric: language-idiom metrics per language, plus the
/// language-agnostic code metrics (`CODE`).
pub fn builtin_metrics() -> Vec<BuiltinMetric> {
    [RUST, TS, CLOJURE, CSHARP, CODE].concat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GaugeHost;
    use std::collections::HashMap;

    /// Run a built-in metric by key over a fixture file map and return its
    /// gauge report (headline `tree:.` total + sparse `file:<path>` per-file
    /// breakdown — the grain split from metrics.md).
    fn report_over(key: &str, files: HashMap<String, String>) -> crate::MetricReport {
        let metric = builtin_metrics()
            .into_iter()
            .find(|m| m.key == key)
            .unwrap_or_else(|| panic!("no builtin metric {key}"));
        let out = metric
            .collector()
            .run_gauge("", GaugeHost::new(files))
            .expect("gauge runs");
        out.as_gauge().expect("gauge output").clone()
    }

    /// Run a built-in metric and return the repo-total (the `tree:.` sample's
    /// value), asserting the invariant that the per-file (`file:<path>`)
    /// breakdown sums exactly to it and that no sample carries a stray subject.
    fn run_over(key: &str, files: HashMap<String, String>) -> f64 {
        let report = report_over(key, files);
        let totals = report
            .samples
            .iter()
            .filter(|s| s.subject.as_deref() == Some("tree:."))
            .count();
        assert_eq!(totals, 1, "{key} must project exactly one tree:. total");
        let total = report
            .samples
            .iter()
            .find(|s| s.subject.as_deref() == Some("tree:."))
            .expect("tree:. total")
            .value;
        let per_file_sum: f64 = report
            .samples
            .iter()
            .filter(|s| {
                s.subject
                    .as_deref()
                    .is_some_and(|sub| sub.starts_with("file:"))
            })
            .map(|s| s.value)
            .sum();
        assert_eq!(
            total, per_file_sum,
            "{key}: tree:. total must equal the sum of per-file samples"
        );
        for s in &report.samples {
            let sub = s.subject.as_deref().unwrap_or("");
            assert!(
                sub == "tree:." || sub.starts_with("file:"),
                "{key}: unexpected sample subject {sub:?}"
            );
        }
        total
    }

    /// Every `oxplow.*` FACT the gauge emits (the durable atomic grain), as
    /// `(measure, value, language)` — the inverted substrate's per-item output.
    fn facts_over(key: &str, files: HashMap<String, String>) -> Vec<(String, f64, Option<String>)> {
        report_over(key, files)
            .facts
            .iter()
            .map(|fc| {
                let lang = fc
                    .dims
                    .as_ref()
                    .and_then(|d| d.get("language"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                (fc.measure.clone(), fc.value, lang)
            })
            .collect()
    }

    /// The `(path, value)` of every per-file (`file:<path>`) sample, in emit
    /// order — the attribution grain the effort rollup reads.
    fn per_file_over(key: &str, files: HashMap<String, String>) -> Vec<(String, f64)> {
        report_over(key, files)
            .samples
            .iter()
            .filter_map(|s| {
                s.subject
                    .as_deref()
                    .and_then(|sub| sub.strip_prefix("file:"))
                    .map(|p| (p.to_string(), s.value))
            })
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
        // tsk30: each per-language idiom gauge emits per-file `oxplow.ast_hit`
        // facts tagged with its rule; the fact values sum to the baked tree total
        // (so the Sum(ast_hit)-by-rule spec reproduces the headline).
        let report = report_over("oxplow.rust.unsafe_blocks", corpus());
        assert!(!report.facts.is_empty(), "emits ast_hit facts");
        assert!(
            report
                .facts
                .iter()
                .all(|f| f.measure == "oxplow.ast_hit" && f.rule.as_deref() == Some("unsafe_block")),
            "every fact is on oxplow.ast_hit tagged rule=unsafe_block"
        );
        let fact_sum: f64 = report.facts.iter().map(|f| f.value).sum();
        let baked = report
            .samples
            .iter()
            .find(|s| s.subject.as_deref() == Some("tree:."))
            .unwrap()
            .value;
        assert_eq!(
            fact_sum, baked,
            "per-file ast_hit facts sum to the baked total"
        );
        assert_eq!(baked, 2.0);
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
        // Only the Rust `complex` (cc 12) exceeds 10 across the whole corpus.
        assert_eq!(run_over("oxplow.high_complexity_fns", mixed_corpus()), 1.0);
    }

    #[test]
    fn unified_long_functions_across_languages() {
        // Only the Rust `big` (>60 lines) is long.
        assert_eq!(run_over("oxplow.long_functions", mixed_corpus()), 1.0);
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
        assert_eq!(run_over("oxplow.fn_count", mixed_corpus()), 4.0);
    }

    #[test]
    fn unified_todos_across_languages() {
        // TS TODO + Clojure FIXME = 2 (comment-scoped); README's "TODO" is not a
        // source file → skipped by source_files().
        assert_eq!(run_over("oxplow.todos", mixed_corpus()), 2.0);
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

    #[test]
    fn builtin_keys_are_reserved_namespace_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for m in builtin_metrics() {
            assert!(m.key.starts_with("oxplow."), "{} not reserved", m.key);
            assert!(seen.insert(m.key), "duplicate builtin key {}", m.key);
        }
    }
}
