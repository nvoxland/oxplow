//! What oxplow-foundation's effort test outcome collector writes from the
//! runs an effort recorded. (The code gauges' goldens are fixtures in the
//! foundation.)

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

// --- an effort's test outcome, from the runs it recorded -------------------

/// One test run an effort recorded, as the run recorder writes it: a
/// `tests` capture (a `test-run` one is a record without measurements) with
/// its detail and an `oxplow.test_case` fact per case it reported, or per
/// asserted failure when it reported counts only.
pub(crate) struct SeedRun {
    pub at_ms: i64,
    pub producer: &'static str,
    /// `(name, status)` of each case the run's report listed.
    pub cases: Vec<(&'static str, &'static str)>,
    /// Failures asserted as a count, with no report.
    pub asserted_failed: i64,
}

impl SeedRun {
    pub fn report(at_ms: i64, cases: &[(&'static str, &'static str)]) -> Self {
        Self {
            at_ms,
            producer: "tests",
            cases: cases.to_vec(),
            asserted_failed: 0,
        }
    }

    pub fn asserted(at_ms: i64, failed: i64) -> Self {
        Self {
            at_ms,
            producer: "tests",
            cases: Vec::new(),
            asserted_failed: failed,
        }
    }
}

/// Record `runs` for `effort` the way a run recorder does.
pub(crate) async fn seed_test_runs(
    facts: &oxplow_db::SqliteFactStore,
    stream: i64,
    thread: i64,
    effort: i64,
    branch: &str,
    runs: &[SeedRun],
) {
    let case = facts
        .get_measure("oxplow.test_case")
        .await
        .unwrap()
        .expect("the test_case measure");
    for run in runs {
        let failed = run
            .cases
            .iter()
            .filter(|(_, status)| *status == "failed")
            .count() as i64
            + run.asserted_failed;
        let suites: Vec<_> = if run.cases.is_empty() {
            Vec::new()
        } else {
            vec![serde_json::json!({
                "name": "suite",
                "cases": run.cases.iter().map(|(name, status)| serde_json::json!({
                    "classname": "mod", "name": name, "status": status
                })).collect::<Vec<_>>()
            })]
        };
        let mut capture = oxplow_db::NewMetricCapture::done(stream, run.producer, "hook");
        capture.thread_id = Some(thread);
        capture.effort_id = Some(effort);
        capture.branch = Some(branch.to_string());
        capture.captured_at = Some(oxplow_domain::Timestamp::from_unix_ms(run.at_ms));
        capture.detail_json = Some(
            serde_json::json!({
                "kind": "test-detail",
                "payload": { "command": "cargo test", "failed": failed, "suites": suites }
            })
            .to_string(),
        );
        let mut rows: Vec<oxplow_db::NewFact> = run
            .cases
            .iter()
            .map(|(name, status)| oxplow_db::NewFact {
                subject: Some(oxplow_domain::uri::build::test_uri("mod", name)),
                dims_json: Some(format!(r#"{{"oxplow.status":"{status}"}}"#)),
                ..oxplow_db::NewFact::new(case.id, 1.0)
            })
            .collect();
        for _ in 0..run.asserted_failed {
            rows.push(oxplow_db::NewFact {
                dims_json: Some(r#"{"oxplow.status":"failed"}"#.to_string()),
                ..oxplow_db::NewFact::new(case.id, 1.0)
            });
        }
        // A record-only run measures nothing.
        if run.producer != "tests" {
            rows.clear();
        }
        facts.record_facts(capture, rows).await.unwrap();
    }
}

/// `(measure, subject, dims, value, numerator, denominator)`.
pub(crate) type OutcomeFact = (
    String,
    Option<String>,
    Option<String>,
    f64,
    Option<f64>,
    Option<f64>,
);

/// What an effort's outcome facts say, in a form to compare:
/// `(measure, subject, dims, value, numerator, denominator)`, sorted.
pub(crate) async fn outcome_facts(facts: &oxplow_db::SqliteFactStore) -> Vec<OutcomeFact> {
    let mut out = Vec::new();
    for key in ["oxplow.effort_test_outcome", "oxplow.effort_time_to_green"] {
        let measure = facts.get_measure(key).await.unwrap().expect("measure");
        for f in facts.facts_for_measure(measure.id).await.unwrap() {
            out.push((
                key.to_string(),
                f.subject,
                f.dims_json,
                f.value,
                f.numerator,
                f.denominator,
            ));
        }
    }
    out.sort_by(|a, b| (&a.0, &a.2).cmp(&(&b.0, &b.2)));
    out
}

/// A red run, a record-only run (ignored), a red-then-recovering run and a
/// green one: the outcome facts both the old producer and the collector
/// must write for it, for the effort ref `effort`.
pub(crate) fn expected_outcome(
    effort: &str,
    stats: [i64; 4],
    time_to_green: Option<i64>,
) -> Vec<OutcomeFact> {
    let subject = Some(effort.to_string());
    let mut want: Vec<_> = ["at_close", "distinct_failed", "peak", "red_runs"]
        .into_iter()
        .zip([stats[0], stats[2], stats[1], stats[3]])
        .map(|(stat, value)| {
            (
                "oxplow.effort_test_outcome".to_string(),
                subject.clone(),
                Some(format!(r#"{{"oxplow.tests_stat":"{stat}"}}"#)),
                value as f64,
                Some(value as f64),
                Some(1.0),
            )
        })
        .collect();
    if let Some(ms) = time_to_green {
        want.push((
            "oxplow.effort_time_to_green".to_string(),
            subject,
            None,
            ms as f64,
            Some(ms as f64),
            Some(1.0),
        ));
    }
    want.sort_by(|a, b| (&a.0, &a.2).cmp(&(&b.0, &b.2)));
    want
}

/// The runs the equivalence tests share: red twice (a and b, then a alone), a
/// record-only run that must not count, and a green one 60.25 s after the
/// first red.
pub(crate) fn red_then_green_runs() -> Vec<SeedRun> {
    let mut record_only =
        SeedRun::report(20_000, &[("a", "failed"), ("b", "failed"), ("c", "failed")]);
    record_only.producer = "test-run";
    vec![
        SeedRun::report(1_000, &[("a", "failed"), ("b", "failed"), ("c", "passed")]),
        record_only,
        SeedRun::report(31_500, &[("a", "failed"), ("b", "passed")]),
        SeedRun::report(61_250, &[("a", "passed")]),
    ]
}

/// An asserted-count run that fails three, then one that recovers.
pub(crate) fn asserted_runs() -> Vec<SeedRun> {
    vec![SeedRun::asserted(5_000, 3), SeedRun::asserted(9_000, 0)]
}

/// Run the test-outcome collector's script over a seeded effort's rows, as its
/// `input` yields them — `(run, failed, cases, at_ms, case)` — and read the
/// facts back as `(measure, value, stat)`.
fn outcome_over(rows: &[(i64, i64, i64, i64, Option<&str>)]) -> Vec<(String, f64, Option<String>)> {
    let rows: Vec<serde_json::Value> = rows
        .iter()
        .map(|(run, failed, cases, at_ms, case)| {
            serde_json::json!({
                "run": run, "failed": failed, "cases": cases, "at_ms": at_ms,
                "case": case, "effort": "eff1"
            })
        })
        .collect();
    oxplow_script::runtime::run_starlark_hosted(
        script("oxplow.effort_test_outcome"),
        &serde_json::json!({ "rows": rows }),
        &oxplow_script::runtime::Hosts::default(),
    )
    .and_then(oxplow_script::facts_of)
    .expect("script runs")
    .into_iter()
    .map(|f| {
        assert_eq!(f.subject.as_deref(), Some("oxplow:oxplow/effort?id=eff1"));
        assert_eq!(f.den, Some(1.0), "non-additive, denominator 1");
        assert_eq!(f.num, Some(f.value));
        let stat = f
            .dims
            .as_ref()
            .and_then(|d| d.get("oxplow.tests_stat"))
            .and_then(|v| v.as_str())
            .map(String::from);
        (f.measure, f.value, stat)
    })
    .collect()
}

fn outcome(stats: [f64; 4]) -> Vec<(String, f64, Option<String>)> {
    ["at_close", "peak", "distinct_failed", "red_runs"]
        .into_iter()
        .zip(stats)
        .map(|(stat, v)| {
            (
                "oxplow.effort_test_outcome".to_string(),
                v,
                Some(stat.to_string()),
            )
        })
        .collect()
}

fn time_to_green(ms: f64) -> (String, f64, Option<String>) {
    ("oxplow.effort_time_to_green".to_string(), ms, None)
}

#[test]
fn no_runs_means_no_facts() {
    assert_eq!(outcome_over(&[]), vec![]);
}

#[test]
fn a_single_green_run_is_all_zeros() {
    assert_eq!(outcome_over(&[(1, 0, 3, 100, None)]), outcome([0.0; 4]));
}

#[test]
fn red_then_green_closes_clean_but_went_red() {
    let mut want = outcome([0.0, 2.0, 2.0, 1.0]);
    want.push(time_to_green(300.0));
    assert_eq!(
        outcome_over(&[
            (1, 2, 2, 100, Some("oxplow:oxplow/test?class=mod&name=a")),
            (1, 2, 2, 100, Some("oxplow:oxplow/test?class=mod&name=b")),
            (2, 0, 2, 400, None),
        ]),
        want
    );
}

#[test]
fn the_same_test_red_twice_is_one_distinct_test() {
    assert_eq!(
        outcome_over(&[
            (1, 1, 1, 100, Some("oxplow:oxplow/test?class=mod&name=a")),
            (2, 1, 1, 200, Some("oxplow:oxplow/test?class=mod&name=a")),
        ]),
        outcome([1.0, 1.0, 1.0, 2.0])
    );
    assert_eq!(
        outcome_over(&[
            (1, 1, 1, 100, Some("oxplow:oxplow/test?class=mod&name=a")),
            (2, 1, 1, 200, Some("oxplow:oxplow/test?class=mod&name=b")),
        ]),
        outcome([1.0, 1.0, 2.0, 2.0])
    );
}

#[test]
fn asserted_failures_have_no_identity_so_distinct_falls_back_to_the_peak() {
    // Three failures asserted as a count, no cases listed.
    assert_eq!(
        outcome_over(&[(1, 3, 0, 100, None)]),
        outcome([3.0, 3.0, 3.0, 1.0])
    );
    // One known and one anonymous in a run is two, not one.
    let want = outcome_over(&[
        (1, 2, 1, 100, Some("oxplow:oxplow/test?class=mod&name=a")),
        (2, 0, 1, 200, None),
    ]);
    assert_eq!(
        want[1].1, 1.0,
        "a report that lists its failures needs no fallback"
    );
}

#[test]
fn a_run_with_no_case_rows_still_counts() {
    // Run 1 failed a; run 2 a green repeat that listed no failed case.
    let mut want = outcome([0.0, 1.0, 1.0, 1.0]);
    want.push(time_to_green(5.0));
    assert_eq!(
        outcome_over(&[
            (1, 1, 2, 100, Some("oxplow:oxplow/test?class=mod&name=a")),
            (2, 0, 0, 105, None)
        ]),
        want
    );
}

#[test]
fn time_to_green_is_the_first_red_to_the_first_green_after_it() {
    // green @0, red @100, red @250, green @400 → 300, whatever follows.
    let mut want = outcome([1.0, 1.0, 2.0, 3.0]);
    want.push(time_to_green(300.0));
    assert_eq!(
        outcome_over(&[
            (1, 0, 1, 0, None),
            (2, 1, 1, 100, Some("oxplow:oxplow/test?class=mod&name=a")),
            (3, 1, 1, 250, Some("oxplow:oxplow/test?class=mod&name=a")),
            (4, 0, 1, 400, None),
            (5, 1, 1, 500, Some("oxplow:oxplow/test?class=mod&name=b")),
        ]),
        want
    );
}

#[test]
fn time_to_green_needs_a_red_then_a_green() {
    let only_outcome = |rows: &[(i64, i64, i64, i64, Option<&str>)]| {
        outcome_over(rows)
            .into_iter()
            .filter(|(m, ..)| m == "oxplow.effort_time_to_green")
            .count()
    };
    assert_eq!(
        only_outcome(&[(1, 0, 1, 5, None), (2, 0, 1, 9, None)]),
        0,
        "never red"
    );
    assert_eq!(
        only_outcome(&[
            (1, 1, 1, 5, Some("oxplow:oxplow/test?class=mod&name=a")),
            (2, 1, 1, 9, Some("oxplow:oxplow/test?class=mod&name=a"))
        ]),
        0,
        "never recovered"
    );
}
