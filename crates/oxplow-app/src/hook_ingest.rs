//! Hook ingest pipeline.
//!
//! Receives a `HookEnvelope` from Claude Code (or a synthetic
//! oxplow-internal source like a pane interrupt), persists it, and
//! drives the agent_turn lifecycle + agent_status transitions:
//!
//! - `UserPromptSubmit`: open a new agent_turn, mark the pane Running.
//! - `Stop`: close the open agent_turn for the thread, mark the pane
//!   Idle (or AwaitingUser if a `mcp__oxplow__await_user` call fired
//!   during the turn — recorded as a sentinel in the payload).
//! - `SubagentStop`: persist the hook event only. The parent turn is
//!   still in flight when a Task-tool subagent finishes, so we MUST
//!   NOT close the open turn or flip status to Idle here — doing so
//!   makes the agent indicator render "waiting" mid-turn whenever the
//!   parent dispatches a subagent.
//! - `Interrupt`: close any open turn with a synthetic answer note,
//!   mark the pane Stopped.
//!
//! Pure orchestration: stores own persistence; this module is the
//! state machine on top.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use oxplow_domain::stores::{AgentStatusStore, AgentTurnStore, HookEventStore};
use oxplow_domain::{
    AgentStatusState, AgentTurn, AgentTurnId, DomainError, HookEvent, HookEventId, HookKind,
    StreamId, ThreadId, Timestamp,
};

use crate::events::{EventBus, OxplowEvent};

/// What the hook subprocess sends us.
///
/// The renderer / Claude Code emit JSON envelopes; the daemon receives
/// them and lands them here. `payload_json` is the verbatim envelope
/// minus the routing fields we hoist into typed columns.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct HookEnvelope {
    pub kind: HookKind,
    pub thread_id: Option<ThreadId>,
    pub stream_id: Option<StreamId>,
    pub session_id: Option<String>,
    pub payload_json: String,
    /// Optional client-supplied prompt body for UserPromptSubmit so
    /// the agent_turn row carries the visible prompt text.
    pub prompt: Option<String>,
}

#[derive(Debug, Error)]
pub enum HookIngestError {
    #[error("storage: {0}")]
    Storage(#[from] DomainError),
}

#[derive(Clone)]
pub struct HookIngestService {
    hooks: Arc<dyn HookEventStore>,
    statuses: Arc<dyn AgentStatusStore>,
    turns: Arc<dyn AgentTurnStore>,
    events: EventBus,
}

impl HookIngestService {
    pub fn new(
        hooks: Arc<dyn HookEventStore>,
        statuses: Arc<dyn AgentStatusStore>,
        turns: Arc<dyn AgentTurnStore>,
        events: EventBus,
    ) -> Self {
        Self {
            hooks,
            statuses,
            turns,
            events,
        }
    }

    /// Persist the envelope and drive the state machine. Returns the
    /// persisted hook event id so callers can correlate events to
    /// downstream reactions in tests.
    pub async fn ingest(&self, env: HookEnvelope) -> Result<HookEventId, HookIngestError> {
        let now = Timestamp::now();
        let stored = HookEvent {
            id: HookEventId::new(),
            thread_id: env.thread_id.clone(),
            stream_id: env.stream_id.clone(),
            kind: env.kind,
            session_id: env.session_id.clone(),
            payload_json: env.payload_json.clone(),
            received_at: now,
        };
        self.hooks.append(&stored).await?;
        self.events.emit(OxplowEvent::HookEventsChanged);

        // The agent_turn / agent_status branches need a thread.
        let thread = match env.thread_id.clone() {
            Some(t) => t,
            None => return Ok(stored.id),
        };

        match env.kind {
            HookKind::UserPromptSubmit => {
                // Open a new turn unless one is already open (a
                // mid-turn re-prompt from the user).
                let open = self.turns.list_open(&thread).await?;
                if open.is_empty() {
                    let turn = AgentTurn {
                        id: AgentTurnId::new(),
                        thread_id: thread.clone(),
                        task_id: None,
                        prompt: env.prompt.unwrap_or_default(),
                        answer: None,
                        session_id: env.session_id.clone(),
                        started_at: now,
                        ended_at: None,
                    };
                    self.turns.open(&turn).await?;
                }
                self.set_status(&thread, AgentStatusState::Running, None)
                    .await?;
            }
            HookKind::Stop => {
                self.close_open_turns(&thread, None).await?;
                let detail = if payload_signals_await_user(&env.payload_json) {
                    Some("await_user".to_string())
                } else {
                    None
                };
                let state = if detail.is_some() {
                    AgentStatusState::AwaitingUser
                } else {
                    AgentStatusState::Idle
                };
                self.set_status(&thread, state, detail).await?;
            }
            HookKind::SubagentStop => {
                // A Task-tool subagent finished. The parent agent is
                // still working — do NOT close the parent turn or
                // flip the status. Hook event is already persisted
                // at the top of ingest; that's all we need here.
            }
            HookKind::Interrupt => {
                self.close_open_turns(&thread, Some("interrupted".into()))
                    .await?;
                self.set_status(&thread, AgentStatusState::Stopped, Some("interrupt".into()))
                    .await?;
            }
            HookKind::AgentBoot => {
                self.set_status(&thread, AgentStatusState::Idle, Some("boot".into()))
                    .await?;
            }
            HookKind::PreToolUse | HookKind::PostToolUse => {
                // No agent_turn / agent_status table transition, but
                // these events DO change the renderer's derived
                // status (PreToolUse(Task) bumps pending_tasks etc.
                // — see agent_status_derive). Re-derive from the
                // hook event log and emit AgentStatusChanged with
                // the new state so the renderer can update without
                // a refetch round-trip.
                let recent = self
                    .hooks
                    .list_recent(Some(&thread), 200)
                    .await
                    .unwrap_or_default();
                let derived = crate::agent_status_derive::derive_thread_status(&recent);
                self.events.emit(OxplowEvent::AgentStatusChanged {
                    thread_id: thread.clone(),
                    pane_target: self.thread_pane(&thread).await,
                    state: derived,
                });
            }
        }

        Ok(stored.id)
    }

    async fn close_open_turns(
        &self,
        thread: &ThreadId,
        answer: Option<String>,
    ) -> Result<(), HookIngestError> {
        let open = self.turns.list_open(thread).await?;
        for t in open {
            self.turns.close(&t.id, answer.clone()).await?;
        }
        Ok(())
    }

    async fn set_status(
        &self,
        thread: &ThreadId,
        state: AgentStatusState,
        detail: Option<String>,
    ) -> Result<(), HookIngestError> {
        let pane_target = self.thread_pane(thread).await;
        let status = self
            .statuses
            .upsert(thread, &pane_target, state, detail)
            .await?;
        self.events.emit(OxplowEvent::AgentStatusChanged {
            thread_id: status.thread_id,
            pane_target: status.pane_target,
            state: status.state,
        });
        Ok(())
    }

    /// Resolve the pane target for the thread. Default to "working" if
    /// we can't figure it out — caller is fault-tolerant.
    async fn thread_pane(&self, _thread: &ThreadId) -> String {
        // ThreadStore lookup avoided here to keep this service free of
        // the thread store dependency. Callers who care about the
        // exact pane can subscribe to status events and query.
        "working".to_string()
    }
}

/// Heuristic: did the agent call mcp__oxplow__await_user during the
/// turn? Encoded as a sentinel in the payload so we don't have to
/// thread state through the pipeline.
fn payload_signals_await_user(payload: &str) -> bool {
    if !payload.contains("await_user") {
        return false;
    }
    // Cheap substring match — a full JSON parse on every Stop is
    // overkill since we control the sentinel writer.
    let lower = payload.to_ascii_lowercase();
    lower.contains("\"await_user\":true") || lower.contains("await_user_called")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread_runtime::ThreadRuntimeRegistry;
    use oxplow_db::{Database, SqliteAgentTurnStore, SqliteStreamStore, SqliteThreadStore};
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{Stream, StreamKind, Thread, ThreadStatus};

    async fn fixture() -> (HookIngestService, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let now = Timestamp::from_unix_ms(1);
        let s = Stream {
            id: StreamId::from("s-1"),
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
            id: ThreadId::from("b-1"),
            stream_id: s.id.clone(),
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
        let svc = HookIngestService::new(
            registry.clone(),
            registry,
            Arc::new(SqliteAgentTurnStore::new(db)),
            EventBus::new(),
        );
        (svc, t.id)
    }

    #[tokio::test]
    async fn user_prompt_opens_turn_and_marks_running() {
        let (svc, tid) = fixture().await;
        let env = HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid.clone()),
            stream_id: None,
            session_id: Some("sess".into()),
            payload_json: "{}".into(),
            prompt: Some("do the thing".into()),
        };
        svc.ingest(env).await.unwrap();
        // Spot-check via stores.
        let turns = svc.turns.list_open(&tid).await.unwrap();
        assert_eq!(turns.len(), 1);
        let status = svc.statuses.get(&tid, "working").await.unwrap().unwrap();
        assert_eq!(status.state, AgentStatusState::Running);
    }

    #[tokio::test]
    async fn stop_closes_turn_and_marks_idle() {
        let (svc, tid) = fixture().await;
        // Open a turn first.
        let prompt_env = HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid.clone()),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("do".into()),
        };
        svc.ingest(prompt_env).await.unwrap();
        let stop = HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid.clone()),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
        };
        svc.ingest(stop).await.unwrap();
        assert!(svc.turns.list_open(&tid).await.unwrap().is_empty());
        let status = svc.statuses.get(&tid, "working").await.unwrap().unwrap();
        assert_eq!(status.state, AgentStatusState::Idle);
    }

    #[tokio::test]
    async fn stop_with_await_user_signal_marks_awaiting() {
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid.clone()),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("do".into()),
        })
        .await
        .unwrap();
        svc.ingest(HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid.clone()),
            stream_id: None,
            session_id: None,
            payload_json: r#"{"await_user":true}"#.into(),
            prompt: None,
        })
        .await
        .unwrap();
        let status = svc.statuses.get(&tid, "working").await.unwrap().unwrap();
        assert_eq!(status.state, AgentStatusState::AwaitingUser);
    }

    #[tokio::test]
    async fn interrupt_closes_open_turn() {
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid.clone()),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("p".into()),
        })
        .await
        .unwrap();
        svc.ingest(HookEnvelope {
            kind: HookKind::Interrupt,
            thread_id: Some(tid.clone()),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
        })
        .await
        .unwrap();
        let status = svc.statuses.get(&tid, "working").await.unwrap().unwrap();
        assert_eq!(status.state, AgentStatusState::Stopped);
        assert!(svc.turns.list_open(&tid).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn subagent_stop_does_not_close_parent_turn_or_flip_status() {
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid.clone()),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("p".into()),
        })
        .await
        .unwrap();
        svc.ingest(HookEnvelope {
            kind: HookKind::SubagentStop,
            thread_id: Some(tid.clone()),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
        })
        .await
        .unwrap();
        // Parent turn must still be open and status still Running.
        assert_eq!(svc.turns.list_open(&tid).await.unwrap().len(), 1);
        let status = svc.statuses.get(&tid, "working").await.unwrap().unwrap();
        assert_eq!(status.state, AgentStatusState::Running);
    }

    #[test]
    fn await_user_payload_detection() {
        assert!(payload_signals_await_user(r#"{"await_user":true}"#));
        assert!(payload_signals_await_user(r#"{"x":"await_user_called"}"#));
        assert!(!payload_signals_await_user(r#"{}"#));
        assert!(!payload_signals_await_user(r#"{"await_user":false}"#));
    }
}
