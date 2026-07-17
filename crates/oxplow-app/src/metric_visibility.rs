//! The metric-ancestry RESOLVER (tsk102) — populates [`Visibility`] from
//! commit-stamped snapshots so the fold's cross-branch rule runs on real
//! ancestry instead of `blind()`.
//!
//! The rule itself lives on [`Visibility`] (`metric_engine.rs`) — read its type
//! docs first. This module supplies its three inputs:
//!
//! - **`base_commit(R)`** is already ON the capture: `closest_git_version` is
//!   HEAD at record time (or the snapshot's own commit when exact) — tsk95's
//!   stamping IS the base, which is why that stamping stays.
//! - **`effective_commit(C)`** comes from the commit-stamped snapshots: the
//!   first same-stream, same-branch stamp at-or-after C names the commit that
//!   ABSORBED C's (dirty) work. An exact capture is its own anchor.
//! - **ancestry** between those anchors comes from git, through the
//!   [`AncestryOracle`] trait — pure data in, so the whole rule is
//!   unit-testable against a fake DAG and never shells out per capture.
//!
//! Absorption is timestamped `max(commit's own time, stamp row's creation)`,
//! which is what makes every resolved `(C, R)` answer immutable (see the
//! `Visibility` docs): a fresh commit postdates recorded readers by its own
//! time, and a PULLED commit — committer time arbitrarily old — postdates
//! them by its stamp row's creation. So no cube seed built earlier is
//! invalidated. (Residual mutation window: an old commit re-stamped onto an
//! old snapshot row; bounded by that row's age, reconciled by any rebuild.)
//!
//! **Every fold must use the same visibility RULE.** The engine's fact fold
//! and the cube's seed (`metric_cube::seed_rows`) share one
//! [`VisibilityResolver`] instance (`AppState.metric_visibility`);
//! `CollectionService` builds its own in `new()` — same pure rule over the
//! same DB, so the answers agree. One fold resolved with another blind is how
//! the cube silently diverges from the facts.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Mutex;

use oxplow_db::{MetricCapture, SqliteSnapshotStore, StampedSnapshot};
use oxplow_domain::Timestamp;

use crate::metric_engine::Visibility;

/// Answers the two git questions [`resolve`] needs. `None` = cannot say —
/// which the resolver degrades to VISIBLE, never stricter.
pub trait AncestryOracle {
    /// Is `ancestor` an ancestor of (or equal to) `descendant`?
    fn is_ancestor_or_equal(&mut self, ancestor: &str, descendant: &str) -> Option<bool>;
    /// The commit's own (committer) time — when its existence became true.
    fn commit_time(&mut self, sha: &str) -> Option<Timestamp>;
}

/// Resolve the [`Visibility`] for one read/build over `captures`.
///
/// `stamped` must be oldest-first (as
/// [`SqliteSnapshotStore::commit_stamped_snapshots`] returns it) — the FIRST
/// matching stamp at-or-after a capture is its absorbing commit.
pub fn resolve(
    captures: &[MetricCapture],
    stamped: &[StampedSnapshot],
    oracle: &mut dyn AncestryOracle,
) -> Visibility {
    let mut vis = Visibility::blind();

    // base_commit — already on the capture (tsk95): HEAD at record time, or
    // the snapshot's own commit when exact. Exactness doesn't matter for the
    // base: either way it is the closest ancestor commit of the code read.
    for c in captures {
        if let Some(sha) = &c.closest_git_version {
            vis.base.insert(c.id, sha.clone());
        }
    }

    // effective_commit — the first commit CONTAINING the capture's code.
    for c in captures {
        let eff = if c.git_version_exact {
            // Clean tree ⇒ the code is already in the stamped commit:
            // absorbed at birth, no snapshot search, no oracle call.
            c.closest_git_version
                .as_ref()
                .map(|sha| (sha.clone(), c.captured_at))
        } else {
            // The capture's OWN snapshot being stamped is the primary anchor —
            // and the ROUTINE one (measured 77/77 on the real DB): the final
            // green run sits between the last snapshot and the commit, whose
            // re-stamp lands on that snapshot row, so its `created_at`
            // PREDATES the capture and a time-window search misses it. The
            // stamp means "this row IS that commit's tree", i.e. exactly the
            // tree the capture ran on. Branch is deliberately not checked
            // here: a clean checkout can re-stamp the row under another
            // branch's name, but the TREE identity still holds.
            let own = c.snapshot_id.and_then(|sid| {
                stamped
                    .iter()
                    .find(|s| s.id == sid && s.stream_id == c.stream_id)
            });
            // Fallback: the work lands in the next same-stream, same-branch
            // commit at-or-after the capture.
            let absorbing = own.or_else(|| {
                c.branch.as_ref()?;
                stamped.iter().find(|s| {
                    s.stream_id == c.stream_id
                        && s.branch == c.branch
                        && s.created_at >= c.captured_at
                })
            });
            // Absorption time = max(the commit's own time, the stamp row's
            // creation). The commit-time half stops a re-stamp of an old row
            // claiming absorption before the commit existed; the row-creation
            // half stops a PULLED commit (committer time arbitrarily old)
            // claiming absorption before the repo ever contained it. Either
            // alone reopens a window where a resolved answer flips under an
            // already-recorded reader; together the answers stay immutable
            // for every flow the app produces (residual: an old commit
            // re-stamped onto an old row — reconciled by any rebuild).
            absorbing.and_then(|s| {
                oracle
                    .commit_time(&s.commit)
                    .map(|at| (s.commit.clone(), at.max(s.created_at)))
            })
        };
        if let Some(e) = eff {
            vis.effective.insert(c.id, e);
        }
    }

    // Ancestry between every distinct (effective, base) anchor pair — the
    // cross product is #commits × #commits, tiny by construction, and the
    // oracle caches each answer for its lifetime anyway. `None` (couldn't
    // resolve) stays ABSENT, which `sees` reads as visible.
    let effs: BTreeSet<&str> = vis.effective.values().map(|(s, _)| s.as_str()).collect();
    let bases: BTreeSet<&str> = vis.base.values().map(String::as_str).collect();
    for e in &effs {
        for b in &bases {
            if e == b {
                continue;
            }
            if let Some(answer) = oracle.is_ancestor_or_equal(e, b) {
                vis.ancestor_of
                    .insert((e.to_string(), b.to_string()), answer);
            }
        }
    }
    vis
}

/// [`AncestryOracle`] over the project's real repository, with answer caches —
/// ancestry between two fixed shas and a commit's time never change, so both
/// are cached for the resolver's lifetime.
pub struct GitAncestryOracle {
    repo: Option<git2::Repository>,
    ancestors: HashMap<(String, String), Option<bool>>,
    times: HashMap<String, Option<Timestamp>>,
}

impl GitAncestryOracle {
    /// A failed open degrades to an oracle that answers `None` for everything
    /// — i.e. blind visibility, never an error surfaced to a metrics read.
    pub fn open(repo_dir: &Path) -> Self {
        Self {
            repo: git2::Repository::open(repo_dir).ok(),
            ancestors: HashMap::new(),
            times: HashMap::new(),
        }
    }
}

impl AncestryOracle for GitAncestryOracle {
    fn is_ancestor_or_equal(&mut self, ancestor: &str, descendant: &str) -> Option<bool> {
        if ancestor == descendant {
            return Some(true);
        }
        let key = (ancestor.to_string(), descendant.to_string());
        if let Some(cached) = self.ancestors.get(&key) {
            return *cached;
        }
        let answer = self.repo.as_ref().and_then(|repo| {
            let anc = git2::Oid::from_str(ancestor).ok()?;
            let desc = git2::Oid::from_str(descendant).ok()?;
            repo.graph_descendant_of(desc, anc).ok()
        });
        self.ancestors.insert(key, answer);
        answer
    }

    fn commit_time(&mut self, sha: &str) -> Option<Timestamp> {
        if let Some(cached) = self.times.get(sha) {
            return *cached;
        }
        let answer = self.repo.as_ref().and_then(|repo| {
            let oid = git2::Oid::from_str(sha).ok()?;
            let commit = repo.find_commit(oid).ok()?;
            Some(Timestamp::from_unix_ms(commit.time().seconds() * 1000))
        });
        self.times.insert(sha.to_string(), answer);
        answer
    }
}

/// The store-backed resolver both folds share: fetches the commit-stamped
/// snapshots and runs [`resolve`] over them with a cached git oracle.
///
/// Every failure path yields [`Visibility::blind`] — resolution is an
/// accelerant for correctness of CROSS-branch reads, and its absence is
/// exactly the pre-tsk102 behavior, so it must never break a metrics read.
pub struct VisibilityResolver {
    snapshots: SqliteSnapshotStore,
    oracle: std::sync::Arc<Mutex<Box<dyn AncestryOracle + Send>>>,
}

impl VisibilityResolver {
    pub fn new(snapshots: SqliteSnapshotStore, repo_dir: &Path) -> Self {
        Self::with_oracle(snapshots, Box::new(GitAncestryOracle::open(repo_dir)))
    }

    /// A resolver with an injected oracle — for tests that need a fake DAG.
    pub fn with_oracle(
        snapshots: SqliteSnapshotStore,
        oracle: Box<dyn AncestryOracle + Send>,
    ) -> Self {
        Self {
            snapshots,
            oracle: std::sync::Arc::new(Mutex::new(oracle)),
        }
    }

    pub async fn for_captures(&self, captures: &[MetricCapture]) -> Visibility {
        let Ok(stamped) = self.snapshots.commit_stamped_snapshots().await else {
            return Visibility::blind();
        };
        // Ancestry walks are synchronous, disk-touching git work, and the
        // first read after boot resolves the whole effective×base cross
        // product — run it off the async workers so a cold cache can't stall
        // the runtime, and so readers queueing on the oracle Mutex park a
        // blocking thread, not a worker (tsk103 review). The lock is never
        // held across an await either way.
        let captures = captures.to_vec();
        let oracle = self.oracle.clone();
        tokio::task::spawn_blocking(move || {
            let mut oracle = oracle.lock().unwrap_or_else(|e| e.into_inner());
            resolve(&captures, &stamped, oracle.as_mut())
        })
        .await
        .unwrap_or_else(|_| Visibility::blind())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// A hand-expanded DAG:
    ///
    /// ```text
    /// A(10s) ── B(40s) ── M(60s)          main   (M merges feat-a)
    ///   │  \______________/
    ///   ├── FA(30s)                       feat-a
    ///   └── FB(50s)                       feat-b
    /// ```
    struct FakeDag {
        ancestors: HashSet<(&'static str, &'static str)>,
        times: HashMap<&'static str, Timestamp>,
    }

    fn dag() -> FakeDag {
        FakeDag {
            ancestors: [
                ("A", "B"),
                ("A", "FA"),
                ("A", "FB"),
                ("A", "M"),
                ("B", "M"),
                ("FA", "M"),
            ]
            .into_iter()
            .collect(),
            times: [
                ("A", ts(10)),
                ("FA", ts(30)),
                ("B", ts(40)),
                ("FB", ts(50)),
                ("M", ts(60)),
            ]
            .into_iter()
            .collect(),
        }
    }

    impl AncestryOracle for FakeDag {
        fn is_ancestor_or_equal(&mut self, ancestor: &str, descendant: &str) -> Option<bool> {
            if !self.times.contains_key(ancestor) || !self.times.contains_key(descendant) {
                return None;
            }
            Some(ancestor == descendant || self.ancestors.contains(&(ancestor, descendant)))
        }

        fn commit_time(&mut self, sha: &str) -> Option<Timestamp> {
            self.times.get(sha).copied()
        }
    }

    fn ts(secs: i64) -> Timestamp {
        Timestamp::from_unix_ms(secs * 1000)
    }

    fn cap(id: i64, branch: Option<&str>, at_secs: i64, base: Option<&str>) -> MetricCapture {
        MetricCapture {
            id,
            stream_id: 1,
            thread_id: None,
            effort_id: None,
            producer: "tests".into(),
            status: "done".into(),
            error: None,
            scope: None,
            trigger: None,
            basis_ref: None,
            provenance: "agent".into(),
            source: "builtin".into(),
            snapshot_id: None,
            closest_git_version: base.map(Into::into),
            git_version_exact: false,
            branch: branch.map(Into::into),
            captured_at: ts(at_secs),
            ended_at: None,
            detail_json: None,
            producer_version: None,
            scan_kind: "delta".into(),
        }
    }

    fn stamp(branch: &str, commit: &str, at_secs: i64) -> StampedSnapshot {
        StampedSnapshot {
            id: 100 + at_secs, // unique, disjoint from the ids tests pick
            stream_id: 1,
            branch: Some(branch.into()),
            commit: commit.into(),
            created_at: ts(at_secs),
        }
    }

    /// The stream's stamped history, oldest first — matching the DAG above.
    fn stamps() -> Vec<StampedSnapshot> {
        vec![
            stamp("main", "A", 10),
            stamp("feat-a", "FA", 30),
            stamp("main", "B", 40),
            stamp("feat-b", "FB", 50),
            stamp("main", "M", 60),
        ]
    }

    #[test]
    fn a_siblings_results_never_leak_into_another_branch() {
        // THE gap this resolver closes: blind visibility lets feat-a's capture
        // seed feat-b, a branch that never contained that code. feat-a's dirty
        // work (t20) was absorbed by FA (t30); FA is no ancestor of feat-b's
        // base A, so a feat-b reader at t55 must NOT see it.
        let on_feat_a = cap(1, Some("feat-a"), 20, Some("A"));
        let on_feat_b = cap(2, Some("feat-b"), 55, Some("A"));
        let vis = resolve(
            &[on_feat_a.clone(), on_feat_b.clone()],
            &stamps(),
            &mut dag(),
        );
        assert!(
            !vis.sees(&on_feat_a, &on_feat_b),
            "a sibling's absorbed work must be invisible — blind() said visible"
        );
    }

    #[test]
    fn a_branch_inherits_mains_prefork_history_and_not_its_postfork_work() {
        // INHERIT (the user's chosen semantic): work absorbed at-or-before the
        // fork point is the branch's own history. Work absorbed after the fork
        // (B) never reached the branch's code.
        let prefork = cap(1, Some("main"), 5, Some("A"));
        let postfork = cap(2, Some("main"), 12, Some("A"));
        let reader = cap(3, Some("feat-b"), 55, Some("A"));
        let vis = resolve(
            &[prefork.clone(), postfork.clone(), reader.clone()],
            &stamps(),
            &mut dag(),
        );
        assert!(
            vis.sees(&prefork, &reader),
            "absorbed at A (= the fork point) ⇒ in the branch's history"
        );
        assert!(
            !vis.sees(&postfork, &reader),
            "absorbed at B (post-fork, main) ⇒ never in the branch's code"
        );
    }

    #[test]
    fn a_merged_branchs_results_become_visible_to_main() {
        // The semantic tsk97 always wanted for free: after the merge commit M,
        // FA is an ancestor of main's base, so feat-a's results ARE main
        // history. Before the merge (base B) they are not.
        let on_feat_a = cap(1, Some("feat-a"), 20, Some("A"));
        let premerge = cap(2, Some("main"), 45, Some("B"));
        let postmerge = cap(3, Some("main"), 65, Some("M"));
        let vis = resolve(
            &[on_feat_a.clone(), premerge.clone(), postmerge.clone()],
            &stamps(),
            &mut dag(),
        );
        assert!(
            !vis.sees(&on_feat_a, &premerge),
            "before the merge, feat-a's work is not main's"
        );
        assert!(vis.sees(&on_feat_a, &postmerge), "after the merge, it is");
    }

    #[test]
    fn absorption_after_the_read_never_hides_the_work() {
        // As-of-R. Main's dirty work at t45 is absorbed by M at t60. A feat-b
        // reader at t55 ran while that work was plausibly still sitting dirty
        // in the shared worktree ⇒ visible; a reader at t65 ran after M
        // existed ⇒ the strict answer applies (M ∉ ancestors(FB)) ⇒ invisible.
        // Same capture, both answers permanent — this is what lets the cube
        // freeze its seeds without any invalidate-on-commit machinery.
        let dirty_main = cap(1, Some("main"), 45, Some("B"));
        let early = cap(2, Some("feat-b"), 55, Some("A"));
        let late = cap(3, Some("feat-b"), 65, Some("FB"));
        let vis = resolve(
            &[dirty_main.clone(), early.clone(), late.clone()],
            &stamps(),
            &mut dag(),
        );
        assert!(
            vis.sees(&dirty_main, &early),
            "not yet absorbed when the reader ran ⇒ visible"
        );
        assert!(
            !vis.sees(&dirty_main, &late),
            "absorbed before this reader ⇒ the ancestry answer applies"
        );
    }

    #[test]
    fn a_dirty_run_anchors_to_the_absorbing_commit_not_the_fork_point() {
        // The disproof pin (tsk97): ancestry on `closest_git_version` (= the
        // fork point A, which IS an ancestor of everything) cannot separate
        // branches. The anchor must be FA — the commit that absorbed the work.
        let on_feat_a = cap(1, Some("feat-a"), 20, Some("A"));
        let vis = resolve(&[on_feat_a], &stamps(), &mut dag());
        assert_eq!(
            vis.effective.get(&1),
            Some(&("FA".to_string(), ts(30))),
            "effective = the absorbing commit and ITS time, never the base"
        );
    }

    #[test]
    fn a_capture_whose_own_snapshot_was_restamped_anchors_to_that_commit() {
        // THE routine flow, measured on the real DB (77/77 own-snapshot anchors
        // look like this): edit → snapshot row s → final green test run (the
        // capture, carrying snapshot_id = s) → commit, which RE-STAMPS s because
        // the worktree didn't change. s.created_at predates the capture, so a
        // time-window search ("first stamp at-or-after the capture") misses it
        // and anchors one commit too late — strict-wrong: a branch forked at
        // the re-stamped commit would fail to inherit the run made just before
        // it. The capture's OWN snapshot is the tree it tested; its stamp wins.
        let mut c = cap(1, Some("main"), 20, Some("A"));
        c.snapshot_id = Some(7);
        let own = StampedSnapshot {
            id: 7,
            stream_id: 1,
            branch: Some("main".into()),
            commit: "A".into(),
            created_at: ts(15), // BEFORE the capture — the re-stamp shape
        };
        let mut all = stamps();
        all.insert(1, own); // keep oldest-first order
        let vis = resolve(&[c], &all, &mut dag());
        assert_eq!(
            vis.effective.get(&1),
            Some(&("A".to_string(), ts(15))),
            "the OWN snapshot's commit — a time-window search would (wrongly) \
             anchor to the next main stamp, B. Absorbed at max(commit time t10, \
             row creation t15): never earlier than the repo contained it"
        );
    }

    #[test]
    fn an_exact_capture_is_its_own_anchor() {
        // A clean-tree capture's code is already IN closest_git_version —
        // absorbed at birth, no snapshot search.
        let mut exact = cap(1, Some("main"), 40, Some("B"));
        exact.git_version_exact = true;
        let vis = resolve(&[exact], &stamps(), &mut dag());
        assert_eq!(vis.effective.get(&1), Some(&("B".to_string(), ts(40))));
    }

    #[test]
    fn the_git_oracle_answers_ancestry_and_time_from_a_real_repo() {
        // The adapter is the only piece the fake DAG can't cover: Oid parsing,
        // the `graph_descendant_of` ARGUMENT ORDER (its parameters are
        // (descendant, ancestor) — swapped, every answer inverts and main
        // inherits its feature branches), and commit-time lookup. Shape:
        // A ── B ── M on main, A ── FA on feat, M merges (B, FA).
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "t@example.com").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let a = repo
            .commit(Some("HEAD"), &sig, &sig, "A", &tree, &[])
            .unwrap();
        let a_commit = repo.find_commit(a).unwrap();
        let fa = repo
            .commit(
                Some("refs/heads/feat"),
                &sig,
                &sig,
                "FA",
                &tree,
                &[&a_commit],
            )
            .unwrap();
        let b = repo
            .commit(Some("HEAD"), &sig, &sig, "B", &tree, &[&a_commit])
            .unwrap();
        let b_commit = repo.find_commit(b).unwrap();
        let fa_commit = repo.find_commit(fa).unwrap();
        let m = repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "M",
                &tree,
                &[&b_commit, &fa_commit],
            )
            .unwrap();

        let mut oracle = GitAncestryOracle::open(dir.path());
        let (a, b, fa, m) = (a.to_string(), b.to_string(), fa.to_string(), m.to_string());
        assert_eq!(oracle.is_ancestor_or_equal(&a, &b), Some(true));
        assert_eq!(
            oracle.is_ancestor_or_equal(&b, &a),
            Some(false),
            "direction matters — a swapped graph_descendant_of inverts this"
        );
        assert_eq!(oracle.is_ancestor_or_equal(&a, &a), Some(true), "or-equal");
        assert_eq!(
            oracle.is_ancestor_or_equal(&fa, &m),
            Some(true),
            "merged in"
        );
        assert_eq!(oracle.is_ancestor_or_equal(&fa, &b), Some(false), "sibling");
        assert_eq!(
            oracle.is_ancestor_or_equal("deadbeef", &a),
            None,
            "unknown sha ⇒ None, which the resolver reads as visible"
        );
        assert!(oracle.commit_time(&a).is_some());
        assert_eq!(oracle.commit_time("deadbeef"), None);
        // And a directory that isn't a repo degrades, never errors.
        let not_repo = tempfile::tempdir().unwrap();
        let mut blind = GitAncestryOracle::open(not_repo.path());
        assert_eq!(blind.is_ancestor_or_equal(&a, &b), None);
    }

    #[test]
    fn a_pulled_commits_old_committer_time_cannot_backdate_absorption() {
        // tsk103 review. A commit made ELSEWHERE and pulled in carries a
        // committer time arbitrarily older than the moment this repo first
        // contained it. If absorption used the commit's own time alone, a
        // reader recorded before the pull would flip from visible to strict
        // the moment the stamp landed — mutating an already-resolved answer
        // and diverging any cube seed that baked in "visible". Absorption is
        // max(commit time, stamp row creation): the pull can't reach back.
        let on_feat_a = cap(1, Some("feat-a"), 20, Some("A"));
        let before_pull = cap(2, Some("feat-b"), 40, Some("A"));
        let after_pull = cap(3, Some("feat-b"), 60, Some("A"));
        // The pulled commit "OLD" (committer time t5!) lands on a stamp row
        // created at t50 — the moment this repo first had it.
        let mut all = stamps();
        all.retain(|s| s.branch.as_deref() != Some("feat-a"));
        all.push(StampedSnapshot {
            id: 300,
            stream_id: 1,
            branch: Some("feat-a".into()),
            commit: "OLD".into(),
            created_at: ts(50),
        });
        let mut oracle = dag();
        oracle.times.insert("OLD", ts(5));
        // OLD is not an ancestor of A (it's sibling work).
        let vis = resolve(
            &[on_feat_a.clone(), before_pull.clone(), after_pull.clone()],
            &all,
            &mut oracle,
        );
        assert_eq!(
            vis.effective.get(&1).map(|(_, at)| *at),
            Some(ts(50)),
            "absorbed when the repo first contained it, not at committer time"
        );
        assert!(
            vis.sees(&on_feat_a, &before_pull),
            "a reader recorded before the pull keeps seeing the dirty work"
        );
        assert!(
            !vis.sees(&on_feat_a, &after_pull),
            "a reader after the pull gets the strict answer"
        );
    }

    #[test]
    fn unresolvable_ancestry_stays_visible() {
        // Degradation, three ways: a branch-less capture has no stamp to match;
        // a reader whose base sha the oracle doesn't know makes the PAIR
        // unresolvable; a reader with no base at all has nothing to check.
        // All three must read VISIBLE — never invent strictness from missing
        // data (this was blind()'s behavior, and worse-than-before is banned).
        let no_branch = cap(1, None, 20, Some("A"));
        let on_feat_a = cap(2, Some("feat-a"), 20, Some("A"));
        let reader = cap(3, Some("feat-b"), 55, Some("A"));
        let reader_unknown_base = cap(4, Some("feat-b"), 55, Some("ZZZ"));
        let baseless_reader = cap(5, Some("feat-b"), 55, None);
        let vis = resolve(
            &[
                no_branch.clone(),
                on_feat_a.clone(),
                reader.clone(),
                reader_unknown_base.clone(),
                baseless_reader.clone(),
            ],
            &stamps(),
            &mut dag(),
        );
        assert!(
            vis.sees(&no_branch, &reader),
            "no branch ⇒ no anchor ⇒ visible"
        );
        assert!(
            vis.sees(&on_feat_a, &reader_unknown_base),
            "the (FA, ZZZ) pair is unresolvable and must not go strict — \
             the resolvable (FA, A) pair right beside it correctly does"
        );
        assert!(vis.sees(&on_feat_a, &baseless_reader), "no base ⇒ visible");
    }
}
