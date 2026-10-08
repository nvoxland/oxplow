//! Agent stall watchdog.
//!
//! Claude Code emits no hook when a turn dies on an API error (socket
//! closed mid-stream, etc.) — the process drops back to its prompt and
//! the hook log just stops while the derived status still says
//! Running. Nothing event-driven can notice that, so this watchdog
//! re-derives every agent session's status against the wall clock on a
//! fixed interval and emits `AgentStatusChanged { state: Stalled }` when a
//! Running session's hook log has gone silent past the stall threshold,
//! so the renderer's dot recovers without any hook arriving.

use std::sync::Arc;
use std::time::Duration;

use oxplow_domain::stores::AgentStatusStore;
use oxplow_domain::{AgentStatusState, Timestamp};

use crate::agent_status_derive::{derive_session_status_with_activity, recent_activity};
use crate::events::{EventBus, OxplowEvent};
use crate::output_activity::OutputActivity;

/// How often the watchdog re-derives. Coarse on purpose — the stall
/// threshold is minutes, so a minute of detection latency is noise.
const CHECK_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct AgentStallWatch {
    statuses: Arc<dyn AgentStatusStore>,
    /// The threads' logged `agent.*` activity.
    log: oxplow_db::SqliteEventLogStore,
    events: EventBus,
    /// Per-session PTY liveness. Folded into the derive so a long turn
    /// still streaming output isn't misread as a stall.
    activity: OutputActivity,
}

impl AgentStallWatch {
    pub fn new(
        statuses: Arc<dyn AgentStatusStore>,
        log: oxplow_db::SqliteEventLogStore,
        activity: OutputActivity,
        events: EventBus,
    ) -> Self {
        Self {
            statuses,
            log,
            events,
            activity,
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
            if let Err(e) = self.check_session(&status, now).await {
                tracing::warn!(thread_id = %status.thread_id, error = %e, "stall watch: session check failed");
            }
        }
    }

    async fn check_session(
        &self,
        status: &oxplow_domain::AgentStatus,
        now: Timestamp,
    ) -> Result<(), oxplow_domain::DomainError> {
        let (thread_id, session) = (status.thread_id, status.agent_session_id);
        let events = recent_activity(&self.log, thread_id, session).await?;
        let last_output = session.and_then(|s| self.activity.last(&s));
        if derive_session_status_with_activity(&events, last_output, now)
            == AgentStatusState::Stalled
        {
            // Push the recovered state to the renderer — no hook will
            // ever arrive to trigger this through the normal path.
            self.events.emit(OxplowEvent::AgentStatusChanged {
                thread_id,
                agent_session_id: session,
                state: AgentStatusState::Stalled,
                detail: None,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_status_derive::AGENT_STALL_AFTER_MS;
    use oxplow_db::{Database, SqliteStreamStore, SqliteThreadStore};
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{HookKind, Stream, StreamId, StreamKind, Thread, ThreadId, ThreadStatus};

    struct Fixture {
        watch: AgentStallWatch,
        log: oxplow_db::SqliteEventLogStore,
        activity: OutputActivity,
        bus: EventBus,
        thread: ThreadId,
        /// The thread's agent session.
        session: oxplow_domain::AgentSessionId,
        db: Database,
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
            host: oxplow_domain::HostId::LOCAL,
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
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        let session = open_session(&db).await;
        let vocabulary = oxplow_domain::vocabulary::VocabularyHandle::core();
        let statuses = Arc::new(oxplow_db::SqliteAgentStatusStore::new(
            db.clone(),
            vocabulary.clone(),
        ));
        let log = oxplow_db::SqliteEventLogStore::new(db.clone(), vocabulary);
        let bus = EventBus::new();
        let activity = OutputActivity::new();
        let watch = AgentStallWatch::new(statuses, log.clone(), activity.clone(), bus.clone());
        Fixture {
            watch,
            log,
            activity,
            bus,
            thread: ThreadId::new(1),
            session,
            db,
        }
    }

    /// Open an agent session on thread 1.
    async fn open_session(db: &Database) -> oxplow_domain::AgentSessionId {
        db.transaction(|tx| {
            oxplow_db::agent_session_store::insert_tx(
                tx,
                &oxplow_domain::agent_session::NewAgentSession::terminal(
                    ThreadId::new(1),
                    "claude",
                ),
                Timestamp::now(),
            )
        })
        .await
        .unwrap()
        .id
    }

    /// Log the `agent.*` event a hook of `kind` records in the thread's
    /// session, at `ms`.
    async fn append(f: &Fixture, kind: HookKind, ms: i64, payload: &str) {
        append_in(f, f.session, kind, ms, payload).await
    }

    /// Log the `agent.*` event a hook of `kind` records in `session`, at `ms`.
    async fn append_in(
        f: &Fixture,
        session: oxplow_domain::AgentSessionId,
        kind: HookKind,
        ms: i64,
        payload: &str,
    ) {
        let tool = serde_json::from_str::<serde_json::Value>(payload)
            .ok()
            .and_then(|v| v["tool_name"].as_str().map(str::to_string))
            .unwrap_or_else(|| "Bash".into());
        let (ty, v, body) = match kind {
            HookKind::UserPromptSubmit => (
                "agent.prompt.submitted",
                1,
                serde_json::json!({"thread": "thread:thr1", "reprompt": false}),
            ),
            HookKind::PreToolUse => (
                "agent.tool.requested",
                1,
                serde_json::json!({"tool": tool, "decision": "allowed"}),
            ),
            HookKind::PostToolUse => ("agent.tool.finished", 1, serde_json::json!({"tool": tool})),
            HookKind::Stop => (
                "agent.turn.ended",
                2,
                serde_json::json!({"turn": "turn:trn1", "thread": "thread:thr1", "outcome": "completed"}),
            ),
            other => panic!("{other:?} is not logged in these tests"),
        };
        let mut env = oxplow_domain::Envelope::new(ty, v, "test", body)
            .unwrap()
            .with_anchors(oxplow_domain::Anchors {
                thread_id: Some(f.thread),
                agent_session_id: Some(session),
                ..Default::default()
            });
        env.at = Timestamp::from_unix_ms(ms);
        f.log.append(env).await.unwrap();
    }

    /// Log the session's status before any of the test's activity.
    async fn seed_status(f: &Fixture, state: AgentStatusState) {
        seed_status_in(f, f.session, state).await
    }

    async fn seed_status_in(
        f: &Fixture,
        session: oxplow_domain::AgentSessionId,
        state: AgentStatusState,
    ) {
        let body = serde_json::json!({"thread": "thread:thr1", "state": state});
        let mut env = oxplow_domain::Envelope::new("agent.status.changed", 1, "test", body)
            .unwrap()
            .with_anchors(oxplow_domain::Anchors {
                thread_id: Some(f.thread),
                agent_session_id: Some(session),
                ..Default::default()
            });
        env.at = Timestamp::from_unix_ms(0);
        f.log.append(env).await.unwrap();
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
        let mut rx = f.bus.subscribe_ui();
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

    /// Two sessions in a thread stall apart: a dead turn beside a live
    /// one is a Stalled for that session only (the thread's dot rolls the
    /// two up to stalled).
    #[tokio::test]
    async fn a_dead_session_stalls_beside_a_working_one() {
        let f = fixture().await;
        let other = open_session(&f.db).await;
        seed_status(&f, AgentStatusState::Running).await;
        seed_status_in(&f, other, AgentStatusState::Running).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        let now = 1 + AGENT_STALL_AFTER_MS + 1;
        append_in(&f, other, HookKind::UserPromptSubmit, now - 1000, "{}").await;
        let mut rx = f.bus.subscribe_ui();
        f.watch.check_once(Timestamp::from_unix_ms(now)).await;
        let stalled: Vec<_> = drain(&mut rx)
            .into_iter()
            .filter_map(|e| match e {
                OxplowEvent::AgentStatusChanged {
                    agent_session_id,
                    state: AgentStatusState::Stalled,
                    ..
                } => Some(agent_session_id),
                _ => None,
            })
            .collect();
        assert_eq!(stalled, vec![Some(f.session)]);
    }

    #[tokio::test]
    async fn running_thread_within_threshold_is_quiet() {
        let f = fixture().await;
        seed_status(&f, AgentStatusState::Running).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        let mut rx = f.bus.subscribe_ui();
        f.watch.check_once(Timestamp::from_unix_ms(1000)).await;
        assert!(drain(&mut rx).is_empty());
    }

    #[tokio::test]
    async fn long_turn_with_live_output_does_not_stall_or_alert() {
        // The hook log is frozen at turn start (well past the threshold)
        // but the agent's PTY is still streaming. Recorded output liveness
        // must keep it Running — no Stalled status push.
        let f = fixture().await;
        seed_status(&f, AgentStatusState::Running).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        let mut rx = f.bus.subscribe_ui();
        let now = Timestamp::from_unix_ms(1 + AGENT_STALL_AFTER_MS * 3);
        // Output advancing right up to `now`.
        f.activity
            .record(f.session, Timestamp::from_unix_ms(now.unix_ms() - 1000));
        f.watch.check_once(now).await;
        let evs = drain(&mut rx);
        assert!(
            !evs.iter().any(|e| matches!(
                e,
                OxplowEvent::AgentStatusChanged {
                    state: AgentStatusState::Stalled,
                    ..
                }
            )),
            "live output must keep the long turn Working, got {evs:?}"
        );
    }

    #[tokio::test]
    async fn stale_output_still_lets_a_dead_turn_stall() {
        // Guard: liveness older than the threshold must NOT mask a real
        // death — the Stalled push still fires.
        let f = fixture().await;
        seed_status(&f, AgentStatusState::Running).await;
        append(&f, HookKind::UserPromptSubmit, 1, "{}").await;
        // Output went quiet long ago, same as the hook log.
        f.activity.record(f.session, Timestamp::from_unix_ms(2));
        let mut rx = f.bus.subscribe_ui();
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
}
