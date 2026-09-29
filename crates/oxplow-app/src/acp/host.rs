//! What an ACP session asks of the rest of oxplow: the policy, and the
//! same recording a terminal agent's hooks get. [`AcpHost`] is the seam
//! (tests use a recording double); [`ServicesAcpHost`] is the real one,
//! routing through `AgentPolicy`, `AgentActivity` and hook ingest so an
//! ACP turn lands in the same tables as a hooked one.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use oxplow_domain::{AgentStatusState, HookKind, StreamId, ThreadId};
use oxplow_runtime::policy::PolicyDecision;
use tracing::warn;

use super::mapping::AcpIntent;
use super::wire::TurnTokens;
use crate::agent_activity::CanonicalToolEvent;
use crate::hook_ingest::HookEnvelope;
use crate::Services;

#[async_trait]
pub trait AcpHost: Send + Sync + 'static {
    /// May the thread run this tool call? `payload` is the canonical
    /// event, logged as the call's PreToolUse.
    async fn check_tool(
        &self,
        thread: &ThreadId,
        session_id: &str,
        intent: &AcpIntent,
        payload: &serde_json::Value,
    ) -> PolicyDecision;
    /// A session is up (new or loaded): remember it for the next resume.
    async fn session_started(&self, thread: &ThreadId, session_id: &str);
    /// Context to attach to the person's next prompt.
    async fn prompt_context(&self, thread: &ThreadId, session_id: &str) -> Option<String>;
    async fn turn_started(&self, thread: &ThreadId, session_id: &str, prompt: &str);
    /// A tool call finished. Returns a nudge to attach to the next prompt.
    async fn tool_finished(
        &self,
        thread: &ThreadId,
        session_id: &str,
        event: &CanonicalToolEvent,
    ) -> Option<String>;
    /// The turn ended: record it and return the Stop directive, if any.
    /// The directive is shown to the person; it is never sent.
    async fn turn_ended(
        &self,
        thread: &ThreadId,
        session_id: &str,
        prompt: &str,
        tokens: Option<&TurnTokens>,
    ) -> Option<String>;
    /// A permission card is (or no longer is) waiting on the person.
    async fn awaiting_user(&self, thread: &ThreadId, question: Option<String>);
    /// The agent went away mid-session.
    async fn interrupted(&self, thread: &ThreadId);
    /// The agent did something (stall-watch liveness).
    fn activity(&self, thread: &ThreadId);
}

/// The real host. Holds `Services` weakly: sessions live inside
/// `Services.acp`, so a strong reference would be a cycle.
pub struct ServicesAcpHost {
    svc: Weak<Services>,
    stream_id: Option<StreamId>,
    /// The thread status a permission card interrupted, restored when the
    /// cards are answered (so an `await_user` question survives one).
    before_card: parking_lot::Mutex<Option<(AgentStatusState, Option<String>)>>,
}

impl ServicesAcpHost {
    pub fn new(svc: &Arc<Services>, stream_id: Option<StreamId>) -> Self {
        Self {
            svc: Arc::downgrade(svc),
            stream_id,
            before_card: parking_lot::Mutex::new(None),
        }
    }

    fn envelope(
        &self,
        kind: HookKind,
        thread: &ThreadId,
        session_id: &str,
        payload: serde_json::Value,
        prompt: Option<String>,
    ) -> HookEnvelope {
        HookEnvelope {
            kind,
            thread_id: Some(*thread),
            stream_id: self.stream_id,
            session_id: Some(session_id.to_string()),
            payload_json: payload.to_string(),
            prompt,
            decision: None,
        }
    }

    async fn ingest(&self, svc: &Services, env: HookEnvelope) {
        if let Err(err) = svc.hook_ingest.ingest(env).await {
            warn!(?err, "acp: hook ingest failed");
        }
    }
}

#[async_trait]
impl AcpHost for ServicesAcpHost {
    async fn check_tool(
        &self,
        thread: &ThreadId,
        session_id: &str,
        intent: &AcpIntent,
        payload: &serde_json::Value,
    ) -> PolicyDecision {
        let Some(svc) = self.svc.upgrade() else {
            return PolicyDecision::Allow;
        };
        let decision = svc
            .agent_policy
            .check_tool(&svc, thread, &intent.as_intent())
            .await;
        let mut env = self.envelope(
            HookKind::PreToolUse,
            thread,
            session_id,
            payload.clone(),
            None,
        );
        env.decision = Some(crate::hook_ingest::ToolDecision {
            allowed: matches!(decision, PolicyDecision::Allow),
            reason: match &decision {
                PolicyDecision::Deny { reason, .. } => Some(reason.clone()),
                PolicyDecision::Allow => None,
            },
        });
        self.ingest(&svc, env).await;
        decision
    }

    async fn session_started(&self, thread: &ThreadId, session_id: &str) {
        let Some(svc) = self.svc.upgrade() else {
            return;
        };
        let env = self.envelope(
            HookKind::AgentBoot,
            thread,
            session_id,
            serde_json::json!({ "session_id": session_id }),
            None,
        );
        // The ingest tracks the session (resume id, `agent.session.started`).
        self.ingest(&svc, env).await;
    }

    async fn prompt_context(&self, thread: &ThreadId, session_id: &str) -> Option<String> {
        let svc = self.svc.upgrade()?;
        svc.agent_activity
            .prompt_context(&svc, thread, Some(session_id))
            .await
    }

    async fn turn_started(&self, thread: &ThreadId, session_id: &str, prompt: &str) {
        let Some(svc) = self.svc.upgrade() else {
            return;
        };
        let env = self.envelope(
            HookKind::UserPromptSubmit,
            thread,
            session_id,
            serde_json::json!({ "prompt": prompt, "session_id": session_id }),
            Some(prompt.to_string()),
        );
        self.ingest(&svc, env).await;
    }

    async fn tool_finished(
        &self,
        thread: &ThreadId,
        session_id: &str,
        event: &CanonicalToolEvent,
    ) -> Option<String> {
        let svc = self.svc.upgrade()?;
        let body = event.to_payload();
        let payload_json = body.to_string();
        let env = self.envelope(
            HookKind::PostToolUse,
            thread,
            session_id,
            body.clone(),
            None,
        );
        self.ingest(&svc, env).await;
        svc.agent_activity
            .on_post_tool(&svc, thread, Some(session_id), &body, &payload_json)
            .await
    }

    async fn turn_ended(
        &self,
        thread: &ThreadId,
        session_id: &str,
        prompt: &str,
        tokens: Option<&TurnTokens>,
    ) -> Option<String> {
        let svc = self.svc.upgrade()?;
        let env = self.envelope(
            HookKind::Stop,
            thread,
            session_id,
            serde_json::json!({ "session_id": session_id }),
            None,
        );
        // What the turn this Stop closed did; none when no turn was open.
        let signals = match svc.hook_ingest.ingest(env).await {
            Ok(outcome) => match outcome.closed_turn {
                Some(turn) => crate::agent_policy::TurnSignals::of_turn(&svc.db, turn)
                    .await
                    .ok(),
                None => None,
            },
            Err(err) => {
                warn!(?err, "acp: hook ingest failed");
                None
            }
        };
        if let Some(t) = tokens {
            let turn = crate::token_usage::Turn {
                prompt: Some(prompt.to_string()),
                usage: crate::token_usage::UsageDelta {
                    input_tokens: t.input as i64,
                    output_tokens: t.output as i64,
                    cache_creation_input_tokens: t.cache_write as i64,
                    cache_read_input_tokens: t.cache_read as i64,
                    message_count: 1,
                    model: None,
                },
            };
            if let Err(err) = svc.token_usage.record_turn(thread, session_id, turn).await {
                warn!(?err, "acp: token usage failed");
            }
        }
        svc.agent_policy
            .on_turn_end(&svc, thread, signals.as_ref())
            .await
            .map(|d| d.reason)
    }

    async fn awaiting_user(&self, thread: &ThreadId, question: Option<String>) {
        let Some(svc) = self.svc.upgrade() else {
            return;
        };
        use oxplow_domain::stores::AgentTurnStore as _;
        let (state, detail) = match question {
            Some(q) => {
                // Remember what the first card interrupted.
                if self.before_card.lock().is_none() {
                    let now = svc
                        .agent_status_store
                        .get(thread, "working")
                        .await
                        .ok()
                        .flatten()
                        .map(|s| (s.state, s.detail));
                    *self.before_card.lock() = now;
                }
                (AgentStatusState::AwaitingUser, Some(q))
            }
            None => {
                // Taken into a local: the guard mustn't live across an await.
                let before = self.before_card.lock().take();
                match before {
                    // The agent had parked on the person before the card.
                    Some((AgentStatusState::AwaitingUser, detail)) => {
                        (AgentStatusState::AwaitingUser, detail)
                    }
                    // Otherwise: working if a turn is open, else idle.
                    _ => {
                        let open = svc
                            .agent_turn_store
                            .list_open(thread)
                            .await
                            .map(|t| !t.is_empty())
                            .unwrap_or(false);
                        let state = if open {
                            AgentStatusState::Running
                        } else {
                            AgentStatusState::Idle
                        };
                        (state, None)
                    }
                }
            }
        };
        if let Err(err) = svc.hook_ingest.set_status(thread, state, detail).await {
            warn!(?err, "acp: status update failed");
        }
    }

    async fn interrupted(&self, thread: &ThreadId) {
        let Some(svc) = self.svc.upgrade() else {
            return;
        };
        let env = HookEnvelope {
            kind: HookKind::Interrupt,
            thread_id: Some(*thread),
            stream_id: self.stream_id,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
            decision: None,
        };
        self.ingest(&svc, env).await;
    }

    fn activity(&self, thread: &ThreadId) {
        if let Some(svc) = self.svc.upgrade() {
            svc.output_activity
                .record(*thread, oxplow_domain::Timestamp::now());
        }
    }
}
