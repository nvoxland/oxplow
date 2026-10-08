//! Bundled built-in metric catalog (epic tsk213, P3b). Every code/language
//! metric oxplow ships is authored through the **public** capability surface
//! (`files()` / `ast_query()` + a collector's `{"facts": [...]}`) and embedded
//! here — never a privileged Rust path. A project enables one with
//! `metrics: - use: oxplow.<lang>.<name>`; the runner resolves it at `built-in`
//! scope and runs it from the embedded script below (no project-disk file).
//!
//! These are the reference implementations a user copies. Each is exercised by a
//! golden test over a fixture corpus (see the tests module).

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
    /// The event types that run it.
    pub on: &'static [&'static str],
    /// Payload fields each of those events must have, with these values.
    pub filter: &'static [(&'static str, &'static str)],
    /// It reads the whole tree as of the snapshot, never only the files
    /// the snapshot recorded: its capture restates every file.
    pub whole_tree: bool,
    /// How its runs are paced; [`IMMEDIATE`] runs on every event.
    pub pacing: BuiltinPacing,
    pub runtime: &'static str,
    pub input: &'static str,
    pub script: &'static str,
}

/// A built-in's pacing (`.context/metrics.md` "Pacing"): run once the
/// triggering events have stopped for `settle_secs`, at most every
/// `at_most_secs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinPacing {
    pub settle_secs: Option<u32>,
    pub at_most_secs: Option<u32>,
}

/// Every triggering event runs it at once.
pub const IMMEDIATE: BuiltinPacing = BuiltinPacing {
    settle_secs: None,
    at_most_secs: None,
};

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
        dimensions: &["oxplow.package", "oxplow.language", "oxplow.vcs_rev"],
        target: Some(0.0),
        on: &["snapshot.taken"],
        filter: &[],
        whole_tree: false,
        pacing: IMMEDIATE,
        runtime: "starlark",
        input: "text",
        script: include_str!("metrics/rust/unsafe_blocks.star"),
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
        dimensions: &["oxplow.package", "oxplow.language", "oxplow.vcs_rev"],
        target: None,
        on: &["snapshot.taken"],
        filter: &[],
        whole_tree: false,
        pacing: IMMEDIATE,
        runtime: "starlark",
        input: "text",
        script: include_str!("metrics/rust/unwrap_expect_calls.star"),
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
        dimensions: &["oxplow.package", "oxplow.language", "oxplow.vcs_rev"],
        target: None,
        on: &["snapshot.taken"],
        filter: &[],
        whole_tree: false,
        pacing: IMMEDIATE,
        runtime: "starlark",
        input: "text",
        script: include_str!("metrics/rust/panic_macros.star"),
    },
];

/// The built-in gauges a project runs without a `metrics: - use:` entry
/// (tsk1034): the ones behind `v_function` — so a new project can answer
/// "which functions are longest" — and the TODO count. An `enabled: false`
/// marker turns one off; every other built-in runs only when `use:`d.
pub const DEFAULT_ON: &[&str] = &[
    "oxplow.todos",
    "oxplow.fn_count",
    "oxplow.high_complexity_fns",
    "oxplow.long_functions",
];

/// Language-agnostic code metrics (tsk314): one metric per concept, driven by
/// the per-language capability layer (`source_files()` + `code_metrics()` /
/// `markers()`). `language: ""` → no single language (the seeded definition's
/// language is NULL; samples carry the per-file `language` dim). These replace
/// the old per-language todo/complexity/fn-count/long-function gauges.
const CODE: &[BuiltinMetric] = &[
    code_metric(
        "oxplow.todos",
        "TODO / FIXME markers",
        "TODO/FIXME/HACK/XXX/BUG markers in comments, across all languages.",
        "lower-better",
        include_str!("metrics/code/todos.star"),
    ),
    code_metric(
        "oxplow.fn_count",
        "function count",
        "Total functions / methods defined, across all languages.",
        "neutral",
        include_str!("metrics/code/fn_count.star"),
    ),
    code_metric(
        "oxplow.high_complexity_fns",
        "high-complexity functions",
        "Functions whose cyclomatic complexity exceeds the threshold, across all languages.",
        "lower-better",
        include_str!("metrics/code/high_complexity_fns.star"),
    ),
    code_metric(
        "oxplow.long_functions",
        "long functions (>60 lines)",
        "Functions longer than 60 lines, across all languages.",
        "lower-better",
        include_str!("metrics/code/long_functions.star"),
    ),
    // Doc coverage is a per-file RATIO (%), not a count, so it can't use the
    // count-based `code_metric` helper (tsk125).
    BuiltinMetric {
        key: "oxplow.doc_coverage",
        kind: "coverage",
        title: "Doc coverage",
        description: "% of public functions/methods carrying a doc comment (or docstring), across all languages.",
        unit: "%",
        direction: "higher-better",
        grain: "tree",
        language: "",
        dimensions: &["oxplow.package", "oxplow.language", "oxplow.vcs_rev"],
        target: None,
        on: &["snapshot.taken"],
        filter: &[],
        whole_tree: false,
        pacing: IMMEDIATE,
        runtime: "starlark",
        input: "text",
        script: include_str!("metrics/code/doc_coverage.star"),
    },
];

/// A language-agnostic tree gauge (the unified code metrics). Like `ast_metric`
/// but `language: ""` (no single language — it sweeps `source_files()` itself).
/// Whole-tree scans: they restate the tree, so they read all of it — on a
/// ref move (`snapshot.taken` with `trigger: git_refs`, logged for every
/// ref move, an unchanged tree included) rather than every save.
const TREE: &[BuiltinMetric] = &[BuiltinMetric {
    key: "oxplow.duplicate_lines",
    kind: "findings",
    title: "Duplicated lines",
    description: "Lines in blocks duplicated elsewhere in the tree, both sides of each copy.",
    unit: "lines",
    direction: "lower-better",
    grain: "tree",
    language: "",
    dimensions: &["oxplow.vcs_rev"],
    target: None,
    on: &["snapshot.taken"],
    filter: &[("trigger", "git_refs")],
    whole_tree: true,
    // A whole-tree scan is minutes of CPU on a large tree, and a commit or
    // rebase moves several refs at once (a restart replays the ones it
    // missed): one scan of the latest tree once they settle.
    pacing: BuiltinPacing {
        settle_secs: Some(60),
        at_most_secs: Some(15 * 60),
    },
    runtime: "starlark",
    input: "text",
    script: include_str!("metrics/code/duplicate_lines.star"),
}];

const fn code_metric(
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
        dimensions: &["oxplow.package", "oxplow.language", "oxplow.vcs_rev"],
        target: None,
        on: &["snapshot.taken"],
        filter: &[],
        whole_tree: false,
        pacing: IMMEDIATE,
        runtime: "starlark",
        input: "text",
        script,
    }
}

/// A `gauge`/`tree`/`on-snapshot`/`starlark`/`text` metric (the common shape for
/// a tree-derived AST scan), so each per-language entry stays terse.
const fn ast_metric(
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
        dimensions: &["oxplow.package", "oxplow.language", "oxplow.vcs_rev"],
        target,
        on: &["snapshot.taken"],
        filter: &[],
        whole_tree: false,
        pacing: IMMEDIATE,
        runtime: "starlark",
        input: "text",
        script,
    }
}

const TS: &[BuiltinMetric] = &[
    ast_metric(
        "oxplow.ts.any_usage",
        "any usage",
        "Uses of the `any` type.",
        "lower-better",
        "typescript",
        None,
        include_str!("metrics/ts/any_usage.star"),
    ),
    ast_metric(
        "oxplow.ts.non_null_assertions",
        "non-null assertions",
        "Non-null assertions (`!`).",
        "lower-better",
        "typescript",
        None,
        include_str!("metrics/ts/non_null_assertions.star"),
    ),
    ast_metric(
        "oxplow.ts.console_calls",
        "console.* calls",
        "Calls to `console.*`.",
        "lower-better",
        "typescript",
        None,
        include_str!("metrics/ts/console_calls.star"),
    ),
    ast_metric(
        "oxplow.ts.ts_ignore",
        "ts-ignore / ts-expect-error",
        "`@ts-ignore` / `@ts-expect-error` suppressions.",
        "lower-better",
        "typescript",
        None,
        include_str!("metrics/ts/ts_ignore.star"),
    ),
];

const CLOJURE: &[BuiltinMetric] = &[ast_metric(
    "oxplow.clojure.defn_count",
    "defn count",
    "Number of `defn` definitions.",
    "neutral",
    "clojure",
    None,
    include_str!("metrics/clojure/defn_count.star"),
)];

const CSHARP: &[BuiltinMetric] = &[
    ast_metric(
        "oxplow.csharp.empty_catch",
        "empty catch blocks",
        "Empty `catch` blocks that swallow exceptions.",
        "lower-better",
        "csharp",
        None,
        include_str!("metrics/csharp/empty_catch.star"),
    ),
    ast_metric(
        "oxplow.csharp.blocking_async_calls",
        "blocking async calls (.Result / .Wait())",
        "Blocking calls on async code (`.Result` / `.Wait()`).",
        "lower-better",
        "csharp",
        None,
        include_str!("metrics/csharp/blocking_async_calls.star"),
    ),
];

/// Every bundled built-in metric: language-idiom metrics per language, the
/// language-agnostic code metrics (`CODE`) and the whole-tree scans (`TREE`).
pub fn builtin_metrics() -> Vec<BuiltinMetric> {
    [RUST, TS, CLOJURE, CSHARP, CODE, TREE].concat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TreeHost;
    use std::collections::HashMap;

    /// Run a built-in metric's script by key over a fixture file map and
    /// return its facts.
    fn report_over(key: &str, files: HashMap<String, String>) -> Vec<crate::CollectedFact> {
        let metric = builtin_metrics()
            .into_iter()
            .find(|m| m.key == key)
            .unwrap_or_else(|| panic!("no builtin metric {key}"));
        crate::run_fact_starlark(
            metric.script,
            &serde_json::json!({}),
            TreeHost::new(files),
            &crate::SandboxBudget::default(),
        )
        .expect("script runs")
    }

    /// Run a built-in idiom metric and return its repo total — the sum of its
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

    /// tsk989: a dimension has one name, its namespaced key (V166): a
    /// built-in metric declares only conformed keys, never a bare alias.
    #[test]
    fn builtin_metrics_slice_by_conformed_keys() {
        for m in builtin_metrics() {
            for d in m.dimensions {
                assert!(d.contains('.'), "{}: `{d}` isn't namespaced", m.key);
            }
        }
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
            oxplow_code_dup::detect_duplicates(
                files.clone(),
                oxplow_code_dup::DupOptions::default(),
            )
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

    #[test]
    fn duplicate_lines_runs_over_the_whole_tree_after_a_ref_move() {
        let m = builtin_metrics()
            .into_iter()
            .find(|m| m.key == "oxplow.duplicate_lines")
            .expect("a built-in");
        assert_eq!(m.on, &["snapshot.taken"]);
        assert_eq!(m.filter, &[("trigger", "git_refs")]);
        assert!(m.whole_tree, "a slice of the tree can't restate it");
        assert!(
            builtin_metrics()
                .iter()
                .filter(|o| o.key != m.key)
                .all(|o| o.on == ["snapshot.taken"] && o.filter.is_empty() && !o.whole_tree),
            "the code metrics run on every snapshot over its delta"
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
