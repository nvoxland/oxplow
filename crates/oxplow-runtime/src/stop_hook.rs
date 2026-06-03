//! Pure decision function for the Claude Stop-hook.
//!
//! Direct port of `src/electron/stop-hook-pipeline.ts`. Lives outside
//! the runtime god-object so each branch can be unit-tested with
//! fixture snapshots, and so the decision and the side effects (e.g.
//! recording the audit signature) stay separate.
//!
//! Pipeline runs in priority order:
//!   1. Awaiting-user gate
//!   2. Q&A turn (no qualifying tool activity) — allow stop
//!   3. Subagent in flight — suppress
//!   4. Filed-but-didn't-ship advisory
//!   5. Stale-epic-children advisory
//!   6. In-progress audit (with signature dedup)
//!   7. Allow stop

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{Task, TaskStatus, Thread, ThreadStatus};

#[derive(Debug, Clone, Default)]
pub struct ThreadSnapshot<'a> {
    pub thread: Option<&'a Thread>,
    pub tasks: &'a [Task],
    /// Signature of the in_progress set the runtime last emitted an
    /// audit directive for on this thread.
    pub last_in_progress_audit_signature: Option<&'a str>,
    pub subagent_in_flight: bool,
    /// `None` is treated as "unknown — don't suppress".
    pub turn_had_activity: Option<bool>,
    pub awaiting_user: bool,
    pub turn_had_writes: bool,
    pub turn_had_filing: bool,
    pub turn_filed_ready_item: bool,
    pub filed_but_didnt_ship_fired: bool,
    /// Efforts whose touched_files claim disagreed with the snapshot
    /// diff and haven't been reviewed by the agent yet. The Stop hook
    /// fires a one-shot directive surfacing the discrepancies; once
    /// fired, the runtime drops these so the prompt doesn't repeat.
    pub pending_effort_reviews: &'a [PendingEffortReview],
}

/// Minimal review summary the Stop hook needs to render its
/// directive. Mirrors `oxplow_app::task_service::EffortFileReview`
/// but lives in the runtime crate so stop_hook stays free of an
/// upstream dependency.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingEffortReview {
    pub effort_id: String,
    pub task_id: i64,
    pub task_title: String,
    pub claimed_but_not_changed: Vec<String>,
    pub changed_but_not_claimed: Vec<String>,
    pub unclaimed_overflow: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct StopDirective {
    pub decision: &'static str, // always "block"
    pub reason: String,
}

impl StopDirective {
    pub fn block(reason: String) -> Self {
        Self {
            decision: "block",
            reason,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StopHookSideEffect {
    RecordAuditSignature(String),
    RecordFiledButDidntShipFired,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StopHookOutcome {
    pub directive: Option<StopDirective>,
    pub side_effects: Vec<StopHookSideEffect>,
}

#[derive(Default)]
#[allow(clippy::type_complexity)]
pub struct DirectiveBuilders<'a> {
    pub build_in_progress_audit_reason: Option<&'a dyn Fn(&[Task]) -> String>,
    pub build_filed_but_didnt_ship_reason: Option<&'a dyn Fn() -> String>,
    pub build_stale_epic_children_reason: Option<&'a dyn Fn(&[StaleEpicPair]) -> String>,
    pub build_effort_file_review_reason: Option<&'a dyn Fn(&[PendingEffortReview]) -> String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StaleEpicPair {
    pub epic: Task,
    pub stale_children: Vec<Task>,
}

pub fn decide_stop_directive(
    snapshot: ThreadSnapshot<'_>,
    builders: DirectiveBuilders<'_>,
) -> StopHookOutcome {
    let mut outcome = StopHookOutcome::default();

    if snapshot.awaiting_user {
        return outcome;
    }

    if snapshot.turn_had_activity == Some(false) {
        return outcome;
    }

    let Some(thread) = snapshot.thread else {
        return outcome;
    };
    if thread.status != ThreadStatus::Active {
        return outcome;
    }

    if snapshot.subagent_in_flight {
        return outcome;
    }

    let in_progress: Vec<&Task> = snapshot
        .tasks
        .iter()
        .filter(|item| item.status == TaskStatus::InProgress)
        .collect();

    if snapshot.turn_filed_ready_item
        && !snapshot.turn_had_writes
        && in_progress.is_empty()
        && !snapshot.filed_but_didnt_ship_fired
    {
        if let Some(build) = builders.build_filed_but_didnt_ship_reason {
            outcome.directive = Some(StopDirective::block(build()));
            outcome
                .side_effects
                .push(StopHookSideEffect::RecordFiledButDidntShipFired);
            return outcome;
        }
    }

    let stale = find_stale_epic_children_pairs(snapshot.tasks);
    if !stale.is_empty() {
        if let Some(build) = builders.build_stale_epic_children_reason {
            outcome.directive = Some(StopDirective::block(build(&stale)));
            return outcome;
        }
    }

    if !snapshot.pending_effort_reviews.is_empty() {
        if let Some(build) = builders.build_effort_file_review_reason {
            outcome.directive = Some(StopDirective::block(build(snapshot.pending_effort_reviews)));
            // No side-effect needed — control-plane already drained
            // the pending set via `take_pending_effort_reviews` when
            // building the snapshot, so the prompt is one-shot by
            // construction.
            return outcome;
        }
    }

    if !in_progress.is_empty() {
        if let Some(build) = builders.build_in_progress_audit_reason {
            let in_progress_owned: Vec<Task> = in_progress.iter().map(|i| (*i).clone()).collect();
            let signature = compute_audit_signature(&in_progress_owned);
            if snapshot.last_in_progress_audit_signature == Some(signature.as_str()) {
                return outcome;
            }
            outcome.directive = Some(StopDirective::block(build(&in_progress_owned)));
            outcome
                .side_effects
                .push(StopHookSideEffect::RecordAuditSignature(signature));
            return outcome;
        }
    }

    outcome
}

/// An "epic" is any task that has children. Find epics in
/// `done`/`blocked` whose children are still `ready`/`in_progress`.
pub fn find_stale_epic_children_pairs(items: &[Task]) -> Vec<StaleEpicPair> {
    let mut pairs = Vec::new();
    for epic in items {
        if !matches!(epic.status, TaskStatus::Done | TaskStatus::Blocked) {
            continue;
        }
        let stale_children: Vec<Task> = items
            .iter()
            .filter(|child| {
                child.parent_id == Some(epic.id)
                    && matches!(child.status, TaskStatus::Ready | TaskStatus::InProgress)
            })
            .cloned()
            .collect();
        if !stale_children.is_empty() {
            pairs.push(StaleEpicPair {
                epic: epic.clone(),
                stale_children,
            });
        }
    }
    pairs
}

/// Per-thread fingerprint of the in_progress set used to detect
/// "nothing changed since last audit fire" and skip a duplicate.
pub fn compute_audit_signature(items: &[Task]) -> String {
    let mut entries: Vec<String> = items
        .iter()
        .map(|item| {
            let updated_at = serde_json::to_string(&item.updated_at)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string();
            format!("{}|{}|{}", item.id, updated_at, item.note_count)
        })
        .collect();
    entries.sort();
    entries.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{
        StreamId, TaskActorKind, TaskAuthor, TaskId, TaskPriority, ThreadId, Timestamp,
    };

    fn now() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    fn active_thread() -> Thread {
        Thread {
            id: ThreadId::from("b-1"),
            stream_id: StreamId::from("s-1"),
            title: "t".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now(),
            updated_at: now(),
            archived_at: None,
        }
    }

    fn item(id: i64, status: TaskStatus, parent: Option<i64>) -> Task {
        Task {
            id: TaskId::new(id),
            thread_id: None,
            parent_id: parent.map(TaskId::new),
            title: format!("t{id}"),
            description: String::new(),
            status,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now(),
            updated_at: now(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
        }
    }

    #[test]
    fn awaiting_user_suppresses_everything() {
        let t = active_thread();
        let snap = ThreadSnapshot {
            thread: Some(&t),
            awaiting_user: true,
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, DirectiveBuilders::default());
        assert!(outcome.directive.is_none());
    }

    #[test]
    fn qa_turn_allows_stop() {
        let t = active_thread();
        let snap = ThreadSnapshot {
            thread: Some(&t),
            turn_had_activity: Some(false),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, DirectiveBuilders::default());
        assert!(outcome.directive.is_none());
    }

    #[test]
    fn non_active_thread_allows_stop() {
        let mut t = active_thread();
        t.status = ThreadStatus::Queued;
        let snap = ThreadSnapshot {
            thread: Some(&t),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, DirectiveBuilders::default());
        assert!(outcome.directive.is_none());
    }

    #[test]
    fn subagent_in_flight_suppresses_audit() {
        let t = active_thread();
        let items = vec![item(1, TaskStatus::InProgress, None)];
        let snap = ThreadSnapshot {
            thread: Some(&t),
            tasks: &items,
            subagent_in_flight: true,
            ..Default::default()
        };
        let build_audit = |_: &[Task]| "audit".to_string();
        let builders = DirectiveBuilders {
            build_in_progress_audit_reason: Some(&build_audit),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, builders);
        assert!(outcome.directive.is_none());
    }

    #[test]
    fn in_progress_items_trigger_audit() {
        let t = active_thread();
        let items = vec![item(1, TaskStatus::InProgress, None)];
        let snap = ThreadSnapshot {
            thread: Some(&t),
            tasks: &items,
            ..Default::default()
        };
        let build_audit = |items: &[Task]| format!("audit {} items", items.len());
        let builders = DirectiveBuilders {
            build_in_progress_audit_reason: Some(&build_audit),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, builders);
        let dir = outcome.directive.expect("audit directive");
        assert!(dir.reason.contains("audit 1 items"));
        assert!(matches!(
            outcome.side_effects.first(),
            Some(StopHookSideEffect::RecordAuditSignature(_))
        ));
    }

    #[test]
    fn pending_effort_review_branch_fires_before_audit() {
        let t = active_thread();
        // Both an in_progress item AND a pending review — review wins
        // because it's about something the agent JUST did and is more
        // time-sensitive than the audit nudge.
        let items = vec![item(1, TaskStatus::InProgress, None)];
        let reviews = vec![PendingEffortReview {
            effort_id: "e-1".into(),
            task_id: 7,
            task_title: "ship the thing".into(),
            claimed_but_not_changed: vec!["src/typo.rs".into()],
            changed_but_not_claimed: vec!["src/extra.rs".into()],
            unclaimed_overflow: None,
        }];
        let snap = ThreadSnapshot {
            thread: Some(&t),
            tasks: &items,
            pending_effort_reviews: &reviews,
            ..Default::default()
        };
        let build_audit = |_: &[Task]| "AUDIT".to_string();
        let build_review = |rs: &[PendingEffortReview]| format!("REVIEW {} efforts", rs.len());
        let builders = DirectiveBuilders {
            build_in_progress_audit_reason: Some(&build_audit),
            build_effort_file_review_reason: Some(&build_review),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, builders);
        let dir = outcome.directive.expect("review directive");
        assert!(dir.reason.contains("REVIEW 1 efforts"));
        assert!(!dir.reason.contains("AUDIT"));
    }

    #[test]
    fn audit_signature_dedup() {
        let t = active_thread();
        let items = vec![item(1, TaskStatus::InProgress, None)];
        let signature = compute_audit_signature(&items);
        let snap = ThreadSnapshot {
            thread: Some(&t),
            tasks: &items,
            last_in_progress_audit_signature: Some(&signature),
            ..Default::default()
        };
        let build_audit = |_: &[Task]| "audit".to_string();
        let builders = DirectiveBuilders {
            build_in_progress_audit_reason: Some(&build_audit),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, builders);
        assert!(
            outcome.directive.is_none(),
            "identical signature must suppress the audit"
        );
    }

    #[test]
    fn filed_but_didnt_ship_branch() {
        let t = active_thread();
        let snap = ThreadSnapshot {
            thread: Some(&t),
            turn_filed_ready_item: true,
            turn_had_writes: false,
            filed_but_didnt_ship_fired: false,
            ..Default::default()
        };
        let build_filed = || "you filed but didn't ship".to_string();
        let builders = DirectiveBuilders {
            build_filed_but_didnt_ship_reason: Some(&build_filed),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, builders);
        let dir = outcome.directive.expect("filed-but-didnt-ship directive");
        assert!(dir.reason.contains("filed but didn't ship"));
        assert!(matches!(
            outcome.side_effects.first(),
            Some(StopHookSideEffect::RecordFiledButDidntShipFired)
        ));
    }

    #[test]
    fn filed_but_didnt_ship_already_fired_suppresses() {
        let t = active_thread();
        let snap = ThreadSnapshot {
            thread: Some(&t),
            turn_filed_ready_item: true,
            filed_but_didnt_ship_fired: true,
            ..Default::default()
        };
        let build_filed = || "filed".to_string();
        let builders = DirectiveBuilders {
            build_filed_but_didnt_ship_reason: Some(&build_filed),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, builders);
        assert!(outcome.directive.is_none());
    }

    #[test]
    fn stale_epic_children_branch() {
        let t = active_thread();
        let items = vec![
            item(1, TaskStatus::Done, None),
            item(2, TaskStatus::Ready, Some(1)),
        ];
        let snap = ThreadSnapshot {
            thread: Some(&t),
            tasks: &items,
            ..Default::default()
        };
        let build_stale = |pairs: &[StaleEpicPair]| {
            format!(
                "stale: {} pairs / {} children",
                pairs.len(),
                pairs[0].stale_children.len()
            )
        };
        let builders = DirectiveBuilders {
            build_stale_epic_children_reason: Some(&build_stale),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, builders);
        let dir = outcome.directive.expect("stale-epic directive");
        assert!(dir.reason.contains("1 pairs / 1 children"));
    }

    #[test]
    fn allow_stop_when_clean() {
        let t = active_thread();
        let snap = ThreadSnapshot {
            thread: Some(&t),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, DirectiveBuilders::default());
        assert!(outcome.directive.is_none());
        assert!(outcome.side_effects.is_empty());
    }

    #[test]
    fn find_stale_epic_children_pairs_filters_correctly() {
        let items = vec![
            // closed parent with stale child → flagged (parent is treated as epic)
            item(1, TaskStatus::Done, None),
            item(2, TaskStatus::Ready, Some(1)),
            // closed parent with all-done children → not flagged
            item(3, TaskStatus::Done, None),
            item(4, TaskStatus::Done, Some(3)),
            // open parent → never flagged
            item(5, TaskStatus::Ready, None),
            item(6, TaskStatus::Ready, Some(5)),
            // childless closed task → never flagged (no stale_children)
            item(7, TaskStatus::Done, None),
        ];
        let pairs = find_stale_epic_children_pairs(&items);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].epic.id, TaskId::new(1));
        assert_eq!(pairs[0].stale_children.len(), 1);
    }

    /// Childless `done` tasks must never produce a stale-children pair
    /// — there are no children to be stale. The audit logic relies on
    /// this so a regular completed task in the Done bucket doesn't get
    /// surfaced as an epic to walk.
    #[test]
    fn find_stale_epic_children_pairs_skips_childless_done() {
        let items = vec![
            item(1, TaskStatus::Done, None),
            item(2, TaskStatus::Blocked, None),
            item(3, TaskStatus::Canceled, None),
        ];
        let pairs = find_stale_epic_children_pairs(&items);
        assert!(pairs.is_empty(), "expected no pairs, got {pairs:?}");
    }

    #[test]
    fn compute_audit_signature_stable_under_reordering() {
        let a = item(10, TaskStatus::InProgress, None);
        let b = item(20, TaskStatus::InProgress, None);
        let s1 = compute_audit_signature(&[a.clone(), b.clone()]);
        let s2 = compute_audit_signature(&[b, a]);
        assert_eq!(s1, s2);
    }
}
