//! The canonical registry of **always-on producer metrics** (tsk286/tsk287).
//!
//! These metrics are emitted automatically by the producers (`token_usage.rs`,
//! `task_service.rs`, `collection.rs`) — there's nothing to opt into. Two
//! consumers need the same descriptors:
//!
//! - the **producers**, which `upsert_definition` the metric the first time they
//!   record a sample, and
//! - the **Catalog** (`MetricsService::catalog`), a registry of *available*
//!   metrics that must list them even before any data exists.
//!
//! To keep those from drifting, the descriptors live here **once**:
//! [`builtin_producer_metrics`] is the sole source. Producers build their
//! `NewMetricDefinition` via [`ProducerMetric::definition`]; the Catalog reads
//! the same list. Add or rename a producer metric in exactly one place.

use oxplow_db::NewMetricDefinition;

/// A built-in always-on producer metric — the full descriptor needed to build
/// its `metric_definition`. Static (`&'static str`) because the set is fixed at
/// compile time.
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

impl ProducerMetric {
    /// Build the `NewMetricDefinition` the producer upserts. The single place
    /// the producer/Catalog descriptors agree — see the module docs.
    pub fn definition(&self) -> NewMetricDefinition {
        let mut def = NewMetricDefinition::new(self.key, self.kind, self.title);
        def.unit = Some(self.unit.into());
        def.direction = self.direction.into();
        def.default_agg = self.default_agg.into();
        def.grain = self.grain.map(Into::into);
        def.producer = Some(self.producer.into());
        def.category = Some(self.category.into());
        def.dimensions_json =
            Some(serde_json::to_string(self.dimensions).unwrap_or_else(|_| "[]".into()));
        def.description = self.description.map(Into::into);
        def
    }
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
        // token-parse (token_usage.rs)
        ProducerMetric {
            key: "agent.tokens.input",
            title: "Input tokens",
            kind: "gauge",
            unit: "tokens",
            direction: "neutral",
            default_agg: "sum",
            grain: Some("entity"),
            category: "operational",
            producer: "token-parse",
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
            producer: "token-parse",
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
            producer: "token-parse",
            dimensions: TOKEN_DIMS,
            description: Some("Total tokens (input + output) used by the agent."),
        },
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
    fn lookup_and_definition_match_registry() {
        let p = producer_metric("oxplow.coverage.abs_pct");
        let def = p.definition();
        assert_eq!(def.key, "oxplow.coverage.abs_pct");
        assert_eq!(def.kind, "coverage");
        assert_eq!(def.category.as_deref(), Some("coverage"));
        assert_eq!(def.grain, None, "coverage has no per-sample grain");
        assert_eq!(
            def.dimensions_json.as_deref(),
            Some(r#"["branch","git_version"]"#)
        );

        // Token dims serialize identically to the old inline literal.
        let tok = producer_metric("agent.tokens.total").definition();
        assert_eq!(tok.dimensions_json.as_deref(), Some(r#"["model","agent"]"#));
        assert_eq!(tok.default_agg, "sum");
    }

    #[test]
    #[should_panic(expected = "unknown producer metric key")]
    fn unknown_key_panics() {
        producer_metric("nope.not.a.metric");
    }
}
