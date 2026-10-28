//! Time buckets for a metric series: collapse the per-capture points of a
//! day / week / month into one point by the measure's declared `fold`
//! ([`crate::metric_engine::fold`], the same rule a headline applies). A
//! bucket the fold has no answer for is left out. See `.context/metrics.md`.

use std::collections::BTreeMap;

use oxplow_domain::Timestamp;
use serde::{Deserialize, Serialize};

use crate::metric_engine::{fold, Fold, SeriesPoint};

/// A calendar bucket (UTC). Weeks start on Monday.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum TimeBucket {
    Day,
    Week,
    Month,
}

impl TimeBucket {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "day" => Self::Day,
            "week" => Self::Week,
            "month" => Self::Month,
            _ => return None,
        })
    }

    /// The start (UTC midnight) of the bucket `at` falls in.
    pub fn start_of(self, at: Timestamp) -> Timestamp {
        let date = at.0.to_offset(time::UtcOffset::UTC).date();
        let first = match self {
            Self::Day => date,
            Self::Week => {
                date - time::Duration::days(date.weekday().number_days_from_monday() as i64)
            }
            Self::Month => date.replace_day(1).expect("day 1 exists in every month"),
        };
        Timestamp(first.midnight().assume_utc())
    }
}

/// Collapse `points` into one point per (bucket, group), oldest first. Each
/// output point is stamped at its bucket's start and carries the bucket's last
/// capture (for drill-in) and that capture's branch / version / provenance. A
/// bucket the `how` fold has no answer for is dropped.
pub fn bucket_series(points: Vec<SeriesPoint>, bucket: TimeBucket, how: Fold) -> Vec<SeriesPoint> {
    let mut groups: BTreeMap<(Timestamp, Option<String>), Vec<SeriesPoint>> = BTreeMap::new();
    for p in points {
        groups
            .entry((bucket.start_of(p.captured_at), p.group.clone()))
            .or_default()
            .push(p);
    }
    groups
        .into_iter()
        .filter_map(|((start, _), mut ps)| {
            ps.sort_by_key(|p| (p.captured_at, p.capture_id));
            let folded = fold(&ps, how)?;
            let last = ps.pop().expect("a bucket holds at least one point");
            Some(SeriesPoint {
                captured_at: start,
                value: folded.value,
                numerator: folded.numerator,
                denominator: folded.denominator,
                ..last
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        serde_json::from_str(&format!("\"{s}\"")).unwrap()
    }

    fn pt(id: i64, at: &str, value: f64, group: Option<&str>) -> SeriesPoint {
        SeriesPoint {
            capture_id: id,
            captured_at: ts(at),
            value,
            numerator: None,
            denominator: None,
            group: group.map(str::to_string),
            branch: None,
            provenance: None,
            vcs_rev: None,
            source: None,
        }
    }

    fn values(ps: &[SeriesPoint]) -> Vec<(String, Option<String>, f64, i64)> {
        ps.iter()
            .map(|p| {
                let at = serde_json::to_value(p.captured_at).unwrap();
                (
                    at.as_str().unwrap()[..10].to_string(),
                    p.group.clone(),
                    p.value,
                    p.capture_id,
                )
            })
            .collect()
    }

    #[test]
    fn bucket_starts_are_utc_midnight_day_monday_and_first_of_month() {
        let at = ts("2026-09-27T18:30:00Z"); // a Sunday
        let s = |b: TimeBucket| serde_json::to_value(b.start_of(at)).unwrap();
        assert_eq!(s(TimeBucket::Day), "2026-09-27T00:00:00.000000Z");
        assert_eq!(s(TimeBucket::Week), "2026-09-21T00:00:00.000000Z");
        assert_eq!(s(TimeBucket::Month), "2026-09-01T00:00:00.000000Z");
    }

    #[test]
    fn events_sum_levels_take_the_last_and_means_average() {
        let ps = || {
            vec![
                pt(1, "2026-09-21T09:00:00Z", 2.0, None),
                pt(3, "2026-09-21T17:00:00Z", 6.0, None),
                pt(2, "2026-09-21T12:00:00Z", 4.0, None),
                pt(4, "2026-09-22T09:00:00Z", 1.0, None),
            ]
        };
        let day = |f| values(&bucket_series(ps(), TimeBucket::Day, f));
        assert_eq!(
            day(Fold::Sum),
            vec![
                ("2026-09-21".into(), None, 12.0, 3),
                ("2026-09-22".into(), None, 1.0, 4)
            ]
        );
        assert_eq!(day(Fold::Last)[0].2, 6.0);
        assert_eq!(day(Fold::First)[0].2, 2.0);
        assert_eq!(day(Fold::Avg)[0].2, 4.0);
        assert!(day(Fold::None).is_empty(), "no fold, no answer, no bucket");
        let week = bucket_series(ps(), TimeBucket::Week, Fold::Sum);
        assert_eq!(values(&week), vec![("2026-09-21".into(), None, 13.0, 4)]);
    }

    /// A chart's bucket and a grid's headline are one rule: every fold, on
    /// the same points, gives the same number — including a ratio whose
    /// denominators total 0, which is no data in both, never 0.
    #[test]
    fn a_bucket_and_a_headline_agree_on_every_fold() {
        let plain = || {
            vec![
                pt(1, "2026-09-21T09:00:00Z", 2.0, None),
                pt(2, "2026-09-21T12:00:00Z", 4.0, None),
                pt(3, "2026-09-21T17:00:00Z", 6.0, None),
            ]
        };
        let with = |parts: &[(f64, f64)]| {
            parts
                .iter()
                .enumerate()
                .map(|(i, (n, d))| {
                    let shown = if *d == 0.0 { 0.0 } else { n / d };
                    let mut p = pt(i as i64 + 1, "2026-09-21T09:00:00Z", shown, None);
                    (p.numerator, p.denominator) = (Some(*n), Some(*d));
                    p
                })
                .collect::<Vec<_>>()
        };
        let cases: Vec<(&str, Vec<SeriesPoint>)> = vec![
            ("plain", plain()),
            ("ratio", with(&[(1.0, 2.0), (9.0, 10.0)])),
            ("empty denominators", with(&[(0.0, 0.0), (0.0, 0.0)])),
            ("numerator only", {
                let mut p = pt(1, "2026-09-21T09:00:00Z", 5.0, None);
                (p.numerator, p.denominator) = (Some(5.0), Some(0.0));
                vec![p]
            }),
        ];
        for (name, points) in cases {
            for how in [Fold::Sum, Fold::Last, Fold::First, Fold::Avg, Fold::None] {
                let bucketed = bucket_series(points.clone(), TimeBucket::Day, how);
                let headline = crate::metric_engine::range_value(&points, how);
                assert_eq!(
                    bucketed.first().map(|p| p.value),
                    headline,
                    "{name} / {how:?}"
                );
                assert_eq!(
                    bucketed.len(),
                    usize::from(headline.is_some()),
                    "{name} / {how:?}"
                );
            }
        }
        // Σden = 0 is no data under sum and avg; the latest point still reads.
        let empty = with(&[(0.0, 0.0), (0.0, 0.0)]);
        assert_eq!(crate::metric_engine::range_value(&empty, Fold::Sum), None);
        assert_eq!(crate::metric_engine::range_value(&empty, Fold::Avg), None);
    }

    #[test]
    fn groups_bucket_separately_and_ratios_redivide_their_parts() {
        let mut a = pt(1, "2026-09-21T09:00:00Z", 0.5, Some("x"));
        (a.numerator, a.denominator) = (Some(1.0), Some(2.0));
        let mut b = pt(2, "2026-09-21T10:00:00Z", 0.9, Some("x"));
        (b.numerator, b.denominator) = (Some(9.0), Some(10.0));
        let c = pt(3, "2026-09-21T11:00:00Z", 7.0, Some("y"));
        let out = bucket_series(vec![a, b, c], TimeBucket::Day, Fold::Sum);
        assert_eq!(out.len(), 2);
        // (1 + 9) / (2 + 10), not the mean of 50% and 90%.
        assert_eq!(
            (
                out[0].group.as_deref(),
                out[0].numerator,
                out[0].denominator
            ),
            (Some("x"), Some(10.0), Some(12.0))
        );
        assert!((out[0].value - 10.0 / 12.0).abs() < 1e-9);
        assert_eq!(out[1].group.as_deref(), Some("y"));
    }
}
