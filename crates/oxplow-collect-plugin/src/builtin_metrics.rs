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
    BuiltinMetric {
        key: "oxplow.rust.todo_markers",
        kind: "gauge",
        title: "TODO / FIXME markers",
        unit: "count",
        direction: "lower-better",
        grain: "tree",
        language: "rust",
        dimensions: &["language", "git_version"],
        target: None,
        trigger: "on-snapshot",
        runtime: "starlark",
        input: "text",
        script: include_str!("plugins/metrics/rust/todo_markers.star"),
    },
    BuiltinMetric {
        key: "oxplow.rust.fn_count",
        kind: "gauge",
        title: "function count",
        unit: "count",
        direction: "neutral",
        grain: "tree",
        language: "rust",
        dimensions: &["language", "git_version"],
        target: None,
        trigger: "on-snapshot",
        runtime: "starlark",
        input: "text",
        script: include_str!("plugins/metrics/rust/fn_count.star"),
    },
    // Complexity / length signals (successors to the in-process code-quality
    // producer, tsk229) via the code_metrics() host builtin.
    BuiltinMetric {
        key: "oxplow.rust.high_complexity_fns",
        kind: "gauge",
        title: "high-complexity functions",
        unit: "count",
        direction: "lower-better",
        grain: "tree",
        language: "rust",
        dimensions: &["language", "git_version"],
        target: None,
        trigger: "on-snapshot",
        runtime: "starlark",
        input: "text",
        script: include_str!("plugins/metrics/rust/high_complexity_fns.star"),
    },
    BuiltinMetric {
        key: "oxplow.rust.long_functions",
        kind: "gauge",
        title: "long functions (>60 lines)",
        unit: "count",
        direction: "lower-better",
        grain: "tree",
        language: "rust",
        dimensions: &["language", "git_version"],
        target: None,
        trigger: "on-snapshot",
        runtime: "starlark",
        input: "text",
        script: include_str!("plugins/metrics/rust/long_functions.star"),
    },
];

/// A `gauge`/`tree`/`on-snapshot`/`starlark`/`text` metric (the common shape for
/// a tree-derived AST scan), so each per-language entry stays terse.
const fn ast_gauge(
    key: &'static str,
    title: &'static str,
    direction: &'static str,
    language: &'static str,
    target: Option<f64>,
    script: &'static str,
) -> BuiltinMetric {
    BuiltinMetric {
        key,
        kind: "gauge",
        title,
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
        "lower-better",
        "typescript",
        None,
        include_str!("plugins/metrics/ts/any_usage.star"),
    ),
    ast_gauge(
        "oxplow.ts.non_null_assertions",
        "non-null assertions",
        "lower-better",
        "typescript",
        None,
        include_str!("plugins/metrics/ts/non_null_assertions.star"),
    ),
    ast_gauge(
        "oxplow.ts.console_calls",
        "console.* calls",
        "lower-better",
        "typescript",
        None,
        include_str!("plugins/metrics/ts/console_calls.star"),
    ),
    ast_gauge(
        "oxplow.ts.ts_ignore",
        "ts-ignore / ts-expect-error",
        "lower-better",
        "typescript",
        None,
        include_str!("plugins/metrics/ts/ts_ignore.star"),
    ),
    ast_gauge(
        "oxplow.ts.fn_count",
        "function count",
        "neutral",
        "typescript",
        None,
        include_str!("plugins/metrics/ts/fn_count.star"),
    ),
    ast_gauge(
        "oxplow.ts.high_complexity_fns",
        "high-complexity functions",
        "lower-better",
        "typescript",
        None,
        include_str!("plugins/metrics/ts/high_complexity_fns.star"),
    ),
];

const CLOJURE: &[BuiltinMetric] = &[
    ast_gauge(
        "oxplow.clojure.defn_count",
        "defn count",
        "neutral",
        "clojure",
        None,
        include_str!("plugins/metrics/clojure/defn_count.star"),
    ),
    ast_gauge(
        "oxplow.clojure.todo_comments",
        "TODO / FIXME comments",
        "lower-better",
        "clojure",
        None,
        include_str!("plugins/metrics/clojure/todo_comments.star"),
    ),
];

/// Every bundled built-in metric, across all languages.
///
/// (C# is intentionally absent: `ast_query` is backed by the grammars bundled in
/// `oxplow-code-metrics`, which does not include `tree-sitter-c-sharp` — adding a
/// C# catalog needs that grammar + a `Language::CSharp` variant first.)
pub fn builtin_metrics() -> Vec<BuiltinMetric> {
    [RUST, TS, CLOJURE].concat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GaugeHost;
    use std::collections::HashMap;

    /// Run a built-in metric by key over a fixture file map and return the
    /// single sample's value.
    fn run_over(key: &str, files: HashMap<String, String>) -> f64 {
        let metric = builtin_metrics()
            .into_iter()
            .find(|m| m.key == key)
            .unwrap_or_else(|| panic!("no builtin metric {key}"));
        let out = metric
            .collector()
            .run_gauge("", GaugeHost::new(files))
            .expect("gauge runs");
        let report = out.as_gauge().expect("gauge output");
        assert_eq!(report.samples.len(), 1, "{key} projects one sample");
        report.samples[0].value
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
    todo!()
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
    fn rust_unwrap_expect_golden() {
        assert_eq!(run_over("oxplow.rust.unwrap_expect_calls", corpus()), 2.0);
    }

    #[test]
    fn rust_panic_macros_golden() {
        // panic! + todo! = 2.
        assert_eq!(run_over("oxplow.rust.panic_macros", corpus()), 2.0);
    }

    #[test]
    fn rust_todo_markers_golden() {
        // TODO in a.rs comment + FIXME in b.rs comment = 2; the TODO inside the
        // string literal is NOT counted (comment-scoped).
        assert_eq!(run_over("oxplow.rust.todo_markers", corpus()), 2.0);
    }

    #[test]
    fn rust_fn_count_golden() {
        // a, b, c = 3 function_items.
        assert_eq!(run_over("oxplow.rust.fn_count", corpus()), 3.0);
    }

    fn ts_corpus() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert(
            "src/a.ts".to_string(),
            r#"
// @ts-ignore
function f(x: any): any {
    console.log(x);
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
        // console.log (a.ts) + console.warn (b.tsx) = 2.
        assert_eq!(run_over("oxplow.ts.console_calls", ts_corpus()), 2.0);
    }

    #[test]
    fn ts_ts_ignore_golden() {
        // @ts-ignore (a.ts) + @ts-expect-error (b.tsx) = 2; the markdown is skipped.
        assert_eq!(run_over("oxplow.ts.ts_ignore", ts_corpus()), 2.0);
    }

    #[test]
    fn ts_fn_count_golden() {
        // a.ts: function f + arrow g = 2; b.tsx: arrow C = 1; total 3.
        assert_eq!(run_over("oxplow.ts.fn_count", ts_corpus()), 3.0);
    }

    fn clj_corpus() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert(
            "src/core.clj".to_string(),
            ";; TODO: refactor\n(defn add [a b] (+ a b))\n(defn- helper [] :ok)\n(def x 1)\n"
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
        assert_eq!(run_over("oxplow.clojure.defn_count", clj_corpus()), 3.0);
    }

    #[test]
    fn clojure_todo_comments_golden() {
        // TODO (core.clj) + FIXME (util.cljs) = 2.
        assert_eq!(run_over("oxplow.clojure.todo_comments", clj_corpus()), 2.0);
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

    #[test]
    fn rust_high_complexity_fns_golden() {
        // Only `complex` (cc 12) exceeds 10; `big` is cc 1.
        assert_eq!(
            run_over("oxplow.rust.high_complexity_fns", metrics_corpus()),
            1.0
        );
    }

    #[test]
    fn rust_long_functions_golden() {
        // Only `big` (>60 lines) is long; `complex` is ~13 lines.
        assert_eq!(
            run_over("oxplow.rust.long_functions", metrics_corpus()),
            1.0
        );
    }

    #[test]
    fn ts_high_complexity_fns_golden() {
        let mut body = String::from("function complex(x: number): number {\n");
        for i in 0..11 {
            body.push_str(&format!("    if (x === {i}) return {i};\n"));
        }
        body.push_str("    return 0;\n}\n");
        let mut m = HashMap::new();
        m.insert("src/c.ts".to_string(), body);
        assert_eq!(run_over("oxplow.ts.high_complexity_fns", m), 1.0);
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
