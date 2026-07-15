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

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

/// Process-local counter for the in-memory hook-event log (the
/// `hook_event` table was dropped in V2 — these rows live only in the
/// `ThreadRuntimeRegistry` ring buffer, so there's no rowid to allocate).
static NEXT_HOOK_EVENT_ID: AtomicI64 = AtomicI64::new(1);

use oxplow_domain::stores::{AgentStatusStore, AgentTurnStore, HookEventStore};
use oxplow_domain::{
    AgentStatus, AgentStatusState, AgentTurn, AgentTurnId, DomainError, HookEvent, HookEventId,
    HookKind, StreamId, ThreadId, Timestamp,
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
            id: HookEventId::new(NEXT_HOOK_EVENT_ID.fetch_add(1, Ordering::Relaxed)),
            thread_id: env.thread_id,
            stream_id: env.stream_id,
            kind: env.kind,
            session_id: env.session_id.clone(),
            payload_json: env.payload_json.clone(),
            received_at: now,
        };
        self.hooks.append(&stored).await?;
        self.events.emit(OxplowEvent::HookEventsChanged);

        // The agent_turn / agent_status branches need a thread.
        let thread = match env.thread_id {
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
                        id: AgentTurnId::placeholder(),
                        thread_id: thread,
                        task_id: None,
                        prompt: env.prompt.unwrap_or_default(),
                        answer: None,
                        session_id: env.session_id.clone(),
                        started_at: now,
                        ended_at: None,
                    };
                    self.turns.open(&turn).await?;
                    self.events
                        .emit(OxplowEvent::AgentTurnsChanged { thread_id: thread });
                }
                self.set_status(&thread, AgentStatusState::Running, None)
                    .await?;
            }
            HookKind::Stop => {
                self.close_open_turns(&thread, None).await?;
                // Did the agent park on the user this turn? Two signals:
                //  - a sentinel in THIS Stop payload (kept for the
                //    synthetic-event path and tests), or
                //  - an AwaitingUser status the `await_user` MCP tool
                //    already set earlier in the turn. The real Claude
                //    Stop payload carries no sentinel, so without the
                //    second check it would clobber the MCP-set
                //    AwaitingUser (and its question) back to Idle within
                //    the same turn — the rail "awaiting you" dot would
                //    never persist. A fresh UserPromptSubmit clears
                //    AwaitingUser first, so a stale flag from a prior
                //    turn can't leak in here.
                let current = self.current_status(&thread).await;
                let currently_awaiting = current
                    .as_ref()
                    .is_some_and(|s| s.state == AgentStatusState::AwaitingUser);
                let (state, detail) =
                    if payload_signals_await_user(&env.payload_json) || currently_awaiting {
                        // Prefer a question carried on this payload; else
                        // keep whatever the MCP tool stored as detail (the
                        // question text).
                        let question = await_user_question(&env.payload_json)
                            .or_else(|| current.and_then(|s| s.detail));
                        (AgentStatusState::AwaitingUser, question)
                    } else {
                        (AgentStatusState::Idle, None)
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
                let derived =
                    crate::agent_status_derive::derive_thread_status(&recent, Timestamp::now());
                self.events.emit(OxplowEvent::AgentStatusChanged {
                    thread_id: thread,
                    pane_target: self.thread_pane(&thread).await,
                    state: derived,
                    detail: None,
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
        let closed_any = !open.is_empty();
        for t in open {
            self.turns.close(&t.id, answer.clone()).await?;
        }
        if closed_any {
            self.events
                .emit(OxplowEvent::AgentTurnsChanged { thread_id: *thread });
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
            detail: status.detail,
        });
        Ok(())
    }

    /// Read the current agent status for a thread's working pane, if any.
    /// Used by the Stop path to avoid clobbering an in-turn AwaitingUser.
    async fn current_status(&self, thread: &ThreadId) -> Option<AgentStatus> {
        let pane = self.thread_pane(thread).await;
        self.statuses.get(thread, &pane).await.ok().flatten()
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

/// Extract the question text from an await_user sentinel payload. Returns
/// None when the payload isn't an await_user signal or carries no
/// (non-empty) `question` field — callers then fall back to any question
/// already stored on `agent_status.detail`.
fn await_user_question(payload: &str) -> Option<String> {
    if !payload_signals_await_user(payload) {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    let q = v.get("question")?.as_str()?.trim();
    (!q.is_empty()).then(|| q.to_string())
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
        let svc = HookIngestService::new(
            registry.clone(),
            registry,
            Arc::new(SqliteAgentTurnStore::new(db)),
            EventBus::new(),
        );
        (svc, t.id)
    }

    #[tokio::test]
    async fn turn_open_and_close_emit_agent_turns_changed() {
        // The Work panel renders open turns as live rows; it needs an
        // event on every open/close to refetch without polling.
        let (svc, tid) = fixture().await;
        let mut rx = svc.events.subscribe();
        let drain_turns = |rx: &mut tokio::sync::broadcast::Receiver<OxplowEvent>| {
            let mut n = 0;
            while let Ok(ev) = rx.try_recv() {
                if matches!(ev, OxplowEvent::AgentTurnsChanged { .. }) {
                    n += 1;
                }
            }
            n
        };
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("p".into()),
        })
        .await
        .unwrap();
        assert_eq!(drain_turns(&mut rx), 1, "open must emit AgentTurnsChanged");
        svc.ingest(HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
        })
        .await
        .unwrap();
        assert_eq!(drain_turns(&mut rx), 1, "close must emit AgentTurnsChanged");
        // A Stop with nothing open closes nothing — no event.
        svc.ingest(HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
        })
        .await
        .unwrap();
        assert_eq!(drain_turns(&mut rx), 0, "no-op close must stay quiet");
    }

    #[tokio::test]
    async fn user_prompt_opens_turn_and_marks_running() {
        let (svc, tid) = fixture().await;
        let env = HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid),
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
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("do".into()),
        };
        svc.ingest(prompt_env).await.unwrap();
        let stop = HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
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
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("do".into()),
        })
        .await
        .unwrap();
        svc.ingest(HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
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
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("p".into()),
        })
        .await
        .unwrap();
        svc.ingest(HookEnvelope {
            kind: HookKind::Interrupt,
            thread_id: Some(tid),
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
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("p".into()),
        })
        .await
        .unwrap();
        svc.ingest(HookEnvelope {
            kind: HookKind::SubagentStop,
            thread_id: Some(tid),
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

    #[tokio::test]
    async fn stop_without_open_turn_still_marks_idle() {
        // Out-of-order: a Stop arriving with no open turn (e.g. after
        // a daemon restart dropped the in-memory turn, or a duplicate
        // Stop) must not error — it just lands the status transition.
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
        })
        .await
        .unwrap();
        assert!(svc.turns.list_open(&tid).await.unwrap().is_empty());
        let status = svc.statuses.get(&tid, "working").await.unwrap().unwrap();
        assert_eq!(status.state, AgentStatusState::Idle);
    }

    #[tokio::test]
    async fn envelope_without_thread_id_persists_event_only() {
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: None,
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("orphan".into()),
        })
        .await
        .unwrap();
        // No turn opened, no status row created — the thread-scoped
        // state machine never ran.
        assert!(svc.turns.list_open(&tid).await.unwrap().is_empty());
        assert!(svc.statuses.get(&tid, "working").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reprompt_while_turn_open_does_not_open_second_turn() {
        let (svc, tid) = fixture().await;
        for prompt in ["first", "mid-turn re-prompt"] {
            svc.ingest(HookEnvelope {
                kind: HookKind::UserPromptSubmit,
                thread_id: Some(tid),
                stream_id: None,
                session_id: None,
                payload_json: "{}".into(),
                prompt: Some(prompt.into()),
            })
            .await
            .unwrap();
        }
        let open = svc.turns.list_open(&tid).await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].prompt, "first");
    }

    #[tokio::test]
    async fn agent_boot_marks_idle_without_touching_turns() {
        let (svc, tid) = fixture().await;
        // Open a turn, then boot — the status flips but the turn
        // survives (a pane restart mid-turn shouldn't lose the turn).
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("p".into()),
        })
        .await
        .unwrap();
        svc.ingest(HookEnvelope {
            kind: HookKind::AgentBoot,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
        })
        .await
        .unwrap();
        assert_eq!(svc.turns.list_open(&tid).await.unwrap().len(), 1);
        let status = svc.statuses.get(&tid, "working").await.unwrap().unwrap();
        assert_eq!(status.state, AgentStatusState::Idle);
        assert_eq!(status.detail.as_deref(), Some("boot"));
    }

    #[tokio::test]
    async fn real_stop_preserves_awaiting_user_set_by_mcp() {
        // The `await_user` MCP tool flips agent_status to AwaitingUser
        // (question as detail) mid-turn. The real Claude Stop that
        // follows carries no await_user sentinel — it must NOT clobber
        // that state back to Idle, or the rail "awaiting you" dot would
        // vanish the instant the turn ends.
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("do".into()),
        })
        .await
        .unwrap();
        // Simulate the MCP await_user call landing on the shared store.
        svc.statuses
            .upsert(
                &tid,
                "working",
                AgentStatusState::AwaitingUser,
                Some("Ship A or B?".into()),
            )
            .await
            .unwrap();
        svc.ingest(HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
        })
        .await
        .unwrap();
        let status = svc.statuses.get(&tid, "working").await.unwrap().unwrap();
        assert_eq!(status.state, AgentStatusState::AwaitingUser);
        assert_eq!(status.detail.as_deref(), Some("Ship A or B?"));
    }

    #[tokio::test]
    async fn stop_await_user_signal_carries_question() {
        // A Stop payload carrying the sentinel + a question lands the
        // question on detail so the renderer can show it in the tooltip.
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: r#"{"await_user":true,"question":"Pick A or B"}"#.into(),
            prompt: None,
        })
        .await
        .unwrap();
        let status = svc.statuses.get(&tid, "working").await.unwrap().unwrap();
        assert_eq!(status.state, AgentStatusState::AwaitingUser);
        assert_eq!(status.detail.as_deref(), Some("Pick A or B"));
    }

    #[test]
    fn await_user_payload_detection() {
        assert!(payload_signals_await_user(r#"{"await_user":true}"#));
        assert!(payload_signals_await_user(r#"{"x":"await_user_called"}"#));
        assert!(!payload_signals_await_user(r#"{}"#));
        assert!(!payload_signals_await_user(r#"{"await_user":false}"#));
    }

    #[test]
    fn await_user_question_extraction() {
        assert_eq!(
            await_user_question(r#"{"await_user":true,"question":"Pick A or B"}"#).as_deref(),
            Some("Pick A or B")
        );
        // Sentinel present but no question → None (caller falls back to
        // whatever detail the MCP tool already stored).
        assert_eq!(await_user_question(r#"{"await_user":true}"#), None);
        // Blank question → None.
        assert_eq!(
            await_user_question(r#"{"await_user":true,"question":"  "}"#),
            None
        );
        // Not an await_user payload → None.
        assert_eq!(await_user_question(r#"{"question":"x"}"#), None);
    }
}
