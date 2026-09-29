//! What an agent's activity records, and the context oxplow hands it,
//! shared by every agent transport (tsk334). The hook route feeds it
//! Claude's hook payloads; the ACP client feeds it [`CanonicalToolEvent`]s
//! rendered into the same shape, so every recorder (tool calls, effort
//! claims, wiki attribution, collection) and every reader of the hook log
//! keys on one vocabulary: Claude's tool names (`Edit`, `Read`, `Bash`, …)
//! and `tool_input.file_path` / `command`.
//!
//! State kept here is runtime-only (losing it on a restart costs at most
//! one repeated context block or DB read): the launch role and last
//! context per agent session, and the last resume id persisted per thread.
//! See `.context/agent-model.md`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use oxplow_domain::stores::{AgentTurnStore, StreamStore, ThreadStore};
use oxplow_domain::{HookKind, ThreadId};
use parking_lot::Mutex;
use tracing::warn;

use crate::agent_policy::TurnSignals;
use crate::{
    build_session_context_block_with_role, role_change_banner, HookEnvelope, RoleMode, Services,
};

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

/// Dedupe key suffix for the decisions block, beside the session-context
/// block's plain session-id key.
const DECISIONS_KEY_SUFFIX: &str = "#decisions";

/// Recording and context shared by every agent transport
/// (`Services.agent_activity`).
#[derive(Default)]
pub struct AgentActivity {
    role_state: Mutex<RoleState>,
    /// Last resume session id persisted per thread; lets repeated events
    /// skip the thread read. A stale entry costs one extra read, never a
    /// wrong write.
    resume_state: Mutex<HashMap<ThreadId, String>>,
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

    /// Adopt the observed session id as the thread's resume token when it
    /// differs. Best-effort: failures are logged and skipped.
    pub async fn track_resume(&self, svc: &Services, env: &HookEnvelope) {
        let Some(observed) = env.session_id.as_deref() else {
            return;
        };
        if observed.is_empty() {
            return;
        }
        let Some(thread_id) = env.thread_id.as_ref() else {
            return;
        };
        // The resume id changes once per session, so a cached thread
        // short-circuits before the DB.
        if resume_cache_allows_skip(
            self.resume_state.lock().get(thread_id).map(|s| s.as_str()),
            observed,
        ) {
            return;
        }
        let thread = match svc.thread_store.get(thread_id).await {
            Ok(Some(t)) => t,
            Ok(None) => return,
            Err(err) => {
                warn!(?err, "resume-tracker: thread lookup failed");
                return;
            }
        };
        if thread.resume_session_id == observed {
            // Already in sync: seed the cache (a cold cache after restart).
            self.resume_state
                .lock()
                .insert(*thread_id, observed.to_string());
            return;
        }
        let mut updated = thread;
        updated.resume_session_id = observed.to_string();
        updated.updated_at = oxplow_domain::Timestamp::now();
        if let Err(err) = svc.thread_store.upsert(&updated).await {
            warn!(?err, "resume-tracker: thread upsert failed");
            return;
        }
        self.resume_state
            .lock()
            .insert(*thread_id, observed.to_string());
    }

    /// SessionEnd: drop the resume token when an explicit `/clear` ended
    /// exactly the session it points at (see [`resume_should_clear`]).
    pub async fn clear_resume_on_session_end(
        &self,
        svc: &Services,
        thread_id: Option<&ThreadId>,
        session_id: Option<&str>,
        body: Option<&serde_json::Value>,
    ) {
        let (Some(thread_id), Some(ended)) = (thread_id, session_id) else {
            return;
        };
        let reason = body.and_then(|v| v.get("reason")).and_then(|r| r.as_str());
        let thread = match svc.thread_store.get(thread_id).await {
            Ok(Some(t)) => t,
            Ok(None) => return,
            Err(err) => {
                warn!(?err, "resume-tracker: thread lookup failed on SessionEnd");
                return;
            }
        };
        if !resume_should_clear(reason, ended, &thread.resume_session_id) {
            return;
        }
        let mut updated = thread;
        updated.resume_session_id = String::new();
        updated.updated_at = oxplow_domain::Timestamp::now();
        if let Err(err) = svc.thread_store.upsert(&updated).await {
            warn!(?err, "resume-tracker: clearing resume token failed");
        }
    }

    /// What this turn did so far (read from the hook log since the open
    /// turn started): the Stop pipeline's Q&A carve-out and write signal.
    /// Call before the Stop event is ingested (ingest closes the turn).
    pub async fn mine_turn_signals(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
    ) -> Option<TurnSignals> {
        let open = svc.agent_turn_store.list_open(thread_id).await.ok()?;
        let started_at = open.first()?.started_at;
        let events = svc
            .hook_event_store
            .list_recent(Some(thread_id), 200)
            .await
            .ok()?;
        let mut signals = TurnSignals::default();
        for evt in events {
            if evt.received_at < started_at {
                continue;
            }
            if !matches!(evt.kind, HookKind::PreToolUse | HookKind::PostToolUse) {
                continue;
            }
            signals.had_activity = true;
            if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&evt.payload_json) {
                if let Some(tool_name) = payload.get("tool_name").and_then(|v| v.as_str()) {
                    if matches!(tool_name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
                        signals.had_writes = true;
                    }
                }
            }
        }
        Some(signals)
    }

    /// Record a finished tool call: wiki attribution, the effort-file
    /// claim, the tool-call row, and collection (test runs, coverage,
    /// analysis) plus post-tool advisories. `payload_json` is `body` as
    /// received (collection reads Bash output from it). Returns context
    /// for the agent: the ROLE CHANGE banner after `ExitPlanMode`, else a
    /// collection nudge or advisory.
    ///
    /// Collection runs DETACHED (tsk62): a test run's recording can outlast
    /// a transport's budget, so it always completes on its own task and
    /// this waits up to 2.5 s for its message (a late one is still
    /// persisted as a nudge by the task).
    pub async fn on_post_tool(
        &self,
        svc: &Arc<Services>,
        thread_id: &ThreadId,
        session_id: Option<&str>,
        body: &serde_json::Value,
        payload_json: &str,
    ) -> Option<String> {
        attribute_wiki_page_edit(svc, thread_id, body).await;
        // Tool paths are the thread's tree's, not the project's (tsk386).
        let worktree = svc.thread_worktree(thread_id).await;
        attribute_effort_file_edit(svc, thread_id, body, &worktree).await;
        record_tool_call(svc, thread_id, payload_json, &worktree).await;

        let services = svc.clone();
        let collection_thread = *thread_id;
        let payload = payload_json.to_string();
        let (nudge_tx, nudge_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let nudge = match services
                .collection
                .on_post_tool_use(&collection_thread, &payload)
                .await
            {
                Ok(nudge) => nudge,
                Err(err) => {
                    warn!(?err, "collection post-tool-use failed");
                    None
                }
            };
            let advisories = crate::advisories::for_thread(
                &services,
                &collection_thread,
                crate::extensions::AdvisoryOn::PostToolUse,
            )
            .await;
            let combined: Vec<String> = nudge
                .into_iter()
                .chain(advisories.into_iter().map(|h| h.text))
                .collect();
            let _ = nudge_tx.send((!combined.is_empty()).then(|| combined.join("\n\n")));
        });
        let collection_nudge =
            match tokio::time::timeout(std::time::Duration::from_millis(2500), nudge_rx).await {
                Ok(Ok(nudge)) => nudge,
                _ => None,
            };

        // ExitPlanMode just settled: a promotion or demotion while the
        // plan-mode prompt was up gets no prompt event before the agent
        // resumes, so the banner rides this call's context. (ExitPlanMode
        // is never a test-run command, so it never races the nudge.)
        if body.get("tool_name").and_then(|v| v.as_str()) == Some("ExitPlanMode") {
            if let Some(banner) = self.role_change_banner(svc, thread_id, session_id).await {
                return Some(banner);
            }
        }
        collection_nudge
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
        let advisory_hits =
            crate::advisories::for_thread(svc, thread_id, crate::extensions::AdvisoryOn::Prompt)
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

/// Attribute an edit of `.oxplow/wiki/<slug>.md` to the thread (the rail's
/// "Finished" list). Best-effort.
async fn attribute_wiki_page_edit(svc: &Services, thread_id: &ThreadId, body: &serde_json::Value) {
    let tool_name = body.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    if !matches!(tool_name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
        return;
    }
    let Some(tool_input) = body.get("tool_input") else {
        return;
    };
    let Some(path) = tool_input
        .get("file_path")
        .or_else(|| tool_input.get("notebook_path"))
        .or_else(|| tool_input.get("path"))
        .and_then(|v| v.as_str())
    else {
        return;
    };
    let Some(slug) = wiki_page_slug_from_path(path, &svc.layout.project_dir) else {
        return;
    };
    if let Err(err) = svc
        .wiki_page_thread_updates
        .touch(thread_id, &slug, oxplow_domain::Timestamp::now())
        .await
    {
        warn!(?err, slug, "wiki-page attribution failed");
    }
}

/// Claim the file a structured edit just wrote onto the thread's open
/// effort (claim-first attribution). Only Edit / Write / MultiEdit /
/// NotebookEdit: Bash and formatter writes are left to snapshot
/// reconciliation. Best-effort.
async fn attribute_effort_file_edit(
    svc: &Services,
    thread_id: &ThreadId,
    body: &serde_json::Value,
    worktree: &Path,
) {
    let tool_name = body.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    let Some(rel) = effort_claim_path_from_edit(tool_name, body.get("tool_input"), worktree) else {
        return;
    };
    if let Err(err) = svc
        .tasks
        .claim_open_effort_file(&svc.effort_store, thread_id, &rel, Some(worktree))
        .await
    {
        warn!(?err, path = rel, "effort file auto-claim failed");
    }
}

/// Persist a finished tool call as an `agent_tool_call` row on the
/// thread's open effort (`v_tool_call`). Best-effort.
async fn record_tool_call(
    svc: &Services,
    thread_id: &ThreadId,
    payload_json: &str,
    worktree: &Path,
) {
    use crate::EffortStore as _;
    let Some(parts) = crate::tool_calls::parse_tool_call(payload_json, worktree) else {
        return;
    };
    let effort_id = match svc.effort_store.find_open_for_thread(thread_id).await {
        Ok(e) => e.map(|e| e.id.value()),
        Err(err) => {
            warn!(?err, "tool-call effort lookup failed");
            None
        }
    };
    let call = oxplow_db::NewToolCall {
        thread_id: thread_id.value(),
        effort_id,
        tool: parts.tool,
        path: parts.path,
        detail: parts.detail,
        ok: parts.ok,
    };
    if let Err(err) = svc.tool_call_store.record(call).await {
        warn!(?err, "tool-call record failed");
    }
}

/// Repo-relative path to claim from a structured edit, or `None` when it
/// isn't one, names no path, or the path is absolute outside the project.
fn effort_claim_path_from_edit(
    tool_name: &str,
    tool_input: Option<&serde_json::Value>,
    project_dir: &Path,
) -> Option<String> {
    if !matches!(tool_name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
        return None;
    }
    let raw = tool_input?
        .get("file_path")
        .or_else(|| tool_input?.get("notebook_path"))
        .or_else(|| tool_input?.get("path"))
        .and_then(|v| v.as_str())?;
    let path = Path::new(raw);
    if path.is_absolute() {
        path.strip_prefix(project_dir)
            .ok()
            .map(|r| r.to_string_lossy().into_owned())
    } else {
        Some(raw.to_string())
    }
}

/// The wiki-page slug for a path directly inside `.oxplow/wiki/` with a
/// `.md` extension (absolute or project-relative).
fn wiki_page_slug_from_path(raw: &str, project_dir: &Path) -> Option<String> {
    let path = Path::new(raw);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_dir.join(path)
    };
    let notes_dir = project_dir.join(".oxplow").join("wiki");
    let rel = abs.strip_prefix(&notes_dir).ok()?;
    if rel
        .parent()
        .map(|p| !p.as_os_str().is_empty())
        .unwrap_or(false)
    {
        return None;
    }
    let stem = rel.file_stem()?.to_string_lossy().into_owned();
    let ext = rel.extension()?.to_string_lossy();
    (ext == "md").then_some(stem)
}

/// Skip the resume tracker's DB work when the cache already holds this
/// exact session id for the thread.
fn resume_cache_allows_skip(cached: Option<&str>, observed: &str) -> bool {
    cached == Some(observed)
}

/// Drop the resume token only when an explicit `/clear` ended exactly the
/// session it points at; normal exits keep it, and clearing a stale
/// session must not wipe a newer token.
fn resume_should_clear(reason: Option<&str>, ended_session: &str, current_resume: &str) -> bool {
    reason == Some("clear") && !ended_session.is_empty() && ended_session == current_resume
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In a worktree stream (a sibling directory of the project) an edit's
    /// absolute path is inside the thread's worktree, not the project: it
    /// is claimed and recorded repo-relative all the same (tsk386).
    #[tokio::test]
    async fn worktree_stream_edits_are_claimed_and_recorded_relative() {
        let f = crate::test_fixtures::services_with_effort().await;
        let worktree = tempfile::tempdir().unwrap();
        let wt = worktree.path().to_string_lossy().to_string();
        let thread = {
            use oxplow_domain::stores::ThreadStore as _;
            f.svc.thread_store.get(&f.thread).await.unwrap().unwrap()
        };
        let stream = thread.stream_id.value();
        f.svc
            .db
            .transaction(move |c| {
                c.execute(
                    "UPDATE streams SET worktree_path = ?1 WHERE id = ?2",
                    (wt.as_str(), stream),
                )
                .map_err(|e| oxplow_domain::DomainError::Invalid(e.to_string()))?;
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(f.svc.thread_worktree(&f.thread).await, worktree.path());

        let file = worktree.path().join("src/a.rs");
        let body = serde_json::json!({
            "tool_name": "Edit",
            "tool_input": { "file_path": file.to_string_lossy() },
            "tool_response": {},
        });
        f.svc
            .agent_activity
            .on_post_tool(&f.svc, &f.thread, None, &body, &body.to_string())
            .await;
        let sl = oxplow_db::SemanticLayer::new(f.svc.db.clone());
        let q = |sql: &'static str| {
            let sl = sl.clone();
            async move {
                serde_json::to_value(sl.query_sql(sql, vec![], None).await.unwrap().rows).unwrap()
            }
        };
        assert_eq!(
            q("SELECT path FROM v_tool_call").await,
            serde_json::json!([["src/a.rs"]])
        );
        assert_eq!(
            q("SELECT path FROM v_effort_file").await,
            serde_json::json!([["src/a.rs"]])
        );
    }

    #[test]
    fn resume_clear_decision() {
        // Only an explicit clear of the exact resume session drops it.
        assert!(resume_should_clear(Some("clear"), "s1", "s1"));
        // Other exit reasons keep the token (restart should resume).
        assert!(!resume_should_clear(Some("other"), "s1", "s1"));
        assert!(!resume_should_clear(Some("prompt_input_exit"), "s1", "s1"));
        assert!(!resume_should_clear(None, "s1", "s1"));
        // A clear of a stale session must not wipe a newer token.
        assert!(!resume_should_clear(Some("clear"), "old", "newer"));
        // Degenerate ids never match.
        assert!(!resume_should_clear(Some("clear"), "", ""));
    }

    #[test]
    fn wiki_slug_from_relative_path_in_notes_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        // A relative path that resolves into .oxplow/wiki returns the slug.
        let slug = wiki_page_slug_from_path(".oxplow/wiki/architecture.md", tmp.path());
        assert_eq!(slug.as_deref(), Some("architecture"));
    }

    #[test]
    fn wiki_slug_from_absolute_path_in_notes_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let abs = tmp.path().join(".oxplow/wiki/data-model.md");
        let slug = wiki_page_slug_from_path(&abs.to_string_lossy(), tmp.path());
        assert_eq!(slug.as_deref(), Some("data-model"));
    }

    #[test]
    fn wiki_slug_rejects_non_md_extension() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(wiki_page_slug_from_path(".oxplow/wiki/foo.txt", tmp.path()).is_none());
        // No extension at all.
        assert!(wiki_page_slug_from_path(".oxplow/wiki/foo", tmp.path()).is_none());
    }

    #[test]
    fn wiki_slug_rejects_subdirectory_paths() {
        // Wiki notes must be flat under .oxplow/wiki — a path with a
        // subdirectory shouldn't accidentally adopt the basename.
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(wiki_page_slug_from_path(".oxplow/wiki/sub/inner.md", tmp.path()).is_none());
    }

    #[test]
    fn wiki_slug_rejects_paths_outside_notes_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(wiki_page_slug_from_path("README.md", tmp.path()).is_none());
        assert!(wiki_page_slug_from_path(".oxplow/other/foo.md", tmp.path()).is_none());
        assert!(wiki_page_slug_from_path("/etc/hosts", tmp.path()).is_none());
    }

    #[test]
    fn effort_claim_path_extracts_repo_relative_for_structured_tools() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Relative file_path → returned as-is (already repo-relative).
        let ti = serde_json::json!({ "file_path": "src/edited.rs" });
        assert_eq!(
            effort_claim_path_from_edit("Edit", Some(&ti), tmp.path()).as_deref(),
            Some("src/edited.rs")
        );
        // Absolute path inside the project → normalized to repo-relative.
        let abs = tmp.path().join("crates/x/lib.rs");
        let ti_abs = serde_json::json!({ "file_path": abs.to_string_lossy() });
        assert_eq!(
            effort_claim_path_from_edit("Write", Some(&ti_abs), tmp.path()).as_deref(),
            Some("crates/x/lib.rs")
        );
        // NotebookEdit uses notebook_path.
        let ti_nb = serde_json::json!({ "notebook_path": "nb/run.ipynb" });
        assert_eq!(
            effort_claim_path_from_edit("NotebookEdit", Some(&ti_nb), tmp.path()).as_deref(),
            Some("nb/run.ipynb")
        );
    }

    #[test]
    fn effort_claim_path_excludes_bash_and_outside_project() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Bash (and any non-structured tool) is intentionally NOT auto-claimed.
        let ti = serde_json::json!({ "command": "echo hi > out.txt" });
        assert!(effort_claim_path_from_edit("Bash", Some(&ti), tmp.path()).is_none());
        // An absolute path outside the project is not an effort file.
        let ti_out = serde_json::json!({ "file_path": "/etc/hosts" });
        assert!(effort_claim_path_from_edit("Edit", Some(&ti_out), tmp.path()).is_none());
        // Missing path → None.
        let ti_empty = serde_json::json!({});
        assert!(effort_claim_path_from_edit("Edit", Some(&ti_empty), tmp.path()).is_none());
    }

    #[test]
    fn resume_cache_skips_only_on_exact_match() {
        // Cache hit: the thread already has this session id persisted →
        // skip the DB round-trip entirely.
        assert!(resume_cache_allows_skip(Some("s1"), "s1"));
        // Cache miss / changed / first-seen → must hit the DB.
        assert!(!resume_cache_allows_skip(None, "s1"));
        assert!(!resume_cache_allows_skip(Some("s0"), "s1"));
        // Degenerate empty cached value never matches a real id.
        assert!(!resume_cache_allows_skip(Some(""), "s1"));
    }

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
        // What the recorders read: a claim path and a tool-call row.
        let body = ev.to_payload();
        assert_eq!(
            effort_claim_path_from_edit("Edit", body.get("tool_input"), Path::new("/p")).as_deref(),
            Some("src/a.rs")
        );
        assert!(crate::tool_calls::parse_tool_call(&body.to_string(), Path::new("/p")).is_some());
    }
}
