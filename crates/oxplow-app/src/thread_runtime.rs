//! Per-thread transient runtime state.
//!
//! Holds two pieces of state that don't deserve persistence:
//! - The recent hook event ring (capped per thread). Hooks fire,
//!   drive state changes, and are uninteresting after — same shape
//!   as main's `HookEventStore`.
//! - The agent_status snapshot (one row per pane_target). This used
//!   to live in SQLite but recovery reset it to "stopped" on every
//!   boot anyway, so persistence bought nothing but a sync surface
//!   to drift from.
//!
//! `agent_turn` is *not* held here — it's the durable record of
//! turns and stays in SQLite for historical reporting.
//!
//! The same registry implements both `HookEventStore` and
//! `AgentStatusStore`, so the rest of the application keeps using
//! the trait objects it already had — only the wiring in
//! `Services::boot` changes.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use oxplow_domain::stores::{AgentStatusStore, HookEventStore};
use oxplow_domain::{
    AgentStatus, AgentStatusState, DomainError, EffortId, HookEvent, HookKind, ThreadId, Timestamp,
};

/// Default per-thread cap on the hook ring. Sized so the typical
/// stop_directive / status derive paths (which look back at most ~50
/// events) always have enough headroom, while a runaway tool-loop
/// can't grow memory unbounded.
pub const DEFAULT_HOOK_CAPACITY: usize = 500;

#[derive(Default)]
struct ThreadRuntime {
    /// Most-recent-LAST. `list_recent` reverses on read.
    hooks: VecDeque<HookEvent>,
    /// Keyed by pane_target.
    statuses: HashMap<String, AgentStatus>,
    /// Effort ids whose touched_files claim disagreed with the auto-
    /// diff at complete_task time. Drained by the Stop hook to fire
    /// a one-shot directive prompting the agent to call
    /// `amend_effort` (or silently agree). Cleared after the
    /// directive fires so a single review never repeats.
    pending_effort_reviews: HashSet<EffortId>,
    /// The page the human currently has open in this thread, as the UI
    /// last reported it. Ephemeral: what an agent reads via
    /// `get_open_page` to "look at what I'm looking at".
    open_page: Option<OpenPage>,
}

/// What the human is looking at in a thread.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenPage {
    /// Tab/page id, e.g. `task:42`, `file:src/a.rs`, `lens:review/waiting`.
    pub page_id: String,
    /// Page kind, e.g. `task`, `file`, `lens`.
    pub kind: String,
    /// Page-specific context as JSON text (for a lens: `{"lensId", "params"}`).
    pub detail_json: Option<String>,
    /// When the UI reported it.
    pub reported_at: Timestamp,
}

pub struct ThreadRuntimeRegistry {
    inner: Mutex<HashMap<ThreadId, ThreadRuntime>>,
    hook_capacity: usize,
}

impl ThreadRuntimeRegistry {
    pub fn new(hook_capacity: usize) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            hook_capacity,
        }
    }

    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_HOOK_CAPACITY)
    }

    /// Convenience for shared ownership: most callers want
    /// `Arc<dyn HookEventStore>` and `Arc<dyn AgentStatusStore>`
    /// referencing the same backing state.
    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }

    /// Stash an effort id whose touched_files claim disagreed with
    /// the snapshot diff. The Stop hook drains the per-thread set
    /// and surfaces a directive listing each. Idempotent.
    /// Record (or clear, with `None`) the page open in `thread`.
    pub fn set_open_page(&self, thread: &ThreadId, page: Option<OpenPage>) {
        let mut m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        m.entry(*thread).or_default().open_page = page;
    }

    /// The page last reported open in `thread`, if any.
    pub fn open_page(&self, thread: &ThreadId) -> Option<OpenPage> {
        let m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        m.get(thread).and_then(|rt| rt.open_page.clone())
    }

    pub fn record_pending_effort_review(&self, thread: &ThreadId, effort: EffortId) {
        let mut m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = m.entry(*thread).or_default();
        runtime.pending_effort_reviews.insert(effort);
    }

    /// Take + clear all pending effort review ids for a thread. The
    /// Stop hook calls this when building its directive — once
    /// surfaced, the review doesn't re-fire. If the agent ignored
    /// the prompt that's fine; this matches the "silent agreement"
    /// path in the design.
    pub fn take_pending_effort_reviews(&self, thread: &ThreadId) -> Vec<EffortId> {
        let mut m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(runtime) = m.get_mut(thread) else {
            return Vec::new();
        };
        runtime.pending_effort_reviews.drain().collect()
    }
}

#[async_trait]
impl HookEventStore for ThreadRuntimeRegistry {
    async fn append(&self, event: &HookEvent) -> Result<(), DomainError> {
        // Hooks without a thread_id can't be attributed to a thread —
        // ingest the event for state effects but don't store it. This
        // matches the prior SQLite behavior which made thread_id
        // nullable (it stored them but no consumer looked for them).
        let Some(tid) = event.thread_id else {
            return Ok(());
        };
        let mut m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = m.entry(tid).or_default();
        runtime.hooks.push_back(event.clone());
        while runtime.hooks.len() > self.hook_capacity {
            runtime.hooks.pop_front();
        }
        Ok(())
    }

    async fn list_recent(
        &self,
        thread: Option<&ThreadId>,
        limit: usize,
    ) -> Result<Vec<HookEvent>, DomainError> {
        let m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let events: Vec<HookEvent> = match thread {
            Some(t) => m
                .get(t)
                .map(|r| r.hooks.iter().rev().take(limit).cloned().collect())
                .unwrap_or_default(),
            None => {
                let mut all: Vec<HookEvent> =
                    m.values().flat_map(|r| r.hooks.iter().cloned()).collect();
                all.sort_by_key(|e| std::cmp::Reverse(e.received_at));
                all.truncate(limit);
                all
            }
        };
        Ok(events)
    }

    async fn list_by_kind(
        &self,
        kind: HookKind,
        limit: usize,
    ) -> Result<Vec<HookEvent>, DomainError> {
        let m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut all: Vec<HookEvent> = m
            .values()
            .flat_map(|r| r.hooks.iter().filter(|e| e.kind == kind).cloned())
            .collect();
        all.sort_by_key(|e| std::cmp::Reverse(e.received_at));
        all.truncate(limit);
        Ok(all)
    }
}

#[async_trait]
impl AgentStatusStore for ThreadRuntimeRegistry {
    async fn upsert(
        &self,
        thread: &ThreadId,
        pane_target: &str,
        state: AgentStatusState,
        detail: Option<String>,
    ) -> Result<AgentStatus, DomainError> {
        let status = AgentStatus {
            thread_id: *thread,
            pane_target: pane_target.to_string(),
            state,
            detail,
            updated_at: Timestamp::now(),
        };
        let mut m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = m.entry(*thread).or_default();
        runtime
            .statuses
            .insert(pane_target.to_string(), status.clone());
        Ok(status)
    }

    async fn get(
        &self,
        thread: &ThreadId,
        pane_target: &str,
    ) -> Result<Option<AgentStatus>, DomainError> {
        let m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Ok(m.get(thread)
            .and_then(|r| r.statuses.get(pane_target).cloned()))
    }

    async fn list_all(&self) -> Result<Vec<AgentStatus>, DomainError> {
        let m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Ok(m.values()
            .flat_map(|r| r.statuses.values().cloned())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::HookEventId;

    #[test]
    fn open_page_is_per_thread_and_clearable() {
        let r = ThreadRuntimeRegistry::with_default_capacity();
        let t1 = ThreadId::new(1);
        let t2 = ThreadId::new(2);
        assert_eq!(r.open_page(&t1), None);
        let page = OpenPage {
            page_id: "lens:review/waiting".into(),
            kind: "lens".into(),
            detail_json: Some(r#"{"lensId":"review/waiting","params":{}}"#.into()),
            reported_at: Timestamp::from_unix_ms(5),
        };
        r.set_open_page(&t1, Some(page.clone()));
        assert_eq!(r.open_page(&t1), Some(page));
        assert_eq!(r.open_page(&t2), None);
        r.set_open_page(&t1, None);
        assert_eq!(r.open_page(&t1), None);
    }

    fn ev(thread: i64, kind: HookKind, ms: i64) -> HookEvent {
        HookEvent {
            id: HookEventId::new(ms),
            thread_id: Some(ThreadId::new(thread)),
            stream_id: None,
            kind,
            session_id: None,
            payload_json: "{}".into(),
            received_at: Timestamp::from_unix_ms(ms),
        }
    }

    #[tokio::test]
    async fn append_then_list_recent_returns_newest_first_within_thread() {
        let r = ThreadRuntimeRegistry::with_default_capacity();
        r.append(&ev(1, HookKind::UserPromptSubmit, 1))
            .await
            .unwrap();
        r.append(&ev(1, HookKind::PreToolUse, 2)).await.unwrap();
        r.append(&ev(2, HookKind::Stop, 3)).await.unwrap();
        let recent = r.list_recent(Some(&ThreadId::new(1)), 10).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].kind, HookKind::PreToolUse); // newest first
        assert_eq!(recent[1].kind, HookKind::UserPromptSubmit);
    }

    #[tokio::test]
    async fn ring_caps_per_thread() {
        let r = ThreadRuntimeRegistry::new(3);
        for i in 0..5 {
            r.append(&ev(1, HookKind::PreToolUse, i)).await.unwrap();
        }
        let recent = r.list_recent(Some(&ThreadId::new(1)), 100).await.unwrap();
        assert_eq!(recent.len(), 3);
        // The newest three should remain (ms 4, 3, 2).
        assert_eq!(recent[0].received_at, Timestamp::from_unix_ms(4));
        assert_eq!(recent[2].received_at, Timestamp::from_unix_ms(2));
    }

    #[tokio::test]
    async fn list_recent_with_no_thread_filter_merges_across_threads_desc() {
        let r = ThreadRuntimeRegistry::with_default_capacity();
        r.append(&ev(1, HookKind::UserPromptSubmit, 1))
            .await
            .unwrap();
        r.append(&ev(2, HookKind::Stop, 3)).await.unwrap();
        r.append(&ev(1, HookKind::PreToolUse, 2)).await.unwrap();
        let all = r.list_recent(None, 10).await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].received_at, Timestamp::from_unix_ms(3));
        assert_eq!(all[2].received_at, Timestamp::from_unix_ms(1));
    }

    #[tokio::test]
    async fn list_by_kind_filters() {
        let r = ThreadRuntimeRegistry::with_default_capacity();
        r.append(&ev(1, HookKind::UserPromptSubmit, 1))
            .await
            .unwrap();
        r.append(&ev(1, HookKind::PreToolUse, 2)).await.unwrap();
        r.append(&ev(1, HookKind::Stop, 3)).await.unwrap();
        let stops = r.list_by_kind(HookKind::Stop, 10).await.unwrap();
        assert_eq!(stops.len(), 1);
    }

    #[tokio::test]
    async fn agent_status_upsert_get_list() {
        let r = ThreadRuntimeRegistry::with_default_capacity();
        let tid = ThreadId::new(1);
        let s = r
            .upsert(&tid, "working", AgentStatusState::Running, None)
            .await
            .unwrap();
        assert_eq!(s.state, AgentStatusState::Running);
        let got = r.get(&tid, "working").await.unwrap().unwrap();
        assert_eq!(got.state, AgentStatusState::Running);
        let all = r.list_all().await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn hooks_without_thread_id_are_dropped() {
        let r = ThreadRuntimeRegistry::with_default_capacity();
        let mut e = ev(0, HookKind::Stop, 1);
        e.thread_id = None;
        r.append(&e).await.unwrap();
        let all = r.list_recent(None, 10).await.unwrap();
        assert!(all.is_empty());
    }
}
