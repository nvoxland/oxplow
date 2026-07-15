//! Agent stall watchdog.
//!
//! Claude Code emits no hook when a turn dies on an API error (socket
//! closed mid-stream, etc.) — the process drops back to its prompt and
//! the hook log just stops while the derived status still says
//! Running. Nothing event-driven can notice that, so this watchdog
//! re-derives every thread's status against the wall clock on a fixed
//! interval and:
//!
//! 1. Emits `AgentStatusChanged { state: Stalled }` when a Running
//!    thread's hook log has gone silent past the stall threshold, so
//!    the renderer's dot recovers without any hook arriving.
//! 2. Emits `AgentStallAlert` — once per stall episode — when a thread
//!    holds in_progress tasks but its agent has not been running for
//!    longer than the alert threshold. This is the user-visible nudge
//!    for "the queue silently stalled".
//!
//! The alert re-arms when the agent runs again or the in_progress
//! bucket empties, so a thread that stalls, recovers, and stalls again
//! alerts each time without spamming every tick in between.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use oxplow_domain::stores::{AgentStatusStore, HookEventStore, TaskStore};
use oxplow_domain::{AgentStatusState, TaskStatus, ThreadId, Timestamp};

use crate::agent_status_derive::{derive_thread_status_with_activity, AGENT_STALL_AFTER_MS};
use crate::events::{EventBus, OxplowEvent};
use crate::output_activity::OutputActivity;

/// How often the watchdog re-derives. Coarse on purpose — the stall
/// threshold is minutes, so a minute of detection latency is noise.
const CHECK_INTERVAL: Duration = Duration::from_secs(60);

/// How long in_progress work may sit on a non-running agent before the
/// user-visible alert fires. Same scale as the stall threshold — the
/// two cases (died mid-turn vs. stopped cleanly and never resumed)
/// should nudge with the same urgency.
pub const AGENT_STALL_ALERT_AFTER_MS: i64 = AGENT_STALL_AFTER_MS;

#[derive(Clone)]
pub struct AgentStallWatch {
    statuses: Arc<dyn AgentStatusStore>,
    hooks: Arc<dyn HookEventStore>,
    tasks: Arc<dyn TaskStore>,
    events: EventBus,
    /// Per-thread PTY liveness. Folded into the derive so a long turn
    /// still streaming output isn't misread as a stall (tsk141).
    activity: OutputActivity,
    /// Threads already alerted this stall episode.
    alerted: Arc<Mutex<HashSet<ThreadId>>>,
}

impl AgentStallWatch {
    pub fn new(
        statuses: Arc<dyn AgentStatusStore>,
        hooks: Arc<dyn HookEventStore>,
        tasks: Arc<dyn TaskStore>,
        activity: OutputActivity,
        events: EventBus,
    ) -> Self {
        Self {
            statuses,
            hooks,
            tasks,
            events,
            activity,
            alerted: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Spawn the periodic loop. Detached — lives for the process.
    pub fn spawn(self) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(CHECK_INTERVAL).await;
                self.check_once(Timestamp::now()).await;
            }
        });
    }

    /// One watchdog pass at `now`. Public so tests drive ticks
    /// directly instead of sleeping.
    pub async fn check_once(&self, now: Timestamp) {
        let statuses = match self.statuses.list_all().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "stall watch: list_all failed");
                return;
            }
        };
        for status in statuses {
            if let Err(e) = self.check_thread(&status, now).await {
                tracing::warn!(thread_id = %status.thread_id, error = %e, "stall watch: thread check failed");
            }
        }
    }

    async fn check_thread(
        &self,
        status: &oxplow_domain::AgentStatus,
        now: Timestamp,
    ) -> Result<(), oxplow_domain::DomainError> {
        let thread_id = status.thread_id;
        let events = self.hooks.list_recent(Some(&thread_id), 200).await?;
        let last_output = self.activity.last(&thread_id);
        let derived = derive_thread_status_with_activity(&events, last_output, now);

        if derived == AgentStatusState::Stalled {
            // Push the recovered state to the renderer — no hook will
            // ever arrive to trigger this through the normal path.
            self.events.emit(OxplowEvent::AgentStatusChanged {
                thread_id,
                pane_target: status.pane_target.clone(),
                state: derived,
                detail: None,
            });
        }

        // Running (actively working) or AwaitingUser (legitimately
        // parked on the user — AskUserQuestion / plan approval) are
        // both non-stall states: clear any latch and stay quiet. Only
        // Stalled (dead) and Idle (clean Stop, never resumed) can strand
        // in_progress work.
        if matches!(
            derived,
            AgentStatusState::Running | AgentStatusState::AwaitingUser
        ) {
            self.alerted.lock().await.remove(&thread_id);
            return Ok(());
        }

        let in_progress = self
            .tasks
            .list_by_status_for_thread(&thread_id, TaskStatus::InProgress)
            .await?;
        if in_progress.is_empty() {
            self.alerted.lock().await.remove(&thread_id);
            return Ok(());
        }

        // Waiting since the last sign of life: newest hook event, or
        // the status row's own update time if the log is empty.
        let waiting_since = events
            .iter()
            .map(|e| e.received_at)
            .max()
            .unwrap_or(status.updated_at);
        let waiting_ms = now.unix_ms() - waiting_since.unix_ms();

        // A Stalled derivation has already cleared its silence threshold
        // (short for a genuine death, long for an open tool — see
        // `derive_thread_status`), so the stranded-work alert fires
        // immediately, surfacing uncommitted work promptly (tsk130). An
        // Idle thread instead waits the full alert window — it stopped
        // cleanly and may just be paused between user prompts.
        let ready_to_alert =
            derived == AgentStatusState::Stalled || waiting_ms > AGENT_STALL_ALERT_AFTER_MS;
        if !ready_to_alert {
            return Ok(());
        }

        if self.alerted.lock().await.insert(thread_id) {
            self.events.emit(OxplowEvent::AgentStallAlert {
                thread_id,
                in_progress_count: in_progress.len() as u32,
                waiting_ms,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_status_derive::AGENT_DEAD_AFTER_MS;
    use crate::thread_runtime::ThreadRuntimeRegistry;
    use oxplow_db::{Database, SqliteStreamStore, SqliteTaskStore, SqliteThreadStore};
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{
        HookEvent, HookEventId, HookKind, Stream, StreamId, StreamKind, Task, TaskActorKind,
        TaskId, TaskPriority, Thread, ThreadStatus,
    };

    struct Fixture {
        watch: AgentStallWatch,
        registry: Arc<ThreadRuntimeRegistry>,
        tasks: Arc<SqliteTaskStore>,
        activity: OutputActivity,
        bus: EventBus,
        thread: ThreadId,
    }

    async fn fixture() -> Fixture {
        let db = Database::in_memory();
        let now = Timestamp::from_unix_ms(1);
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
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
            created_at: now,
            updated_at: now,
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
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        let registry = Arc::new(ThreadRuntimeRegistry::with_default_capacity());
        let tasks = Arc::new(SqliteTaskStore::new(db));
        let bus = EventBus::new();
        let activity = OutputActivity::new();
        let watch = AgentStallWatch::new(
            registry.clone(),
            registry.clone(),
            tasks.clone(),
            activity.clone(),
            bus.clone(),
        );
        Fixture {
            watch,
            registry,
            tasks,
            activity,
            bus,
            thread: ThreadId::new(1),
        }
    }

    async fn append(f: &Fixture, kind: HookKind, ms: i64, payload: &str) {
        let ev = HookEvent {
            id: HookEventId::new(ms),
            thread_id: Some(f.thread),
            stream_id: None,
            kind,
            session_id: None,
            payload_json: payload.to_string(),
            received_at: Timestamp::from_unix_ms(ms),
        };
        let hooks: Arc<dyn HookEventStore> = f.registry.clone();
        hooks.append(&ev).await.unwrap();
    }

    async fn seed_status(f: &Fixture, state: AgentStatusState) {
        let statuses: Arc<dyn AgentStatusStore> = f.registry.clone();
        statuses
            .upsert(&f.thread, "working", state, None)
            .await
            .unwrap();
    }

    async fn seed_in_progress_task(f: &Fixture) {
        let now = Timestamp::from_unix_ms(1);
        let t = Task {
            id: TaskId::new(0),
            thread_id: Some(f.thread),
            parent_id: None,
            title: "stalled work".into(),
            description: String::new(),
            status: TaskStatus::InProgress,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: None,
        };
        f.tasks.insert(&t).await.unwrap();
    }

    fn drain(rx: &mut tokio::sync::broadcast::Receiver<OxplowEvent>) -> Vec<OxplowEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            out.push(ev);
        }
        out
    }

    #[tokio::test]
    async fn stalled_thread_emits_status_changed() {
        let f = fixture().await;
        seed_status(&f, AgentStatusState::Running).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        let mut rx = f.bus.subscribe();
        f.watch
            .check_once(Timestamp::from_unix_ms(1 + AGENT_STALL_AFTER_MS + 1))
            .await;
        let evs = drain(&mut rx);
        assert!(
            evs.iter().any(|e| matches!(
                e,
                OxplowEvent::AgentStatusChanged {
                    state: AgentStatusState::Stalled,
                    ..
                }
            )),
            "expected Stalled AgentStatusChanged, got {evs:?}"
        );
    }

    #[tokio::test]
    async fn running_thread_within_threshold_is_quiet() {
        let f = fixture().await;
        seed_status(&f, AgentStatusState::Running).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        seed_in_progress_task(&f).await;
        let mut rx = f.bus.subscribe();
        f.watch.check_once(Timestamp::from_unix_ms(1000)).await;
        assert!(drain(&mut rx).is_empty());
    }

    #[tokio::test]
    async fn stalled_with_in_progress_tasks_alerts_once() {
        let f = fixture().await;
        seed_status(&f, AgentStatusState::Running).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        seed_in_progress_task(&f).await;
        let mut rx = f.bus.subscribe();
        let late = Timestamp::from_unix_ms(1 + AGENT_STALL_ALERT_AFTER_MS + 1);
        f.watch.check_once(late).await;
        f.watch.check_once(late).await;
        let alerts: Vec<_> = drain(&mut rx)
            .into_iter()
            .filter(|e| matches!(e, OxplowEvent::AgentStallAlert { .. }))
            .collect();
        assert_eq!(alerts.len(), 1, "alert must fire exactly once per episode");
        match &alerts[0] {
            OxplowEvent::AgentStallAlert {
                thread_id,
                in_progress_count,
                waiting_ms,
            } => {
                assert_eq!(*thread_id, f.thread);
                assert_eq!(*in_progress_count, 1);
                assert!(*waiting_ms > AGENT_STALL_ALERT_AFTER_MS);
            }
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn idle_agent_with_in_progress_tasks_alerts() {
        // The clean-Stop variant: agent finished its turn (Idle) but
        // left in_progress work behind and never resumed.
        let f = fixture().await;
        seed_status(&f, AgentStatusState::Idle).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        append(&f, HookKind::Stop, 2, "{}").await;
        seed_in_progress_task(&f).await;
        let mut rx = f.bus.subscribe();
        f.watch
            .check_once(Timestamp::from_unix_ms(2 + AGENT_STALL_ALERT_AFTER_MS + 1))
            .await;
        let evs = drain(&mut rx);
        assert!(
            evs.iter()
                .any(|e| matches!(e, OxplowEvent::AgentStallAlert { .. })),
            "expected alert for idle agent with in_progress work, got {evs:?}"
        );
    }

    #[tokio::test]
    async fn no_in_progress_tasks_no_alert() {
        let f = fixture().await;
        seed_status(&f, AgentStatusState::Running).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        let mut rx = f.bus.subscribe();
        f.watch
            .check_once(Timestamp::from_unix_ms(1 + AGENT_STALL_ALERT_AFTER_MS + 1))
            .await;
        let evs = drain(&mut rx);
        assert!(
            !evs.iter()
                .any(|e| matches!(e, OxplowEvent::AgentStallAlert { .. })),
            "no alert without in_progress work, got {evs:?}"
        );
    }

    #[tokio::test]
    async fn awaiting_user_with_in_progress_does_not_alert() {
        // tsk130/tsk128: an agent parked on AskUserQuestion derives
        // AwaitingUser. Waiting on the user is legitimate — even with
        // in_progress work and a long wait, it must NOT raise a stall
        // alert (that would be the very false alarm tsk128 was about).
        let f = fixture().await;
        seed_status(&f, AgentStatusState::Running).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        append(
            &f,
            HookKind::PreToolUse,
            2,
            r#"{"tool_name":"AskUserQuestion"}"#,
        )
        .await;
        seed_in_progress_task(&f).await;
        let mut rx = f.bus.subscribe();
        f.watch
            .check_once(Timestamp::from_unix_ms(2 + AGENT_STALL_ALERT_AFTER_MS * 10))
            .await;
        let evs = drain(&mut rx);
        assert!(
            !evs.iter()
                .any(|e| matches!(e, OxplowEvent::AgentStallAlert { .. })),
            "waiting-on-user must not alert, got {evs:?}"
        );
    }

    #[tokio::test]
    async fn dead_thread_with_in_progress_alerts_at_short_threshold() {
        // tsk130: a genuinely-dead turn (no open tool call) is detected
        // by the derivation at the short threshold. The stranded-work
        // alert must fire then — not wait the full 15-min alert window —
        // so uncommitted in_progress work surfaces promptly.
        let f = fixture().await;
        seed_status(&f, AgentStatusState::Running).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        seed_in_progress_task(&f).await;
        let mut rx = f.bus.subscribe();
        // Past the short death threshold but well under the long alert
        // window — the old code stayed quiet here.
        let dead_at = Timestamp::from_unix_ms(1 + AGENT_DEAD_AFTER_MS + 1);
        assert!(dead_at.unix_ms() < 1 + AGENT_STALL_ALERT_AFTER_MS);
        f.watch.check_once(dead_at).await;
        let evs = drain(&mut rx);
        assert!(
            evs.iter()
                .any(|e| matches!(e, OxplowEvent::AgentStallAlert { .. })),
            "dead thread must alert promptly, got {evs:?}"
        );
    }

    #[tokio::test]
    async fn long_turn_with_live_output_does_not_stall_or_alert() {
        // tsk141: the hook log is frozen at turn start (well past the
        // threshold) but the agent's PTY is still streaming. Recorded
        // output liveness must keep it Running — no Stalled status push,
        // no stranded-work alert — even with in_progress work.
        let f = fixture().await;
        seed_status(&f, AgentStatusState::Running).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        seed_in_progress_task(&f).await;
        let mut rx = f.bus.subscribe();
        let now = Timestamp::from_unix_ms(1 + AGENT_STALL_ALERT_AFTER_MS + 1);
        // Output advancing right up to `now`.
        f.activity
            .record(f.thread, Timestamp::from_unix_ms(now.unix_ms() - 1000));
        f.watch.check_once(now).await;
        let evs = drain(&mut rx);
        assert!(
            !evs.iter().any(|e| matches!(
                e,
                OxplowEvent::AgentStatusChanged {
                    state: AgentStatusState::Stalled,
                    ..
                } | OxplowEvent::AgentStallAlert { .. }
            )),
            "live output must keep the long turn Working, got {evs:?}"
        );
    }

    #[tokio::test]
    async fn stale_output_still_lets_a_dead_turn_stall() {
        // Guard: liveness older than the threshold must NOT mask a real
        // death — the Stalled push and alert still fire (tsk130 intact).
        let f = fixture().await;
        seed_status(&f, AgentStatusState::Running).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        seed_in_progress_task(&f).await;
        // Output went quiet long ago, same as the hook log.
        f.activity.record(f.thread, Timestamp::from_unix_ms(2));
        let mut rx = f.bus.subscribe();
        f.watch
            .check_once(Timestamp::from_unix_ms(2 + AGENT_STALL_AFTER_MS + 1))
            .await;
        let evs = drain(&mut rx);
        assert!(
            evs.iter().any(|e| matches!(
                e,
                OxplowEvent::AgentStatusChanged {
                    state: AgentStatusState::Stalled,
                    ..
                }
            )),
            "stale output must not revive a dead turn, got {evs:?}"
        );
    }

    #[tokio::test]
    async fn alert_rearms_after_agent_runs_again() {
        let f = fixture().await;
        seed_status(&f, AgentStatusState::Running).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        seed_in_progress_task(&f).await;
        let mut rx = f.bus.subscribe();
        let late = Timestamp::from_unix_ms(1 + AGENT_STALL_ALERT_AFTER_MS + 1);
        f.watch.check_once(late).await;

        // Agent comes back: a fresh prompt lands, status derives
        // Running again — the alert latch must clear.
        let resume_ms = late.unix_ms() + 1000;
        append(&f, HookKind::UserPromptSubmit, resume_ms, "{}").await;
        f.watch
            .check_once(Timestamp::from_unix_ms(resume_ms + 1000))
            .await;

        // …and a second stall episode alerts again.
        let second_stall = Timestamp::from_unix_ms(resume_ms + AGENT_STALL_ALERT_AFTER_MS + 1);
        f.watch.check_once(second_stall).await;
        let alerts = drain(&mut rx)
            .into_iter()
            .filter(|e| matches!(e, OxplowEvent::AgentStallAlert { .. }))
            .count();
        assert_eq!(alerts, 2, "each stall episode alerts once");
    }
}
