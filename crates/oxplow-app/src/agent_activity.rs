//! What an agent's activity records, and the context oxplow hands it,
//! shared by every agent transport (tsk334). The hook route feeds it
//! Claude's hook payloads; the ACP client feeds it [`CanonicalToolEvent`]s
//! rendered into the same shape, so every recorder (tool calls, effort
//! claims, wiki attribution, collection) and every reader of the hook log
//! keys on one vocabulary: Claude's tool names (`Edit`, `Read`, `Bash`, …)
//! and `tool_input.file_path` / `command`.
//!
//! State kept here is runtime-only (losing it on a restart costs at most
//! one repeated context block): the launch role and last context per agent
//! session. The session's resume id is the hook ingest's (`hook_ingest`).
//! See `.context/agent-model.md`.

use std::collections::HashMap;

use oxplow_domain::stores::{StreamStore, ThreadStore};
use oxplow_domain::ThreadId;
use parking_lot::Mutex;
use tracing::warn;

use crate::{build_session_context_block_with_role, role_change_banner, RoleMode, Services};

/// A tool call in the canonical (Claude-shaped) vocabulary, for transports
/// whose agents don't speak it natively. [`Self::to_payload`] is the one
/// place that shape is built.
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalToolEvent {
    /// `Edit`, `Write`, `Read`, `Grep`, `Bash`, `WebFetch`, `mcp__…`, …
    pub tool_name: String,
    /// `{file_path}`, `{command}`, `{pattern}`, `{url}`, …
    pub tool_input: serde_json::Value,
    /// `{is_error: bool, …}` once the call finished.
    pub tool_response: Option<serde_json::Value>,
    pub session_id: Option<String>,
}

impl CanonicalToolEvent {
    /// The hook-payload shape every recorder reads.
    pub fn to_payload(&self) -> serde_json::Value {
        let mut v = serde_json::json!({
            "tool_name": self.tool_name,
            "tool_input": self.tool_input,
        });
        if let Some(r) = &self.tool_response {
            v["tool_response"] = r.clone();
        }
        if let Some(s) = &self.session_id {
            v["session_id"] = serde_json::Value::String(s.clone());
        }
        v
    }
}

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
/// (`Services.agent_activity`).
#[derive(Default)]
pub struct AgentActivity {
    role_state: Mutex<RoleState>,
}

impl AgentActivity {
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
    /// `ExitPlanMode`, else the thread's undelivered nudges (collection and
    /// post-tool advisories). The recording itself is the pump's
    /// (`tool_call_reactors`, `post_tool_reactors`, P3.5–P3.6): this waits
    /// up to [`POST_TOOL_SETTLE`] for the two reactors that write nudges,
    /// then takes whatever is undelivered — a nudge that lands after the
    /// window goes out on the thread's next tool call instead of being lost.
    /// Collection nudges come before advisories.
    pub async fn on_post_tool(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
        body: &serde_json::Value,
    ) -> Option<String> {
        // ExitPlanMode just settled: a promotion or demotion while the
        // plan-mode prompt was up gets no prompt event before the agent
        // resumes, so the banner rides this call's context. (ExitPlanMode is
        // never a test-run command; any nudges wait for the next call.)
        if body.get("tool_name").and_then(|v| v.as_str()) == Some("ExitPlanMode") {
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
        let mut nudges = match svc
            .nudge_store
            .take_undelivered(&thread_id.to_string())
            .await
        {
            Ok(n) => n,
            Err(err) => {
                warn!(?err, "reading undelivered nudges failed");
                return None;
            }
        };
        // Advisory kinds are `<extension>/<id>`; oxplow's own come first.
        nudges.sort_by_key(|n| (n.kind.contains('/'), n.id));
        let text: Vec<String> = nudges.into_iter().map(|n| n.message).collect();
        (!text.is_empty()).then(|| text.join("\n\n"))
    }

    /// Context for a human's prompt: the `<session-context>` block (only
    /// when it changed for this session), prompt advisories, and the open
    /// effort's decisions (once per session, and when they change).
    pub async fn prompt_context(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
    ) -> Option<String> {
        let ctx_block = self
            .refreshed_session_context(svc, thread_id, session_id)
            .await;
        let advisory_hits = crate::advisories::for_thread(
            &svc.advisory_deps(),
            thread_id,
            crate::extensions::AdvisoryOn::Prompt,
            None,
        )
        .await;
        let advisory_block = (!advisory_hits.is_empty()).then(|| {
            advisory_hits
                .into_iter()
                .map(|h| h.text)
                .collect::<Vec<_>>()
                .join("\n\n")
        });
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
        let block = build_session_context_block_with_role(&stream, Some(&thread), initial);
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
        let block = crate::reasoning::effort_decisions_block(
            &oxplow_db::SemanticLayer::new(svc.db.clone()),
            effort.id.value(),
        )
        .await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn session_context_emits_initial_and_changed_blocks_only() {
        let state = AgentActivity::default();

        assert!(state.should_emit(Some("session-1"), "context-a"));
        assert!(!state.should_emit(Some("session-1"), "context-a"));
        assert!(state.should_emit(Some("session-1"), "context-b"));
    }

    #[test]
    fn session_context_without_session_id_is_never_suppressed() {
        let state = AgentActivity::default();
        assert!(state.should_emit(None, "context"));
        assert!(state.should_emit(None, "context"));
    }

    #[test]
    fn clearing_session_context_baseline_allows_fresh_emission() {
        let state = AgentActivity::default();
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

    #[test]
    fn a_canonical_event_renders_the_hook_payload_shape() {
        let ev = CanonicalToolEvent {
            tool_name: "Edit".into(),
            tool_input: serde_json::json!({"file_path": "src/a.rs"}),
            tool_response: Some(serde_json::json!({"is_error": false})),
            session_id: Some("s1".into()),
        };
        assert_eq!(
            ev.to_payload(),
            serde_json::json!({"tool_name": "Edit", "tool_input": {"file_path": "src/a.rs"}, "tool_response": {"is_error": false}, "session_id": "s1"})
        );
        // What the ingest reads: the tool, its path and outcome.
        let body = ev.to_payload();
        let parts = crate::tool_calls::parse_tool_call(&body.to_string(), Path::new("/p")).unwrap();
        assert_eq!(
            (parts.tool.as_str(), parts.path.as_deref()),
            ("Edit", Some("src/a.rs"))
        );
    }
}
