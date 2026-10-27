//! What oxplow-foundation's effort test outcome collector's script makes of
//! the rows its input yields. (The code gauges' goldens and the collector run
//! over recorded test runs are fixtures in the foundation.)

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
