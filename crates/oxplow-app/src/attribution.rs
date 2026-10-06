//! How an effort's metric deltas are attributed: which family a metric
//! belongs to, and so which of the effort's records its delta reads — its
//! files (`effort_file`, claimed or observed) or its own captures
//! (`metric_capture.effort_id`, stamped from the tool call that caused the
//! run). Attribution itself is observed, never declared
//! (`.context/work-tracking.md`).

use oxplow_db::MetricSpec;

/// Which **attribution family** a metric SPEC belongs to — the single source
/// of truth for how an effort's delta for that metric is computed. Adding a
/// new fact-kind is a variant here + one match arm in
/// `CollectionService::effort_metric_deltas`.
///
/// - [`File`](Self::File) reads the effort's files (`effort_file`);
/// - [`Coverage`](Self::Coverage) / [`Run`](Self::Run) /
///   [`Window`](Self::Window) read the effort's own captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortAttributionFamily {
    /// Per-file code gauges: Σ over the effort's files of each file's
    /// `(current − baseline)`.
    File,
    /// Coverage: effort-relative — the diff is DERIVED at read against the
    /// effort's start snapshot (`coverage_delta`), a documented special case
    /// on the capture's detail payload (line-sets aren't in facts yet).
    Coverage,
    /// Other run-kind facts (tests, analysis): before→after / `sum` over the facts
    /// of the effort's own captures (`metric_capture.effort_id`, stamped at
    /// ingest).
    Run,
    /// Operational / everything else (tokens, nudges, cycle-time): before→after /
    /// `sum` over the facts of the effort's own captures. Read identically to
    /// [`Run`](Self::Run); a distinct family to document what it holds.
    Window,
}

/// Classify a metric SPEC into its [`EffortAttributionFamily`]. Routing, in
/// order:
/// 1. `coverage` category → [`Coverage`](EffortAttributionFamily::Coverage)
///    (the diff-at-read special case);
/// 2. `testing` category → [`Run`](EffortAttributionFamily::Run);
/// 3. operational `agent.*`/`effort.*`/`task.*` keys →
///    [`Window`](EffortAttributionFamily::Window) — thread+time facts on
///    effort-stamped captures, never per-file even when gauge-display;
/// 4. any other built-in PRODUCER metric (the `oxplow.analysis.*` pair) →
///    [`Run`](EffortAttributionFamily::Run): its facts arrive per run-ingest
///    on effort-stamped captures, and analysis must never fall to the per-file
///    branch even though its facts are path-grained;
/// 5. a snapshot-scan gauge spec (display `gauge`/`findings`, a source measure,
///    no formula) → [`File`](EffortAttributionFamily::File). Gauge captures are
///    never effort-stamped (routing these by their `static-quality` category to
///    Run made every bundled code metric vanish from effort rollups);
///    the File read attributes path-grained facts by the effort's files
///    and falls back to the repo-wide time-window before→after for repo-scalar
///    facts that carry no path;
/// 6. everything else → [`Window`](EffortAttributionFamily::Window): formula
///    specs (no facts of their own) and event metrics.
pub fn classify_effort_attribution(spec: &MetricSpec) -> EffortAttributionFamily {
    if spec.category.as_deref() == Some("coverage") {
        EffortAttributionFamily::Coverage
    } else if spec.category.as_deref() == Some("testing") {
        EffortAttributionFamily::Run
    } else if is_operational_metric_key(&spec.key) {
        EffortAttributionFamily::Window
    } else if crate::producer_metrics::is_producer_metric_key(&spec.key) {
        EffortAttributionFamily::Run
    } else if matches!(spec.display_kind.as_str(), "gauge" | "findings")
        && spec.source_measure.is_some()
        && spec.formula.is_none()
    {
        EffortAttributionFamily::File
    } else {
        EffortAttributionFamily::Window
    }
}

/// Operational metric namespaces (tokens, cost, cycle-time, redo-rate, nudges) —
/// thread+window facts, never per-file gauges. The oxplow-bundled
/// `metric-deltas` advisory skips the same prefixes in its SQL.
pub fn is_operational_metric_key(key: &str) -> bool {
    key.starts_with("agent.") || key.starts_with("effort.") || key.starts_with("task.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Timestamp;

    /// Minimal `MetricSpec` carrying only the fields the classifier reads
    /// (`display_kind`/`category`/`key`/`source_measure`); the rest are dummy. A
    /// `None` source measure models a formula spec (no facts of its own).
    fn spec(
        display_kind: &str,
        category: Option<&str>,
        key: &str,
        source_measure: Option<&str>,
    ) -> MetricSpec {
        MetricSpec {
            id: 1,
            key: key.into(),
            title: "t".into(),
            unit: None,
            source_measure: source_measure.map(Into::into),
            aggregation: "last".into(),
            filter_json: None,
            formula: None,
            sliceable_dims_json: None,
            direction: "neutral".into(),
            target: None,
            warn_at: None,
            fail_at: None,
            description: None,
            category: category.map(Into::into),
            language: None,
            scope: "project".into(),
            display_kind: display_kind.into(),
            entity_json: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        }
    }

    #[test]
    fn classify_routes_each_family() {
        use EffortAttributionFamily::*;
        // Coverage by category (its own diff-at-read branch).
        assert_eq!(
            classify_effort_attribution(&spec(
                "coverage",
                Some("coverage"),
                "oxplow.coverage.abs_pct",
                Some("oxplow.coverage"),
            )),
            Coverage
        );
        // Tests are run-attributed (by category, before the gauge check).
        assert_eq!(
            classify_effort_attribution(&spec(
                "test",
                Some("testing"),
                "oxplow.tests.total",
                Some("oxplow.test_case"),
            )),
            Run
        );
        // Analysis MUST be Run, not File: it is a
        // producer metric whose facts arrive on effort-stamped run-ingest
        // captures, even though its facts are path-grained.
        assert_eq!(
            classify_effort_attribution(&spec(
                "gauge",
                Some("static-quality"),
                "oxplow.analysis.errors",
                Some("oxplow.lint_hit"),
            )),
            Run
        );
        // A built-in code gauge is File even though it is seeded
        // `static-quality` — its snapshot-scan captures are never
        // effort-stamped, so routing it by category (to Run) would silently
        // drop it from every effort rollup.
        assert_eq!(
            classify_effort_attribution(&spec(
                "findings",
                Some("static-quality"),
                "oxplow.todos",
                Some("oxplow.todo"),
            )),
            File
        );
        // A custom code-health gauge over a measure is File.
        assert_eq!(
            classify_effort_attribution(&spec(
                "gauge",
                Some("custom"),
                "acme.unsafe_blocks",
                Some("acme.unsafe_blocks.m"),
            )),
            File
        );
        // A repo-scalar gauge is also File — its path-less facts take the File
        // read's repo-wide time-window fallback (per-file summing over claimed
        // paths would read 0/0 and silently drop the row).
        assert_eq!(
            classify_effort_attribution(&spec(
                "gauge",
                Some("custom"),
                "acme.bundle_size",
                Some("acme.size"),
            )),
            File
        );
        // Operational keys are window-attributed even when gauge-display.
        assert_eq!(
            classify_effort_attribution(&spec(
                "gauge",
                None,
                "effort.cycle_time_ms",
                Some("oxplow.cycle_time"),
            )),
            Window
        );
        // An event metric falls through to Window.
        assert_eq!(
            classify_effort_attribution(&spec(
                "event",
                None,
                "agent.nudges.fired",
                Some("oxplow.nudge"),
            )),
            Window
        );
        // A formula spec (no source measure) falls through to Window (no facts).
        assert_eq!(
            classify_effort_attribution(&spec("gauge", Some("custom"), "acme.ratio", None)),
            Window
        );
    }

    #[test]
    fn operational_keys_are_recognized() {
        assert!(is_operational_metric_key("agent.tokens.total"));
        assert!(is_operational_metric_key("effort.cycle_time_ms"));
        assert!(is_operational_metric_key("task.efforts"));
        assert!(!is_operational_metric_key("oxplow.rust.unsafe_blocks"));
        assert!(!is_operational_metric_key("acme.custom"));
    }
}
