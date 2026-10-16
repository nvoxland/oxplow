//! The context oxplow hands an agent, shared by every transport (tsk334,
//! P3.8): the `<session-context>` block (deduped per session — it names
//! the other live agents in the worktree, so it changes as they come and
//! go), prompt advisories and the open effort's decisions on a prompt; the
//! thread's undelivered nudges after a tool call. What an agent *did* is
//! recorded elsewhere — the hook ingest logs `agent.*` events and pump
//! reactors record from them (`tool_call_reactors`, `post_tool_reactors`,
//! the token reactor).
//!
//! State kept here is runtime-only (losing it on a restart costs at most
//! one repeated context block): the last context per agent session. The
//! session's resume id is the hook ingest's (`hook_ingest`). See
//! `.context/agent-model.md`.

use std::collections::HashMap;

use oxplow_domain::stores::{StreamStore, ThreadStore};
use oxplow_domain::{AgentSessionId, StreamId, ThreadId};
use parking_lot::Mutex;
use tracing::warn;

use crate::agent_prompt::{build_session_context_block, OtherAgent};
use crate::Services;

/// The last context block per agent session.
#[derive(Default)]
struct ContextState {
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
    state: Mutex<ContextState>,
}

impl AgentContext {
    /// A fresh agent context (startup / resume / clear / compact): forget
    /// the session's baselines so the next prompt carries fresh context.
    pub fn reset_session(&self, session_id: Option<&str>) {
        let Some(session_id) = session_id else {
            return;
        };
        let mut state = self.state.lock();
        state.last_context_by_session_id.remove(session_id);
        state
            .last_context_by_session_id
            .remove(&format!("{session_id}{DECISIONS_KEY_SUFFIX}"));
    }

    /// What the agent hears after a tool call: the thread's undelivered
    /// nudges (collection and post-tool advisories). The recording itself is the pump's
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
        harness: Option<&str>,
    ) -> Option<String> {
        if !carries(svc, harness, TOOL_CONTEXT) {
            return None;
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

    /// Context for a human's prompt in agent session `agent_session` (its
    /// harness's own id is `session_id`): the `<session-context>` block
    /// (only when it changed for this session), the thread's undelivered nudges
    /// (prompt advisories, and the hints its last turn's end left), and the
    /// open effort's decisions (once per session, and when they change).
    /// Nothing for a session whose harness can't carry context on a prompt
    /// (`prompt_context`): nothing meant for its agent is taken.
    pub async fn prompt_context(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
        agent_session: Option<AgentSessionId>,
        harness: Option<&str>,
    ) -> Option<String> {
        if !carries(svc, harness, PROMPT_CONTEXT) {
            return None;
        }
        let ctx_block = self
            .refreshed_session_context(svc, thread_id, session_id, agent_session)
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

    /// A fresh `<session-context>` block, naming the other live agents in
    /// the worktree, unless it's unchanged for this session or the project
    /// turned injection off.
    async fn refreshed_session_context(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
        agent_session: Option<AgentSessionId>,
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
        let others = others_in_worktree(svc, stream.id, agent_session).await;
        let block = build_session_context_block(&stream, Some(&thread), &others);
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
        let mut state = self.state.lock();
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

/// The live agents in stream `stream`'s worktree but `except` (the asking
/// session): its open sessions on open threads whose process still runs,
/// named as the person sees them (`v_agent_session.display_name`).
pub async fn others_in_worktree(
    svc: &Services,
    stream: StreamId,
    except: Option<AgentSessionId>,
) -> Vec<OtherAgent> {
    use oxplow_db::semantic_layer::SqlCell;
    let Ok(out) = svc
        .sql
        .query_sql(
            "SELECT s.id, s.display_name, s.harness, s.pid, t.id, t.title
               FROM v_agent_session s JOIN v_thread t ON t.id = s.thread_id
              WHERE t.stream_id = ?1 AND s.closed_at IS NULL AND t.closed_at IS NULL
                AND s.pid IS NOT NULL
              ORDER BY t.sort_index, s.opened_at, s.id",
            vec![SqlCell::Int(stream.value())],
            None,
        )
        .await
    else {
        return Vec::new();
    };
    let int = |c: &SqlCell| match c {
        SqlCell::Int(i) => Some(*i),
        _ => None,
    };
    let text = |c: &SqlCell| match c {
        SqlCell::Text(t) => t.clone(),
        _ => String::new(),
    };
    out.rows
        .iter()
        .filter(|r| int(&r[0]).map(AgentSessionId::new) != except)
        .filter(|r| {
            int(&r[3])
                .and_then(|pid| u32::try_from(pid).ok())
                .is_some_and(crate::daemon_supervisor::process_alive)
        })
        .filter_map(|r| {
            Some(OtherAgent {
                name: text(&r[1]),
                harness: text(&r[2]),
                thread_id: ThreadId::new(int(&r[4])?),
                thread_title: text(&r[5]),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Record `pid` as `session`'s process.
    async fn set_pid(svc: &Services, session: AgentSessionId, pid: u32) {
        svc.db
            .transaction(move |tx| {
                oxplow_db::agent_session_store::set_pid_tx(
                    tx,
                    session,
                    pid,
                    oxplow_domain::Timestamp::now(),
                )
            })
            .await
            .unwrap();
    }

    /// An agent's context names every other live agent in its worktree —
    /// in its own thread or another of the stream — but not one whose
    /// process is gone, one with no process, or itself.
    #[tokio::test]
    async fn an_agent_is_told_who_else_is_live_in_its_worktree() {
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
        let theirs = crate::test_fixtures::open_session(svc, other.id).await;
        let gone = crate::test_fixtures::open_session(svc, other.id).await;
        let _no_process = crate::test_fixtures::open_session(svc, other.id).await;
        // This test process stands in for a live agent; a pid past any the
        // OS hands out, for one that's gone.
        let me = std::process::id();
        set_pid(svc, fx.session, me).await;
        set_pid(svc, theirs, me).await;
        set_pid(svc, gone, i32::MAX as u32).await;

        let mine = AgentContext::default()
            .refreshed_session_context(svc, &fx.thread, Some("s1"), Some(fx.session))
            .await
            .unwrap();
        let line = mine
            .lines()
            .find(|l| l.starts_with("- **Also in this worktree:**"))
            .unwrap_or_else(|| panic!("{mine}"));
        assert!(
            line.contains(&format!("(`claude`, thread other (`{}`))", other.id)),
            "{line}"
        );
        assert_eq!(
            line.matches("(`claude`").count(),
            1,
            "only the live one: {line}"
        );

        // From the other thread, the first session is the one named.
        let theirs_ctx = AgentContext::default()
            .refreshed_session_context(svc, &other.id, Some("s2"), Some(theirs))
            .await
            .unwrap();
        assert!(
            theirs_ctx.contains(&format!("thread Thread (`{}`)", fx.thread)),
            "{theirs_ctx}"
        );
    }

    /// Alone in the worktree, the context has no such line.
    #[tokio::test]
    async fn the_calling_session_is_not_listed() {
        let fx = crate::test_fixtures::services_with_effort().await;
        set_pid(&fx.svc, fx.session, std::process::id()).await;
        let ctx = AgentContext::default()
            .refreshed_session_context(&fx.svc, &fx.thread, Some("s1"), Some(fx.session))
            .await
            .unwrap();
        assert!(!ctx.contains("Also in this worktree"), "{ctx}");
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
        assert!(state.should_emit(Some("session-1"), "context"));
        assert!(!state.should_emit(Some("session-1"), "context"));
        state.reset_session(Some("session-1"));
        assert!(state.should_emit(Some("session-1"), "context"));
    }
}
