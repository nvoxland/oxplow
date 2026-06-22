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
];

/// Every bundled built-in metric, across all languages.
pub fn builtin_metrics() -> &'static [BuiltinMetric] {
    RUST
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
            .iter()
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

    #[test]
    fn builtin_keys_are_reserved_namespace_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for m in builtin_metrics() {
            assert!(m.key.starts_with("oxplow."), "{} not reserved", m.key);
            assert!(seen.insert(m.key), "duplicate builtin key {}", m.key);
        }
    }
}
