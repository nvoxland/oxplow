//! What an ACP session asks of the rest of oxplow: the policy, and the
//! same recording a terminal agent's hooks get. [`AcpHost`] is the seam
//! (tests use a recording double); [`ServicesAcpHost`] is the real one,
//! routing through `AgentPolicy`, `AgentContext` and hook ingest so an
//! ACP turn lands in the same tables as a hooked one.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use oxplow_domain::{AgentStatusState, HookKind, StreamId, ThreadId};
use oxplow_runtime::policy::PolicyDecision;
use tracing::warn;

use oxplow_domain::agent::tool::ToolUse;

use super::wire::TurnTokens;
use crate::hook_ingest::HookEnvelope;
use crate::Services;

#[async_trait]
pub trait AcpHost: Send + Sync + 'static {
    /// May the thread run this tool call? Logged as the call's PreToolUse;
    /// `content` is its input as the agent sent it (`mapping::content`).
    async fn check_tool(
        &self,
        thread: &ThreadId,
        session_id: &str,
        tool: &ToolUse,
        content: &serde_json::Value,
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
        tool: &ToolUse,
        content: &serde_json::Value,
    ) -> Option<String>;
    /// The turn ended: record it, with its final answer and the counts the
    /// agent reported.
    async fn turn_ended(
        &self,
        thread: &ThreadId,
        session_id: &str,
        answer: Option<&str>,
        tokens: Option<&TurnTokens>,
    );
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
    /// The agent session it hosts.
    session: oxplow_domain::AgentSessionId,
    /// The thread status a permission card interrupted, restored when the
    /// cards are answered (so a question the thread was waiting on survives
    /// one).
    before_card: parking_lot::Mutex<Option<(AgentStatusState, Option<String>)>>,
}

impl ServicesAcpHost {
    /// The host of agent session `session` (in stream `stream_id`): every
    /// envelope and status it records names that session.
    pub fn new(
        svc: &Arc<Services>,
        stream_id: Option<StreamId>,
        session: oxplow_domain::AgentSessionId,
    ) -> Self {
        Self {
            svc: Arc::downgrade(svc),
            stream_id,
            session,
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
            agent_session_id: Some(self.session),
            session_id: Some(session_id.to_string()),
            payload_json: payload.to_string(),
            prompt,
            decision: None,
            tool: None,
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
        tool: &ToolUse,
        content: &serde_json::Value,
    ) -> PolicyDecision {
        let Some(svc) = self.svc.upgrade() else {
            return PolicyDecision::Allow;
        };
        let decision = svc
            .agent_policy
            .check_tool(&svc, thread, &crate::agent_policy::intent_of(tool))
            .await;
        let mut env = self.envelope(
            HookKind::PreToolUse,
            thread,
            session_id,
            content.clone(),
            None,
        );
        env.tool = Some(tool.clone());
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
            HookKind::SessionStart,
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
        svc.agent_context
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
        tool: &ToolUse,
        content: &serde_json::Value,
    ) -> Option<String> {
        let svc = self.svc.upgrade()?;
        let mut env = self.envelope(
            HookKind::PostToolUse,
            thread,
            session_id,
            content.clone(),
            None,
        );
        env.tool = Some(tool.clone());
        self.ingest(&svc, env).await;
        svc.agent_context
            .post_tool_context(&svc, thread, Some(session_id), Some(tool))
            .await
    }

    async fn turn_ended(
        &self,
        thread: &ThreadId,
        session_id: &str,
        answer: Option<&str>,
        tokens: Option<&TurnTokens>,
    ) {
        let Some(svc) = self.svc.upgrade() else {
            return;
        };
        // The turn's own counts ride its `agent.turn.ended`; the
        // `token_usage.turns` reactor records them.
        let mut body = serde_json::json!({
            "session_id": session_id,
            crate::hook_ingest::LAST_ASSISTANT_MESSAGE: answer,
        });
        if let Some(t) = tokens {
            body[crate::hook_ingest::TURN_USAGE_KEY] = serde_json::json!({
                "input": t.input,
                "output": t.output,
                "cache_write": t.cache_write,
                "cache_read": t.cache_read,
            });
        }
        let env = self.envelope(HookKind::Stop, thread, session_id, body, None);
        if let Err(err) = svc.hook_ingest.ingest(env).await {
            warn!(?err, "acp: hook ingest failed");
        }
    }

    async fn awaiting_user(&self, thread: &ThreadId, question: Option<String>) {
        let Some(svc) = self.svc.upgrade() else {
            return;
        };
        use oxplow_domain::stores::AgentTurnStore as _;
        let session = Some(self.session);
        let (state, detail) = match question {
            Some(q) => {
                // Remember what the first card interrupted.
                if self.before_card.lock().is_none() {
                    let now = svc
                        .agent_status_store
                        .get(thread, session)
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
        if let Err(err) = svc
            .hook_ingest
            .set_status(thread, session, state, detail)
            .await
        {
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
            agent_session_id: Some(self.session),
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
            decision: None,
            tool: None,
        };
        self.ingest(&svc, env).await;
    }

    fn activity(&self, _thread: &ThreadId) {
        if let Some(svc) = self.svc.upgrade() {
            svc.output_activity
                .record(self.session, oxplow_domain::Timestamp::now());
        }
    }
}
