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
        // Prompt-cache economics (tsk73) — token-denominated, never dollars.
        // Cache facts ride `oxplow.cache_tokens`/`oxplow.cache_usage`, NOT
        // `oxplow.tokens`, so `agent.tokens.total` keeps its meaning.
        ProducerMetric {
            key: "agent.tokens.cache_read",
            title: "Cache-read tokens",
            kind: "gauge",
            unit: "tokens",
            direction: "neutral",
            default_agg: "sum",
            grain: Some("entity"),
            category: "operational",
            producer: "otel-tokens",
            dimensions: TOKEN_DIMS,
            description: Some("Prompt tokens served from cache instead of re-processed."),
        },
        ProducerMetric {
            key: "agent.tokens.cache_creation",
            title: "Cache-write tokens",
            kind: "gauge",
            unit: "tokens",
            direction: "neutral",
            default_agg: "sum",
            grain: Some("entity"),
            category: "operational",
            producer: "otel-tokens",
            dimensions: TOKEN_DIMS,
            description: Some("Prompt tokens written into the cache (the cache-warming cost)."),
        },
        ProducerMetric {
            key: "agent.tokens.cache_hit_pct",
            title: "Cache hit ratio",
            kind: "gauge",
            unit: "%",
            direction: "higher-better",
            default_agg: "ratio",
            grain: Some("entity"),
            category: "operational",
            producer: "otel-tokens",
            dimensions: TOKEN_DIMS,
            description: Some(
                "Prompt-side cache hit ratio: cache-read / (input + cache-read + cache-write).",
            ),
        },
        ProducerMetric {
            key: "task.tokens",
            title: "Tokens per effort",
            kind: "gauge",
            unit: "tokens",
            direction: "lower-better",
            default_agg: "avg",
            grain: Some("effort"),
            category: "operational",
            producer: "effort-lifecycle",
            dimensions: EFFORT_DIMS,
            description: Some(
                "Avg tokens (all kinds) a closed effort spent — the cost of a unit of work, in tokens.",
            ),
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
        // Wasted-token pair (tsk77) — both fold `oxplow.token_waste`, the
        // append-only ratio measure: closes contribute (num 0 / den spend),
        // a detected revert contributes (num spend / den 0, value spend). So
        // SUM over values = wasted tokens, RATIO = wasted share. Token-
        // denominated by decision — never dollars (tsk73).
        ProducerMetric {
            key: "task.tokens.wasted",
            title: "Wasted tokens",
            kind: "gauge",
            unit: "tokens",
            direction: "lower-better",
            default_agg: "sum",
            grain: Some("effort"),
            category: "operational",
            producer: "revert-detect",
            dimensions: EFFORT_DIMS,
            description: Some(
                "Tokens spent in closed efforts whose commits were later reverted.",
            ),
        },
        ProducerMetric {
            key: "task.tokens.wasted_pct",
            title: "Wasted-token ratio",
            kind: "gauge",
            unit: "%",
            direction: "lower-better",
            default_agg: "ratio",
            grain: Some("effort"),
            category: "operational",
            producer: "revert-detect",
            dimensions: EFFORT_DIMS,
            description: Some(
                "Share of effort token spend that was later reverted — wasted ÷ all metered \
                 spend, across closed efforts.",
            ),
        },
        // Usage metrics phase 2 (tsk76) — the autonomy/velocity pair, both
        // per-close lifecycle means like task.tokens.
        ProducerMetric {
            key: "task.steering",
            title: "Steering per effort",
            kind: "gauge",
            unit: "per effort",
            direction: "lower-better",
            default_agg: "avg",
            grain: Some("effort"),
            category: "operational",
            producer: "effort-lifecycle",
            dimensions: EFFORT_DIMS,
            description: Some(
                "Avg steering events per closed effort (user prompts + Stop-hook nudges + \
                 review comments) — the autonomy number; lower means more autonomous.",
            ),
        },
        ProducerMetric {
            key: "effort.time_to_green_ms",
            title: "Time to green",
            kind: "gauge",
            unit: "ms",
            direction: "lower-better",
            default_agg: "avg",
            grain: Some("effort"),
            category: "testing",
            producer: "effort-lifecycle",
            dimensions: EFFORT_DIMS,
            description: Some(
                "Avg wall-clock from an effort's first red test run to its first green — \
                 only efforts that went red and recovered count.",
            ),
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
            description: Some("Known tests currently passing — each test's latest recorded status, unioned across runners."),
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
            description: Some("Known tests currently failing — each test's latest recorded status, unioned across runners."),
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
            description: Some("All known tests — union across runners, each counted at its latest recorded status."),
        },
        // Per-test duration (tsk46). `oxplow.test_duration` is `per-subject`, so each
        // test's LATEST timing wins — a partial run refreshes only what it ran and the
        // suite total stays a real total.
        ProducerMetric {
            key: "oxplow.tests.duration_ms",
            title: "Test suite duration",
            kind: "gauge",
            unit: "ms",
            direction: "lower-better",
            default_agg: "sum",
            grain: Some("effort"),
            category: "testing",
            producer: "tests",
            dimensions: BRANCH_DIMS,
            description: Some("Total wall-clock time of the known test suite."),
        },
        ProducerMetric {
            key: "oxplow.tests.slowest_ms",
            title: "Slowest test",
            kind: "gauge",
            unit: "ms",
            direction: "lower-better",
            default_agg: "max",
            grain: Some("effort"),
            category: "testing",
            producer: "tests",
            dimensions: BRANCH_DIMS,
            description: Some("Wall-clock time of the slowest single test."),
        },
        // Per-effort test-outcome scalars (tsk38) — materialized at effort close
        // by the lifecycle producer on `oxplow.effort_test_outcome`, sliced by
        // `oxplow.tests_stat`. Split "tests failed" into a close-state gate vs
        // three "went red during the effort" flavors the engine's cross-time
        // collapse can't express as a plain spec. The headline is the MEAN
        // across closed efforts (agg `avg`), so it's legitimately fractional —
        // unit is "per effort", not "count" (tsk63).
        ProducerMetric {
            key: "oxplow.tests.failed_at_close",
            title: "Tests failed at close",
            kind: "gauge",
            unit: "per effort",
            direction: "lower-better",
            default_agg: "avg",
            grain: Some("effort"),
            category: "testing",
            producer: "tests",
            dimensions: BRANCH_DIMS,
            description: Some("Failing tests in the effort's final run, averaged across closed efforts — did work close green."),
        },
        ProducerMetric {
            key: "oxplow.tests.peak_failed",
            title: "Peak tests failed",
            kind: "gauge",
            unit: "per effort",
            direction: "lower-better",
            default_agg: "avg",
            grain: Some("effort"),
            category: "testing",
            producer: "tests",
            dimensions: BRANCH_DIMS,
            description: Some("Most tests failing in any single run during an effort, averaged across closed efforts."),
        },
        ProducerMetric {
            key: "oxplow.tests.distinct_failed",
            title: "Distinct tests failed",
            kind: "gauge",
            unit: "per effort",
            direction: "lower-better",
            default_agg: "avg",
            grain: Some("effort"),
            category: "testing",
            producer: "tests",
            dimensions: BRANCH_DIMS,
            description: Some("Distinct tests that went red at least once during an effort, averaged across closed efforts."),
        },
        ProducerMetric {
            key: "oxplow.tests.red_runs",
            title: "Red test runs",
            kind: "gauge",
            unit: "per effort",
            direction: "lower-better",
            default_agg: "avg",
            grain: Some("effort"),
            category: "testing",
            producer: "tests",
            dimensions: BRANCH_DIMS,
            description: Some("Test runs with at least one failure during an effort, averaged across closed efforts."),
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
        // Prompt-cache economics (tsk73). Cache facts ride their own measures
        // — see the V59 migration header for why they can't share
        // `oxplow.tokens` (the unfiltered `total`) or each other (per-measure
        // cross-time collapse: additive sum vs non-additive Σn/Σd).
        "agent.tokens.cache_read" => (
            "oxplow.cache_tokens",
            "sum",
            Some(dim_eq("oxplow.token_kind", "cache_read")),
        ),
        "agent.tokens.cache_creation" => (
            "oxplow.cache_tokens",
            "sum",
            Some(dim_eq("oxplow.token_kind", "cache_creation")),
        ),
        "agent.tokens.cache_hit_pct" => ("oxplow.cache_usage", "ratio", None),
        "task.tokens" => ("oxplow.effort_tokens", "avg", None),
        "task.steering" => ("oxplow.effort_steering", "avg", None),
        "task.tokens.wasted" => ("oxplow.token_waste", "sum", None),
        "task.tokens.wasted_pct" => ("oxplow.token_waste", "ratio", None),
        "effort.time_to_green_ms" => ("oxplow.effort_time_to_green", "avg", None),
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
        // One duration fact per test; the per-subject fold keeps the latest per test,
        // so `sum` is the suite's wall-clock and `max` is the slowest single test.
        "oxplow.tests.duration_ms" => ("oxplow.test_duration", "sum", None),
        "oxplow.tests.slowest_ms" => ("oxplow.test_duration", "max", None),
        // Per-effort test-outcome scalars: one fact per stat per closed effort,
        // averaged across efforts (non-additive measure) for the headline.
        "oxplow.tests.failed_at_close" => (
            "oxplow.effort_test_outcome",
            "avg",
            Some(dim_eq("oxplow.tests_stat", "at_close")),
        ),
        "oxplow.tests.peak_failed" => (
            "oxplow.effort_test_outcome",
            "avg",
            Some(dim_eq("oxplow.tests_stat", "peak")),
        ),
        "oxplow.tests.distinct_failed" => (
            "oxplow.effort_test_outcome",
            "avg",
            Some(dim_eq("oxplow.tests_stat", "distinct_failed")),
        ),
        "oxplow.tests.red_runs" => (
            "oxplow.effort_test_outcome",
            "avg",
            Some(dim_eq("oxplow.tests_stat", "red_runs")),
        ),
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

    #[test]
    fn effort_outcome_averages_are_labelled_per_effort() {
        // tsk63: the effort-outcome family's headline is a MEAN across closed
        // efforts (agg `avg`), so it's legitimately fractional. A "count" unit
        // made 0.08 read like a bug — the unit must say what the number is.
        let metrics = builtin_producer_metrics();
        let by_key: std::collections::HashMap<&str, &ProducerMetric> =
            metrics.iter().map(|m| (m.key, m)).collect();
        for key in [
            "oxplow.tests.failed_at_close",
            "oxplow.tests.peak_failed",
            "oxplow.tests.distinct_failed",
            "oxplow.tests.red_runs",
        ] {
            assert_eq!(by_key[key].default_agg, "avg", "{key}");
            assert_eq!(by_key[key].unit, "per effort", "{key}");
        }
    }
}
