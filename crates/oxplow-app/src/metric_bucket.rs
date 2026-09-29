//! Time buckets for a metric series (tsk321): collapse the per-capture points
//! of a day / week / month into one point, by the measure's rule over time —
//! the last capture for a level (semi-additive), the sum for events
//! (additive), the mean otherwise (non-additive). Ratios re-divide their summed
//! parts instead of averaging percentages. See `.context/metrics.md`.

use std::collections::BTreeMap;

use oxplow_domain::Timestamp;
use serde::{Deserialize, Serialize};

use crate::metric_engine::{Aggregation, SeriesPoint, Temporal};

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
/// capture (for drill-in) and that capture's branch / version / provenance.
pub fn bucket_series(
    points: Vec<SeriesPoint>,
    bucket: TimeBucket,
    temporal: Temporal,
    agg: Aggregation,
) -> Vec<SeriesPoint> {
    let mut groups: BTreeMap<(Timestamp, Option<String>), Vec<SeriesPoint>> = BTreeMap::new();
    for p in points {
        groups
            .entry((bucket.start_of(p.captured_at), p.group.clone()))
            .or_default()
            .push(p);
    }
    groups
        .into_iter()
        .map(|((start, _), mut ps)| {
            ps.sort_by_key(|p| (p.captured_at, p.capture_id));
            let (value, numerator, denominator) = collapse(&ps, temporal, agg);
            let last = ps.pop().expect("a bucket holds at least one point");
            SeriesPoint {
                captured_at: start,
                value,
                numerator,
                denominator,
                ..last
            }
        })
        .collect()
}

fn collapse(
    ps: &[SeriesPoint],
    temporal: Temporal,
    agg: Aggregation,
) -> (f64, Option<f64>, Option<f64>) {
    let last = ps.last().expect("non-empty bucket");
    if temporal == Temporal::SemiAdditive {
        return (last.value, last.numerator, last.denominator);
    }
    if agg == Aggregation::Ratio {
        let num: f64 = ps.iter().filter_map(|p| p.numerator).sum();
        let den: f64 = ps.iter().filter_map(|p| p.denominator).sum();
        let value = if den == 0.0 { 0.0 } else { num / den };
        return (value, Some(num), Some(den));
    }
    let sum: f64 = ps.iter().map(|p| p.value).sum();
    match temporal {
        Temporal::Additive => (sum, None, None),
        _ => (sum / ps.len() as f64, None, None),
    }
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
            git_version: None,
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
    fn events_sum_levels_take_the_last_and_the_rest_average() {
        let ps = || {
            vec![
                pt(1, "2026-09-21T09:00:00Z", 2.0, None),
                pt(3, "2026-09-21T17:00:00Z", 6.0, None),
                pt(2, "2026-09-21T12:00:00Z", 4.0, None),
                pt(4, "2026-09-22T09:00:00Z", 1.0, None),
            ]
        };
        let day = |t| values(&bucket_series(ps(), TimeBucket::Day, t, Aggregation::Sum));
        assert_eq!(
            day(Temporal::Additive),
            vec![
                ("2026-09-21".into(), None, 12.0, 3),
                ("2026-09-22".into(), None, 1.0, 4)
            ]
        );
        assert_eq!(day(Temporal::SemiAdditive)[0].2, 6.0);
        assert_eq!(day(Temporal::NonAdditive)[0].2, 4.0);
        let week = bucket_series(ps(), TimeBucket::Week, Temporal::Additive, Aggregation::Sum);
        assert_eq!(values(&week), vec![("2026-09-21".into(), None, 13.0, 4)]);
    }

    #[test]
    fn groups_bucket_separately_and_ratios_redivide_their_parts() {
        let mut a = pt(1, "2026-09-21T09:00:00Z", 0.5, Some("x"));
        (a.numerator, a.denominator) = (Some(1.0), Some(2.0));
        let mut b = pt(2, "2026-09-21T10:00:00Z", 0.9, Some("x"));
        (b.numerator, b.denominator) = (Some(9.0), Some(10.0));
        let c = pt(3, "2026-09-21T11:00:00Z", 7.0, Some("y"));
        let out = bucket_series(
            vec![a, b, c],
            TimeBucket::Day,
            Temporal::NonAdditive,
            Aggregation::Ratio,
        );
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
