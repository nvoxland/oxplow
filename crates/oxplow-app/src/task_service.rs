//! TaskService — orchestration over the Task store.
//!
//! Encapsulates the create/update/reorder/move use-cases. The store
//! itself is a thin row-CRUD layer; everything that requires composing
//! reads and writes (e.g. computing the next sort_index, transitioning
//! status with the associated timestamp side-effects, moving a task
//! between thread and backlog) lives here.
//!
//! The service does not emit events itself — the Tauri command layer
//! does, after a successful service call. That keeps `oxplow-app`
//! independent of the tauri-specta layering and lets the MCP surface
//! reuse the same service without paying for renderer notifications.

use oxplow_domain::refs::build::work_item_ref;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use oxplow_db::SqliteTaskStore;
use oxplow_db::SqliteThreadStore;
use oxplow_db::{
    Effort, EffortFileChange, EffortStore, NewFact, NewMetricCapture, SqliteAttributionStore,
    SqliteEffortStore, SqliteFactStore, SqliteSnapshotStore,
};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::stores::{TaskLinkStore, TaskStore};
use oxplow_domain::EffortId;
use oxplow_domain::{
    DomainError, Task, TaskActorKind, TaskAuthor, TaskId, TaskImpact, TaskLinkType, TaskPriority,
    TaskStatus, ThreadId, Timestamp,
};

use crate::events::EventBus;

#[derive(Debug, Error)]
pub enum TaskServiceError {
    #[error("task not found: {0}")]
    NotFound(TaskId),
    #[error("storage: {0}")]
    Storage(#[from] DomainError),
}

async fn item_is_blocked(
    id: TaskId,
    link_store: &dyn TaskLinkStore,
    by_id: &std::collections::HashMap<TaskId, Task>,
) -> Result<bool, DomainError> {
    let incoming = link_store.list_incoming(id).await?;
    for link in incoming {
        if !matches!(link.link_type, TaskLinkType::Blocks) {
            continue;
        }
        if let Some(blocker) = by_id.get(&link.from_item_id) {
            if !matches!(
                blocker.status,
                TaskStatus::Done | TaskStatus::Canceled | TaskStatus::Archived
            ) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Discriminated result for `read_task_options`. The shape mirrors
/// main's TS contract so the agent-side skill text stays accurate
/// without a translation layer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "mode", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ReadWorkOptionsResult {
    Empty,
    Epic { epic: Task, children: Vec<Task> },
    Standalone { items: Vec<Task> },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct CreateTaskInput {
    pub title: String,
    /// The task's prose body (canonical markdown).
    pub description: Option<String>,
    pub parent_id: Option<TaskId>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub author: Option<TaskAuthor>,
}

/// Partial-patch for `update_task`. Each `Option` follows
/// "missing -> keep, present -> replace" semantics.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct UpdateTaskChanges {
    pub title: Option<String>,
    pub description: Option<String>,
    pub parent_id: Option<Option<TaskId>>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
}

#[derive(Clone)]
pub struct TaskService {
    store: Arc<SqliteTaskStore>,
    /// Optional. When set, `update()` opens/closes an effort row on
    /// `in_progress` entry/exit. Held as an `Option` so test paths
    /// that construct a TaskService without the full Services boot
    /// still work — they just skip the lifecycle effort.
    effort_store: Option<Arc<SqliteEffortStore>>,
    /// Per-stream snapshot capture registry. When set alongside
    /// `thread_store`, lifecycle snapshots resolve the right service
    /// via the task's thread → stream — so a task running in a
    /// non-primary worktree captures snapshots against THAT
    /// worktree's fs-watch, not the primary's.
    snapshot_captures: Option<crate::snapshot_capture_registry::SnapshotCaptureRegistry>,
    /// Looks up a thread to read its `stream_id`. Required to drive
    /// the registry-based lookup.
    thread_store: Option<Arc<SqliteThreadStore>>,
    /// Durable fact layer (epic tsk12): when set, closing an effort projects
    /// derived process metrics (`effort.cycle_time_ms`, `task.efforts`) as
    /// facts under a capture that stamps the producing `effort_id`. Optional
    /// so bare TaskService tests skip the projection. Paired with `events` so
    /// the renderer refetches on a new capture.
    fact_store: Option<Arc<SqliteFactStore>>,
    events: Option<EventBus>,
    /// Kind-agnostic attribution ledger (tsk263). When set (with
    /// `effort_store`), closing an effort reconciles the run kinds too — the
    /// concurrent-effort runs left unattributed become the close residue.
    attribution: Option<Arc<SqliteAttributionStore>>,
    /// Steering-signal sources (tsk76): agent turns (user prompt submissions)
    /// and comment threads, counted into `oxplow.effort_steering` at close.
    /// Optional so bare TaskService tests skip that fact.
    agent_turn_store: Option<Arc<oxplow_db::SqliteAgentTurnStore>>,
    comment_store: Option<Arc<oxplow_db::SqliteCommentStore>>,
    /// Woken after a transition commits so the `work_item.transitioned`
    /// event it logged is delivered now rather than on the idle timer.
    event_pump: Option<Arc<crate::event_pump::EventPump>>,
}

/// How long a status change waits for the effort-lifecycle consumer (the
/// `effort_start` / `effort_end` snapshot) before returning anyway.
/// Long, like the inline capture it replaced (which had no limit): the
/// start snapshot is the effort's baseline, and on a huge repo it waits
/// for the startup sweep. It only bounds a stuck pump.
const LIFECYCLE_SETTLE: std::time::Duration = std::time::Duration::from_secs(600);

/// The producer of an effort's lifecycle facts (one capture per effort).
const LIFECYCLE_PRODUCER: &str = "effort-lifecycle";

/// Returns true iff any item in `items` has this id as its parent_id.
fn is_epic(item: &Task, items: &[Task]) -> bool {
    items.iter().any(|c| c.parent_id == Some(item.id))
}

/// How many recent agent turns the steering producer scans for a closing
/// effort's window (tsk76). `list_for_thread` is newest-first, so an effort
/// with more turns than this undercounts its oldest prompts — acceptable for
/// a per-close mean; a thousand-prompt effort has bigger problems.
const STEERING_TURN_SCAN: usize = 1000;

impl TaskService {
    pub fn new(store: Arc<SqliteTaskStore>) -> Self {
        Self {
            store,
            effort_store: None,
            snapshot_captures: None,
            thread_store: None,
            fact_store: None,
            events: None,
            attribution: None,
            agent_turn_store: None,
            comment_store: None,
            event_pump: None,
        }
    }

    /// Attach the event pump so a committed transition is delivered to
    /// its consumers right away.
    pub fn with_event_pump(mut self, pump: Arc<crate::event_pump::EventPump>) -> Self {
        self.event_pump = Some(pump);
        self
    }

    /// This service without its pump — what a pump consumer holds, so the
    /// pump → consumer → service → pump chain isn't a reference cycle.
    pub fn without_event_pump(&self) -> Self {
        Self {
            event_pump: None,
            ..self.clone()
        }
    }

    /// Attach the attribution ledger so closing an effort reconciles the run
    /// kinds (test/coverage/analysis) alongside files (tsk263).
    pub fn with_attribution(mut self, store: Arc<SqliteAttributionStore>) -> Self {
        self.attribution = Some(store);
        self
    }

    /// Attach the durable fact layer + event bus. When present (together with
    /// `with_effort_store`), closing an effort projects derived process
    /// metrics (`effort.cycle_time_ms`, `task.efforts`) as facts (epic tsk12).
    pub fn with_metrics(mut self, facts: Arc<SqliteFactStore>, events: EventBus) -> Self {
        self.fact_store = Some(facts);
        self.events = Some(events);
        self
    }

    /// Attach the effort store. Required (together with
    /// `with_snapshot_capture`) for automatic effort lifecycle on
    /// in_progress transitions.
    pub fn with_effort_store(mut self, store: Arc<SqliteEffortStore>) -> Self {
        self.effort_store = Some(store);
        self
    }

    /// Attach the per-stream registry. When present alongside
    /// `with_thread_store`, lifecycle snapshots route to the service
    /// matching the task's stream instead of always using the
    /// singleton attached via `with_snapshot_capture`.
    pub fn with_snapshot_captures(
        mut self,
        reg: crate::snapshot_capture_registry::SnapshotCaptureRegistry,
    ) -> Self {
        self.snapshot_captures = Some(reg);
        self
    }

    /// Attach the thread store. Together with `with_snapshot_captures`
    /// this enables per-stream lifecycle snapshots — `TaskService`
    /// loads the task's thread to learn which `stream_id` (and thus
    /// which `SnapshotCaptureService`) to drive.
    /// Wire the steering-signal sources (tsk76): agent turns (user prompt
    /// submissions) + comment threads. Optional so bare TaskService tests
    /// skip the steering fact — the other lifecycle facts still project.
    pub fn with_steering_sources(
        mut self,
        turns: Arc<oxplow_db::SqliteAgentTurnStore>,
        comments: Arc<oxplow_db::SqliteCommentStore>,
    ) -> Self {
        self.agent_turn_store = Some(turns);
        self.comment_store = Some(comments);
        self
    }

    pub fn with_thread_store(mut self, store: Arc<SqliteThreadStore>) -> Self {
        self.thread_store = Some(store);
        self
    }

    /// Resolve the snapshot service that should handle a lifecycle
    /// event for `thread_id`. Prefers the per-stream registry when
    /// configured. Returns `None` (and logs) when either the registry
    /// isn't wired or the thread / stream can't be resolved — callers
    /// then skip the snapshot step entirely.
    async fn service_for_thread(
        &self,
        thread_id: &ThreadId,
    ) -> Option<Arc<crate::snapshot_capture::SnapshotCaptureService>> {
        let reg = self.snapshot_captures.as_ref()?;
        let threads = self.thread_store.as_ref()?;
        match threads.get(thread_id).await {
            Ok(Some(thread)) => {
                let svc = reg.get(&thread.stream_id);
                if svc.is_none() {
                    tracing::debug!(
                        thread_id = %thread_id,
                        stream_id = %thread.stream_id,
                        "lifecycle: stream has no registered capture service",
                    );
                }
                svc
            }
            Ok(None) => {
                tracing::debug!(thread_id = %thread_id, "lifecycle: thread row missing");
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, thread_id = %thread_id, "lifecycle: thread lookup failed");
                None
            }
        }
    }

    /// Create a task attached to `thread` (or to the backlog if
    /// `thread` is `None`). Allocates a fresh id and sort_index.
    pub async fn create(
        &self,
        thread: Option<ThreadId>,
        input: CreateTaskInput,
    ) -> Result<Task, TaskServiceError> {
        let next_sort = self.next_sort_index(thread.as_ref()).await?;
        let now = Timestamp::now();
        let mut item = Task {
            // id assigned by store.insert
            id: TaskId::placeholder(),
            thread_id: thread,
            parent_id: input.parent_id,
            title: input.title,
            description: input.description.unwrap_or_default(),
            status: input.status.unwrap_or(TaskStatus::Ready),
            priority: input.priority.unwrap_or(TaskPriority::Medium),
            sort_index: next_sort,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: input.author.or(Some(TaskAuthor::User)),
        };
        // Filing straight into a status is that change from `ready`: into
        // `in_progress` (the path CLAUDE.md recommends to "start the work in
        // the same call") it opens the effort in the insert's transaction,
        // and any status but `ready` logs `work_item.transitioned`. The
        // start snapshot comes from the effort-lifecycle consumer.
        if item.status == TaskStatus::Done {
            item.completed_at = Some(now);
        }
        let (id, _effort) = self.store.insert_logged(&item).await?;
        item.id = id;
        if item.status != TaskStatus::Ready {
            self.settle_lifecycle().await;
        }
        Ok(item)
    }

    /// Apply a partial-patch to an existing task. Returns the
    /// post-patch row.
    pub async fn update(
        &self,
        id: TaskId,
        changes: UpdateTaskChanges,
    ) -> Result<Task, TaskServiceError> {
        let mut item = self.load(id).await?;
        if let Some(t) = changes.title {
            item.title = t;
        }
        if let Some(d) = changes.description {
            item.description = d;
        }
        if let Some(p) = changes.parent_id {
            item.parent_id = p;
        }
        if let Some(p) = changes.priority {
            item.priority = p;
        }
        item.updated_at = Timestamp::now();
        // Fields and any status change commit together; the status moves
        // from what's committed (read in the transaction), with what it
        // implies — the effort open/close and `work_item.transitioned`. The
        // effort's snapshot pin, reconciliation and lifecycle metrics are
        // the effort-lifecycle consumer's; settle so they're done when this
        // returns. Changes made for someone (MCP, RPC) run the
        // `work_item.update` command instead, which audits the actor.
        let prior_status = item.status;
        let after = self.store.update_with_status(&item, changes.status).await?;
        if after.status != prior_status {
            self.settle_lifecycle().await;
        }
        Ok(after)
    }

    /// Run the event pump now and wait (bounded) for the effort-lifecycle
    /// consumer to take and pin the snapshot a just-committed open/close
    /// logged, so a caller that reads the effort next (`complete_task`'s
    /// file review) sees its bracket. On timeout the work finishes on a
    /// later run; nothing is lost.
    pub async fn settle_lifecycle(&self) {
        if let Some(pump) = self.event_pump.as_ref() {
            if !pump
                .settle(&[crate::effort_lifecycle::NAME], LIFECYCLE_SETTLE)
                .await
            {
                tracing::debug!("effort lifecycle: pump didn't settle in time; continuing");
            }
        }
    }

    /// `effort.opened` (the effort-lifecycle consumer, P2.6.2): take the
    /// `effort_start` snapshot and pin it. Idempotent: an effort already
    /// pinned — or already closed, when delivery lagged — is left alone.
    /// A missing capture service or an empty tree pins nothing.
    pub(crate) async fn on_effort_opened(&self, effort_id: EffortId) -> Result<(), DomainError> {
        let Some(effort_store) = self.effort_store.as_ref() else {
            return Ok(());
        };
        let Some(effort) = effort_store.get_effort(&effort_id).await? else {
            return Ok(());
        };
        if effort.start_snapshot_id.is_some() {
            return Ok(());
        }
        let Some(snapshot) = self.service_for_thread(&effort.thread_id).await else {
            return Ok(());
        };
        if effort.ended_at.is_some() {
            // Delivered after its close (nothing settled in between): a
            // capture now would include the effort's own work, so the
            // baseline is the stream's last snapshot from before it began.
            if let Some(prior) = snapshot
                .store()
                .latest_snapshot_at_or_before(*snapshot.stream_id(), effort.started_at)
                .await?
            {
                effort_store.set_start_snapshot(&effort_id, prior).await?;
            }
            return Ok(());
        }
        // An effort's start baseline must reflect the full pre-edit tree:
        // wait for the startup sweep (a no-op once it's done).
        snapshot.await_initial_ready().await;
        let captured = snapshot
            .request_snapshot(crate::snapshot_capture::TakeRequest {
                trigger: oxplow_domain::snapshot::SnapshotTrigger::EffortStart,
                thread_id: Some(effort.thread_id),
                turn_id: None,
                effort_id: Some(effort_id),
                budget: None,
            })
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, effort = %effort_id, "effort lifecycle: start snapshot failed");
                None
            });
        if let Some(id) = captured {
            effort_store.set_start_snapshot(&effort_id, id).await?;
        }
        Ok(())
    }

    /// `effort.closed` (the effort-lifecycle consumer, P2.6.2): take and
    /// pin the `effort_end` snapshot, reconcile unclaimed files and runs,
    /// and project the lifecycle metrics. A retroactive effort (recorded
    /// after the fact) has no bracket, so it gets the metrics only.
    /// Re-delivery re-pins nothing (the pin is checked) and re-reconciles
    /// idempotently. Returns the finished effort (the consumer then logs
    /// `effort.finished`), or `None` when there is nothing to finish.
    pub(crate) async fn on_effort_closed(
        &self,
        effort_id: EffortId,
        retroactive: bool,
    ) -> Result<Option<Effort>, DomainError> {
        let Some(effort_store) = self.effort_store.as_ref() else {
            return Ok(None);
        };
        let Some(effort) = effort_store.get_effort(&effort_id).await? else {
            return Ok(None);
        };
        if effort.ended_at.is_none() {
            return Ok(None);
        }
        if !retroactive {
            if let Some(snapshot) = self.service_for_thread(&effort.thread_id).await {
                if effort.end_snapshot_id.is_none() {
                    let captured = snapshot
                        .request_snapshot(crate::snapshot_capture::TakeRequest {
                            trigger: oxplow_domain::snapshot::SnapshotTrigger::EffortEnd,
                            thread_id: Some(effort.thread_id),
                            turn_id: None,
                            effort_id: Some(effort_id),
                            budget: None,
                        })
                        .await
                        .unwrap_or_else(|e| {
                            tracing::warn!(error = %e, effort = %effort_id, "effort lifecycle: end snapshot failed");
                            None
                        });
                    // Keep "end_snapshot_id null ⇔ effort open": a close
                    // that captured nothing falls back to the start pin.
                    if let Some(id) = close_end_snapshot(captured, effort.start_snapshot_id) {
                        effort_store.set_end_snapshot(&effort_id, id).await?;
                    }
                }
                // Claim-first reconciliation: changed-but-not-claimed paths
                // become unattributed residue, and so do the concurrent
                // case's runs (tsk263/tsk269).
                let marked =
                    reconcile_unattributed_on_close(effort_store, snapshot.store(), &effort_id)
                        .await;
                if !marked.is_empty() {
                    tracing::debug!(effort = %effort_id, count = marked.len(), "effort close: recorded unattributed changes");
                }
            }
            if let (Some(attribution), Some(facts)) = (&self.attribution, &self.fact_store) {
                let kind = crate::attribution::RunKind::runs(effort_store, facts, attribution);
                let _ = crate::attribution::reconcile_close(&kind, &effort_id).await;
            }
        }
        self.project_effort_lifecycle_metrics(&effort, retroactive)
            .await;
        effort_store.get_effort(&effort_id).await
    }

    /// Project derived process metrics into the unified substrate when an
    /// effort closes (tsk216): `effort.cycle_time_ms` (how long the effort
    /// was open) and `task.efforts` (efforts-so-far for this task — the
    /// redo-rate signal). Reads `effort` as the source of truth; the
    /// table is untouched. Best-effort — a metric write error is logged and
    /// never blocks the status transition.
    /// `synthesized` marks an effort that `record_effort` created and closed in
    /// one action because the task was never `in_progress` (tsk172). Such an
    /// effort has no real duration — `started_at == ended_at` — so its
    /// `cycle_time` is suppressed rather than reported as 0, which would drag
    /// the mean down with a number that describes bookkeeping, not work. Every
    /// other lifecycle fact still lands, so the work stops being invisible to
    /// the pairing metrics.
    async fn project_effort_lifecycle_metrics(&self, effort: &Effort, synthesized: bool) {
        let Some(effort_store) = self.effort_store.as_ref() else {
            return;
        };
        let effort_id = &effort.id;
        let thread_id = &effort.thread_id;
        // The `task.efforts` redo-rate fact is per oxplow task.
        let task_id = effort.task_id();
        // Resolve the stream the thread belongs to (the hard CASCADE scope).
        let Some(thread_store) = self.thread_store.as_ref() else {
            return;
        };
        let stream_val = match thread_store.get(thread_id).await {
            Ok(Some(t)) => t.stream_id.value(),
            _ => return,
        };
        // Once per effort: a redelivered close (a crash before the pump's
        // checkpoint, a dead-letter retry) finds its capture and stops.
        if let Some(facts) = self.fact_store.as_ref() {
            match facts.captures_for_effort(effort_id.value()).await {
                Ok(caps) if caps.iter().any(|c| c.producer == LIFECYCLE_PRODUCER) => return,
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "effort lifecycle metrics: capture lookup failed");
                    return;
                }
            }
        }
        let Some(ended_at) = effort.ended_at else {
            return; // not actually closed — nothing to measure
        };
        let cycle_ms = (ended_at.unix_ms() - effort.started_at.unix_ms()).max(0);
        let efforts_so_far = match effort_store.list_for_work_item(&effort.work_item).await {
            Ok(rows) => rows.len() as i64,
            Err(e) => {
                tracing::warn!(error = %e, "effort lifecycle metrics: list_for_item failed");
                return;
            }
        };
        // Capture branch best-effort (process fact, tied to the worktree's
        // current branch). NULL when the stream has no capture service.
        let branch = match self.service_for_thread(thread_id).await {
            Some(svc) => svc.branch().await,
            None => None,
        };

        // Write the durable facts (epic tsk12; the legacy run/sample writes are
        // gone, T-E2): cycle time as a fact on `oxplow.cycle_time` (subject =
        // the just-closed effort) + the efforts-so-far count on
        // `oxplow.task_effort` (subject = the task, the redo-rate signal). The
        // capture stamps `effort_id` directly — this producer knows the exact
        // producing effort, so attribution is unambiguous (decision #11) — plus
        // the thread/branch spine. Best-effort.
        if let Some(facts) = self.fact_store.as_ref() {
            let dual = async {
                let mut rows = Vec::new();
                // Both measures are NON-ADDITIVE with denominator 1 (V47): the
                // cross-time collapse Σn/Σd is the MEAN across closes (average
                // cycle time / efforts-per-task), never a lifetime sum.
                // Stop-collecting gate (tsk31): only emit each lifecycle fact when
                // an enabled metric consumes its measure (`effort.cycle_time_ms` /
                // `task.efforts`).
                if !synthesized
                    && facts
                        .measure_has_active_spec("oxplow.cycle_time")
                        .await
                        .unwrap_or(true)
                {
                    if let Some(measure) = facts.get_measure("oxplow.cycle_time").await? {
                        rows.push(NewFact {
                            subject_kind: Some("effort".into()),
                            subject_ref: Some(effort_id.to_string()),
                            numerator: Some(cycle_ms as f64),
                            denominator: Some(1.0),
                            ..NewFact::new(measure.id, cycle_ms as f64)
                        });
                    }
                }
                if let Some(task_id) = task_id {
                    if facts
                        .measure_has_active_spec("oxplow.task_effort")
                        .await
                        .unwrap_or(true)
                    {
                        if let Some(measure) = facts.get_measure("oxplow.task_effort").await? {
                            rows.push(NewFact {
                                subject_kind: Some("task".into()),
                                subject_ref: Some(task_id.to_string()),
                                numerator: Some(efforts_so_far as f64),
                                denominator: Some(1.0),
                                ..NewFact::new(measure.id, efforts_so_far as f64)
                            });
                        }
                    }
                }
                // The effort's captures back several per-close producers below
                // (test outcome, time-to-green, tokens, steering) — fetch once.
                let effort_caps = facts.captures_for_effort(effort_id.value()).await?;
                let cap_ids: Vec<i64> = effort_caps.iter().map(|c| c.id).collect();
                // Per-effort test-outcome scalars (tsk38) + time-to-green
                // (tsk76): both read the effort's `oxplow.test_case` facts, so
                // those are fetched once when either gate is open.
                let outcome_gate = facts
                    .measure_has_active_spec("oxplow.effort_test_outcome")
                    .await
                    .unwrap_or(true);
                let ttg_gate = facts
                    .measure_has_active_spec("oxplow.effort_time_to_green")
                    .await
                    .unwrap_or(true);
                if outcome_gate || ttg_gate {
                    // The effort's runs are its run records: a green run
                    // that repeated the branch's results wrote no case facts
                    // (tsk733) and still counts.
                    let run_ids: Vec<i64> = effort_caps
                        .iter()
                        .filter(|c| c.producer == "tests")
                        .map(|c| c.id)
                        .collect();
                    if let Some(case_measure) = facts.get_measure("oxplow.test_case").await? {
                        let case_facts = facts
                            .facts_for_captures(case_measure.id, cap_ids.clone())
                            .await?;
                        // (capture_id, is_failed, subject_ref) per case fact — the
                        // grouping/ordering + scalar math live in `test_outcome`.
                        let tuples: Vec<(i64, bool, Option<String>)> = case_facts
                            .iter()
                            .map(|f| {
                                let failed = f
                                    .dims_json
                                    .as_deref()
                                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                                    .and_then(|v| {
                                        v.get("oxplow.status")
                                            .and_then(|x| x.as_str())
                                            .map(str::to_string)
                                    })
                                    .as_deref()
                                    == Some("failed");
                                (f.capture_id, failed, f.subject_ref.clone())
                            })
                            .collect();
                        if outcome_gate {
                            if let Some(outcome_measure) =
                                facts.get_measure("oxplow.effort_test_outcome").await?
                            {
                                let runs =
                                    crate::test_outcome::runs_with_failures(&run_ids, &tuples);
                                if let Some(outcome) =
                                    crate::test_outcome::compute_effort_test_outcome(&runs)
                                {
                                    for (stat, value) in [
                                        ("at_close", outcome.at_close),
                                        ("peak", outcome.peak),
                                        ("distinct_failed", outcome.distinct_failed),
                                        ("red_runs", outcome.red_runs),
                                    ] {
                                        rows.push(NewFact {
                                            subject_kind: Some("effort".into()),
                                            subject_ref: Some(effort_id.to_string()),
                                            numerator: Some(value as f64),
                                            denominator: Some(1.0),
                                            dims_json: serde_json::to_string(&serde_json::json!({
                                                "oxplow.tests_stat": stat
                                            }))
                                            .ok(),
                                            ..NewFact::new(outcome_measure.id, value as f64)
                                        });
                                    }
                                }
                            }
                        }
                        // Time-to-green (tsk76): wall-clock from the FIRST red
                        // run to the first green after it. Emitted only when
                        // that transition exists — always-green or never-green
                        // is "no data", not a zero.
                        if ttg_gate {
                            if let Some(ttg_measure) =
                                facts.get_measure("oxplow.effort_time_to_green").await?
                            {
                                let mut timed: Vec<(i64, bool)> = effort_caps
                                    .iter()
                                    .filter(|c| c.producer == "tests")
                                    .map(|c| {
                                        let red = tuples
                                            .iter()
                                            .any(|(cap, failed, _)| *cap == c.id && *failed);
                                        (c.captured_at.unix_ms(), red)
                                    })
                                    .collect();
                                timed.sort_by_key(|(at, _)| *at);
                                if let Some(ms) = crate::test_outcome::time_to_green_ms(&timed) {
                                    rows.push(NewFact {
                                        subject_kind: Some("effort".into()),
                                        subject_ref: Some(effort_id.to_string()),
                                        numerator: Some(ms as f64),
                                        denominator: Some(1.0),
                                        ..NewFact::new(ttg_measure.id, ms as f64)
                                    });
                                }
                            }
                        }
                    }
                }
                // Tokens the effort spent (tsk73) — ALL kinds (input + output +
                // cache read/write), summed from the effort-stamped otel token
                // facts. One fact per closed effort on `oxplow.effort_tokens`
                // (non-additive, den=1 → the collapse is MEAN tokens per close,
                // read by `task.tokens`). Token-denominated by decision — never
                // dollars. No fact when the effort has no token captures (an
                // unmetered effort is "no data", not a zero).
                if facts
                    .measure_has_active_spec("oxplow.effort_tokens")
                    .await
                    .unwrap_or(true)
                {
                    if let (Some(effort_tokens_measure), Some(tokens_measure)) = (
                        facts.get_measure("oxplow.effort_tokens").await?,
                        facts.get_measure("oxplow.tokens").await?,
                    ) {
                        let mut total: f64 = facts
                            .facts_for_captures(tokens_measure.id, cap_ids.clone())
                            .await?
                            .iter()
                            .map(|f| f.value)
                            .sum();
                        let mut any = total > 0.0;
                        if let Some(cache_measure) =
                            facts.get_measure("oxplow.cache_tokens").await?
                        {
                            let cache: f64 = facts
                                .facts_for_captures(cache_measure.id, cap_ids.clone())
                                .await?
                                .iter()
                                .map(|f| f.value)
                                .sum();
                            any = any || cache > 0.0;
                            total += cache;
                        }
                        if any {
                            rows.push(NewFact {
                                subject_kind: Some("effort".into()),
                                subject_ref: Some(effort_id.to_string()),
                                numerator: Some(total),
                                denominator: Some(1.0),
                                ..NewFact::new(effort_tokens_measure.id, total)
                            });
                            // Wasted-token ratio, denominator side (tsk77):
                            // the metered close enters the ratio as num 0 /
                            // den = spend, value 0 (SUM reads stay untouched
                            // by closes). The revert leg in collection.rs
                            // later adds (num = spend, den = 0) if this
                            // effort's commits get reverted — Σn/Σd across
                            // the measure is then wasted ÷ all metered spend.
                            // Rides inside the effort_tokens gate: the
                            // denominator IS this spend computation.
                            if facts
                                .measure_has_active_spec("oxplow.token_waste")
                                .await
                                .unwrap_or(true)
                            {
                                if let Some(waste_measure) =
                                    facts.get_measure("oxplow.token_waste").await?
                                {
                                    rows.push(NewFact {
                                        subject_kind: Some("effort".into()),
                                        subject_ref: Some(effort_id.to_string()),
                                        numerator: Some(0.0),
                                        denominator: Some(total),
                                        ..NewFact::new(waste_measure.id, 0.0)
                                    });
                                }
                            }
                        }
                    }
                }
                // Steering events (tsk76): how many times a human (or oxplow
                // on their behalf) had to intervene — user prompt submissions
                // (agent_turn rows opened in the effort window) + Stop-hook
                // nudges (the effort's `oxplow.nudge` facts) + user-authored
                // comments in the thread window. One fact per close on
                // `oxplow.effort_steering` (non-additive, den=1 → MEAN per
                // close, read by `task.steering`). ZERO is emitted — a fully
                // autonomous effort is real data, unlike unmetered tokens.
                // Interrupts are not counted: nothing records them yet.
                if facts
                    .measure_has_active_spec("oxplow.effort_steering")
                    .await
                    .unwrap_or(true)
                {
                    if let Some(steering_measure) =
                        facts.get_measure("oxplow.effort_steering").await?
                    {
                        let start_ms = effort.started_at.unix_ms();
                        let end_ms = ended_at.unix_ms();
                        let in_window = |ms: i64| ms >= start_ms && ms <= end_ms;
                        let mut total = 0.0_f64;
                        if let Some(turns) = self.agent_turn_store.as_ref() {
                            use oxplow_domain::stores::AgentTurnStore;
                            match turns.list_for_thread(thread_id, STEERING_TURN_SCAN).await {
                                Ok(rows) => {
                                    total +=
                                        rows.iter()
                                            .filter(|t| in_window(t.started_at.unix_ms()))
                                            .count() as f64;
                                }
                                Err(e) => {
                                    tracing::warn!(error = %e, "steering: turn scan failed")
                                }
                            }
                        }
                        if let Some(nudge_measure) = facts.get_measure("oxplow.nudge").await? {
                            total += facts
                                .facts_for_captures(nudge_measure.id, cap_ids.clone())
                                .await?
                                .iter()
                                .map(|f| f.value)
                                .sum::<f64>();
                        }
                        if let Some(comments) = self.comment_store.as_ref() {
                            use oxplow_domain::stores::CommentStore;
                            match comments.list_for_thread(thread_id).await {
                                Ok(rows) => {
                                    total +=
                                        rows.iter()
                                            .filter(|c| {
                                                c.comment.author != "agent"
                                                    && in_window(c.comment.created_at.unix_ms())
                                            })
                                            .count() as f64;
                                }
                                Err(e) => {
                                    tracing::warn!(error = %e, "steering: comment scan failed")
                                }
                            }
                        }
                        rows.push(NewFact {
                            subject_kind: Some("effort".into()),
                            subject_ref: Some(effort_id.to_string()),
                            numerator: Some(total),
                            denominator: Some(1.0),
                            ..NewFact::new(steering_measure.id, total)
                        });
                    }
                }
                if rows.is_empty() {
                    return Ok::<(), DomainError>(());
                }
                let mut capture =
                    NewMetricCapture::done(stream_val, LIFECYCLE_PRODUCER, LIFECYCLE_PRODUCER);
                capture.thread_id = Some(thread_id.value());
                capture.effort_id = Some(effort_id.value());
                capture.trigger = Some("on-effort-complete".into());
                capture.branch = branch.clone();
                facts.record_facts(capture, rows).await?;
                Ok(())
            }
            .await;
            // The change loop announces the facts that landed (P7.B1).
            if let Err(e) = dual {
                tracing::warn!(error = %e, "effort lifecycle: fact write failed");
            }
        }
    }

    pub async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<Task>, TaskServiceError> {
        Ok(self.store.list_for_thread(thread).await?)
    }

    /// Open + record + close an effort for `item` against `thread`.
    /// Declared `impacts` are persisted before finish so the
    /// page_ref projection runs once with the full payload.
    ///
    /// `worktree_root`, when supplied, lets the store classify each
    /// touched file as `Deleted` (file no longer on disk) vs.
    /// `Updated` (file still present). Without a baseline snapshot
    /// "Created" can't be distinguished from "Updated" by stat
    /// alone, so callers needing that signal should declare it via
    /// `impacts` (`{kind:"file", action:"created"}`). Pass `None`
    /// from tests / callers that don't have a worktree handle — the
    /// store falls back to `Updated` for every path, matching the
    /// pre-change behavior.
    // Each parameter is doing distinct semantic work — bundling
    // into a struct would hide that without buying anything.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_effort(
        &self,
        effort_store: &SqliteEffortStore,
        item: TaskId,
        thread: &ThreadId,
        touched_files: &[String],
        summary: Option<String>,
        impacts: &[TaskImpact],
        worktree_root: Option<&Path>,
    ) -> Result<(), TaskServiceError> {
        // Resolve the version triple from the most-recent effort
        // BEFORE the transaction — it reads the snapshot store. The
        // attribution itself (attach-or-start + files + impacts +
        // finish/summary) commits as one transaction, so a crash can
        // no longer leave files recorded without their summary/finish.
        // No prior effort means the atomic op below will SYNTHESIZE one: the
        // task was closed without ever being `in_progress` (tsk172). Its
        // `effort.opened` / `closed` are logged `retroactive`, and the
        // effort-lifecycle consumer projects its metrics — otherwise the work
        // is invisible to exactly the metrics that measure the pairing.
        let prior = effort_store
            .most_recent_for_work_item(&work_item_ref(item))
            .await?;
        let version = match prior {
            Some(e) => self.resolve_effort_file_version(&e).await,
            // No effort yet — the atomic op will open one with no
            // snapshot pin, so the version triple is the unpinned
            // default.
            None => crate::file_ref_version::ResolvedFileVersion {
                local_snapshot_id: 0,
                closest_vcs_rev: None,
                vcs_rev_exact: false,
            },
        };
        let files: Vec<(String, oxplow_db::EffortFileChange)> = self
            .claimable_paths(thread, touched_files)
            .await
            .into_iter()
            .map(|p| {
                let change = classify_change(worktree_root, &p);
                (p, change)
            })
            .collect();
        effort_store
            .record_effort_atomic(oxplow_db::RecordEffortAtomic {
                work_item: work_item_ref(item),
                thread: *thread,
                files,
                version: oxplow_db::OwnedFileRefVersion {
                    local_snapshot_id: version.local_snapshot_id,
                    closest_vcs_rev: version.closest_vcs_rev,
                    vcs_rev_exact: version.vcs_rev_exact,
                },
                impacts: impacts.to_vec(),
                summary,
            })
            .await?;
        // A synthesized effort's lifecycle metrics come from the
        // effort-lifecycle consumer (its `effort.closed` is retroactive).
        self.settle_lifecycle().await;
        Ok(())
    }

    /// Auto-claim a single file the agent just edited onto the thread's
    /// OPEN effort, in real time from the PostToolUse hook (Child 1 of the
    /// claim-first attribution epic). Idempotent — `record_file` is
    /// `INSERT OR REPLACE` keyed on `(effort_id, path)`, so the agent's
    /// `touched_files` at completion merely confirms/amends rather than
    /// enumerating from scratch. Returns `Ok(true)` when a claim was
    /// recorded, `Ok(false)` when no effort is open (no-op). Best-effort:
    /// the PostToolUse caller swallows errors so the hook never fails.
    pub async fn claim_open_effort_file(
        &self,
        effort_store: &SqliteEffortStore,
        thread: &ThreadId,
        path: &str,
        worktree_root: Option<&Path>,
    ) -> Result<bool, TaskServiceError> {
        self.claim_effort_file(effort_store, thread, None, path, worktree_root)
            .await
    }

    /// [`Self::claim_open_effort_file`] for an edit recorded with the effort
    /// it happened in (`anchored`, the event's effort anchor — P3.5): that
    /// effort takes the claim even when it has closed since (the reactor
    /// can run after the close; `record_file` also clears the path from
    /// the close's unattributed list). With no anchor, the thread's open
    /// efforts decide as below.
    pub async fn claim_effort_file(
        &self,
        effort_store: &SqliteEffortStore,
        thread: &ThreadId,
        anchored: Option<oxplow_domain::EffortId>,
        path: &str,
        worktree_root: Option<&Path>,
    ) -> Result<bool, TaskServiceError> {
        if path.is_empty() {
            return Ok(false);
        }
        // A path the project never snapshots can't be attributed — see
        // `claimable_paths`.
        if self
            .claimable_paths(thread, &[path.to_string()])
            .await
            .is_empty()
        {
            return Ok(false);
        }
        // One open effort ⇒ unambiguous, claim it.
        //
        // Several open ⇒ ASK WHICH ONE rather than giving up (tsk186). This used
        // to return early on the grounds that "we can't know which one edited
        // the file" — but that switched claiming off in exactly the situation
        // where attribution is hardest, and the cost compounds: run attribution
        // scores against claimed files, so an unclaimed file also means
        // unattributed test runs, which means a close-time reconcile the user
        // has to do by hand.
        //
        // The same scoring the run auto-claim uses decides it: the edited path
        // against each open effort's claimed files ∪ its task's named paths,
        // strict unique winner only. A tie still declines — a WRONG claim
        // misreports what an effort did, which is worse than a missing one the
        // agent can add at close.
        let anchored = match anchored {
            Some(id) => effort_store.get_effort(&id).await?,
            None => None,
        };
        let single = match anchored {
            Some(e) => Some(e),
            None => effort_store.find_single_open_for_thread(thread).await?,
        };
        let effort = match single {
            Some(e) => e,
            None => {
                let open = effort_store.list_open_for_thread(thread).await?;
                match crate::attribution::resolve_by_targets(
                    effort_store,
                    &self.store,
                    open,
                    &[path.to_string()],
                )
                .await
                {
                    Some(e) => e,
                    None => return Ok(false),
                }
            }
        };
        let version = self.resolve_effort_file_version(&effort).await;
        let change = classify_change(worktree_root, path);
        effort_store
            .record_file(&effort.id, path, change, version.as_ref())
            .await?;
        Ok(true)
    }

    /// The subset of `paths` that effort attribution can actually own,
    /// in input order: non-empty, and not excluded from snapshot capture
    /// by the stream's workspace filter (the project's
    /// `generated.exclude` list or `.gitignore`).
    ///
    /// tsk249: an excluded path is deliberately never snapshotted, so
    /// the close-time diff can never confirm it. Recording a claim on
    /// one guarantees it lands in `claimed_but_not_changed` on every
    /// close — a nudge the agent can only ever answer with "yes, that
    /// was right". Silently dropping it is the honest outcome: oxplow
    /// doesn't track the file, so it doesn't ask about it either.
    /// (The reverse direction needs no filtering — an excluded path is
    /// absent from the diff, so it can't be `changed_but_not_claimed`.)
    ///
    /// Paths pass through unfiltered when no capture service is
    /// reachable for the thread (bare TaskService in tests, a stream
    /// with no registered capture) — filtering is a noise reduction,
    /// never a reason to lose a claim.
    pub async fn claimable_paths(&self, thread: &ThreadId, paths: &[String]) -> Vec<String> {
        let named: Vec<String> = paths.iter().filter(|p| !p.is_empty()).cloned().collect();
        if named.is_empty() {
            return named;
        }
        let svc = match self.service_for_thread(thread).await {
            Some(svc) => svc,
            None => return named,
        };
        named
            .into_iter()
            .filter(|p| {
                let excluded = svc.excluded_from_capture(Path::new(p));
                if excluded {
                    tracing::debug!(
                        path = %p,
                        "attribution: dropping claim on a path excluded from snapshot capture",
                    );
                }
                !excluded
            })
            .collect()
    }

    pub async fn list_backlog(&self) -> Result<Vec<Task>, TaskServiceError> {
        Ok(self.store.list_backlog().await?)
    }

    /// Pin the local snapshot id used by the effort's file-ref rows
    /// and resolve its closest git commit. Falls back to a 0
    /// snapshot id when neither end nor start is set (rare —
    /// only happens for an effort opened without a snapshot pin and
    /// no snapshot service attached). Stamping the snapshot with a
    /// revision later (`stamp_revision_tx`) retroactively flips
    /// `vcs_rev_exact` to true if a commit lands on the chosen
    /// snapshot later.
    pub async fn resolve_effort_file_version(
        &self,
        effort: &oxplow_db::Effort,
    ) -> crate::file_ref_version::ResolvedFileVersion {
        let snapshot_id = effort
            .end_snapshot_id
            .or(effort.start_snapshot_id)
            .unwrap_or(0);
        let svc = self.service_for_thread(&effort.thread_id).await;
        match svc {
            Some(svc) if snapshot_id != 0 => svc.resolve_file_version(snapshot_id).await.unwrap_or(
                crate::file_ref_version::ResolvedFileVersion {
                    local_snapshot_id: snapshot_id,
                    closest_vcs_rev: None,
                    vcs_rev_exact: false,
                },
            ),
            _ => crate::file_ref_version::ResolvedFileVersion {
                local_snapshot_id: snapshot_id,
                closest_vcs_rev: None,
                vcs_rev_exact: false,
            },
        }
    }
}

/// Set-wise diff between what the agent claimed in `touched_files`
/// and what the snapshot bracket actually shows changed during the
/// effort. Returned alongside the task on `complete_task` and
/// surfaced via the Stop hook so the agent can choose to amend.
/// Skipped (None) entirely when the auto-diff matches the claim, or
/// when no snapshot bracket is available (effort has no start/end
/// snapshot pin yet).
#[derive(Debug, Clone, serde::Serialize)]
pub struct EffortFileReview {
    pub effort_id: String,
    /// The effort's work item (`work_item:oxplow:tsk42`).
    pub work_item: String,
    /// Paths the agent claimed but the auto-diff doesn't see as
    /// changed. Disclaim via `amend_effort(remove_files=…)` if not
    /// actually touched.
    pub claimed_but_not_changed: Vec<String>,
    /// Paths the auto-diff sees as changed but the agent didn't
    /// claim. Capped at `MAX_UNCLAIMED_FOR_REVIEW`; when larger,
    /// the field is empty and `unclaimed_overflow` is set.
    pub changed_but_not_claimed: Vec<String>,
    /// Number of changed-but-not-claimed paths the diff actually
    /// contained, before any cap was applied. `None` means the
    /// list is the full set.
    pub unclaimed_overflow: Option<usize>,
}

/// Cap on the "files in the diff that the agent didn't claim" list
/// surfaced to the agent. Above this volume something else is
/// happening (overlapping efforts, formatter, codegen, user edits)
/// and the agent can't be expected to triage a wall of paths.
pub const MAX_UNCLAIMED_FOR_REVIEW: usize = crate::attribution::MAX_UNCLAIMED_FOR_REVIEW;

/// Compare the agent's declared `touched_files` for a task's
/// most-recent effort against the auto-diff between
/// start_snapshot_id and end_snapshot_id. Returns `None` when
/// nothing's worth showing the agent — claim and diff agree, or no
/// snapshot bracket exists yet.
pub async fn compute_effort_file_review(
    effort_store: &SqliteEffortStore,
    snapshot_store: &SqliteSnapshotStore,
    task_id: TaskId,
    claimed: &[String],
) -> Option<EffortFileReview> {
    let effort = effort_store
        .most_recent_for_work_item(&work_item_ref(task_id))
        .await
        .ok()
        .flatten()?;
    let changed = effort_changed_paths(snapshot_store, &effort).await?;
    let acknowledged = effort_store
        .list_acknowledged_paths(&effort.id)
        .await
        .ok()?;
    let other_claimed = effort_store
        .paths_claimed_by_intervening_efforts(&effort.id)
        .await
        .ok()?;
    review_from_lists(
        &effort.id,
        &effort.work_item,
        claimed,
        &changed,
        &acknowledged,
        &other_claimed,
    )
}

/// The set of paths whose content changed between the effort's start
/// and end snapshots, via the shared snapshot diff (content/hash-based,
/// not snapshot-row membership). `None` if the effort has no snapshot
/// bracket yet.
async fn effort_changed_paths(
    snapshot_store: &SqliteSnapshotStore,
    effort: &oxplow_db::Effort,
) -> Option<Vec<String>> {
    let (start, end) = (effort.start_snapshot_id?, effort.end_snapshot_id?);
    let changes = snapshot_store.diff_snapshots(Some(start), end).await.ok()?;
    Some(changes.into_iter().map(|c| c.path).collect())
}

/// Recompute a review for a specific effort id. The Stop hook
/// uses this to refresh a stale review after the agent may have
/// called `amend_effort`. Returns `None` when the effort no longer
/// has a discrepancy (or doesn't exist / has no snapshot bracket).
pub async fn recompute_effort_file_review(
    effort_store: &SqliteEffortStore,
    snapshot_store: &SqliteSnapshotStore,
    effort_id: &EffortId,
) -> Option<EffortFileReview> {
    let effort = effort_store.get_effort(effort_id).await.ok().flatten()?;
    let files = effort_store.list_files(effort_id).await.ok()?;
    let claimed: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
    let changed = effort_changed_paths(snapshot_store, &effort).await?;
    let acknowledged = effort_store.list_acknowledged_paths(effort_id).await.ok()?;
    let other_claimed = effort_store
        .paths_claimed_by_intervening_efforts(effort_id)
        .await
        .ok()?;
    review_from_lists(
        effort_id,
        &effort.work_item,
        &claimed,
        &changed,
        &acknowledged,
        &other_claimed,
    )
}

/// Reconcile an effort's claimed files against the snapshot diff at CLOSE
/// time and persist the `changed_but_not_claimed` delta as **unattributed**
/// audit residue (Child 2 of the claim-first attribution epic). Runs on
/// every snapshot-bracketed close (`TaskService::update` out of
/// `in_progress`, so IPC `update_task`, MCP `update_task`, and the close
/// half of `complete_task` all flow through it). Best-effort: returns the
/// marked paths, or an empty vec when the effort has no snapshot bracket
/// (e.g. a recovery-closed orphan with no end snapshot) or on any error —
/// it never blocks the close. The existing agent nudge
/// (`compute_effort_file_review`) is unaffected; this writes a separate
/// table so a path stays in exactly one of {claimed, unattributed}
/// (`record_file` clears the residue when a path is later claimed).
pub async fn reconcile_unattributed_on_close(
    effort_store: &SqliteEffortStore,
    snapshot_store: &SqliteSnapshotStore,
    effort_id: &EffortId,
) -> Vec<String> {
    // Files are now one `AttributionKind`; the close-time residue runs through
    // the shared engine (tsk261). Behavior-identical to the pre-refactor path.
    let kind = crate::attribution::FileKind::new(effort_store, snapshot_store);
    crate::attribution::reconcile_close(&kind, effort_id).await
}

/// Build a file review from the claimed/changed/acknowledged/other-claimed sets
/// via the kind-agnostic differ ([`crate::attribution::diff`]) — the file view
/// onto the shared reconciliation core.
fn review_from_lists(
    effort_id: &EffortId,
    work_item: &str,
    claimed: &[String],
    changed: &[String],
    acknowledged: &[String],
    other_claimed: &[String],
) -> Option<EffortFileReview> {
    let sets = crate::attribution::AttrSets {
        claimed: claimed.to_vec(),
        observed: changed.to_vec(),
        acknowledged: acknowledged.to_vec(),
        other_claimed: other_claimed.to_vec(),
    };
    let (claimed_but_not_changed, changed_but_not_claimed, unclaimed_overflow) =
        crate::attribution::diff(&sets, MAX_UNCLAIMED_FOR_REVIEW)?;
    Some(EffortFileReview {
        effort_id: effort_id.to_string(),
        work_item: work_item.to_string(),
        claimed_but_not_changed,
        changed_but_not_claimed,
        unclaimed_overflow,
    })
}

impl TaskService {
    /// Return the next dispatch unit for the orchestrator.
    pub async fn read_task_options(
        &self,
        thread: &ThreadId,
        link_store: &dyn TaskLinkStore,
    ) -> Result<ReadWorkOptionsResult, TaskServiceError> {
        let all = self.store.list_for_thread(thread).await?;
        let by_id: std::collections::HashMap<TaskId, Task> =
            all.iter().map(|i| (i.id, i.clone())).collect();

        let mut ready: Vec<Task> = all
            .iter()
            .filter(|i| i.status == TaskStatus::Ready)
            .cloned()
            .collect();
        ready.sort_by_key(|i| (i.sort_index, i.created_at));

        let mut unblocked_ready: Vec<Task> = Vec::new();
        for item in &ready {
            if !item_is_blocked(item.id, link_store, &by_id).await? {
                unblocked_ready.push(item.clone());
            }
        }

        let Some(head) = unblocked_ready.first().cloned() else {
            return Ok(ReadWorkOptionsResult::Empty);
        };

        if is_epic(&head, &all) {
            let mut children: Vec<Task> = Vec::new();
            let mut frontier = vec![head.id];
            while let Some(parent_id) = frontier.pop() {
                for it in &all {
                    if it.parent_id == Some(parent_id) {
                        if it.status == TaskStatus::Ready
                            && !item_is_blocked(it.id, link_store, &by_id).await?
                        {
                            children.push(it.clone());
                        }
                        frontier.push(it.id);
                    }
                }
            }
            children.sort_by_key(|i| (i.sort_index, i.created_at));
            return Ok(ReadWorkOptionsResult::Epic {
                epic: head,
                children,
            });
        }

        let standalone: Vec<Task> = unblocked_ready
            .into_iter()
            .filter(|i| !is_epic(i, &all))
            .collect();
        Ok(ReadWorkOptionsResult::Standalone { items: standalone })
    }

    pub(crate) async fn load(&self, id: TaskId) -> Result<Task, TaskServiceError> {
        self.store
            .get(id)
            .await?
            .ok_or(TaskServiceError::NotFound(id))
    }

    async fn next_sort_index(&self, thread: Option<&ThreadId>) -> Result<i64, TaskServiceError> {
        let items = match thread {
            Some(t) => self.store.list_for_thread(t).await?,
            None => self.store.list_backlog().await?,
        };
        Ok(items.iter().map(|i| i.sort_index).max().unwrap_or(-1) + 1)
    }
}

/// Classify how a path changed during an effort by stat-ing the
/// worktree. Without a baseline snapshot we can't reliably tell
/// "created" apart from "updated" (the agent might have edited a
/// pre-existing file too), so this returns:
///
///  - `Deleted` if the file is missing on disk now
///  - `Updated` if the file is present (the dominant case)
///
/// Agents that want explicit "created" attribution should declare
/// it via the `impacts` parameter on `complete_task`. Returns
/// `Updated` when `worktree_root` is `None` so test fixtures that
/// don't carry a real worktree keep their old behavior.
fn classify_change(worktree_root: Option<&Path>, path: &str) -> EffortFileChange {
    let Some(root) = worktree_root else {
        return EffortFileChange::Updated;
    };
    let resolved = root.join(path);
    match std::fs::symlink_metadata(&resolved) {
        Ok(_) => EffortFileChange::Updated,
        Err(_) => EffortFileChange::Deleted,
    }
}

/// The snapshot id to pin as an effort's `end_snapshot_id` on close.
/// Prefer the freshly-captured snapshot; when capture yields nothing
/// (a no-op close — nothing changed and the stream has no prior
/// snapshot — or a capture failure) fall back to the effort's own
/// `start_snapshot_id`. This keeps the invariant `end_snapshot_id`
/// null ⇔ effort in progress: a closed effort with any baseline is
/// never end-null (it degrades to an empty-diff effort). Returns
/// `None` only for a degenerate effort that has no baseline at all.
fn close_end_snapshot(captured: Option<i64>, effort_start: Option<i64>) -> Option<i64> {
    captured.or(effort_start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteStreamStore, SqliteThreadStore};
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadStatus};

    #[test]
    fn close_end_snapshot_prefers_capture_then_start() {
        // Fresh capture wins.
        assert_eq!(close_end_snapshot(Some(7), Some(3)), Some(7));
        // No-op close (nothing captured) falls back to the start pin,
        // so a closed effort with a baseline is never end-null.
        assert_eq!(close_end_snapshot(None, Some(3)), Some(3));
        // Truly empty effort (no baseline either) stays null.
        assert_eq!(close_end_snapshot(None, None), None);
    }

    #[test]
    fn classify_change_defaults_to_updated_without_worktree() {
        // No worktree → caller (test or a path that hasn't plumbed
        // the root yet) gets the same behavior as before the
        // detection landed.
        assert_eq!(
            classify_change(None, "src/anything.rs"),
            EffortFileChange::Updated
        );
    }

    #[test]
    fn classify_change_detects_deletion() {
        let tmp = tempfile::tempdir().unwrap();
        // File doesn't exist → Deleted.
        assert_eq!(
            classify_change(Some(tmp.path()), "missing.rs"),
            EffortFileChange::Deleted
        );
        // File exists → Updated (we can't tell created from
        // modified without a baseline snapshot).
        let real = tmp.path().join("real.rs");
        std::fs::write(&real, "fn main() {}").unwrap();
        assert_eq!(
            classify_change(Some(tmp.path()), "real.rs"),
            EffortFileChange::Updated
        );
    }

    #[test]
    fn review_subtracts_acknowledged_paths_from_unclaimed() {
        // Diff sees `extra.rs` plus `claimed.rs`; agent only claimed
        // `claimed.rs`. Without ack: `extra.rs` shows in
        // `changed_but_not_claimed`. With ack: it's filtered out
        // and the review collapses to `None`.
        let effort = EffortId::new(1);
        let task = "work_item:oxplow:tsk1";
        let claimed = vec!["claimed.rs".to_string()];
        let changed = vec!["claimed.rs".to_string(), "extra.rs".to_string()];
        let no_ack = review_from_lists(&effort, task, &claimed, &changed, &[], &[]);
        let r = no_ack.expect("unclaimed extra.rs should produce a review");
        assert_eq!(r.changed_but_not_claimed, vec!["extra.rs".to_string()]);
        let with_ack = review_from_lists(
            &effort,
            task,
            &claimed,
            &changed,
            &["extra.rs".to_string()],
            &[],
        );
        assert!(
            with_ack.is_none(),
            "acknowledged path should clear the discrepancy: {with_ack:?}",
        );
        // Same effect when another effort already claimed the path.
        let with_other = review_from_lists(
            &effort,
            task,
            &claimed,
            &changed,
            &[],
            &["extra.rs".to_string()],
        );
        assert!(
            with_other.is_none(),
            "path claimed by an intervening effort should clear the discrepancy: {with_other:?}",
        );
    }

    #[test]
    fn classify_change_treats_symlink_as_present() {
        // Even a broken symlink reports via symlink_metadata, so the
        // path is "present" from the agent's point of view —
        // resolving the link is a deletion concern.
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink("nowhere", &link).unwrap();
        #[cfg(not(unix))]
        {
            let _ = link;
            return;
        }
        assert_eq!(
            classify_change(Some(tmp.path()), "link"),
            EffortFileChange::Updated
        );
    }

    async fn fixture() -> (TaskService, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let store = Arc::new(SqliteTaskStore::new(db));
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/p".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::new(1),
            stream_id: s.id,
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            acp_agent: None,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        (TaskService::new(store), t.id)
    }

    async fn fixture_with_lifecycle() -> (
        TaskService,
        ThreadId,
        Arc<SqliteEffortStore>,
        tempfile::TempDir,
        crate::snapshot_capture_registry::SnapshotCaptureRegistry,
    ) {
        let project = tempfile::tempdir().unwrap();
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let task_store = Arc::new(SqliteTaskStore::new(db.clone()));
        let effort_store = Arc::new(SqliteEffortStore::new(db.clone()));
        let snapshot_store = Arc::new(oxplow_db::SqliteSnapshotStore::new(db.clone()));
        let blobs = crate::blob_store::BlobStore::new(project.path().join(".oxplow/snapshots"));
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: project.path().to_string_lossy().into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        // Build a single-entry registry pointing at the test project's
        // worktree. Per-stream is the registry's whole point — the
        // primary stream IS the only stream in these tests, but
        // TaskService routes through `get(&stream_id)` either way.
        let event_bus = crate::events::EventBus::new();
        let snapshot_captures = crate::snapshot_capture_registry::SnapshotCaptureRegistry::new(
            crate::snapshot_capture_registry::SnapshotCaptureRegistryConfig {
                vcs: std::sync::Arc::new(crate::vcs::GitProvider),
                snapshot_store: snapshot_store.clone(),
                blobs: blobs.clone(),
                max_file_bytes: 1_000_000,
                workspace_filter: oxplow_fs_watch::WorkspaceFilter::default(),
                open_turn_probe: None,
            },
        );
        // Drop the default-built service; tests need overridden
        // settle / predrain durations, so we re-insert a custom one
        // below. Both gates are independently covered in
        // `snapshot_capture::tests`.
        let _ = snapshot_captures
            .register(&s)
            .expect("test stream's worktree exists on disk");
        let snapshot_svc = Arc::new(
            crate::snapshot_capture::SnapshotCaptureService::new(
                snapshot_store,
                blobs,
                project.path().to_path_buf(),
                std::sync::Arc::new(crate::vcs::GitProvider),
                s.id,
                1_000_000,
                oxplow_fs_watch::WorkspaceFilter::default(),
            )
            .with_settle_duration(std::time::Duration::ZERO)
            .with_predrain_delay(std::time::Duration::ZERO),
        );
        snapshot_captures.unregister(&s.id);
        snapshot_captures.insert_for_test(s.id, snapshot_svc);
        snapshot_captures.set_primary(s.id);
        let t = Thread {
            id: ThreadId::new(2),
            stream_id: s.id,
            title: "t".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            acp_agent: None,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        let thread_store_for_svc = Arc::new(oxplow_db::SqliteThreadStore::new(db.clone()));
        let fact_store = Arc::new(oxplow_db::SqliteFactStore::new(db.clone()));
        // Seed the producer specs as boot does — the lifecycle producer gates on
        // `measure_has_active_spec` (tsk31).
        for spec in crate::producer_metrics::builtin_producer_specs() {
            fact_store.upsert_spec(spec).await.unwrap();
        }
        let svc = TaskService::new(task_store)
            .with_effort_store(effort_store.clone())
            .with_snapshot_captures(snapshot_captures.clone())
            .with_thread_store(thread_store_for_svc)
            .with_metrics(fact_store, event_bus.clone())
            .with_steering_sources(
                Arc::new(oxplow_db::SqliteAgentTurnStore::new(db.clone())),
                Arc::new(oxplow_db::SqliteCommentStore::new(
                    db.clone(),
                    std::sync::Arc::new(oxplow_domain::events::schema::EventSchemaRegistry::core()),
                )),
            );
        let svc = with_lifecycle_pump(svc, &db);
        (svc, t.id, effort_store, project, snapshot_captures)
    }

    /// The pump with the effort-lifecycle consumer on it, as `Services`
    /// wires it: effort snapshots, reconciliation and lifecycle metrics
    /// run there (P2.6.2).
    fn with_lifecycle_pump(svc: TaskService, db: &Database) -> TaskService {
        let log = oxplow_db::SqliteEventLogStore::new(
            db.clone(),
            Arc::new(oxplow_domain::EventSchemaRegistry::core()),
        );
        let pump = Arc::new(crate::event_pump::EventPump::new(db.clone(), log, vec![]));
        pump.register_async(Arc::new(
            crate::effort_lifecycle::EffortLifecycleConsumer::new(
                svc.without_event_pump(),
                oxplow_db::SqliteEventLogStore::new(
                    db.clone(),
                    Arc::new(oxplow_domain::EventSchemaRegistry::core()),
                ),
            ),
        ));
        svc.with_event_pump(pump)
    }

    /// Review of P2.6 (tsk461): an open and its close both pending when the
    /// pump runs (nothing settled in between) still get a baseline — the
    /// stream's last snapshot from before the effort started, rather than
    /// no start pin at all.
    #[tokio::test]
    async fn an_open_delivered_after_its_close_pins_the_prior_snapshot() {
        let (svc, tid, effort_store, project, captures) = fixture_with_lifecycle().await;
        let primary = captures.primary().unwrap();
        std::fs::write(project.path().join("base.txt"), "b").unwrap();
        primary.mark_dirty(
            project.path().join("base.txt"),
            oxplow_fs_watch::WatchEventKind::Other,
        );
        let baseline = primary
            .request_snapshot(crate::snapshot_capture::TakeRequest {
                trigger: oxplow_domain::snapshot::SnapshotTrigger::Manual,
                thread_id: None,
                turn_id: None,
                effort_id: None,
                budget: None,
            })
            .await
            .unwrap()
            .expect("a baseline snapshot");
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "fast".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let quiet = svc.without_event_pump();
        for status in [TaskStatus::InProgress, TaskStatus::Done] {
            quiet
                .update(
                    item.id,
                    UpdateTaskChanges {
                        status: Some(status),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
        }
        svc.event_pump.clone().unwrap().run_once().await.unwrap();
        let effort = &effort_store
            .list_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap()[0];
        assert_eq!(effort.start_snapshot_id, Some(baseline));
        assert!(effort.end_snapshot_id.is_some());
    }

    /// P2.6.2 (tsk454): the effort-start snapshot is the pump's, so a
    /// process that dies after the transition commits — before the snapshot
    /// — has it taken by the next pump run instead of losing the pin.
    #[tokio::test]
    async fn a_committed_open_is_pinned_by_the_next_pump_run_after_a_crash() {
        let (svc, tid, effort_store, project, captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "crash".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let primary = captures.primary().unwrap();
        std::fs::write(project.path().join("a.txt"), "v").unwrap();
        primary.mark_dirty(
            project.path().join("a.txt"),
            oxplow_fs_watch::WatchEventKind::Other,
        );
        // The "crashed" process: it commits the transition, and no pump runs.
        svc.without_event_pump()
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let open = effort_store
            .find_open_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap()
            .unwrap();
        assert!(open.start_snapshot_id.is_none(), "nothing pinned yet");

        // The restart's pump picks the logged `effort.opened` up.
        let pump = svc.event_pump.clone().unwrap();
        pump.run_once().await.unwrap();
        let pinned = effort_store.get_effort(&open.id).await.unwrap().unwrap();
        assert!(pinned.start_snapshot_id.is_some(), "{pinned:?}");
        // Re-delivery (a crash before the checkpoint) changes nothing.
        svc.on_effort_opened(open.id).await.unwrap();
        let still = effort_store.get_effort(&open.id).await.unwrap().unwrap();
        assert_eq!(still.start_snapshot_id, pinned.start_snapshot_id);
    }

    #[tokio::test]
    async fn in_progress_transition_opens_effort_with_start_snapshot() {
        let (svc, tid, effort_store, _project, captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "lifecycle".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Ready → InProgress: opens an effort with start_snapshot_id.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let open = effort_store
            .find_open_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap()
            .expect("effort should be open");
        // Dirty set is empty in tests (no actual fs writes), so the
        // first snapshot returns None. The effort still opens but
        // start_snapshot_id is None — that's the "nothing to pin"
        // case and is fine. To verify the snapshot path actually
        // ran, write a file first.
        assert!(open.ended_at.is_none());
        assert!(open.start_snapshot_id.is_none());

        // Mark a file dirty so the next request_snapshot produces
        // a non-empty result.
        let svc_for_dirty = captures
            .primary()
            .expect("primary service registered in fixture");
        std::fs::write(_project.path().join("a.txt"), "v").unwrap();
        svc_for_dirty.mark_dirty(
            _project.path().join("a.txt"),
            oxplow_fs_watch::WatchEventKind::Other,
        );

        // InProgress → Done: closes the open effort with end_snapshot_id.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let efforts = effort_store
            .list_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap();
        assert_eq!(efforts.len(), 1);
        let closed = &efforts[0];
        assert!(closed.ended_at.is_some());
        assert!(closed.end_snapshot_id.is_some());
        // And no new effort was opened.
        assert!(effort_store
            .find_open_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn closing_a_never_started_task_still_counts_toward_efforts_per_task() {
        // tsk172: `complete_task` on a task that was never `in_progress`
        // synthesizes the effort so `touched_files` attributes — but the status
        // transition never crosses OUT of the in-progress band, so
        // `project_effort_lifecycle_metrics` never ran and the work was invisible
        // to the pairing metrics. Verified in a real DB: two such efforts had
        // files recorded but ZERO facts on `oxplow.cycle_time` /
        // `oxplow.effort_steering`, where properly-opened efforts all had them.
        //
        // The bias mattered: this shape is most common for small, quick tasks,
        // so the redo-rate signal skewed optimistic.
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "closed without ever starting".into(),
                    status: Some(TaskStatus::Ready),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        svc.record_effort(
            &effort_store,
            item.id,
            &tid,
            &["src/a.rs".to_string()],
            Some("done".into()),
            &[],
            None,
        )
        .await
        .unwrap();

        let facts = svc.fact_store.as_ref().expect("fact store attached");
        let effort_measure = facts
            .get_measure("oxplow.task_effort")
            .await
            .unwrap()
            .expect("effort measure seeded by V43");
        let effort_facts = facts.facts_for_measure(effort_measure.id).await.unwrap();
        assert_eq!(
            effort_facts.len(),
            1,
            "the synthesized effort must count toward efforts-per-task"
        );
        assert_eq!(effort_facts[0].subject_kind.as_deref(), Some("task"));

        // Cycle time is deliberately NOT emitted: `started_at == ended_at` on a
        // synthesized effort, so a fact here would report 0 and drag the mean
        // down with a number that describes bookkeeping rather than work.
        let cycle_measure = facts
            .get_measure("oxplow.cycle_time")
            .await
            .unwrap()
            .expect("cycle_time measure seeded by V43");
        assert!(
            facts
                .facts_for_measure(cycle_measure.id)
                .await
                .unwrap()
                .is_empty(),
            "a synthesized effort has no real duration — better absent than 0"
        );
    }

    /// Review of P2.6 (tsk462): re-delivering a close (a crash before the
    /// checkpoint, a dead-letter retry) projects the lifecycle facts once
    /// and keeps the first end pin.
    #[tokio::test]
    async fn a_redelivered_close_records_its_lifecycle_once() {
        let (svc, tid, effort_store, project, captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "twice".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let primary = captures.primary().unwrap();
        std::fs::write(project.path().join("work.txt"), "w").unwrap();
        primary.mark_dirty(
            project.path().join("work.txt"),
            oxplow_fs_watch::WatchEventKind::Other,
        );
        svc.update(
            item.id,
            UpdateTaskChanges {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let effort = effort_store
            .list_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap()
            .remove(0);
        let facts = svc.fact_store.as_ref().unwrap();
        let lifecycle = |caps: Vec<oxplow_db::MetricCapture>| {
            caps.iter()
                .filter(|c| c.producer == "effort-lifecycle")
                .count()
        };
        let before = lifecycle(facts.captures_for_effort(effort.id.value()).await.unwrap());
        assert_eq!(before, 1);

        assert!(effort.end_snapshot_id.is_some());
        // The tree moves on, then the close is handled again.
        std::fs::write(project.path().join("later.txt"), "x").unwrap();
        primary.mark_dirty(
            project.path().join("later.txt"),
            oxplow_fs_watch::WatchEventKind::Other,
        );
        svc.on_effort_closed(effort.id, false).await.unwrap();
        let after = lifecycle(facts.captures_for_effort(effort.id.value()).await.unwrap());
        assert_eq!(after, 1, "no second lifecycle capture");
        let again = effort_store.get_effort(&effort.id).await.unwrap().unwrap();
        assert_eq!(again.end_snapshot_id, effort.end_snapshot_id);
    }

    #[tokio::test]
    async fn closing_an_effort_projects_lifecycle_metrics() {
        // tsk216: leaving in_progress closes the effort and projects
        // `effort.cycle_time_ms` + `task.efforts` into the metric substrate,
        // reading `effort` as the source of truth.
        let (svc, tid, _effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "measured".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // No facts while the effort is still open.
        let facts = svc.fact_store.as_ref().expect("fact store attached");
        let cycle_measure = facts
            .get_measure("oxplow.cycle_time")
            .await
            .unwrap()
            .expect("cycle_time measure seeded by V43");
        assert!(facts
            .facts_for_measure(cycle_measure.id)
            .await
            .unwrap()
            .is_empty());

        // InProgress → Done closes the effort and fires the projection: one
        // `oxplow.cycle_time` fact, subject = the closed effort, on a capture
        // that stamped the producing effort_id (unambiguous, decision #11).
        // (The legacy definition/sample writes are gone, T-E2.)
        svc.update(
            item.id,
            UpdateTaskChanges {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let cycle_measure = facts
            .get_measure("oxplow.cycle_time")
            .await
            .unwrap()
            .expect("cycle_time measure seeded by V43");
        let cycle_facts = facts.facts_for_measure(cycle_measure.id).await.unwrap();
        assert_eq!(cycle_facts.len(), 1, "one cycle-time fact per close");
        assert_eq!(cycle_facts[0].subject_kind.as_deref(), Some("effort"));
        assert!(cycle_facts[0].value >= 0.0, "cycle time is non-negative");
        // Ratio components (tsk42): the measure is non-additive with den=1, so
        // the cross-time collapse Σn/Σd is the MEAN cycle time across closed
        // efforts — never a lifetime sum.
        assert_eq!(cycle_facts[0].numerator, Some(cycle_facts[0].value));
        assert_eq!(cycle_facts[0].denominator, Some(1.0));
        assert!(
            cycle_facts[0].effort_id.is_some(),
            "capture stamped the producing effort_id (unambiguous close)"
        );
        assert_eq!(
            cycle_facts[0].subject_ref.as_deref(),
            Some(
                EffortId::new(cycle_facts[0].effort_id.unwrap())
                    .to_string()
                    .as_str()
            ),
            "subject_ref is the producing effort's display id"
        );

        // …and the efforts-so-far count on `oxplow.task_effort`, subject = the
        // task (the redo-rate signal the `task.efforts` spec averages).
        let effort_measure = facts
            .get_measure("oxplow.task_effort")
            .await
            .unwrap()
            .expect("effort measure seeded by V46");
        let effort_facts = facts.facts_for_measure(effort_measure.id).await.unwrap();
        assert_eq!(effort_facts.len(), 1, "one effort fact per close");
        assert_eq!(effort_facts[0].value, 1.0, "first effort for the task");
        assert_eq!(effort_facts[0].subject_kind.as_deref(), Some("task"));
        assert_eq!(
            effort_facts[0].subject_ref.as_deref(),
            Some(item.id.to_string().as_str())
        );
        // Ratio components (tsk42): `task.efforts` collapses Σn/Σd across closes
        // — the mean efforts-per-task, not the last-closed task's count.
        assert_eq!(effort_facts[0].numerator, Some(1.0));
        assert_eq!(effort_facts[0].denominator, Some(1.0));
    }

    #[tokio::test]
    async fn closing_an_effort_projects_its_token_spend() {
        // tsk73: at close, the effort's token spend (ALL kinds, from its
        // effort-stamped otel captures) lands as one `oxplow.effort_tokens`
        // fact — token-denominated, never dollars. `task.tokens` averages it.
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "token-metered".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let effort = effort_store
            .find_single_open_for_thread(&tid)
            .await
            .unwrap()
            .expect("open effort");

        // Simulate the OTLP ingest: an effort-stamped otel-tokens capture with
        // input/output on `oxplow.tokens` and a cache fact on
        // `oxplow.cache_tokens`.
        let facts = svc.fact_store.as_ref().expect("fact store attached");
        let tokens = facts.get_measure("oxplow.tokens").await.unwrap().unwrap();
        let cache = facts
            .get_measure("oxplow.cache_tokens")
            .await
            .unwrap()
            .unwrap();
        let mut capture = oxplow_db::NewMetricCapture::done(1, "otel-tokens", "otel");
        capture.thread_id = Some(tid.value());
        capture.effort_id = Some(effort.id.value());
        facts
            .record_facts(
                capture,
                vec![
                    oxplow_db::NewFact::new(tokens.id, 100.0),
                    oxplow_db::NewFact::new(tokens.id, 20.0),
                    oxplow_db::NewFact::new(cache.id, 700.0),
                ],
            )
            .await
            .unwrap();

        svc.update(
            item.id,
            UpdateTaskChanges {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let effort_tokens = facts
            .get_measure("oxplow.effort_tokens")
            .await
            .unwrap()
            .expect("effort_tokens measure seeded by V59");
        let spend = facts.facts_for_measure(effort_tokens.id).await.unwrap();
        assert_eq!(spend.len(), 1, "one token-spend fact per close");
        assert_eq!(spend[0].value, 820.0, "input + output + cache summed");
        assert_eq!(spend[0].subject_kind.as_deref(), Some("effort"));
        // Non-additive den=1: `task.tokens` collapses to MEAN per close.
        assert_eq!(spend[0].numerator, Some(820.0));
        assert_eq!(spend[0].denominator, Some(1.0));

        // tsk77: the close also enters the wasted-token ratio's DENOMINATOR —
        // one `oxplow.token_waste` row with num 0 / den = the spend, value 0
        // (so `task.tokens.wasted`'s SUM stays untouched by closes). The
        // numerator side comes later from the revert leg, if ever.
        let waste = facts
            .get_measure("oxplow.token_waste")
            .await
            .unwrap()
            .expect("token_waste measure seeded by V61");
        let waste_rows = facts.facts_for_measure(waste.id).await.unwrap();
        assert_eq!(waste_rows.len(), 1, "one denominator row per metered close");
        assert_eq!(waste_rows[0].value, 0.0);
        assert_eq!(waste_rows[0].numerator, Some(0.0));
        assert_eq!(waste_rows[0].denominator, Some(820.0));
        assert_eq!(waste_rows[0].subject_kind.as_deref(), Some("effort"));
    }

    #[tokio::test]
    async fn closing_an_effort_projects_steering_events() {
        // tsk76: at close, steering = user prompt submissions (agent_turn rows
        // opened in the effort window) + Stop-hook nudges (the effort's
        // `oxplow.nudge` facts) + user-authored comments in the thread window,
        // as ONE `oxplow.effort_steering` fact. `task.steering` averages it.
        use oxplow_domain::stores::AgentTurnStore;
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "steered".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let effort = effort_store
            .find_single_open_for_thread(&tid)
            .await
            .unwrap()
            .expect("open effort");

        // Two prompts inside the window + one ancient one outside it.
        let turns = svc.agent_turn_store.as_ref().expect("turn store attached");
        let in_window = Timestamp::from_unix_ms(effort.started_at.unix_ms() + 1);
        for started_at in [in_window, in_window, Timestamp::from_unix_ms(1)] {
            turns
                .open(&oxplow_domain::AgentTurn {
                    id: oxplow_domain::AgentTurnId::placeholder(),
                    thread_id: tid,
                    prompt: "steer".into(),
                    answer: None,
                    session_id: None,
                    started_at,
                    ended_at: None,
                    start_snapshot_id: None,
                    snapshot_id: None,
                })
                .await
                .unwrap();
        }

        // One nudge fact under an effort-stamped capture.
        let facts = svc.fact_store.as_ref().expect("fact store attached");
        let nudge = facts.get_measure("oxplow.nudge").await.unwrap().unwrap();
        let mut cap = oxplow_db::NewMetricCapture::done(1, "nudges", "nudges");
        cap.thread_id = Some(tid.value());
        cap.effort_id = Some(effort.id.value());
        facts
            .record_facts(cap, vec![oxplow_db::NewFact::new(nudge.id, 1.0)])
            .await
            .unwrap();

        // One user review comment in the thread + one agent-authored comment
        // that must NOT count (the agent steering itself isn't steering).
        for author in ["user", "agent"] {
            let new = oxplow_db::comment_store::NewComment {
                stream: StreamId::new(1),
                thread: Some(tid),
                target: oxplow_domain::CommentTarget {
                    kind: "work_item".into(),
                    id: format!("oxplow:{}", item.id),
                },
                quote: String::new(),
                selectors_json: "[]".into(),
                context_chain: Vec::new(),
                referenced_refs: Vec::new(),
                intent: oxplow_domain::CommentIntent::Followup,
                author: author.into(),
                body: "please adjust".into(),
            };
            facts
                .database()
                .transaction(move |tx| oxplow_db::comment_store::create_tx(tx, &new).map(|_| ()))
                .await
                .unwrap();
        }

        svc.update(
            item.id,
            UpdateTaskChanges {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let steering = facts
            .get_measure("oxplow.effort_steering")
            .await
            .unwrap()
            .expect("effort_steering measure seeded by V60");
        let got = facts.facts_for_measure(steering.id).await.unwrap();
        assert_eq!(got.len(), 1, "one steering fact per close");
        assert_eq!(
            got[0].value, 4.0,
            "2 in-window prompts + 1 nudge + 1 user comment; ancient prompt and agent comment excluded"
        );
        assert_eq!(got[0].subject_kind.as_deref(), Some("effort"));
        assert_eq!(got[0].numerator, Some(4.0));
        assert_eq!(got[0].denominator, Some(1.0));

        // No test runs happened → no time-to-green fact (None path).
        let ttg = facts
            .get_measure("oxplow.effort_time_to_green")
            .await
            .unwrap()
            .expect("effort_time_to_green measure seeded by V60");
        assert!(facts.facts_for_measure(ttg.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn closing_an_effort_projects_time_to_green() {
        // tsk76: red run @1s, green run @61s → one 60_000ms
        // `oxplow.effort_time_to_green` fact at close.
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "tdd".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let effort = effort_store
            .find_single_open_for_thread(&tid)
            .await
            .unwrap()
            .expect("open effort");

        let facts = svc.fact_store.as_ref().expect("fact store attached");
        let case = facts
            .get_measure("oxplow.test_case")
            .await
            .unwrap()
            .unwrap();
        for (at_ms, status) in [(1_000, "failed"), (61_000, "passed")] {
            let mut cap = oxplow_db::NewMetricCapture::done(1, "tests", "junit");
            cap.thread_id = Some(tid.value());
            cap.effort_id = Some(effort.id.value());
            cap.captured_at = Some(Timestamp::from_unix_ms(at_ms));
            facts
                .record_facts(
                    cap,
                    vec![oxplow_db::NewFact {
                        subject_kind: Some("test".into()),
                        subject_ref: Some("test:mod::case".into()),
                        dims_json: Some(format!(r#"{{"oxplow.status":"{status}"}}"#)),
                        ..oxplow_db::NewFact::new(case.id, 1.0)
                    }],
                )
                .await
                .unwrap();
        }

        svc.update(
            item.id,
            UpdateTaskChanges {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let ttg = facts
            .get_measure("oxplow.effort_time_to_green")
            .await
            .unwrap()
            .expect("effort_time_to_green measure seeded by V60");
        let got = facts.facts_for_measure(ttg.id).await.unwrap();
        assert_eq!(got.len(), 1, "one time-to-green fact per close");
        assert_eq!(got[0].value, 60_000.0, "first red → first green wall-clock");
        assert_eq!(got[0].subject_kind.as_deref(), Some("effort"));
        assert_eq!(got[0].numerator, Some(60_000.0));
        assert_eq!(got[0].denominator, Some(1.0));
    }

    /// tsk733: a green run that repeats the branch's previous results writes
    /// no per-case facts, so an effort's runs come from its run records — an
    /// effort whose only run is such a repeat still closed green.
    #[tokio::test]
    async fn an_effort_whose_run_wrote_no_case_facts_still_closed_green() {
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "no-op change".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let effort = effort_store
            .find_single_open_for_thread(&tid)
            .await
            .unwrap()
            .expect("open effort");
        let facts = svc.fact_store.as_ref().expect("fact store attached");
        let case = facts
            .get_measure("oxplow.test_case")
            .await
            .unwrap()
            .unwrap();
        let mut cap = oxplow_db::NewMetricCapture::done(1, "tests", "junit");
        cap.thread_id = Some(tid.value());
        cap.effort_id = Some(effort.id.value());
        facts.record_facts(cap, Vec::new()).await.unwrap();
        let _ = case;

        svc.update(
            item.id,
            UpdateTaskChanges {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let outcome = facts
            .get_measure("oxplow.effort_test_outcome")
            .await
            .unwrap()
            .expect("effort_test_outcome measure");
        let got = facts.facts_for_measure(outcome.id).await.unwrap();
        assert_eq!(got.len(), 4, "the run counts: four outcome stats at close");
        assert!(
            got.iter().all(|f| f.value == 0.0),
            "it closed green: {got:?}"
        );
    }

    #[tokio::test]
    async fn create_with_in_progress_opens_lifecycle_effort() {
        // Filing a task directly in `in_progress` (the path CLAUDE.md
        // recommends for "start the work in the same call") must run
        // the lifecycle hook — otherwise complete_task's TaskEnd
        // snapshot has no open effort to attach to and the snapshot
        // is orphaned.
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "born running".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let open = effort_store
            .find_open_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap()
            .expect("lifecycle effort should be open after in_progress create");
        assert!(open.ended_at.is_none());
    }

    #[tokio::test]
    async fn create_with_done_skips_effort_lifecycle() {
        // Filing directly in a terminal status (e.g. retroactively
        // logging completed work) must NOT open a lifecycle effort —
        // record_effort handles that synthesis itself, with the
        // touched_files payload.
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "retro".into(),
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(effort_store
            .find_open_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn record_effort_merges_into_lifecycle_effort() {
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "merge".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Open the lifecycle effort.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Close it.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Now record_effort comes in with touched files + summary.
        // It should attach to the already-closed lifecycle effort,
        // NOT create a second row.
        svc.record_effort(
            &effort_store,
            item.id,
            &tid,
            &["src/x.rs".to_string()],
            Some("did the thing".into()),
            &[],
            None,
        )
        .await
        .unwrap();
        let efforts = effort_store
            .list_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap();
        assert_eq!(efforts.len(), 1, "should still be a single effort row");
        let row = &efforts[0];
        assert_eq!(row.summary.as_deref(), Some("did the thing"));
        let files = effort_store.list_files(&row.id).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/x.rs");
    }

    /// tsk249: a path the workspace filter ignores (project
    /// `generated.exclude`, `.gitignore`) is never snapshotted, so it can
    /// never show up in the effort's diff — claiming one would be flagged
    /// as "claimed but not changed" on every single close. Drop it from
    /// the claim silently; the authored paths beside it still land.
    #[tokio::test]
    async fn record_effort_drops_paths_that_are_never_snapshotted() {
        let (svc, tid, effort_store, _project, captures) = fixture_with_lifecycle().await;
        captures.set_workspace_filter(oxplow_fs_watch::WorkspaceFilter::with_user_entries([
            "generated",
        ]));
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "codegen".into(),
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        svc.record_effort(
            &effort_store,
            item.id,
            &tid,
            &[
                "src/authored.rs".to_string(),
                "apps/desktop/src/generated/bindings.ts".to_string(),
            ],
            Some("regenerated the bindings".into()),
            &[],
            None,
        )
        .await
        .unwrap();
        let efforts = effort_store
            .list_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap();
        let files = effort_store.list_files(&efforts[0].id).await.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["src/authored.rs"],
            "generated path should be silently dropped, authored path kept"
        );
    }

    /// The PostToolUse auto-claim gets the same treatment — writing a
    /// generated file mid-effort records nothing rather than seeding a
    /// claim the diff can never confirm.
    #[tokio::test]
    async fn claim_open_effort_file_skips_paths_that_are_never_snapshotted() {
        let (svc, tid, effort_store, _project, captures) = fixture_with_lifecycle().await;
        captures.set_workspace_filter(oxplow_fs_watch::WorkspaceFilter::with_user_entries([
            "generated",
        ]));
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "codegen".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let claimed = svc
            .claim_open_effort_file(
                &effort_store,
                &tid,
                "apps/desktop/src/generated/bindings.ts",
                None,
            )
            .await
            .unwrap();
        assert!(!claimed, "a never-snapshotted path is not claimable");
        let efforts = effort_store
            .list_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap();
        let files = effort_store.list_files(&efforts[0].id).await.unwrap();
        assert!(files.is_empty(), "nothing should have been recorded");

        // An authored path in the same effort still claims normally.
        assert!(svc
            .claim_open_effort_file(&effort_store, &tid, "src/authored.rs", None)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn record_effort_creates_fresh_effort_when_no_lifecycle() {
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "direct".into(),
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // No lifecycle ran — task filed directly as done.
        svc.record_effort(
            &effort_store,
            item.id,
            &tid,
            &["a.rs".to_string()],
            Some("retro".into()),
            &[],
            None,
        )
        .await
        .unwrap();
        let efforts = effort_store
            .list_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap();
        assert_eq!(efforts.len(), 1);
        assert!(efforts[0].ended_at.is_some());
        assert_eq!(efforts[0].summary.as_deref(), Some("retro"));
    }

    #[tokio::test]
    async fn claim_open_effort_file_resolves_which_effort_under_concurrency() {
        // tsk186: with several efforts open this used to decline outright, so
        // nothing got claimed in exactly the case where attribution is hardest —
        // and because run scoring reads claimed files, that also left test runs
        // unattributed and forced a close-time reconcile by hand.
        //
        // Now the edited path is scored against each open effort's claimed files
        // ∪ its task's named paths. Neither task has claimed a file here, so the
        // task text is what decides — the realistic case, since a claim happens
        // on the FIRST edit of an effort.
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let mk = |title: &str, description: &str| {
            let svc = svc.clone();
            let (title, description) = (title.to_string(), description.to_string());
            async move {
                svc.create(
                    Some(tid),
                    CreateTaskInput {
                        title,
                        description: Some(description),
                        status: Some(TaskStatus::InProgress),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
            }
        };
        let api = mk("api work", "Rework `[[crates/oxplow-api/src/routes.rs]]`.").await;
        let ui = mk("ui work", "Restyle `[[apps/desktop/src/pages/Home.tsx]]`.").await;

        // Both efforts are open, so the old single-open rule declines.
        assert!(
            effort_store
                .find_single_open_for_thread(&tid)
                .await
                .unwrap()
                .is_none(),
            "fixture must have two open efforts"
        );

        let claimed = svc
            .claim_open_effort_file(&effort_store, &tid, "crates/oxplow-api/src/routes.rs", None)
            .await
            .unwrap();
        assert!(claimed, "the edited path names one effort's area");

        let api_effort = effort_store
            .find_open_for_work_item(&work_item_ref(api.id))
            .await
            .unwrap()
            .unwrap();
        let ui_effort = effort_store
            .find_open_for_work_item(&work_item_ref(ui.id))
            .await
            .unwrap()
            .unwrap();
        let api_files = effort_store.list_files(&api_effort.id).await.unwrap();
        let ui_files = effort_store.list_files(&ui_effort.id).await.unwrap();
        assert_eq!(
            api_files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec!["crates/oxplow-api/src/routes.rs"],
            "claimed by the effort whose task names it"
        );
        assert!(ui_files.is_empty(), "and NOT by the other one");
    }

    #[tokio::test]
    async fn claim_open_effort_file_declines_when_the_path_names_no_one() {
        // The safety property: a wrong claim misreports what an effort did, so
        // an ambiguous path must still claim nothing rather than pick.
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        for (title, description) in [
            ("a", "Touches `[[crates/oxplow-api/src/routes.rs]]`."),
            ("b", "Also touches `[[crates/oxplow-api/src/routes.rs]]`."),
        ] {
            svc.create(
                Some(tid),
                CreateTaskInput {
                    title: title.into(),
                    description: Some(description.into()),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        // Both name the same file → tie → decline.
        let claimed = svc
            .claim_open_effort_file(&effort_store, &tid, "crates/oxplow-api/src/routes.rs", None)
            .await
            .unwrap();
        assert!(!claimed, "a tie must not be broken by guessing");

        // A path neither names → no overlap → decline.
        let unrelated = svc
            .claim_open_effort_file(&effort_store, &tid, "docs/unrelated.md", None)
            .await
            .unwrap();
        assert!(!unrelated, "no overlap must decline");
    }

    #[tokio::test]
    async fn claim_open_effort_file_claims_on_open_effort_and_is_idempotent() {
        // Auto-claim (PostToolUse path) records a effort_file on the
        // thread's open effort, and a repeat claim of the same path is
        // idempotent (INSERT OR REPLACE → still one row).
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "claim".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let open = effort_store
            .find_open_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap()
            .expect("effort open");

        let claimed = svc
            .claim_open_effort_file(&effort_store, &tid, "src/edited.rs", None)
            .await
            .unwrap();
        assert!(claimed, "a claim should be recorded on the open effort");
        // Idempotent: claiming the same path again doesn't duplicate.
        let again = svc
            .claim_open_effort_file(&effort_store, &tid, "src/edited.rs", None)
            .await
            .unwrap();
        assert!(again);
        let files = effort_store.list_files(&open.id).await.unwrap();
        assert_eq!(files.len(), 1, "idempotent — one row");
        assert_eq!(files[0].path, "src/edited.rs");
    }

    #[tokio::test]
    async fn out_of_band_close_marks_unclaimed_changes_unattributed() {
        // An effort that changes a file nobody claimed, closed via a plain
        // status transition (not complete_task), records that file as
        // unattributed audit residue.
        let (svc, tid, effort_store, project, captures) = fixture_with_lifecycle().await;
        let dirty = captures.primary().expect("primary capture service");
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "oob".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Seed start-snapshot content.
        std::fs::write(project.path().join("a.rs"), "v1").unwrap();
        dirty.mark_dirty(
            project.path().join("a.rs"),
            oxplow_fs_watch::WatchEventKind::Other,
        );
        svc.update(
            item.id,
            UpdateTaskChanges {
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        // A parallel/unclaimed change during the effort.
        std::fs::write(project.path().join("parallel.rs"), "x").unwrap();
        dirty.mark_dirty(
            project.path().join("parallel.rs"),
            oxplow_fs_watch::WatchEventKind::Other,
        );
        // Out-of-band close: a plain Done transition (no complete_task,
        // no touched_files claim).
        svc.update(
            item.id,
            UpdateTaskChanges {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let efforts = effort_store
            .list_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap();
        let eff = &efforts[0];
        let unattributed = effort_store.list_unattributed_files(&eff.id).await.unwrap();
        assert!(
            unattributed.contains(&"parallel.rs".to_string()),
            "unclaimed change should be marked unattributed: {unattributed:?}"
        );
    }

    #[tokio::test]
    async fn claimed_change_is_not_marked_unattributed_on_close() {
        // A file the agent claimed in real time (Child 1 auto-claim) is NOT
        // marked unattributed when the effort closes.
        let (svc, tid, effort_store, project, captures) = fixture_with_lifecycle().await;
        let dirty = captures.primary().expect("primary capture service");
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "claimed".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        std::fs::write(project.path().join("a.rs"), "v1").unwrap();
        dirty.mark_dirty(
            project.path().join("a.rs"),
            oxplow_fs_watch::WatchEventKind::Other,
        );
        svc.update(
            item.id,
            UpdateTaskChanges {
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        std::fs::write(project.path().join("mine.rs"), "x").unwrap();
        dirty.mark_dirty(
            project.path().join("mine.rs"),
            oxplow_fs_watch::WatchEventKind::Other,
        );
        // Claim it (as the PostToolUse auto-claim would).
        svc.claim_open_effort_file(&effort_store, &tid, "mine.rs", Some(project.path()))
            .await
            .unwrap();
        svc.update(
            item.id,
            UpdateTaskChanges {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let efforts = effort_store
            .list_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap();
        let eff = &efforts[0];
        let unattributed = effort_store.list_unattributed_files(&eff.id).await.unwrap();
        assert!(
            !unattributed.contains(&"mine.rs".to_string()),
            "a claimed change must not be unattributed: {unattributed:?}"
        );
    }

    #[tokio::test]
    async fn claim_open_effort_file_no_open_effort_is_noop() {
        // No open effort on the thread → the auto-claim is a no-op
        // (returns false, records nothing).
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let claimed = svc
            .claim_open_effort_file(&effort_store, &tid, "src/edited.rs", None)
            .await
            .unwrap();
        assert!(!claimed, "no open effort → no claim");
    }

    #[tokio::test]
    async fn non_in_progress_transitions_skip_effort_lifecycle() {
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "skip".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Ready → Blocked: no effort row.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Blocked),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(effort_store
            .list_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn create_assigns_increasing_sort_index() {
        let (svc, tid) = fixture().await;
        let a = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "a".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let b = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "b".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(a.sort_index, 0);
        assert_eq!(b.sort_index, 1);
    }

    #[tokio::test]
    async fn update_title_keeps_other_fields() {
        let (svc, tid) = fixture().await;
        let it = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "before".into(),
                    description: Some("desc".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let updated = svc
            .update(
                it.id,
                UpdateTaskChanges {
                    title: Some("after".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.title, "after");
        assert_eq!(updated.description, "desc");
    }

    #[tokio::test]
    async fn transition_to_done_sets_completed_at() {
        let (svc, tid) = fixture().await;
        let it = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "x".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(it.completed_at.is_none());
        let done = svc
            .update(
                it.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(done.completed_at.is_some());
        let reopened = svc
            .update(
                done.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(reopened.completed_at.is_none());
    }

    // ---- read_task_options edge cases ----

    async fn link_store_fixture() -> (TaskService, oxplow_db::SqliteTaskLinkStore, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let store = Arc::new(SqliteTaskStore::new(db.clone()));
        let link_store = oxplow_db::SqliteTaskLinkStore::new(db.clone());
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/p".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::new(1),
            stream_id: s.id,
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            acp_agent: None,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        (TaskService::new(store), link_store, t.id)
    }

    #[tokio::test]
    async fn read_work_options_empty_when_no_ready_items() {
        let (svc, links, tid) = link_store_fixture().await;
        let a = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "in flight".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        svc.update(
            a.id,
            UpdateTaskChanges {
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let result = svc.read_task_options(&tid, &links).await.unwrap();
        assert!(matches!(result, ReadWorkOptionsResult::Empty));
    }

    #[tokio::test]
    async fn read_work_options_returns_standalone_for_plain_task() {
        let (svc, links, tid) = link_store_fixture().await;
        svc.create(
            Some(tid),
            CreateTaskInput {
                title: "ready task".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let result = svc.read_task_options(&tid, &links).await.unwrap();
        match result {
            ReadWorkOptionsResult::Standalone { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].title, "ready task");
            }
            other => panic!("expected Standalone, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_work_options_returns_epic_with_ready_children() {
        let (svc, links, tid) = link_store_fixture().await;
        let epic = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "the epic".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let _child_a = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "child A".into(),
                    parent_id: Some(epic.id),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let _child_b = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "child B".into(),
                    parent_id: Some(epic.id),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let result = svc.read_task_options(&tid, &links).await.unwrap();
        match result {
            ReadWorkOptionsResult::Epic { epic: e, children } => {
                assert_eq!(e.id, epic.id);
                assert_eq!(children.len(), 2);
            }
            other => panic!("expected Epic, got {other:?}"),
        }
    }

    /// Regression: a task running on a non-primary stream must capture
    /// snapshots against THAT stream's worktree. Before the per-stream
    /// registry, the lifecycle would always hit the primary's
    /// fs-watcher, leaving the bracket diff empty for any edit landing
    /// in a worktree-stream.
    #[tokio::test]
    async fn non_primary_stream_lifecycle_captures_against_its_own_worktree() {
        // Two on-disk directories — one for the "primary" stream and a
        // separate one for the worktree stream. Both write file_snapshot
        // rows to the same DB but the rows are tagged per-stream.
        let primary_dir = tempfile::tempdir().unwrap();
        let worktree_dir = tempfile::tempdir().unwrap();
        let db = Database::in_memory();
        let stream_store = SqliteStreamStore::new(db.clone());
        let thread_store_handle = SqliteThreadStore::new(db.clone());
        let task_store = Arc::new(SqliteTaskStore::new(db.clone()));
        let effort_store = Arc::new(SqliteEffortStore::new(db.clone()));
        let snapshot_store = Arc::new(oxplow_db::SqliteSnapshotStore::new(db.clone()));
        let blobs = crate::blob_store::BlobStore::new(primary_dir.path().join(".oxplow/snapshots"));

        let primary = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: primary_dir.path().to_string_lossy().into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        let worktree = Stream {
            id: StreamId::new(2),
            kind: StreamKind::Worktree,
            title: "feature".into(),
            branch: "feature".into(),
            branch_ref: "refs/heads/feature".into(),
            branch_source: "main".into(),
            worktree_path: worktree_dir.path().to_string_lossy().into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(2),
            updated_at: Timestamp::from_unix_ms(2),
            archived_at: None,
        };
        stream_store.upsert(&primary).await.unwrap();
        stream_store.upsert(&worktree).await.unwrap();

        let snapshot_captures = crate::snapshot_capture_registry::SnapshotCaptureRegistry::new(
            crate::snapshot_capture_registry::SnapshotCaptureRegistryConfig {
                vcs: std::sync::Arc::new(crate::vcs::GitProvider),
                snapshot_store: snapshot_store.clone(),
                blobs: blobs.clone(),
                max_file_bytes: 1_000_000,
                workspace_filter: oxplow_fs_watch::WorkspaceFilter::default(),
                open_turn_probe: None,
            },
        );
        // Register both streams the same way Services::boot does, then
        // swap each entry for a settle/predrain-zero variant so tests
        // don't burn the debounce windows.
        for s in [&primary, &worktree] {
            snapshot_captures.register(s).expect("worktree dir exists");
            snapshot_captures.unregister(&s.id);
            let svc = Arc::new(
                crate::snapshot_capture::SnapshotCaptureService::new(
                    snapshot_store.clone(),
                    blobs.clone(),
                    std::path::PathBuf::from(&s.worktree_path),
                    std::sync::Arc::new(crate::vcs::GitProvider),
                    s.id,
                    1_000_000,
                    oxplow_fs_watch::WorkspaceFilter::default(),
                )
                .with_settle_duration(std::time::Duration::ZERO)
                .with_predrain_delay(std::time::Duration::ZERO),
            );
            snapshot_captures.insert_for_test(s.id, svc);
        }
        snapshot_captures.set_primary(primary.id);

        // Thread is on the WORKTREE stream — this is the case the bug
        // was about.
        let thread = Thread {
            id: ThreadId::new(3),
            stream_id: worktree.id,
            title: "t".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            acp_agent: None,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(3),
            updated_at: Timestamp::from_unix_ms(3),
            archived_at: None,
        };
        thread_store_handle.upsert(&thread).await.unwrap();

        let svc = with_lifecycle_pump(
            TaskService::new(task_store)
                .with_effort_store(effort_store.clone())
                .with_snapshot_captures(snapshot_captures.clone())
                .with_thread_store(Arc::new(SqliteThreadStore::new(db.clone()))),
            &db,
        );

        // Seed a baseline snapshot in the worktree stream so the
        // EffortStart capture has something to anchor `start_snapshot_id`
        // against (an empty dirty set returns the latest-existing id,
        // which would otherwise be NULL on a fresh DB).
        let seed = worktree_dir.path().join("seed.txt");
        std::fs::write(&seed, "baseline").unwrap();
        let worktree_svc_pre = snapshot_captures.get(&worktree.id).unwrap();
        worktree_svc_pre.mark_dirty(seed, oxplow_fs_watch::WatchEventKind::Other);
        let _ = worktree_svc_pre
            .request_snapshot(oxplow_domain::snapshot::SnapshotTrigger::Startup)
            .await
            .unwrap();

        // File the task in_progress — that opens an effort + captures
        // start_snapshot_id.
        let item = svc
            .create(
                Some(thread.id),
                CreateTaskInput {
                    title: "fix something in the worktree".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Edit a file in the WORKTREE stream's directory and mark it
        // dirty against the worktree stream's service.
        let edited = worktree_dir.path().join("changed.txt");
        std::fs::write(&edited, "hello").unwrap();
        let worktree_svc = snapshot_captures
            .get(&worktree.id)
            .expect("worktree service registered");
        worktree_svc.mark_dirty(edited.clone(), oxplow_fs_watch::WatchEventKind::Other);
        // Also mark something dirty against the primary's service.
        // This file would have been captured under the old code too —
        // we're asserting it does NOT show up in the worktree's effort.
        let primary_edit = primary_dir.path().join("other.txt");
        std::fs::write(&primary_edit, "ignored").unwrap();
        let primary_svc = snapshot_captures
            .get(&primary.id)
            .expect("primary service registered");
        primary_svc.mark_dirty(primary_edit.clone(), oxplow_fs_watch::WatchEventKind::Other);

        // Done: closes the effort and captures the end snapshot —
        // routes through the worktree stream's service because the
        // task's thread.stream_id == worktree.id.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let closed = effort_store
            .list_for_work_item(&work_item_ref(item.id))
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("one effort recorded");
        assert!(closed.ended_at.is_some());
        assert!(closed.start_snapshot_id.is_some());
        assert!(closed.end_snapshot_id.is_some());

        // Nothing claimed → the worktree edit lands in the `unclaimed`
        // half of the split; the primary-stream edit appears in neither.
        let changed =
            oxplow_db::EffortStore::list_changed_paths_for_effort(&*effort_store, &closed.id)
                .await
                .unwrap();
        assert!(
            changed.unclaimed.iter().any(|p| p == "changed.txt"),
            "worktree edit must be visible in the bracket diff; got {changed:?}",
        );
        assert!(
            !changed.claimed.iter().any(|p| p == "changed.txt"),
            "nothing was claimed, so it must not be in the claimed half; got {changed:?}",
        );
        assert!(
            !changed.unclaimed.iter().any(|p| p == "other.txt")
                && !changed.claimed.iter().any(|p| p == "other.txt"),
            "primary-stream edit must NOT bleed into the worktree's effort; got {changed:?}",
        );

        // Claiming the path moves it from `unclaimed` to `claimed`.
        let v = crate::file_ref_version::ResolvedFileVersion {
            local_snapshot_id: 0,
            closest_vcs_rev: None,
            vcs_rev_exact: false,
        };
        oxplow_db::EffortStore::record_file(
            &*effort_store,
            &closed.id,
            "changed.txt",
            oxplow_db::EffortFileChange::Updated,
            v.as_ref(),
        )
        .await
        .unwrap();
        let split =
            oxplow_db::EffortStore::list_changed_paths_for_effort(&*effort_store, &closed.id)
                .await
                .unwrap();
        assert!(
            split.claimed.iter().any(|p| p == "changed.txt"),
            "a claimed change must appear in the claimed half; got {split:?}",
        );
        assert!(
            !split.unclaimed.iter().any(|p| p == "changed.txt"),
            "a claimed change must not also be unclaimed; got {split:?}",
        );
    }

    #[tokio::test]
    async fn closing_an_effort_logs_effort_finished_once_its_bracket_is_pinned() {
        // P2.6b (tsk451): the durable `effort.finished@1` — what the effort
        // reactors consume — follows the lifecycle work, once per effort.
        let f = crate::test_fixtures::services_with_effort().await;
        f.svc
            .tasks
            .update(
                f.task,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let finished = |events: &[oxplow_domain::StoredEvent]| {
            events
                .iter()
                .filter(|e| e.envelope.event_type == "effort.finished")
                .map(|e| e.envelope.clone())
                .collect::<Vec<_>>()
        };
        let events = f.svc.event_log_store.read_after(0, 100).await.unwrap();
        let done = finished(&events);
        assert_eq!(done.len(), 1, "{done:#?}");
        assert_eq!(done[0].payload["effort"], format!("effort:{}", f.effort));
        assert_eq!(done[0].anchors.effort_id, Some(f.effort));
        let closed = events
            .iter()
            .find(|e| e.envelope.event_type == "effort.closed")
            .unwrap();
        assert_eq!(done[0].cause.as_ref(), Some(&closed.envelope.id));
        // Re-delivery (a crash before the checkpoint) doesn't log it twice.
        let consumer = crate::effort_lifecycle::EffortLifecycleConsumer::new(
            f.svc.tasks.without_event_pump(),
            (*f.svc.event_log_store).clone(),
        );
        use crate::event_pump::AsyncEventConsumer as _;
        consumer.handle(closed).await.unwrap();
        let again = f.svc.event_log_store.read_after(0, 100).await.unwrap();
        assert_eq!(finished(&again).len(), 1);
    }
}
