//! Kind-agnostic claim → verify → reconcile attribution (epic tsk260).
//!
//! Generalizes the file claim-first model (`.context/agent-model.md`) so any
//! fact-kind — `file`, the unified `run` (tests/analysis/coverage), … — shares
//! one engine. The agent
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
    MetricSpec, SqliteAttributionStore, SqliteSnapshotStore, SqliteTaskEffortStore,
    TaskEffortStore as _, STATE_ACKNOWLEDGED, STATE_CLAIMED,
};
use oxplow_domain::EffortId;

/// Cap on the observed-but-unclaimed list surfaced to the agent per kind. Above
/// this, something else is happening (overlapping efforts, formatter, codegen,
/// user/other-actor edits) and the agent can't triage a wall of items.
pub const MAX_UNCLAIMED_FOR_REVIEW: usize = 10;

/// Which **read-side attribution family** a metric SPEC belongs to — the
/// single source of truth for how an effort's delta for that metric is computed
/// AND which claim-set (the write/reconcile path's [`AttributionKind`]) backs it.
/// Adding a new fact-kind is a variant here + one match arm in
/// `CollectionService::effort_metric_deltas` — not edits scattered across an
/// if/else chain plus predicate fns that can silently drift (the analysis
/// mis-routing bug, tsk272/tsk274).
///
/// Family ↔ the write-side [`AttributionKind`] it reads:
/// - [`File`](Self::File) ↔ [`FileKind`] — the effort's `task_effort_file` claims
/// - [`Coverage`](Self::Coverage) / [`Run`](Self::Run) ↔ [`RunKind`] — the
///   effort's `effort_attribution` ledger `"run"` claims
/// - [`Window`](Self::Window) ↔ no claim — operational thread+time facts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortAttributionFamily {
    /// Per-file code gauges: Σ over the effort's CLAIMED FILES of each file's
    /// `(current − baseline)`. Backed by [`FileKind`].
    File,
    /// Coverage: effort-relative — the diff is DERIVED at read against the
    /// effort's start snapshot (`coverage_delta`), a documented special case still
    /// on the legacy detail payload (line-sets aren't in facts yet). Backed by
    /// [`RunKind`].
    Coverage,
    /// Other run-kind facts (tests, analysis): before→after / `sum` over the facts
    /// of the effort's own captures (`metric_capture.effort_id`, stamped at ingest
    /// — tsk37). Backed by [`RunKind`].
    Run,
    /// Operational / everything else (tokens, nudges, cycle-time): before→after /
    /// `sum` over the facts of the effort's own captures. Read identically to
    /// [`Run`](Self::Run) now that captures carry `effort_id`; kept a distinct
    /// family only to document that it has no run-claim write side.
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
///    branch (the tsk272 regression) even though its facts are path-grained;
/// 5. a snapshot-scan gauge spec (display `gauge`/`findings`, a source measure,
///    no formula) → [`File`](EffortAttributionFamily::File). Gauge captures are
///    never effort-stamped (routing these by their `static-quality` category to
///    Run made every bundled code metric vanish from effort rollups — tsk43);
///    the File read attributes path-grained facts by the effort's claimed files
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
/// thread+window facts, never per-file gauges. Shared by the classifier above and
/// the effort-metric prompt context (`effort_metric_context`), which skips them.
pub fn is_operational_metric_key(key: &str) -> bool {
    key.starts_with("agent.") || key.starts_with("effort.") || key.starts_with("task.")
}

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

/// The unified run kind `"run"` — every agent-work run CAPTURE (tests,
/// coverage, analysis) oxplow OBSERVES in the effort's thread+time window
/// (filtered by `trigger = "on-report"`), attributed via the generic ledger
/// (`effort_attribution`). Unlike files, a run isn't an object you diff — oxplow
/// can see *that* it ran and *what* it returned, but not which sub-agent/effort,
/// so the boundary claim resolves it. The capture IS the run (T-E1, tsk48):
/// `ref` = the `metric_capture` id as `run:<id>`.
pub struct RunKind<'a> {
    pub efforts: &'a SqliteTaskEffortStore,
    pub facts: &'a oxplow_db::SqliteFactStore,
    pub ledger: &'a SqliteAttributionStore,
    /// Attribution kind name — `"run"` (the unified run kind, tsk269).
    pub kind: &'static str,
    /// The capture `trigger` to observe — `"on-report"`, which every
    /// agent-work run (tests/coverage/analysis) stamps regardless of its
    /// (per-analyzer, varying) producer. One filter captures all three.
    pub trigger: &'static str,
}

impl<'a> RunKind<'a> {
    /// All agent-work runs — tests, coverage, analysis — under one kind `"run"`,
    /// observed by `trigger = "on-report"`. Attribution is per-capture,
    /// producer-agnostic; the producer only drives rendering.
    pub fn runs(
        efforts: &'a SqliteTaskEffortStore,
        facts: &'a oxplow_db::SqliteFactStore,
        ledger: &'a SqliteAttributionStore,
    ) -> Self {
        Self {
            efforts,
            facts,
            ledger,
            kind: "run",
            trigger: "on-report",
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
        // OBSERVE = the run captures on the effort's thread in its time window.
        // (A run a concurrent effort owns falls in BOTH windows; the
        // cross-effort dedup below keeps it off this effort's residue.)
        let runs = self
            .facts
            .captures_in_window_by_trigger(
                effort.thread_id.value(),
                self.trigger,
                effort.started_at,
                effort.ended_at,
            )
            .await
            .ok()?;
        let claimed = self
            .ledger
            .list_refs(effort_id, self.kind, STATE_CLAIMED)
            .await
            .unwrap_or_default();
        // Window-dominance (tsk267): a run that falls inside a strictly-nested
        // sibling effort's window is that *narrower* effort's to own, so drop it
        // from this (wider) effort's observed set — UNLESS this effort explicitly
        // claimed it (an explicit claim always beats the geometric heuristic).
        // Truly-overlapping (non-nested) siblings have no dominant effort, so the
        // run stays observed by both and the agent disambiguates via its claim.
        let nested = self
            .efforts
            .nested_efforts(effort_id)
            .await
            .unwrap_or_default();
        let claimed_set: std::collections::HashSet<&str> =
            claimed.iter().map(String::as_str).collect();
        let observed: Vec<String> = runs
            .iter()
            .filter_map(|r| {
                let ref_ = format!("run:{}", r.id);
                let dominated = nested.iter().any(|f| match f.ended_at {
                    Some(end) => {
                        let t = r.captured_at.unix_ms();
                        t >= f.started_at.unix_ms() && t <= end.unix_ms()
                    }
                    None => false,
                });
                if dominated && !claimed_set.contains(ref_.as_str()) {
                    None
                } else {
                    Some(ref_)
                }
            })
            .collect();
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
        // Analysis MUST be Run, not File (the tsk272 regression guard): it is a
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
        // drop it from every effort rollup (tsk43).
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
        // paths would read 0/0 and silently drop the row, tsk43).
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

// ---- target scoring: which open effort does this run / edit belong to? ----
//
// Shared by BOTH attribution call sites (tsk186): the run auto-claim in
// `collection.rs` and the per-file auto-claim in `task_service.rs`. One
// implementation on purpose — two copies of "which effort owns this" would
// drift, and the whole point is that a file claim and a run claim agree.

/// Source-file extensions that make a bare argument a PATH target rather than a
/// test-name filter. `cargo test -p oxplow-git symlink` names a test, not a file.
pub(crate) const PATH_LIKE_EXTS: &[&str] = &[
    ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".java", ".clj", ".cs", ".c", ".cpp", ".h",
];

/// What a test command says it is testing — cargo package names (`-p` /
/// `--package`) and path-ish arguments (tsk169).
///
/// Used to disambiguate which open effort a run belongs to when several are
/// open and no `OXPLOW_TASK=` token pinned it. Deliberately conservative: a
/// bare word with no `/` and no source extension is a TEST-NAME filter
/// (`symlink`, `gauge`) and contributes nothing, because matching those against
/// file paths invents relationships that aren't there.
pub(crate) fn run_targets(command: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut toks = command.split_whitespace().peekable();
    while let Some(tok) = toks.next() {
        if tok == "-p" || tok == "--package" {
            if let Some(pkg) = toks.next() {
                let pkg = pkg.trim_matches(['"', '\'']);
                if !pkg.is_empty() && !pkg.starts_with('-') {
                    out.push(pkg.to_string());
                }
            }
            continue;
        }
        if tok.starts_with('-') {
            continue;
        }
        let arg = tok.trim_matches(['"', '\'']).trim_end_matches('/');
        if arg.is_empty() {
            continue;
        }
        let looks_like_path = arg.contains('/') || PATH_LIKE_EXTS.iter().any(|e| arg.ends_with(e));
        if looks_like_path {
            out.push(arg.to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Repo paths a task's own text names — `[[wikilinks]]` and backticked spans
/// that look like paths (tsk185).
///
/// The second scoring source for run attribution. An effort's `touched_files`
/// arrive via snapshot capture, which is asynchronous, so an effort that opened
/// seconds ago has claimed NOTHING yet — and that is exactly when its first
/// (red-phase) test run happens. Scoring against files alone therefore cannot
/// resolve the run you most want recorded. The task text is available
/// immediately and routinely names the file being worked on.
///
/// Deliberately narrow: only spans already marked up as a reference, only ones
/// that look like paths, and a trailing `:42` line suffix trimmed. Free-prose
/// scraping would invent matches.
pub(crate) fn task_target_paths(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |raw: &str| {
        let mut s = raw.trim();
        // `[[path:42]]` / `` `path:42` `` → `path`.
        if let Some((head, tail)) = s.rsplit_once(':') {
            if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
                s = head;
            }
        }
        let looks_like_path = s.contains('/') || PATH_LIKE_EXTS.iter().any(|e| s.ends_with(e));
        if looks_like_path && !s.is_empty() && !out.iter().any(|p| p == s) {
            out.push(s.to_string());
        }
    };
    // `[[ ... ]]`
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else { break };
        push(&after[..end]);
        rest = &after[end + 2..];
    }
    // `` ` ... ` `` — split on the delimiter; odd-indexed pieces are the spans.
    for (i, piece) in text.split('`').enumerate() {
        if i % 2 == 1 {
            push(piece);
        }
    }
    out
}

/// Whether `target` (from [`run_targets`]) refers to `path` (a file an effort
/// claimed). A package name matches when it is a whole PATH SEGMENT — so
/// `oxplow-git` matches `crates/oxplow-git/src/lib.rs` but not
/// `crates/oxplow-github/…`. A path target matches on containment either way,
/// which covers both a narrower run (`src/tabs/x.test.ts` under a claimed
/// `apps/desktop/src/tabs/x.test.ts`) and a broader one (`src/pages/`).
pub(crate) fn target_matches_path(target: &str, path: &str) -> bool {
    if target.contains('/') || PATH_LIKE_EXTS.iter().any(|e| target.ends_with(e)) {
        if path.contains(target) || target.contains(path) {
            return true;
        }
        // Sibling fallback: editing `foo.ts` and running `foo.test.ts` is one
        // piece of work, but neither path contains the other. Compare the
        // containing directories instead. Still decisive in practice, because
        // two efforts in the SAME directory tie and therefore decline.
        fn dir(p: &str) -> &str {
            p.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
        }
        let (td, pd) = (dir(target), dir(path));
        return !td.is_empty() && !pd.is_empty() && (pd.contains(td) || td.contains(pd));
    }
    path.split('/').any(|seg| seg == target)
}

/// How many of `targets` any of `paths` accounts for. Counting TARGETS (not
/// path hits) keeps a broad effort from outscoring a precise one purely by
/// having touched more files.
pub(crate) fn target_overlap(targets: &[String], paths: &[String]) -> usize {
    targets
        .iter()
        .filter(|t| paths.iter().any(|p| target_matches_path(t, p)))
        .count()
}

/// The single open effort a run's targets point at, or `None` when the answer
/// isn't unique (tsk169).
///
/// Attribution must never be *wrong-exact* — a mis-attributed run is worse than
/// an unattributed one, since the agent can still claim the latter at close.
/// So this returns a winner ONLY on a strict maximum: any tie, or no overlap at
/// all, defers to the existing unclaimed path. A generic target like `src` that
/// matches every candidate therefore self-neutralizes rather than guessing.
pub(crate) fn unique_best_by_targets<'a, T>(
    targets: &[String],
    candidates: &'a [(T, Vec<String>)],
) -> Option<&'a T> {
    if targets.is_empty() {
        return None;
    }
    let mut best: Option<(&T, usize)> = None;
    let mut tied = false;
    for (item, paths) in candidates {
        let score = target_overlap(targets, paths);
        if score == 0 {
            continue;
        }
        match best {
            Some((_, b)) if score < b => {}
            Some((_, b)) if score == b => tied = true,
            _ => {
                best = Some((item, score));
                tied = false;
            }
        }
    }
    if tied {
        return None;
    }
    best.map(|(item, _)| item)
}

#[cfg(test)]
mod target_scoring_tests {
    use super::*;

    #[test]
    fn run_targets_reads_packages_and_paths_but_not_test_names() {
        // The real commands from the session that motivated tsk169.
        assert_eq!(
            run_targets("cargo test -p oxplow-config gauge"),
            vec!["oxplow-config"],
            "`gauge` is a test-name filter, not a path"
        );
        assert_eq!(
            run_targets("cargo test -p oxplow-git symlink"),
            vec!["oxplow-git"]
        );
        assert_eq!(
            run_targets("cargo test -p oxplow-app --lib metric_engine::tests::ratio_"),
            vec!["oxplow-app"],
            "a `--lib` flag and a module path are not targets"
        );
        assert_eq!(
            run_targets("bun test src/tabs/pageRefs.test.ts"),
            vec!["src/tabs/pageRefs.test.ts"]
        );
        assert_eq!(
            run_targets("bun test src/pages/"),
            vec!["src/pages"],
            "a directory arg keeps working after the trailing slash is trimmed"
        );
        assert!(
            run_targets("bun run test:collect").is_empty(),
            "a whole-suite run names no target"
        );
    }

    #[test]
    fn target_matching_is_segment_exact_for_packages() {
        assert!(target_matches_path(
            "oxplow-git",
            "crates/oxplow-git/src/smart_merge.rs"
        ));
        assert!(
            !target_matches_path("oxplow-git", "crates/oxplow-github/src/lib.rs"),
            "a package name must match a WHOLE segment, not a prefix"
        );
        // A narrower run under a claimed file, and a broader directory run.
        assert!(target_matches_path(
            "src/tabs/pageRefs.test.ts",
            "apps/desktop/src/tabs/pageRefs.test.ts"
        ));
        assert!(target_matches_path(
            "src/pages",
            "apps/desktop/src/pages/CustomDashboardPage.tsx"
        ));
    }

    #[test]
    fn task_text_yields_the_paths_it_names() {
        // tsk185: the second scoring source, for the window before an effort has
        // claimed any files.
        let desc = "Fixes `[[crates/oxplow-app/src/producer_metrics.rs]]` where \
                    the dims are dropped. See `apps/desktop/src/App.tsx:2854` and \
                    the note in [[docs/guide/metrics.md]].";
        let got = task_target_paths(desc);
        assert!(
            got.contains(&"crates/oxplow-app/src/producer_metrics.rs".to_string()),
            "{got:?}"
        );
        assert!(
            got.contains(&"apps/desktop/src/App.tsx".to_string()),
            "a `:line` suffix is trimmed: {got:?}"
        );
        assert!(
            got.contains(&"docs/guide/metrics.md".to_string()),
            "{got:?}"
        );
    }

    #[test]
    fn task_text_ignores_prose_and_non_paths() {
        // Only marked-up spans that look like paths — scraping free prose would
        // invent matches and turn a decline into a wrong-exact attribution.
        let got = task_target_paths(
            "The `agent` dimension is dead. Rename [[tsk42]] and check `TOKEN_DIMS`. \
             crates/oxplow-app/src/lib.rs is only mentioned in prose.",
        );
        assert!(got.is_empty(), "should extract nothing, got {got:?}");
    }

    #[test]
    fn unique_best_picks_the_owner_across_concurrent_efforts() {
        // The exact shape that went unattributed: several efforts open, each
        // in a different crate, runs naming their crate.
        let candidates = vec![
            (
                "eff-config",
                vec!["crates/oxplow-config/src/lib.rs".to_string()],
            ),
            (
                "eff-git",
                vec!["crates/oxplow-git/src/smart_merge.rs".to_string()],
            ),
            // Note this effort claimed the SOURCE file while the run below
            // names its test sibling — the everyday case, and the one the
            // directory fallback in `target_matches_path` exists for.
            (
                "eff-tabs",
                vec!["apps/desktop/src/tabs/pageRefs.ts".to_string()],
            ),
        ];
        let pick = |cmd: &str| unique_best_by_targets(&run_targets(cmd), &candidates).copied();
        assert_eq!(
            pick("cargo test -p oxplow-config gauge"),
            Some("eff-config")
        );
        assert_eq!(pick("cargo test -p oxplow-git symlink"), Some("eff-git"));
        assert_eq!(pick("bun test src/tabs/pageRefs.test.ts"), Some("eff-tabs"));
    }

    #[test]
    fn unique_best_declines_rather_than_guessing() {
        // A mis-attributed run is worse than an unattributed one — the agent can
        // still claim the latter at close. So every ambiguous case must decline.
        let two_in_one_crate = vec![
            (
                "eff-a",
                vec!["crates/oxplow-app/src/collection.rs".to_string()],
            ),
            (
                "eff-b",
                vec!["crates/oxplow-app/src/metric_engine.rs".to_string()],
            ),
        ];
        assert_eq!(
            unique_best_by_targets(&run_targets("cargo test -p oxplow-app"), &two_in_one_crate),
            None,
            "a tie must decline"
        );
        assert_eq!(
            unique_best_by_targets(&run_targets("bun run test:collect"), &two_in_one_crate),
            None,
            "no targets at all must decline"
        );
        assert_eq!(
            unique_best_by_targets(&run_targets("cargo test -p oxplow-tmux"), &two_in_one_crate),
            None,
            "no overlap must decline"
        );
        // A target generic enough to match everything self-neutralizes.
        let generic = vec![
            ("eff-a", vec!["apps/desktop/src/a.ts".to_string()]),
            ("eff-b", vec!["apps/desktop/src/b.ts".to_string()]),
        ];
        assert_eq!(unique_best_by_targets(&["src".to_string()], &generic), None);
    }
}

/// Every path an open effort is known to be about: the files it has CLAIMED,
/// union the paths its task's own text names.
///
/// One implementation for both attribution call sites (tsk186) — the run
/// auto-claim and the per-file auto-claim must agree about which effort owns
/// what, and two copies of this would drift apart the first time one was
/// tuned.
///
/// The task-text half matters because claimed files can legitimately be empty:
/// a brand-new effort has claimed nothing yet, and a file edited through Bash
/// (codegen, a formatter, an agent using a heredoc) is deliberately never
/// auto-claimed at all.
pub(crate) async fn effort_known_paths(
    efforts: &oxplow_db::SqliteTaskEffortStore,
    tasks: &oxplow_db::SqliteTaskStore,
    effort: &oxplow_db::TaskEffort,
) -> Vec<String> {
    use oxplow_domain::stores::TaskStore;
    let mut paths: Vec<String> = efforts
        .list_files(&effort.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|f| f.path)
        .collect();
    if let Ok(Some(task)) = TaskStore::get(tasks, effort.task_id).await {
        for text in [task.title.as_str(), task.description.as_str()] {
            for p in task_target_paths(text) {
                if !paths.contains(&p) {
                    paths.push(p);
                }
            }
        }
    }
    paths
}

/// The single open effort a `target` (a run's target, or an edited file path)
/// belongs to, or `None` when the answer isn't unique.
///
/// The shared decision for both auto-claims. Returns a winner ONLY on a strict
/// maximum: ties and misses decline, because a WRONG attribution is worse than
/// a missing one — the agent can still claim a missing one at close, but a
/// wrong one silently misreports what an effort did.
pub(crate) async fn resolve_by_targets(
    efforts: &oxplow_db::SqliteTaskEffortStore,
    tasks: &oxplow_db::SqliteTaskStore,
    open: Vec<oxplow_db::TaskEffort>,
    targets: &[String],
) -> Option<oxplow_db::TaskEffort> {
    if targets.is_empty() || open.len() < 2 {
        return None;
    }
    let mut candidates: Vec<(oxplow_db::TaskEffort, Vec<String>)> = Vec::new();
    for e in open {
        let paths = effort_known_paths(efforts, tasks, &e).await;
        candidates.push((e, paths));
    }
    unique_best_by_targets(targets, &candidates).cloned()
}
