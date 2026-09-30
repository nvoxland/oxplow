//! Co-change history: which files are usually committed together?
//!
//! Built from the commit index (`v_commit_file`, kept by the commit
//! indexer from every stream's head) rather than a walk of the VCS, it
//! accumulates two maps:
//!
//! - **co-change counts**: `(file_a, file_b) → how often they appear in
//!   the same commit`;
//! - **last-touched time**: `file → its most recent commit`.
//!
//! Given those and the files of the diff under review,
//! [`analyze_surprise`] classifies each file as `Normal`,
//! `UsualCoChangersAbsent` (historically it moves with X and Y, which
//! aren't in this change) or `Dormant` (untouched for a long time) — the
//! CodeScene `change_coupling` signal restated. Pure: no I/O.

use std::collections::{HashMap, HashSet};

use oxplow_db::Changeset;
use serde::{Deserialize, Serialize};
use specta::Type;

/// How many days back to look by default (~6 months): a couple of
/// release cycles for most projects.
pub const DEFAULT_WINDOW_DAYS: i64 = 180;

/// Co-occurrences before a file counts as a "usual co-changer". Fewer
/// is noise — every file co-occurs with the README once.
pub const DEFAULT_MIN_COOCCURRENCES: u32 = 3;

/// A file untouched for this many days (roughly a quarter) is dormant.
pub const DEFAULT_DORMANT_DAYS: i64 = 90;

/// Commits touching more files than this (mass renames, formatter
/// sweeps) are skipped; they would drown the signal.
pub const COMMIT_FILE_LIMIT: usize = 50;

/// Pre-aggregated co-change history. Built once, queried many times.
#[derive(Debug, Clone, Default)]
pub struct CoChangeHistory {
    /// `file → frequent co-changers`, most frequent first; only pairs
    /// seen at least `DEFAULT_MIN_COOCCURRENCES` times.
    co_changers: HashMap<String, Vec<(String, u32)>>,
    /// `file → most recent touch` (seconds since epoch).
    last_touched: HashMap<String, i64>,
    /// When the analysis ran (seconds since epoch), for dormancy.
    analyzed_at_secs: i64,
}

impl CoChangeHistory {
    /// Aggregate `changesets` (each a commit's files) as of `now_secs`.
    /// Empty changesets and ones over [`COMMIT_FILE_LIMIT`] files are
    /// skipped.
    pub fn from_changesets(changesets: &[Changeset], now_secs: i64) -> Self {
        let mut pair_counts: HashMap<(&str, &str), u32> = HashMap::new();
        let mut last_touched: HashMap<String, i64> = HashMap::new();
        for set in changesets {
            if set.paths.is_empty() || set.paths.len() > COMMIT_FILE_LIMIT {
                continue;
            }
            for path in &set.paths {
                let at = last_touched
                    .entry(path.clone())
                    .or_insert(set.committed_secs);
                *at = (*at).max(set.committed_secs);
            }
            for (i, a) in set.paths.iter().enumerate() {
                for b in &set.paths[i + 1..] {
                    let pair = if a < b {
                        (a.as_str(), b.as_str())
                    } else {
                        (b.as_str(), a.as_str())
                    };
                    *pair_counts.entry(pair).or_insert(0) += 1;
                }
            }
        }
        let mut co_changers: HashMap<String, Vec<(String, u32)>> = HashMap::new();
        for ((a, b), count) in pair_counts {
            if count < DEFAULT_MIN_COOCCURRENCES {
                continue;
            }
            co_changers
                .entry(a.into())
                .or_default()
                .push((b.into(), count));
            co_changers
                .entry(b.into())
                .or_default()
                .push((a.into(), count));
        }
        for list in co_changers.values_mut() {
            list.sort_by(|x, y| y.1.cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
        }
        Self {
            co_changers,
            last_touched,
            analyzed_at_secs: now_secs,
        }
    }

    /// Files with at least one recorded touch.
    pub fn file_count(&self) -> usize {
        self.last_touched.len()
    }

    /// Up to `max` most frequent co-changers of `file`.
    pub fn co_changers_for(&self, file: &str, max: usize) -> &[(String, u32)] {
        self.co_changers
            .get(file)
            .map(|v| &v[..v.len().min(max)])
            .unwrap_or(&[])
    }

    /// `file`'s most recent touch, if any.
    pub fn last_touched_secs(&self, file: &str) -> Option<i64> {
        self.last_touched.get(file).copied()
    }
}

/// Why a file was flagged as surprising.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SurpriseReason {
    /// Nothing surprising: no strong co-changers, or they're in this
    /// change too.
    Normal,
    /// The file has well-established co-changers, none of which are in
    /// this change; carries the top three for the tooltip.
    UsualCoChangersAbsent { expected: Vec<String> },
    /// Untouched for `last_touched_days`.
    Dormant { last_touched_days: i64 },
}

impl SurpriseReason {
    pub fn is_surprising(&self) -> bool {
        !matches!(self, SurpriseReason::Normal)
    }
}

/// One row of [`analyze_surprise`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FileSurprise {
    pub path: String,
    pub reason: SurpriseReason,
}

/// Classify each of `files` (the diff under review) against `history`,
/// in order. Dormancy is checked first — the cheaper, clearer signal; a
/// file with no recorded touch counts as dormant.
pub fn analyze_surprise(
    history: &CoChangeHistory,
    files: &[String],
    dormant_days: i64,
) -> Vec<FileSurprise> {
    let touched: HashSet<&str> = files.iter().map(String::as_str).collect();
    files
        .iter()
        .map(|file| {
            let reason = match history.last_touched_secs(file) {
                None => SurpriseReason::Dormant {
                    last_touched_days: dormant_days.max(1),
                },
                Some(ts) if (history.analyzed_at_secs - ts) / 86_400 >= dormant_days => {
                    SurpriseReason::Dormant {
                        last_touched_days: (history.analyzed_at_secs - ts) / 86_400,
                    }
                }
                Some(_) => {
                    let usual = history.co_changers_for(file, 3);
                    if usual.is_empty() || usual.iter().any(|(o, _)| touched.contains(o.as_str())) {
                        SurpriseReason::Normal
                    } else {
                        SurpriseReason::UsualCoChangersAbsent {
                            expected: usual.iter().map(|(o, _)| o.clone()).collect(),
                        }
                    }
                }
            };
            FileSurprise {
                path: file.clone(),
                reason,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    fn set(sha: &str, days_ago: i64, paths: &[&str]) -> Changeset {
        Changeset {
            sha: sha.into(),
            committed_secs: NOW - days_ago * DAY,
            paths: paths.iter().map(|p| (*p).to_string()).collect(),
        }
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn no_history_makes_every_file_dormant() {
        let history = CoChangeHistory::from_changesets(&[], NOW);
        assert_eq!(history.file_count(), 0);
        let result = analyze_surprise(&history, &strings(&["any.rs"]), DEFAULT_DORMANT_DAYS);
        assert!(matches!(result[0].reason, SurpriseReason::Dormant { .. }));
    }

    #[test]
    fn a_file_without_its_usual_co_changer_is_surprising() {
        let mut sets: Vec<_> = (0..4)
            .map(|i| set(&format!("ab{i}"), i, &["a.rs", "b.rs"]))
            .collect();
        sets.push(set("c", 1, &["c.rs"]));
        let history = CoChangeHistory::from_changesets(&sets, NOW);
        assert_eq!(
            history.co_changers_for("a.rs", 3),
            &[("b.rs".to_string(), 4)]
        );

        let together =
            analyze_surprise(&history, &strings(&["a.rs", "b.rs"]), DEFAULT_DORMANT_DAYS);
        assert!(together.iter().all(|f| f.reason == SurpriseReason::Normal));

        let alone = analyze_surprise(&history, &strings(&["a.rs", "c.rs"]), DEFAULT_DORMANT_DAYS);
        assert_eq!(
            alone[0].reason,
            SurpriseReason::UsualCoChangersAbsent {
                expected: strings(&["b.rs"])
            }
        );
        assert_eq!(alone[1].reason, SurpriseReason::Normal);
    }

    #[test]
    fn a_long_untouched_file_is_dormant_whatever_its_co_changers() {
        let sets: Vec<_> = (0..3)
            .map(|i| set(&format!("ab{i}"), 200 + i, &["a.rs", "b.rs"]))
            .collect();
        let history = CoChangeHistory::from_changesets(&sets, NOW);
        let result = analyze_surprise(&history, &strings(&["a.rs"]), DEFAULT_DORMANT_DAYS);
        assert_eq!(
            result[0].reason,
            SurpriseReason::Dormant {
                last_touched_days: 200
            }
        );
    }

    #[test]
    fn mass_commits_contribute_nothing() {
        let paths: Vec<String> = (0..COMMIT_FILE_LIMIT + 5)
            .map(|i| format!("f{i}.rs"))
            .collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let sets: Vec<_> = (0..4).map(|i| set(&format!("mass{i}"), i, &refs)).collect();
        let history = CoChangeHistory::from_changesets(&sets, NOW);
        assert_eq!(history.file_count(), 0);
        assert!(history.co_changers_for("f0.rs", 3).is_empty());
    }
}
