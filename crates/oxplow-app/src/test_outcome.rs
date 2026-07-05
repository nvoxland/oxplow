//! Per-effort test-outcome scalars (tsk38).
//!
//! "Tests failed" splits into two questions that show different things:
//! *did it close green* (quality gate) vs *did it go red during the work*
//! (development friction / TDD red phases). The metric engine's cross-time
//! collapse only expresses sum / last / Σn÷Σd — it can't do "max across an
//! effort's runs", "distinct tests ever red", or "count of red runs" — so these
//! are materialized once at effort close by the lifecycle producer (the same
//! pattern as `oxplow.cycle_time`). This module holds the pure computation over
//! the effort's test runs, kept free of the store so it's unit-testable.

use std::collections::HashSet;

/// One test run within an effort. `failed_refs` holds the failing test cases'
/// subject refs (`test:<classname>::<name>`); a `None` entry is an
/// asserted-count failure with no per-case identity (so it can't participate in
/// distinct-case dedup).
#[derive(Debug, Clone)]
pub struct TestRunFailures {
    pub failed_refs: Vec<Option<String>>,
}

impl TestRunFailures {
    fn failed_count(&self) -> i64 {
        self.failed_refs.len() as i64
    }
    fn is_red(&self) -> bool {
        !self.failed_refs.is_empty()
    }
}

/// The four per-effort test-outcome scalars, each emitted as one fact on
/// `oxplow.effort_test_outcome` sliced by the `oxplow.tests_stat` dim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffortTestOutcome {
    /// Failing tests in the **last** run of the effort — the quality gate.
    pub at_close: i64,
    /// Max failing tests in any single run — how red it got at worst.
    pub peak: i64,
    /// Distinct test cases red in ≥1 run — what the effort broke (then maybe fixed).
    pub distinct_failed: i64,
    /// Number of runs with ≥1 failure — how many times it went red.
    pub red_runs: i64,
}

/// Compute the outcome from the effort's test runs, ordered **captured-at ASC**.
/// Returns `None` when the effort recorded no test runs — nothing to emit.
pub fn compute_effort_test_outcome(runs: &[TestRunFailures]) -> Option<EffortTestOutcome> {
    let last = runs.last()?;
    let at_close = last.failed_count();
    let peak = runs
        .iter()
        .map(TestRunFailures::failed_count)
        .max()
        .unwrap_or(0);
    let red_runs = runs.iter().filter(|r| r.is_red()).count() as i64;

    // Distinct cases red in ≥1 run. Asserted-count failures (`None`) carry no
    // identity, so when any are present we can't fully dedupe — fall back to
    // `peak` so distinct never under-reports the worst single run.
    let mut distinct: HashSet<&str> = HashSet::new();
    let mut has_anonymous = false;
    for run in runs {
        for r in &run.failed_refs {
            match r {
                Some(s) => {
                    distinct.insert(s.as_str());
                }
                None => has_anonymous = true,
            }
        }
    }
    let distinct_known = distinct.len() as i64;
    let distinct_failed = if has_anonymous {
        distinct_known.max(peak)
    } else {
        distinct_known
    };

    Some(EffortTestOutcome {
        at_close,
        peak,
        distinct_failed,
        red_runs,
    })
}

/// Group flat `oxplow.test_case` facts into per-run failure lists, one run per
/// capture in **first-seen order** (`facts` must be captured-at ASC, as
/// `facts_for_captures` returns). Every capture with any case fact becomes a run
/// — so a fully-green run is an empty `failed_refs`, which keeps `at_close` /
/// `red_runs` correct. Each input is `(capture_id, is_failed, subject_ref)`.
pub fn runs_from_case_facts(facts: &[(i64, bool, Option<String>)]) -> Vec<TestRunFailures> {
    let mut order: Vec<i64> = Vec::new();
    let mut by_cap: std::collections::HashMap<i64, Vec<Option<String>>> =
        std::collections::HashMap::new();
    for (cap, failed, subject_ref) in facts {
        if !by_cap.contains_key(cap) {
            order.push(*cap);
            by_cap.insert(*cap, Vec::new());
        }
        if *failed {
            by_cap
                .get_mut(cap)
                .expect("just inserted")
                .push(subject_ref.clone());
        }
    }
    order
        .into_iter()
        .map(|cap| TestRunFailures {
            failed_refs: by_cap.remove(&cap).unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(refs: &[Option<&str>]) -> TestRunFailures {
        TestRunFailures {
            failed_refs: refs.iter().map(|r| r.map(String::from)).collect(),
        }
    }

    #[test]
    fn no_runs_yields_none() {
        assert_eq!(compute_effort_test_outcome(&[]), None);
    }

    #[test]
    fn single_green_run_is_all_zeros() {
        let out = compute_effort_test_outcome(&[run(&[])]).unwrap();
        assert_eq!(
            out,
            EffortTestOutcome {
                at_close: 0,
                peak: 0,
                distinct_failed: 0,
                red_runs: 0
            }
        );
    }

    #[test]
    fn red_then_green_is_the_healthy_tdd_signature() {
        // Two tests red, then a final green run: closed clean, but it went red.
        let out = compute_effort_test_outcome(&[run(&[Some("test:a"), Some("test:b")]), run(&[])])
            .unwrap();
        assert_eq!(
            out,
            EffortTestOutcome {
                at_close: 0,
                peak: 2,
                distinct_failed: 2,
                red_runs: 1
            }
        );
    }

    #[test]
    fn same_test_failing_across_runs_counts_once_distinct_but_stays_at_close() {
        let out =
            compute_effort_test_outcome(&[run(&[Some("test:a")]), run(&[Some("test:a")])]).unwrap();
        assert_eq!(
            out,
            EffortTestOutcome {
                at_close: 1,
                peak: 1,
                distinct_failed: 1,
                red_runs: 2
            }
        );
    }

    #[test]
    fn different_tests_across_runs_accumulate_distinct() {
        let out =
            compute_effort_test_outcome(&[run(&[Some("test:a")]), run(&[Some("test:b")])]).unwrap();
        assert_eq!(
            out,
            EffortTestOutcome {
                at_close: 1,
                peak: 1,
                distinct_failed: 2,
                red_runs: 2
            }
        );
    }

    #[test]
    fn anonymous_asserted_failures_fall_back_to_peak_for_distinct() {
        // Asserted counts (no case identity): 3 failed in one run.
        let out = compute_effort_test_outcome(&[run(&[None, None, None])]).unwrap();
        assert_eq!(
            out,
            EffortTestOutcome {
                at_close: 3,
                peak: 3,
                distinct_failed: 3,
                red_runs: 1
            }
        );
    }

    #[test]
    fn mixed_identity_never_under_reports_distinct_below_peak() {
        // One run: one known-red + one anonymous → peak 2, distinct floored at peak.
        let out = compute_effort_test_outcome(&[run(&[Some("test:a"), None])]).unwrap();
        assert_eq!(out.peak, 2);
        assert_eq!(out.distinct_failed, 2);
    }

    #[test]
    fn runs_from_case_facts_groups_by_capture_and_keeps_green_runs() {
        // Capture 1: a failed + b passed. Capture 2: all green (only a passed).
        let facts = [
            (1, true, Some("test:a".to_string())),
            (1, false, Some("test:b".to_string())),
            (2, false, Some("test:a".to_string())),
        ];
        let runs = runs_from_case_facts(&facts);
        assert_eq!(runs.len(), 2, "both captures become runs");
        assert_eq!(runs[0].failed_refs, vec![Some("test:a".to_string())]);
        assert!(runs[1].failed_refs.is_empty(), "green run has no failures");

        // Feeds the outcome: went red (1) then green → closes clean.
        let out = compute_effort_test_outcome(&runs).unwrap();
        assert_eq!(
            out,
            EffortTestOutcome {
                at_close: 0,
                peak: 1,
                distinct_failed: 1,
                red_runs: 1
            }
        );
    }
}
