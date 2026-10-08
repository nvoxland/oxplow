//! EffortService — what an effort's lifecycle does around the effort
//! store, whichever work list its work item is on: the `effort_start` /
//! `effort_end` snapshot pins and the lifecycle metrics (the
//! effort-lifecycle consumer's work), file claims onto the effort an edit
//! happened in, and the file version an effort's files are read at.
//!
//! Efforts are core's (`.context/work-tracking.md`); this service knows
//! nothing about any work list's items.

use std::path::Path;
use std::sync::Arc;

use oxplow_db::{
    Effort, EffortFileChange, EffortStore, NewFact, NewMetricCapture, SqliteEffortStore,
    SqliteFactStore, SqliteThreadStore,
};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{DomainError, EffortId, ThreadId};

use crate::events::EventBus;

/// The producer of an effort's lifecycle facts (one capture per effort).
const LIFECYCLE_PRODUCER: &str = "effort-lifecycle";
/// How many recent agent turns the steering producer scans for a closing
/// effort's window (tsk76). `list_for_thread` is newest-first, so an effort
/// with more turns than this undercounts its oldest prompts — acceptable for
/// a per-close mean; a thousand-prompt effort has bigger problems.
const STEERING_TURN_SCAN: usize = 1000;
#[derive(Clone)]
pub struct EffortService {
    effort_store: Arc<SqliteEffortStore>,
    /// Looks up an effort's thread to learn its stream, and so which
    /// worktree's snapshot capture service to drive.
    thread_store: Arc<SqliteThreadStore>,
    /// Per-stream snapshot capture registry. Optional so bare tests skip
    /// the snapshot pins.
    snapshot_captures: Option<crate::snapshot_capture_registry::SnapshotCaptureRegistry>,
    /// Durable fact layer: when set, closing an effort projects derived
    /// process metrics (`effort.cycle_time_ms`, `work_item.efforts`, …) as
    /// facts under a capture that stamps the producing `effort_id`.
    fact_store: Option<Arc<SqliteFactStore>>,
    events: Option<EventBus>,
    /// Steering-signal sources: agent turns (user prompt submissions) and
    /// comment threads, counted into `oxplow.effort_steering` at close.
    agent_turn_store: Option<Arc<oxplow_db::SqliteAgentTurnStore>>,
    comment_store: Option<Arc<oxplow_db::SqliteCommentStore>>,
    /// Run so a just-committed open/close is handled before a caller
    /// reads the effort (`settle_lifecycle`).
    event_pump: Option<Arc<crate::event_pump::EventPump>>,
}

impl EffortService {
    pub fn new(effort_store: Arc<SqliteEffortStore>, thread_store: Arc<SqliteThreadStore>) -> Self {
        Self {
            effort_store,
            thread_store,
            snapshot_captures: None,
            fact_store: None,
            events: None,
            agent_turn_store: None,
            comment_store: None,
            event_pump: None,
        }
    }

    /// Attach the event pump `settle_lifecycle` runs.
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

    /// Attach the durable fact layer + event bus: closing an effort then
    /// projects its lifecycle metrics.
    pub fn with_metrics(mut self, facts: Arc<SqliteFactStore>, events: EventBus) -> Self {
        self.fact_store = Some(facts);
        self.events = Some(events);
        self
    }

    /// Attach the per-stream registry: lifecycle snapshots route to the
    /// service of the effort's thread's stream.
    pub fn with_snapshot_captures(
        mut self,
        reg: crate::snapshot_capture_registry::SnapshotCaptureRegistry,
    ) -> Self {
        self.snapshot_captures = Some(reg);
        self
    }

    /// Wire the steering-signal sources: agent turns (user prompt
    /// submissions) + comment threads. Optional so bare tests skip the
    /// steering fact — the other lifecycle facts still project.
    pub fn with_steering_sources(
        mut self,
        turns: Arc<oxplow_db::SqliteAgentTurnStore>,
        comments: Arc<oxplow_db::SqliteCommentStore>,
    ) -> Self {
        self.agent_turn_store = Some(turns);
        self.comment_store = Some(comments);
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
        match self.thread_store.get(thread_id).await {
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

    /// Run the event pump now and wait (bounded) for the effort-lifecycle
    /// consumer to take and pin the snapshot a just-committed open/close
    /// logged, so a caller that reads the effort next (the close's
    /// file review) sees its bracket. On timeout the work finishes on a
    /// later run; nothing is lost.
    pub async fn settle_lifecycle(&self) {
        if let Some(pump) = self.event_pump.as_ref() {
            crate::effort_lifecycle::settle(pump).await;
        }
    }

    /// `effort.opened` (the effort-lifecycle consumer, P2.6.2): take the
    /// `effort_start` snapshot and pin it. Idempotent: an effort already
    /// pinned — or already closed, when delivery lagged — is left alone.
    /// A missing capture service or an empty tree pins nothing.
    pub(crate) async fn on_effort_opened(&self, effort_id: EffortId) -> Result<(), DomainError> {
        let effort_store = &self.effort_store;
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
        // An effort that starts where its predecessor on the thread closed
        // (an effort opened after a commit closed the last) begins at that
        // close's end snapshot.
        if let Some(end) = effort_store
            .end_snapshot_closed_at(effort.thread_id, effort.started_at)
            .await?
        {
            effort_store.set_start_snapshot(&effort_id, end).await?;
            return Ok(());
        }
        // One that adopted work already taken (rule 2 opens after the
        // turn's end take) begins at the stream's last snapshot from
        // before it began: a capture now would include that work.
        let store = snapshot.store();
        let before = store
            .latest_snapshot_at_or_before(*snapshot.stream_id(), effort.started_at)
            .await?;
        if let Some(prior) = before {
            if before
                != store
                    .latest_snapshot_id_for_stream(*snapshot.stream_id())
                    .await?
            {
                effort_store.set_start_snapshot(&effort_id, prior).await?;
                return Ok(());
            }
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

    /// `effort.closed` (the effort-lifecycle consumer): take and pin the
    /// `effort_end` snapshot and project the lifecycle metrics.
    /// Re-delivery re-pins nothing (the pin is checked). Returns the
    /// finished effort (the consumer then logs `effort.finished`), or
    /// `None` when there is nothing to finish.
    pub(crate) async fn on_effort_closed(
        &self,
        effort_id: EffortId,
    ) -> Result<Option<Effort>, DomainError> {
        let effort_store = &self.effort_store;
        let Some(effort) = effort_store.get_effort(&effort_id).await? else {
            return Ok(None);
        };
        if effort.ended_at.is_none() {
            return Ok(None);
        }
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
                // Keep "end_snapshot_id null ⇔ effort open": a close that
                // captured nothing falls back to the start pin.
                if let Some(id) = close_end_snapshot(captured, effort.start_snapshot_id) {
                    effort_store.set_end_snapshot(&effort_id, id).await?;
                }
            }
        }
        self.project_effort_lifecycle_metrics(&effort).await;
        effort_store.get_effort(&effort_id).await
    }

    /// Project derived process metrics into the unified substrate when an
    /// effort closes (tsk216): `effort.cycle_time_ms` (how long the effort
    /// was open) and `work_item.efforts` (efforts-so-far on its work item — the
    /// redo-rate signal). Reads `effort` as the source of truth; the
    /// table is untouched. Best-effort — a metric write error is logged and
    /// never blocks the status transition.
    async fn project_effort_lifecycle_metrics(&self, effort: &Effort) {
        let effort_store = &self.effort_store;
        let effort_id = &effort.id;
        let thread_id = &effort.thread_id;
        // Resolve the stream the thread belongs to (the hard CASCADE scope).
        let stream_val = match self.thread_store.get(thread_id).await {
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
        // Efforts so far on its work item (none while unlinked).
        let efforts_so_far = match effort.work_item.as_deref() {
            None => 0,
            Some(w) => match effort_store.list_for_work_item(w).await {
                Ok(rows) => rows.len() as i64,
                Err(e) => {
                    tracing::warn!(error = %e, "effort lifecycle metrics: list_for_item failed");
                    return;
                }
            },
        };
        // Capture branch best-effort (process fact, tied to the worktree's
        // current branch). NULL when the stream has no capture service.
        let branch = match self.service_for_thread(thread_id).await {
            Some(svc) => svc.branch().await,
            None => None,
        };

        // Write the durable facts (epic tsk12): cycle time as a fact on `oxplow.cycle_time` (subject =
        // the just-closed effort) + the efforts-so-far count on
        // `oxplow.work_item_effort` (subject = the work item, the redo-rate signal). The
        // capture stamps `effort_id` directly — this producer knows the exact
        // producing effort, so attribution is unambiguous (decision #11) — plus
        // the thread/branch spine. Best-effort.
        if let Some(facts) = self.fact_store.as_ref() {
            let dual = async {
                let mut rows = Vec::new();
                // Both measures are NON-ADDITIVE with denominator 1 (V47): the
                // cross-time collapse Σn/Σd is the MEAN across closes (average
                // cycle time / efforts per work item), never a lifetime sum.
                // Stop-collecting gate (tsk31): only emit each lifecycle fact when
                // an enabled metric consumes its measure (`effort.cycle_time_ms` /
                // `work_item.efforts`).
                if facts
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
                if let Some(work_item) = effort.work_item.as_deref() {
                    if facts
                        .measure_has_active_spec("oxplow.work_item_effort")
                        .await
                        .unwrap_or(true)
                    {
                        if let Some(measure) = facts.get_measure("oxplow.work_item_effort").await? {
                            rows.push(NewFact {
                                subject_kind: Some("work_item".into()),
                                subject_ref: Some(work_item.to_string()),
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

    /// Claim a single file an edit tool just named onto the thread's OPEN
    /// effort. Idempotent — `record_file` is `INSERT OR REPLACE` keyed on
    /// `(effort_id, path)`, and a claim replaces an observation. Returns
    /// `Ok(true)` when a claim was
    /// recorded, `Ok(false)` when no effort is open (no-op). Best-effort:
    /// the PostToolUse caller swallows errors so the hook never fails.
    pub async fn claim_open_effort_file(
        &self,
        thread: &ThreadId,
        path: &str,
        worktree_root: Option<&Path>,
    ) -> Result<bool, DomainError> {
        self.claim_effort_file(thread, None, path, worktree_root)
            .await
    }

    /// [`Self::claim_open_effort_file`] for an edit recorded with the effort
    /// it happened in (`anchored`, the event's effort anchor — P3.5): that
    /// effort takes the claim even when it has closed since (the reactor
    /// can run after the close). With no anchor, the thread's open effort
    /// takes it.
    pub async fn claim_effort_file(
        &self,
        thread: &ThreadId,
        anchored: Option<oxplow_domain::EffortId>,
        path: &str,
        worktree_root: Option<&Path>,
    ) -> Result<bool, DomainError> {
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
        // The effort the edit happened in, else the thread's open one.
        let effort_store = &self.effort_store;
        let anchored = match anchored {
            Some(id) => effort_store.get_effort(&id).await?,
            None => None,
        };
        let effort = match anchored {
            Some(e) => e,
            None => match effort_store.find_open_for_thread(thread).await? {
                Some(e) => e,
                None => return Ok(false),
            },
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
    /// An excluded path is deliberately never snapshotted: oxplow doesn't
    /// track the file, so no effort owns it.
    ///
    /// Paths pass through unfiltered when no capture service is
    /// reachable for the thread (a bare service in tests, a stream
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
        self.file_version_at(&effort.thread_id, snapshot_id).await
    }

    /// The version a file of `thread`'s stream was at in `snapshot_id`:
    /// that snapshot and its nearest VCS revision.
    pub async fn file_version_at(
        &self,
        thread: &ThreadId,
        snapshot_id: i64,
    ) -> crate::file_ref_version::ResolvedFileVersion {
        let svc = self.service_for_thread(thread).await;
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

/// Classify how a path changed during an effort by stat-ing the
/// worktree. Without a baseline snapshot we can't reliably tell
/// "created" apart from "updated" (the agent might have edited a
/// pre-existing file too), so this returns:
///
///  - `Deleted` if the file is missing on disk now
///  - `Updated` if the file is present (the dominant case)
///
/// Agents that want explicit "created" attribution should declare
/// it via the `impacts` parameter on the close (`oxplow.effort.report`). Returns
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
    use oxplow_db::{Database, SqliteStreamStore};
    use oxplow_domain::stores::StreamStore;
    use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadStatus, Timestamp};

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

    async fn fixture_with_lifecycle() -> (
        EffortService,
        ThreadId,
        Arc<SqliteEffortStore>,
        tempfile::TempDir,
        crate::snapshot_capture_registry::SnapshotCaptureRegistry,
    ) {
        let project = tempfile::tempdir().unwrap();
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
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
        // the service routes through `get(&stream_id)` either way.
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
        let svc = EffortService::new(effort_store.clone(), thread_store_for_svc)
            .with_snapshot_captures(snapshot_captures.clone())
            .with_metrics(fact_store, event_bus.clone())
            .with_steering_sources(
                Arc::new(oxplow_db::SqliteAgentTurnStore::new(db.clone())),
                Arc::new(oxplow_db::SqliteCommentStore::new(
                    db.clone(),
                    oxplow_domain::vocabulary::VocabularyHandle::core(),
                )),
            );
        let svc = with_lifecycle_pump(svc, &db);
        (svc, t.id, effort_store, project, snapshot_captures)
    }

    /// The pump with the effort-lifecycle consumer on it, as `Services`
    /// wires it: effort snapshots, reconciliation and lifecycle metrics
    /// run there (P2.6.2).
    fn with_lifecycle_pump(svc: EffortService, db: &Database) -> EffortService {
        let log = oxplow_db::SqliteEventLogStore::new(
            db.clone(),
            oxplow_domain::vocabulary::VocabularyHandle::core(),
        );
        let pump = Arc::new(crate::event_pump::EventPump::new(db.clone(), log, vec![]));
        pump.register_async(Arc::new(
            crate::effort_lifecycle::EffortLifecycleConsumer::new(
                svc.without_event_pump(),
                oxplow_db::SqliteEventLogStore::new(
                    db.clone(),
                    oxplow_domain::vocabulary::VocabularyHandle::core(),
                ),
            ),
        ));
        svc.with_event_pump(pump)
    }

    /// Open `item`'s effort on `thread`, as the effort policy would, and let
    /// the lifecycle consumer pin its start.
    async fn open_effort(
        svc: &EffortService,
        efforts: &SqliteEffortStore,
        item: &str,
        thread: ThreadId,
    ) -> oxplow_db::Effort {
        let effort = efforts.start(item, &thread, None).await.unwrap();
        svc.settle_lifecycle().await;
        effort
    }

    /// Close an effort and let the lifecycle consumer pin and project it.
    async fn close_effort(svc: &EffortService, efforts: &SqliteEffortStore, id: EffortId) {
        efforts.finish(&id, None, None).await.unwrap();
        svc.settle_lifecycle().await;
    }

    /// An open and its close both pending when the
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
        let item = "work_item:test:1".to_string();
        let opened = effort_store.start(&item.clone(), &tid, None).await.unwrap();
        effort_store.finish(&opened.id, None, None).await.unwrap();
        svc.event_pump.clone().unwrap().run_once().await.unwrap();
        let effort = &effort_store
            .list_for_work_item(&item.clone())
            .await
            .unwrap()[0];
        assert_eq!(effort.start_snapshot_id, Some(baseline));
        assert!(effort.end_snapshot_id.is_some());
    }

    /// The effort-start snapshot is the pump's, so a process that dies
    /// after the open commits — before the snapshot
    /// — has it taken by the next pump run instead of losing the pin.
    #[tokio::test]
    async fn a_committed_open_is_pinned_by_the_next_pump_run_after_a_crash() {
        let (svc, tid, effort_store, project, captures) = fixture_with_lifecycle().await;
        let item = "work_item:test:2".to_string();
        let primary = captures.primary().unwrap();
        std::fs::write(project.path().join("a.txt"), "v").unwrap();
        primary.mark_dirty(
            project.path().join("a.txt"),
            oxplow_fs_watch::WatchEventKind::Other,
        );
        // The "crashed" process: it commits the open, and no pump runs.
        let open = effort_store.start(&item.clone(), &tid, None).await.unwrap();
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
    async fn an_effort_is_pinned_at_its_open_and_its_close() {
        let (svc, tid, effort_store, _project, captures) = fixture_with_lifecycle().await;
        let item = "work_item:test:3".to_string();

        // Nothing is dirty yet, so the open pins no start snapshot: the
        // "nothing to pin" case.
        let opened = open_effort(&svc, &effort_store, &item, tid).await;
        let open = effort_store.get_effort(&opened.id).await.unwrap().unwrap();
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

        // The close pins its end snapshot.
        close_effort(&svc, &effort_store, open.id).await;
        let efforts = effort_store
            .list_for_work_item(&item.clone())
            .await
            .unwrap();
        assert_eq!(efforts.len(), 1);
        let closed = &efforts[0];
        assert!(closed.ended_at.is_some());
        assert!(closed.end_snapshot_id.is_some());
        // And nothing opened another.
        assert!(effort_store
            .find_open_for_work_item(&item.clone())
            .await
            .unwrap()
            .is_none());
    }

    /// Re-delivering a close (a crash before the
    /// checkpoint, a dead-letter retry) projects the lifecycle facts once
    /// and keeps the first end pin.
    #[tokio::test]
    async fn a_redelivered_close_records_its_lifecycle_once() {
        let (svc, tid, effort_store, project, captures) = fixture_with_lifecycle().await;
        let item = "work_item:test:4".to_string();
        let effort = open_effort(&svc, &effort_store, &item, tid).await;
        let primary = captures.primary().unwrap();
        std::fs::write(project.path().join("work.txt"), "w").unwrap();
        primary.mark_dirty(
            project.path().join("work.txt"),
            oxplow_fs_watch::WatchEventKind::Other,
        );
        close_effort(&svc, &effort_store, effort.id).await;
        let effort = effort_store
            .list_for_work_item(&item.clone())
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
        svc.on_effort_closed(effort.id).await.unwrap();
        let after = lifecycle(facts.captures_for_effort(effort.id.value()).await.unwrap());
        assert_eq!(after, 1, "no second lifecycle capture");
        let again = effort_store.get_effort(&effort.id).await.unwrap().unwrap();
        assert_eq!(again.end_snapshot_id, effort.end_snapshot_id);
    }

    #[tokio::test]
    async fn closing_an_effort_projects_lifecycle_metrics() {
        // Closing an effort projects `effort.cycle_time_ms` + `work_item.efforts`
        // into the metric substrate, reading `effort` as the source of truth.
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = "work_item:test:5".to_string();
        let effort = open_effort(&svc, &effort_store, &item, tid).await;

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

        // The close fires the projection: one `oxplow.cycle_time` fact,
        // subject = the closed effort, on a capture that stamped the
        // producing effort_id (unambiguous).
        close_effort(&svc, &effort_store, effort.id).await;
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

        // …and the efforts-so-far count on `oxplow.work_item_effort`, subject =
        // the work item (the redo-rate signal `work_item.efforts` averages).
        let effort_measure = facts
            .get_measure("oxplow.work_item_effort")
            .await
            .unwrap()
            .expect("effort measure seeded");
        let effort_facts = facts.facts_for_measure(effort_measure.id).await.unwrap();
        assert_eq!(effort_facts.len(), 1, "one effort fact per close");
        assert_eq!(effort_facts[0].value, 1.0, "first effort for the work item");
        assert_eq!(effort_facts[0].subject_kind.as_deref(), Some("work_item"));
        assert_eq!(effort_facts[0].subject_ref.as_deref(), Some(item.as_str()));
        // Ratio components: `work_item.efforts` collapses Σn/Σd across closes
        // — the mean efforts per work item, not the last-closed one's count.
        assert_eq!(effort_facts[0].numerator, Some(1.0));
        assert_eq!(effort_facts[0].denominator, Some(1.0));
    }

    #[tokio::test]
    async fn closing_an_effort_projects_its_token_spend() {
        // tsk73: at close, the effort's token spend (ALL kinds, from its
        // effort-stamped otel captures) lands as one `oxplow.effort_tokens`
        // fact — token-denominated, never dollars. `task.tokens` averages it.
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = "work_item:test:6".to_string();
        let effort = open_effort(&svc, &effort_store, &item, tid).await;

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

        close_effort(&svc, &effort_store, effort.id).await;

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
        let item = "work_item:test:7".to_string();
        let effort = open_effort(&svc, &effort_store, &item, tid).await;

        // Two prompts inside the window + one ancient one outside it. At the
        // window's (inclusive) start: one millisecond after it could land
        // past the close, when the whole test runs inside a millisecond.
        let turns = svc.agent_turn_store.as_ref().expect("turn store attached");
        let in_window = effort.started_at;
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
                    id: item.trim_start_matches("work_item:").to_string(),
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
                .transaction(move |tx| {
                    oxplow_db::comment_store::create_tx(
                        tx,
                        &oxplow_domain::refs::kind::core_kinds(),
                        &new,
                    )
                    .map(|_| ())
                })
                .await
                .unwrap();
        }

        close_effort(&svc, &effort_store, effort.id).await;

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
        let item = "work_item:test:8".to_string();
        let effort = open_effort(&svc, &effort_store, &item, tid).await;

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

        close_effort(&svc, &effort_store, effort.id).await;

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
        let item = "work_item:test:9".to_string();
        let effort = open_effort(&svc, &effort_store, &item, tid).await;
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

        close_effort(&svc, &effort_store, effort.id).await;

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

    /// The PostToolUse auto-claim gets the same treatment — writing a
    /// generated file mid-effort records nothing rather than seeding a
    /// claim the diff can never confirm.
    #[tokio::test]
    async fn claim_open_effort_file_skips_paths_that_are_never_snapshotted() {
        let (svc, tid, effort_store, _project, captures) = fixture_with_lifecycle().await;
        captures.set_workspace_filter(oxplow_fs_watch::WorkspaceFilter::with_user_entries([
            "generated",
        ]));
        let item = "work_item:test:10".to_string();
        open_effort(&svc, &effort_store, &item, tid).await;
        let claimed = svc
            .claim_open_effort_file(&tid, "apps/desktop/src/generated/bindings.ts", None)
            .await
            .unwrap();
        assert!(!claimed, "a never-snapshotted path is not claimable");
        let efforts = effort_store
            .list_for_work_item(&item.clone())
            .await
            .unwrap();
        let files = effort_store.list_files(&efforts[0].id).await.unwrap();
        assert!(files.is_empty(), "nothing should have been recorded");

        // An authored path in the same effort still claims normally.
        assert!(svc
            .claim_open_effort_file(&tid, "src/authored.rs", None)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn claim_open_effort_file_claims_on_open_effort_and_is_idempotent() {
        // Auto-claim (PostToolUse path) records a effort_file on the
        // thread's open effort, and a repeat claim of the same path is
        // idempotent (INSERT OR REPLACE → still one row).
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = "work_item:test:11".to_string();
        let open = open_effort(&svc, &effort_store, &item, tid).await;

        let claimed = svc
            .claim_open_effort_file(&tid, "src/edited.rs", None)
            .await
            .unwrap();
        assert!(claimed, "a claim should be recorded on the open effort");
        // Idempotent: claiming the same path again doesn't duplicate.
        let again = svc
            .claim_open_effort_file(&tid, "src/edited.rs", None)
            .await
            .unwrap();
        assert!(again);
        let files = effort_store.list_files(&open.id).await.unwrap();
        assert_eq!(files.len(), 1, "idempotent — one row");
        assert_eq!(files[0].path, "src/edited.rs");
    }

    #[tokio::test]
    async fn claim_open_effort_file_no_open_effort_is_noop() {
        // No open effort on the thread → the auto-claim is a no-op
        // (returns false, records nothing).
        let (svc, tid, _effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let claimed = svc
            .claim_open_effort_file(&tid, "src/edited.rs", None)
            .await
            .unwrap();
        assert!(!claimed, "no open effort → no claim");
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
            EffortService::new(
                effort_store.clone(),
                Arc::new(SqliteThreadStore::new(db.clone())),
            )
            .with_snapshot_captures(snapshot_captures.clone()),
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

        // Its effort opens, capturing start_snapshot_id.
        let item = "work_item:test:12".to_string();
        let effort = open_effort(&svc, &effort_store, &item, thread.id).await;

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

        // The close captures the end snapshot — through the worktree
        // stream's service, because the effort's thread is on it.
        close_effort(&svc, &effort_store, effort.id).await;

        let closed = effort_store
            .list_for_work_item(&item.clone())
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("one effort recorded");
        assert!(closed.ended_at.is_some());
        assert!(closed.start_snapshot_id.is_some());
        assert!(closed.end_snapshot_id.is_some());

        // The effort's bracket diff holds the worktree edit and nothing
        // from the primary stream.
        let changed: Vec<String> = snapshot_store
            .diff_snapshots(closed.start_snapshot_id, closed.end_snapshot_id.unwrap())
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.path)
            .collect();
        assert!(
            changed.iter().any(|p| p == "changed.txt"),
            "worktree edit must be visible in the bracket diff; got {changed:?}",
        );
        assert!(
            !changed.iter().any(|p| p == "other.txt"),
            "primary-stream edit must NOT bleed into the worktree's effort; got {changed:?}",
        );
    }

    #[tokio::test]
    async fn closing_an_effort_logs_effort_finished_once_its_bracket_is_pinned() {
        // The durable `effort.finished@1` — what the effort
        // reactors consume — follows the lifecycle work, once per effort.
        let f = crate::test_fixtures::services_with_effort().await;
        f.svc
            .effort_store
            .finish(&f.effort, None, None)
            .await
            .unwrap();
        f.svc.efforts.settle_lifecycle().await;
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
            f.svc.efforts.without_event_pump(),
            (*f.svc.event_log_store).clone(),
        );
        use crate::event_pump::AsyncEventConsumer as _;
        consumer.handle(closed).await.unwrap();
        let again = f.svc.event_log_store.read_after(0, 100).await.unwrap();
        assert_eq!(finished(&again).len(), 1);
    }
}
