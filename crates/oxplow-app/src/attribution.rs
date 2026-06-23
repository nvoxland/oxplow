//! Kind-agnostic claim → verify → reconcile attribution (epic tsk260).
//!
//! Generalizes the file claim-first model (`.context/agent-model.md`) so any
//! fact-kind — `file`, `test-run`, `coverage`, … — shares one engine. The agent
//! **claims** what it did, oxplow **observes** independently, and the
//! turn/effort boundary **reconciles** the two, surfacing discrepancies for
//! confirmation ("changed but not claimed → you, or someone else?"). This needs
//! no visibility into sub-agents and absorbs concurrent users/threads: anything
//! observed-but-unclaimed is presumed not-the-agent's.
//!
//! Per `(effort, kind, ref)` there are four input sets — **claimed**,
//! **observed**, **acknowledged** (explicitly disclaimed), **other-claimed**
//! (owned by an intervening effort). [`diff`] turns them into the review and
//! [`unattributed`] into the residue persisted on close. A [`AttributionKind`]
//! supplies the sets + the agent-facing wording; [`FileKind`] is the reference
//! implementation over the existing file-attribution storage (no migration).

use async_trait::async_trait;

use oxplow_db::{
    SqliteAttributionStore, SqliteMetricStore, SqliteSnapshotStore, SqliteTaskEffortStore,
    TaskEffortStore as _, STATE_ACKNOWLEDGED, STATE_CLAIMED,
};
use oxplow_domain::EffortId;

/// Cap on the observed-but-unclaimed list surfaced to the agent per kind. Above
/// this, something else is happening (overlapping efforts, formatter, codegen,
/// user/other-actor edits) and the agent can't triage a wall of items.
pub const MAX_UNCLAIMED_FOR_REVIEW: usize = 10;

/// The four input sets for one kind's reconciliation of an effort.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AttrSets {
    /// What the agent claimed it did (auto-claim + boundary `touched_files`).
    pub claimed: Vec<String>,
    /// What oxplow independently observed happened in the effort window.
    pub observed: Vec<String>,
    /// Items the agent explicitly disclaimed (so they don't re-flag).
    pub acknowledged: Vec<String>,
    /// Items an intervening effort already owns (not this agent's to claim).
    pub other_claimed: Vec<String>,
}

/// One kind's reconciliation result for an effort (the non-empty discrepancy).
#[derive(Debug, Clone, PartialEq)]
pub struct KindReview {
    pub kind: &'static str,
    /// Items the agent claimed that oxplow didn't observe.
    pub claimed_but_not_observed: Vec<String>,
    /// Items oxplow observed that the agent didn't claim (capped).
    pub observed_but_not_claimed: Vec<String>,
    /// Pre-cap count of `observed_but_not_claimed` when capped, else `None`.
    pub overflow: Option<usize>,
}

/// The kind-agnostic differ — the shared core of every kind's reconcile. Returns
/// `None` when the claim and the observation agree (nothing to surface).
pub fn diff(sets: &AttrSets, cap: usize) -> Option<(Vec<String>, Vec<String>, Option<usize>)> {
    use std::collections::HashSet;
    let claimed: HashSet<&str> = sets.claimed.iter().map(String::as_str).collect();
    let observed: HashSet<&str> = sets.observed.iter().map(String::as_str).collect();
    let ack: HashSet<&str> = sets.acknowledged.iter().map(String::as_str).collect();
    let other: HashSet<&str> = sets.other_claimed.iter().map(String::as_str).collect();

    let mut claimed_but_not_observed: Vec<String> = claimed
        .difference(&observed)
        .map(|s| (*s).to_string())
        .collect();
    // An observed item another effort already claimed (it finished inside this
    // effort's window) isn't this agent's to claim — drop it.
    let mut observed_but_not_claimed: Vec<String> = observed
        .difference(&claimed)
        .filter(|s| !ack.contains(*s) && !other.contains(*s))
        .map(|s| (*s).to_string())
        .collect();
    claimed_but_not_observed.sort();
    observed_but_not_claimed.sort();
    if claimed_but_not_observed.is_empty() && observed_but_not_claimed.is_empty() {
        return None;
    }
    let overflow = if observed_but_not_claimed.len() > cap {
        let total = observed_but_not_claimed.len();
        observed_but_not_claimed.clear();
        Some(total)
    } else {
        None
    };
    Some((claimed_but_not_observed, observed_but_not_claimed, overflow))
}

/// The residue set (`observed − claimed − acknowledged − other-claimed`),
/// sorted/deduped, **uncapped** — what gets persisted as "unattributed" on
/// close so an out-of-band close can't leave external writes looking authored.
pub fn unattributed(sets: &AttrSets) -> Vec<String> {
    use std::collections::HashSet;
    let claimed: HashSet<&str> = sets.claimed.iter().map(String::as_str).collect();
    let ack: HashSet<&str> = sets.acknowledged.iter().map(String::as_str).collect();
    let other: HashSet<&str> = sets.other_claimed.iter().map(String::as_str).collect();
    let mut out: Vec<String> = sets
        .observed
        .iter()
        .filter(|s| {
            !claimed.contains(s.as_str())
                && !ack.contains(s.as_str())
                && !other.contains(s.as_str())
        })
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

/// A reconcilable fact-kind: gathers the four sets for an effort, persists the
/// unattributed residue, and renders the agent-facing wording. Implemented by
/// [`FileKind`] today; `RunKind`/`CoverageKind`/… plug in the same way.
#[async_trait]
pub trait AttributionKind: Send + Sync {
    fn kind(&self) -> &'static str;
    fn cap(&self) -> usize {
        MAX_UNCLAIMED_FOR_REVIEW
    }
    /// The four input sets for `effort_id`. `None` ⇒ skip this kind (e.g. no
    /// snapshot bracket / no observation basis yet).
    async fn gather(&self, effort_id: &EffortId) -> Option<AttrSets>;
    /// Persist the unattributed residue at close. Returns `false` on error
    /// (best-effort; the close never blocks).
    async fn persist_unattributed(&self, effort_id: &EffortId, refs: &[String]) -> bool;
}

/// Reconcile one kind for an effort at close: gather → persist residue. Returns
/// the residue (the `observed-but-unclaimed` set), or empty when there's no
/// observation basis or persistence failed.
pub async fn reconcile_close(kind: &dyn AttributionKind, effort_id: &EffortId) -> Vec<String> {
    let Some(sets) = kind.gather(effort_id).await else {
        return Vec::new();
    };
    let residue = unattributed(&sets);
    if !kind.persist_unattributed(effort_id, &residue).await {
        return Vec::new();
    }
    residue
}

/// Reference kind: files, over the EXISTING claim-first storage
/// (`task_effort_file`, `effort_unattributed_file`, acknowledged, intervening,
/// and the snapshot-bracket diff). No data migration — behavior-identical to
/// the pre-generalization path.
pub struct FileKind<'a> {
    pub efforts: &'a SqliteTaskEffortStore,
    pub snapshots: &'a SqliteSnapshotStore,
}

impl<'a> FileKind<'a> {
    pub fn new(efforts: &'a SqliteTaskEffortStore, snapshots: &'a SqliteSnapshotStore) -> Self {
        Self { efforts, snapshots }
    }
}

#[async_trait]
impl AttributionKind for FileKind<'_> {
    fn kind(&self) -> &'static str {
        "file"
    }

    async fn gather(&self, effort_id: &EffortId) -> Option<AttrSets> {
        let effort = self.efforts.get_effort(effort_id).await.ok().flatten()?;
        // OBSERVE = the snapshot-bracket content diff; `None` (skip) when the
        // effort has no start/end snapshot pin yet.
        let (start, end) = (effort.start_snapshot_id?, effort.end_snapshot_id?);
        let observed: Vec<String> = self
            .snapshots
            .diff_snapshots(Some(start), end)
            .await
            .ok()?
            .into_iter()
            .map(|c| c.path)
            .collect();
        // CLAIM / acknowledged / other-claimed read from the file stores;
        // errors degrade to empty (matching the pre-refactor close path).
        let claimed: Vec<String> = self
            .efforts
            .list_files(effort_id)
            .await
            .map(|fs| fs.into_iter().map(|f| f.path).collect())
            .unwrap_or_default();
        let acknowledged = self
            .efforts
            .list_acknowledged_paths(effort_id)
            .await
            .unwrap_or_default();
        let other_claimed = self
            .efforts
            .paths_claimed_by_intervening_efforts(effort_id)
            .await
            .unwrap_or_default();
        Some(AttrSets {
            claimed,
            observed,
            acknowledged,
            other_claimed,
        })
    }

    async fn persist_unattributed(&self, effort_id: &EffortId, refs: &[String]) -> bool {
        self.efforts
            .replace_unattributed_files(effort_id, refs)
            .await
            .is_ok()
    }
}

/// Run kinds (`test-run`, `coverage`, `analysis`): events oxplow OBSERVES as
/// `metric_run` rows in the effort's thread+time window, attributed via the
/// generic ledger (`effort_attribution`). Unlike files, a run isn't an object
/// you diff — oxplow can see *that* tests ran and *what* they returned, but not
/// which sub-agent/effort, so the boundary claim resolves it. `ref` = the
/// `metric_run` id as `run:<id>`. One struct serves every run producer.
pub struct RunKind<'a> {
    pub efforts: &'a SqliteTaskEffortStore,
    pub metrics: &'a SqliteMetricStore,
    pub ledger: &'a SqliteAttributionStore,
    /// Attribution kind name, e.g. `test-run`.
    pub kind: &'static str,
    /// The `metric_run.producer` to observe, e.g. `tests`.
    pub producer: &'static str,
}

impl<'a> RunKind<'a> {
    /// Test runs (`producer = "tests"`, kind `"test-run"`).
    pub fn tests(
        efforts: &'a SqliteTaskEffortStore,
        metrics: &'a SqliteMetricStore,
        ledger: &'a SqliteAttributionStore,
    ) -> Self {
        Self {
            efforts,
            metrics,
            ledger,
            kind: "test-run",
            producer: "tests",
        }
    }
}

#[async_trait]
impl AttributionKind for RunKind<'_> {
    fn kind(&self) -> &'static str {
        self.kind
    }

    async fn gather(&self, effort_id: &EffortId) -> Option<AttrSets> {
        let effort = self.efforts.get_effort(effort_id).await.ok().flatten()?;
        // OBSERVE = this producer's runs on the effort's thread in its time
        // window. (A run a concurrent effort owns falls in BOTH windows; the
        // cross-effort dedup below keeps it off this effort's residue.)
        let runs = self
            .metrics
            .runs_in_window(
                effort.thread_id.value(),
                self.producer,
                effort.started_at,
                effort.ended_at,
            )
            .await
            .ok()?;
        let observed: Vec<String> = runs.iter().map(|r| format!("run:{}", r.id)).collect();
        let claimed = self
            .ledger
            .list_refs(effort_id, self.kind, STATE_CLAIMED)
            .await
            .unwrap_or_default();
        let acknowledged = self
            .ledger
            .list_refs(effort_id, self.kind, STATE_ACKNOWLEDGED)
            .await
            .unwrap_or_default();
        let other_claimed = self
            .ledger
            .refs_claimed_by_other_efforts(effort_id, self.kind)
            .await
            .unwrap_or_default();
        Some(AttrSets {
            claimed,
            observed,
            acknowledged,
            other_claimed,
        })
    }

    async fn persist_unattributed(&self, effort_id: &EffortId, refs: &[String]) -> bool {
        self.ledger
            .replace_unattributed(effort_id, self.kind, refs)
            .await
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sets(claimed: &[&str], observed: &[&str], ack: &[&str], other: &[&str]) -> AttrSets {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
        AttrSets {
            claimed: v(claimed),
            observed: v(observed),
            acknowledged: v(ack),
            other_claimed: v(other),
        }
    }

    #[test]
    fn diff_reports_both_discrepancy_directions_sorted() {
        let s = sets(&["a", "z"], &["a", "b"], &[], &[]);
        let (cbno, obnc, overflow) = diff(&s, MAX_UNCLAIMED_FOR_REVIEW).unwrap();
        assert_eq!(cbno, vec!["z"]); // claimed, not observed
        assert_eq!(obnc, vec!["b"]); // observed, not claimed
        assert_eq!(overflow, None);
    }

    #[test]
    fn diff_none_when_claim_matches_observation() {
        assert!(diff(
            &sets(&["a", "b"], &["a", "b"], &[], &[]),
            MAX_UNCLAIMED_FOR_REVIEW
        )
        .is_none());
    }

    #[test]
    fn diff_drops_acknowledged_and_other_claimed_from_unclaimed() {
        // c observed-but-unclaimed but acknowledged; d owned by another effort.
        let s = sets(&["a"], &["a", "c", "d"], &["c"], &["d"]);
        assert!(diff(&s, MAX_UNCLAIMED_FOR_REVIEW).is_none());
    }

    #[test]
    fn diff_caps_unclaimed_and_reports_overflow() {
        let observed: Vec<&str> = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"].to_vec();
        let (_cbno, obnc, overflow) =
            diff(&sets(&[], &observed, &[], &[]), MAX_UNCLAIMED_FOR_REVIEW).unwrap();
        assert!(obnc.is_empty(), "list cleared when over the cap");
        assert_eq!(overflow, Some(11));
    }

    #[test]
    fn unattributed_is_uncapped_residue() {
        let observed: Vec<&str> = (0..15).map(|_| "x").collect();
        // dedup collapses the 15 "x" to one; claimed/ack/other subtract.
        let s = sets(&["claimed1"], &observed, &[], &[]);
        assert_eq!(unattributed(&s), vec!["x"]);
    }
}
