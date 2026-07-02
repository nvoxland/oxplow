//! The canonical registry of **always-on producer metrics** (tsk286/tsk287).
//!
//! These metrics are emitted automatically by the producers (`token_usage.rs`,
//! `task_service.rs`, `collection.rs`) — there's nothing to opt into. Two
//! consumers need the same descriptors:
//!
//! - the **spec seeding** (`builtin_producer_specs`), which turns each
//!   descriptor into the `metric_spec` the engine aggregates, and
//! - the **Catalog** (`MetricsService::catalog`), a registry of *available*
//!   metrics that must list them even before any data exists.
//!
//! To keep those from drifting, the descriptors live here **once**:
//! [`builtin_producer_metrics`] is the sole source. Add or rename a producer
//! metric in exactly one place.

use oxplow_db::NewMetricSpec;

/// A built-in always-on producer metric descriptor. Static (`&'static str`)
/// because the set is fixed at compile time.
pub struct ProducerMetric {
    pub key: &'static str,
    pub title: &'static str,
    /// `gauge` | `coverage` | `event` (the kinds the producers emit).
    pub kind: &'static str,
    pub unit: &'static str,
    /// `higher-better` | `lower-better` | `neutral`.
    pub direction: &'static str,
    /// `sum` | `avg` | `last` (roll-up aggregation).
    pub default_agg: &'static str,
    /// Sample grain (`entity` | `effort` | `tree` | …); `None` for the
    /// whole-report coverage scalar, which has no per-sample grain.
    pub grain: Option<&'static str>,
    /// `operational` | `testing` | `static-quality` | `coverage` — Catalog grouping.
    pub category: &'static str,
    /// The producer id stamped on the definition.
    pub producer: &'static str,
    /// Conformed dimensions the producer slices by.
    pub dimensions: &'static [&'static str],
    pub description: Option<&'static str>,
}

/// The canonical list of always-on producer metrics. Mirrors exactly what the
/// producers used to inline; now they build from this.
pub fn builtin_producer_metrics() -> &'static [ProducerMetric] {
    const TOKEN_DIMS: &[&str] = &["model", "agent"];
    const EFFORT_DIMS: &[&str] = &["branch", "effort"];
    const NUDGE_DIMS: &[&str] = &["subject", "branch", "thread"];
    const BRANCH_DIMS: &[&str] = &["branch"];
    const TREE_DIMS: &[&str] = &["branch", "git_version"];
    &[
        // otel-tokens (token_usage.rs::ingest_otlp_tokens, via the OTLP
        // receiver — tsk22). The `producer` field is descriptor metadata; the
        // token facts arrive under the `otel-tokens` capture producer.
        ProducerMetric {
            key: "agent.tokens.input",
            title: "Input tokens",
            kind: "gauge",
            unit: "tokens",
            direction: "neutral",
            default_agg: "sum",
            grain: Some("entity"),
            category: "operational",
            producer: "otel-tokens",
            dimensions: TOKEN_DIMS,
            description: Some("Input tokens consumed by the agent."),
        },
        ProducerMetric {
            key: "agent.tokens.output",
            title: "Output tokens",
            kind: "gauge",
            unit: "tokens",
            direction: "neutral",
            default_agg: "sum",
            grain: Some("entity"),
            category: "operational",
            producer: "otel-tokens",
            dimensions: TOKEN_DIMS,
            description: Some("Output tokens produced by the agent."),
        },
        ProducerMetric {
            key: "agent.tokens.total",
            title: "Total tokens",
            kind: "gauge",
            unit: "tokens",
            direction: "neutral",
            default_agg: "sum",
            grain: Some("entity"),
            category: "operational",
            producer: "otel-tokens",
            dimensions: TOKEN_DIMS,
            description: Some("Total tokens (input + output) used by the agent."),
        },
        // token-parse (token_usage.rs::on_stop) — turns stay transcript-derived.
        ProducerMetric {
            key: "agent.turns",
            title: "Agent turns",
            kind: "gauge",
            unit: "count",
            direction: "neutral",
            default_agg: "sum",
            grain: Some("entity"),
            category: "operational",
            producer: "token-parse",
            dimensions: TOKEN_DIMS,
            description: Some("Number of agent turns."),
        },
        // effort-lifecycle (task_service.rs)
        ProducerMetric {
            key: "effort.cycle_time_ms",
            title: "Effort cycle time",
            kind: "gauge",
            unit: "ms",
            direction: "lower-better",
            default_agg: "avg",
            grain: Some("effort"),
            category: "operational",
            producer: "effort-lifecycle",
            dimensions: EFFORT_DIMS,
            description: Some("How long an effort stayed open (close minus start)."),
        },
        ProducerMetric {
            key: "task.efforts",
            title: "Efforts per task",
            kind: "gauge",
            unit: "count",
            direction: "lower-better",
            default_agg: "avg",
            grain: Some("effort"),
            category: "operational",
            producer: "effort-lifecycle",
            dimensions: EFFORT_DIMS,
            description: Some("Number of efforts spent on a task (the redo-rate signal)."),
        },
        // nudges (collection.rs)
        ProducerMetric {
            key: "agent.nudges.fired",
            title: "Nudges fired",
            kind: "event",
            unit: "count",
            direction: "lower-better",
            default_agg: "sum",
            grain: Some("effort"),
            category: "operational",
            producer: "nudges",
            dimensions: NUDGE_DIMS,
            description: Some("Count of nudges fired to the agent."),
        },
        // tests (collection.rs)
        ProducerMetric {
            key: "oxplow.tests.passed",
            title: "Tests passed",
            kind: "gauge",
            unit: "count",
            direction: "higher-better",
            default_agg: "last",
            grain: Some("effort"),
            category: "testing",
            producer: "tests",
            dimensions: BRANCH_DIMS,
            description: Some("Tests that passed in the latest run."),
        },
        ProducerMetric {
            key: "oxplow.tests.failed",
            title: "Tests failed",
            kind: "gauge",
            unit: "count",
            direction: "lower-better",
            default_agg: "last",
            grain: Some("effort"),
            category: "testing",
            producer: "tests",
            dimensions: BRANCH_DIMS,
            description: Some("Tests that failed in the latest run."),
        },
        ProducerMetric {
            key: "oxplow.tests.total",
            title: "Tests total",
            kind: "gauge",
            unit: "count",
            direction: "neutral",
            default_agg: "last",
            grain: Some("effort"),
            category: "testing",
            producer: "tests",
            dimensions: BRANCH_DIMS,
            description: Some("Total tests in the latest run."),
        },
        // coverage (collection.rs) — whole-report absolute %, no per-sample grain.
        ProducerMetric {
            key: "oxplow.coverage.abs_pct",
            title: "Coverage",
            kind: "coverage",
            unit: "%",
            direction: "higher-better",
            default_agg: "last",
            grain: None,
            category: "coverage",
            producer: "coverage",
            dimensions: TREE_DIMS,
            description: Some("Whole-report coverage %."),
        },
        // analysis (collection.rs)
        ProducerMetric {
            key: "oxplow.analysis.errors",
            title: "Analysis errors",
            kind: "gauge",
            unit: "count",
            direction: "lower-better",
            default_agg: "last",
            grain: Some("tree"),
            category: "static-quality",
            producer: "analysis",
            dimensions: TREE_DIMS,
            description: Some("Static-analysis errors in the latest run."),
        },
        ProducerMetric {
            key: "oxplow.analysis.warnings",
            title: "Analysis warnings",
            kind: "gauge",
            unit: "count",
            direction: "lower-better",
            default_agg: "last",
            grain: Some("tree"),
            category: "static-quality",
            producer: "analysis",
            dimensions: TREE_DIMS,
            description: Some("Static-analysis warnings in the latest run."),
        },
    ]
}

/// The producer metrics as `metric_spec`s (epic tsk12, T-B) — the aggregation
/// each producer metric is, over the built-in measures its producer now emits
/// facts on. Conformed dims (not extra measures) distinguish the variants: token
/// in/out slice `oxplow.tokens` by `oxplow.token_kind`; test pass/fail slice
/// `oxplow.test_case` by `oxplow.status`; analysis errors/warnings filter
/// `oxplow.lint_hit` by severity; coverage is a `ratio` over `oxplow.coverage`.
/// Seeded beside the built-in gauge specs (`MetricsService::seed_catalog`);
/// consumed by the engine at the read-flip (tsk26). Surface fields (title/unit/
/// direction/display/category) come from [`builtin_producer_metrics`].
pub fn builtin_producer_specs() -> Vec<NewMetricSpec> {
    builtin_producer_metrics()
        .iter()
        .filter_map(|m| {
            let (source_measure, aggregation, filter_json) = producer_spec_shape(m.key)?;
            let mut s = NewMetricSpec::base(m.key, m.title, source_measure, aggregation);
            s.unit = Some(m.unit.into());
            s.direction = m.direction.into();
            s.display_kind = m.kind.into();
            s.category = Some(m.category.into());
            s.description = m.description.map(Into::into);
            s.filter_json = filter_json;
            if m.key == "oxplow.coverage.abs_pct" {
                // Coverage red/green policy in DATA, not a hardcoded UI ramp
                // (tsk220): fail < 50%, warn < 80%, ok ≥ 80%. Lives on the spec
                // now that the legacy definition write is gone (T-E2).
                s.target = Some(crate::collection::COVERAGE_TARGET_PCT);
                s.warn_at = Some(crate::collection::COVERAGE_TARGET_PCT);
                s.fail_at = Some(crate::collection::COVERAGE_FAIL_PCT);
            }
            Some(s)
        })
        .collect()
}

/// The `(source_measure, aggregation, filter_json)` a producer metric aggregates.
/// `None` for a key with no measure home (a producer metric not yet inverted).
fn producer_spec_shape(key: &str) -> Option<(&'static str, &'static str, Option<String>)> {
    let dim_eq = |k: &str, v: &str| format!("{{\"dim_eq\":[\"{k}\",\"{v}\"]}}");
    let severity = |v: &str| format!("{{\"severity\":\"{v}\"}}");
    Some(match key {
        "agent.tokens.input" => (
            "oxplow.tokens",
            "sum",
            Some(dim_eq("oxplow.token_kind", "input")),
        ),
        "agent.tokens.output" => (
            "oxplow.tokens",
            "sum",
            Some(dim_eq("oxplow.token_kind", "output")),
        ),
        "agent.tokens.total" => ("oxplow.tokens", "sum", None),
        "agent.turns" => ("oxplow.turn", "sum", None),
        "effort.cycle_time_ms" => ("oxplow.cycle_time", "avg", None),
        "task.efforts" => ("oxplow.task_effort", "avg", None),
        "agent.nudges.fired" => ("oxplow.nudge", "sum", None),
        "oxplow.tests.passed" => (
            "oxplow.test_case",
            "count",
            Some(dim_eq("oxplow.status", "passed")),
        ),
        "oxplow.tests.failed" => (
            "oxplow.test_case",
            "count",
            Some(dim_eq("oxplow.status", "failed")),
        ),
        "oxplow.tests.total" => ("oxplow.test_case", "count", None),
        "oxplow.coverage.abs_pct" => ("oxplow.coverage", "ratio", None),
        "oxplow.analysis.errors" => ("oxplow.lint_hit", "count", Some(severity("error"))),
        "oxplow.analysis.warnings" => ("oxplow.lint_hit", "count", Some(severity("warning"))),
        _ => return None,
    })
}

/// Whether `key` names a built-in PRODUCER metric (tokens/turns/tests/coverage/
/// analysis/lifecycle/nudges). The attribution classifier uses this to keep the
/// analysis pair run-attributed: their facts arrive on effort-stamped run-ingest
/// captures, unlike the (also path-grained, also static-quality) code gauges.
pub fn is_producer_metric_key(key: &str) -> bool {
    builtin_producer_metrics().iter().any(|m| m.key == key)
}

/// Look up a producer metric's descriptor by key. Producers call this and
/// `.definition()` instead of inlining the descriptor.
pub fn producer_metric(key: &str) -> &'static ProducerMetric {
    builtin_producer_metrics()
        .iter()
        .find(|m| m.key == key)
        .unwrap_or_else(|| panic!("unknown producer metric key: {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "unknown producer metric key")]
    fn unknown_key_panics() {
        producer_metric("nope.not.a.metric");
    }

    #[test]
    fn every_producer_metric_has_a_spec() {
        // T-B: each always-on producer metric inverts to a `metric_spec` over a
        // built-in measure. The set must be complete (no producer metric left
        // without a fact home) so the read-flip can serve every one from the
        // engine.
        let specs = builtin_producer_specs();
        assert_eq!(
            specs.len(),
            builtin_producer_metrics().len(),
            "every producer metric maps to exactly one spec"
        );
        let by_key: std::collections::HashMap<&str, &NewMetricSpec> =
            specs.iter().map(|s| (s.key.as_str(), s)).collect();

        // Token in/out slice the ONE tokens measure by a conformed dim.
        let input = by_key["agent.tokens.input"];
        assert_eq!(input.source_measure.as_deref(), Some("oxplow.tokens"));
        assert_eq!(input.aggregation, "sum");
        assert_eq!(
            input.filter_json.as_deref(),
            Some(r#"{"dim_eq":["oxplow.token_kind","input"]}"#)
        );
        assert!(by_key["agent.tokens.total"].filter_json.is_none());

        // Coverage is a ratio over Σnum/Σden.
        assert_eq!(by_key["oxplow.coverage.abs_pct"].aggregation, "ratio");
        // Tests count the test_case facts, sliced by status.
        assert_eq!(
            by_key["oxplow.tests.failed"].source_measure.as_deref(),
            Some("oxplow.test_case")
        );
        assert_eq!(by_key["oxplow.tests.failed"].aggregation, "count");
        // Analysis filters lint hits by severity.
        assert_eq!(
            by_key["oxplow.analysis.errors"].filter_json.as_deref(),
            Some(r#"{"severity":"error"}"#)
        );
        // Surface fields carry over from the producer descriptor.
        assert_eq!(by_key["oxplow.coverage.abs_pct"].display_kind, "coverage");
    }
}
