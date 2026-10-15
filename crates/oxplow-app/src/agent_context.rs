//! The context oxplow hands an agent, shared by every transport (tsk334,
//! P3.8): the `<session-context>` block (deduped per session), prompt
//! advisories and the open effort's decisions on a prompt; the ROLE CHANGE
//! banner or the thread's undelivered nudges after a tool call. What an
//! agent *did* is recorded elsewhere — the hook ingest logs `agent.*` events
//! and pump reactors record from them (`tool_call_reactors`,
//! `post_tool_reactors`, the token reactor).
//!
//! State kept here is runtime-only (losing it on a restart costs at most
//! one repeated context block): the launch role and last context per agent
//! session. The session's resume id is the hook ingest's (`hook_ingest`).
//! See `.context/agent-model.md`.

use std::collections::HashMap;

use oxplow_domain::agent::tool::{ToolKind, ToolUse};
use oxplow_domain::stores::{StreamStore, ThreadStore};
use oxplow_domain::ThreadId;
use parking_lot::Mutex;
use tracing::warn;

use crate::agent_prompt::{build_session_context_block_full, WriterAgent};
use crate::{role_change_banner, RoleMode, Services};

/// The launch role and the last context block per agent session.
#[derive(Default)]
struct RoleState {
    initial_role_by_session_id: HashMap<String, RoleMode>,
    /// Last context block returned per session (and per `…#decisions`
    /// key). The launch prompt already carries this data, so a
    /// byte-identical repeat adds noise without new information.
    last_context_by_session_id: HashMap<String, String>,
}

/// How long a tool call's hook waits for the reactors that write nudges.
const POST_TOOL_SETTLE: std::time::Duration = std::time::Duration::from_millis(2500);

/// Dedupe key suffix for the decisions block, beside the session-context
/// block's plain session-id key.
const DECISIONS_KEY_SUFFIX: &str = "#decisions";

/// Recording and context shared by every agent transport
/// (`Services.agent_context`).
#[derive(Default)]
pub struct AgentContext {
    role_state: Mutex<RoleState>,
}

impl AgentContext {
    /// A fresh agent context (startup / resume / clear / compact): forget
    /// the session's baselines so the next prompt carries fresh context.
    pub fn reset_session(&self, session_id: Option<&str>) {
        let Some(session_id) = session_id else {
            return;
        };
        let mut state = self.role_state.lock();
        state.initial_role_by_session_id.remove(session_id);
        state.last_context_by_session_id.remove(session_id);
        state
            .last_context_by_session_id
            .remove(&format!("{session_id}{DECISIONS_KEY_SUFFIX}"));
    }

    /// What the agent hears after a tool call: the ROLE CHANGE banner after
    /// a plan settles, else the thread's undelivered nudges (collection and
    /// post-tool advisories). The recording itself is the pump's
    /// (`tool_call_reactors`, `post_tool_reactors`, P3.5–P3.6): this waits
    /// up to [`POST_TOOL_SETTLE`] for the two reactors that write nudges,
    /// then takes whatever is undelivered — a nudge that lands after the
    /// window goes out on the thread's next tool call instead of being lost.
    /// Collection nudges come before advisories.
    /// Nothing for a session whose harness can't carry context after a
    /// tool call (`tool_context`): its nudges wait for a prompt.
    pub async fn post_tool_context(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
        harness: Option<&str>,
        tool: Option<&ToolUse>,
    ) -> Option<String> {
        if !carries(svc, harness, TOOL_CONTEXT) {
            return None;
        }
        // A plan just settled: a promotion or demotion while the plan was
        // up for approval gets no prompt event before the agent resumes, so
        // the banner rides this call's context. (A plan is never a test-run
        // command; any nudges wait for the next call.)
        if tool.is_some_and(|t| t.kind == ToolKind::Plan) {
            if let Some(banner) = self.role_change_banner(svc, thread_id, session_id).await {
                return Some(banner);
            }
        }
        svc.event_pump
            .settle(
                &[
                    crate::post_tool_reactors::COLLECTION,
                    crate::post_tool_reactors::POST_TOOL_ADVISORIES,
                ],
                POST_TOOL_SETTLE,
            )
            .await;
        undelivered(svc, thread_id).await
    }

    /// Context for a human's prompt: the `<session-context>` block (only
    /// when it changed for this session), the thread's undelivered nudges
    /// (prompt advisories, and the hints its last turn's end left), and the
    /// open effort's decisions (once per session, and when they change).
    /// Nothing for a session whose harness can't carry context on a prompt
    /// (`prompt_context`): nothing meant for its agent is taken.
    pub async fn prompt_context(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
        harness: Option<&str>,
    ) -> Option<String> {
        if !carries(svc, harness, PROMPT_CONTEXT) {
            return None;
        }
        let ctx_block = self
            .refreshed_session_context(svc, thread_id, session_id)
            .await;
        // Prompt advisories land as nudges beside any a turn's end or a
        // late tool call left; the prompt carries them all.
        crate::advisories::for_thread(
            &svc.advisory_deps(),
            thread_id,
            crate::extensions::AdvisoryOn::Prompt,
            None,
        )
        .await;
        let advisory_block = undelivered(svc, thread_id).await;
        let decisions_block = self
            .refreshed_decisions_context(svc, thread_id, session_id)
            .await;
        let combined: String = [ctx_block, advisory_block, decisions_block]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n");
        (!combined.is_empty()).then_some(combined)
    }

    /// The role this thread had when `session_id` started (captured on
    /// first sight). `None` without a session id.
    async fn capture_or_get_initial_role(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
    ) -> Option<RoleMode> {
        let session_id = session_id?.to_string();
        let thread = svc.thread_store.get(thread_id).await.ok().flatten()?;
        let current = RoleMode::from_thread(&thread);
        let mut st = self.role_state.lock();
        Some(
            *st.initial_role_by_session_id
                .entry(session_id)
                .or_insert(current),
        )
    }

    /// A fresh `<session-context>` block (with a ROLE CHANGE banner when
    /// the role flipped), unless it's unchanged for this session or the
    /// project turned injection off.
    async fn refreshed_session_context(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
    ) -> Option<String> {
        let cfg = svc.config.read().ok()?.clone();
        if !cfg.inject_session_context {
            return None;
        }
        let thread = svc.thread_store.get(thread_id).await.ok().flatten()?;
        let stream = svc
            .stream_store
            .get(&thread.stream_id)
            .await
            .ok()
            .flatten()?;
        let initial = self
            .capture_or_get_initial_role(svc, thread_id, session_id)
            .await;
        let writer = if RoleMode::from_thread(&thread) == RoleMode::ReadOnly {
            live_writer_agent(svc, &thread).await
        } else {
            None
        };
        let block =
            build_session_context_block_full(&stream, Some(&thread), initial, writer.as_ref());
        self.should_emit(session_id, &block).then_some(block)
    }

    /// The open effort's recorded decisions, on the first prompt of a
    /// session and whenever they change.
    async fn refreshed_decisions_context(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
    ) -> Option<String> {
        use crate::EffortStore as _;
        let effort = svc
            .effort_store
            .find_open_for_thread(thread_id)
            .await
            .ok()
            .flatten()?;
        let block = crate::reasoning::effort_decisions_block(&svc.sql, effort.id.value()).await?;
        let key = session_id.map(|s| format!("{s}{DECISIONS_KEY_SUFFIX}"));
        self.should_emit(key.as_deref(), &block).then_some(block)
    }

    /// Whether `block` is new for `session_id` (recording it if so).
    /// Without a session id it always emits: suppressing could hide a
    /// change from another session sharing the thread.
    fn should_emit(&self, session_id: Option<&str>, block: &str) -> bool {
        let Some(session_id) = session_id else {
            return true;
        };
        let mut state = self.role_state.lock();
        match state.last_context_by_session_id.get(session_id) {
            Some(previous) if previous == block => false,
            _ => {
                state
                    .last_context_by_session_id
                    .insert(session_id.to_string(), block.to_string());
                true
            }
        }
    }

    /// Just the ROLE CHANGE sentence, when the thread's role differs from
    /// the role recorded at the start of `session_id`.
    async fn role_change_banner(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
    ) -> Option<String> {
        let session_id = session_id?.to_string();
        let thread = svc.thread_store.get(thread_id).await.ok().flatten()?;
        let current = RoleMode::from_thread(&thread);
        let initial = self
            .role_state
            .lock()
            .initial_role_by_session_id
            .get(&session_id)
            .copied()?;
        (initial != current).then(|| role_change_banner(initial, current))
    }
}

/// The agent-harness feature: context oxplow returns to a prompt hook
/// reaches the agent.
pub const PROMPT_CONTEXT: &str = "prompt_context";
/// The agent-harness feature: context oxplow returns to a tool hook
/// reaches the agent before its next step.
pub const TOOL_CONTEXT: &str = "tool_context";

/// Whether `harness` (a session's, by registry key) declares `channel`.
/// With no harness known (a hook from no session) the answer is sent.
pub fn carries(svc: &Services, harness: Option<&str>, channel: &str) -> bool {
    let Some(harness) = harness else {
        return true;
    };
    svc.capabilities
        .get("agent_harness", harness)
        .is_some_and(|h| h.has_feature(channel))
}

/// Characters of nudges one hook carries to the agent; the rest wait for
/// its next.
pub const NUDGE_BUDGET: usize = 2_000;

/// The agent's undelivered nudges on the thread as one block, up to
/// [`NUDGE_BUDGET`], marked delivered: each reaches the agent once, on
/// whichever hook comes first, oxplow's own before advisories.
async fn undelivered(svc: &Services, thread_id: &ThreadId) -> Option<String> {
    let nudges = match svc
        .nudge_store
        .take_for_agent(&thread_id.to_string(), NUDGE_BUDGET)
        .await
    {
        Ok(n) => n,
        Err(err) => {
            warn!(?err, "reading undelivered nudges failed");
            return None;
        }
    };
    let text: Vec<String> = nudges.into_iter().map(|n| n.message).collect();
    (!text.is_empty()).then(|| text.join("\n\n"))
}

/// The live agent on the writer thread of `thread`'s stream: its newest
/// open session whose process still exists. `None` with no writer thread
/// or no live agent on it.
async fn live_writer_agent(svc: &Services, thread: &oxplow_domain::Thread) -> Option<WriterAgent> {
    use oxplow_domain::stores::AgentSessionStore as _;
    let writer = svc
        .thread_store
        .list_for_stream(&thread.stream_id)
        .await
        .ok()?
        .into_iter()
        .find(|t| t.id != thread.id && t.status.is_writer())?;
    let sessions = svc
        .agent_session_store
        .list_open_for_thread(&writer.id)
        .await
        .ok()?;
    let (session, pid) = sessions
        .into_iter()
        .rev()
        .filter_map(|s| s.pid.map(|pid| (s, pid)))
        .find(|(_, pid)| crate::daemon_supervisor::process_alive(*pid))?;
    Some(WriterAgent {
        thread_title: writer.title,
        thread_id: writer.id,
        harness: session.harness,
        pid: Some(pid),
        session: session.resume_session_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A read-only thread's context names the writer thread's live agent;
    /// a writer's own does not, and a dead pid names nothing.
    #[tokio::test]
    async fn a_read_only_thread_is_told_the_writer_threads_live_agent() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &*fx.svc;
        let stream = svc
            .thread_store
            .get(&fx.thread)
            .await
            .unwrap()
            .unwrap()
            .stream_id;
        let other = crate::test_fixtures::new_thread(svc, stream, "other").await;
        assert!(!other.status.is_writer());

        // No pid yet: nothing to name.
        let ctx = AgentContext::default();
        let none = ctx
            .refreshed_session_context(svc, &other.id, Some("s1"))
            .await
            .unwrap();
        assert!(!none.contains("Writer thread"), "{none}");

        // This test process stands in for the agent.
        let me = std::process::id();
        svc.db
            .transaction(move |tx| {
                oxplow_db::agent_session_store::set_pid_tx(
                    tx,
                    fx.session,
                    me,
                    oxplow_domain::Timestamp::now(),
                )
            })
            .await
            .unwrap();
        let ctx = AgentContext::default();
        let named = ctx
            .refreshed_session_context(svc, &other.id, Some("s2"))
            .await
            .unwrap();
        assert!(
            named.contains(&format!("agent `claude`, pid {me}")),
            "{named}"
        );
        let own = AgentContext::default()
            .refreshed_session_context(svc, &fx.thread, Some("s3"))
            .await
            .unwrap();
        assert!(!own.contains("Writer thread"), "{own}");
    }

    #[test]
    fn session_context_emits_initial_and_changed_blocks_only() {
        let state = AgentContext::default();

        assert!(state.should_emit(Some("session-1"), "context-a"));
        assert!(!state.should_emit(Some("session-1"), "context-a"));
        assert!(state.should_emit(Some("session-1"), "context-b"));
    }

    #[test]
    fn session_context_without_session_id_is_never_suppressed() {
        let state = AgentContext::default();
        assert!(state.should_emit(None, "context"));
        assert!(state.should_emit(None, "context"));
    }

    #[test]
    fn clearing_session_context_baseline_allows_fresh_emission() {
        let state = AgentContext::default();
        state
            .role_state
            .lock()
            .initial_role_by_session_id
            .insert("session-1".into(), RoleMode::Writer);
        assert!(state.should_emit(Some("session-1"), "context"));
        state.reset_session(Some("session-1"));
        assert!(!state
            .role_state
            .lock()
            .initial_role_by_session_id
            .contains_key("session-1"));
        assert!(state.should_emit(Some("session-1"), "context"));
    }
}
