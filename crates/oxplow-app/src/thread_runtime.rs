//! Per-thread transient runtime state: the page the person has open on a
//! thread and the effort reviews waiting for its next Stop. Nothing here
//! outlives the process; agent status is on the event log
//! (`oxplow_db::SqliteAgentStatusStore`).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use oxplow_domain::{EffortId, ThreadId, Timestamp};

#[derive(Default)]
struct ThreadRuntime {
    /// Effort ids whose touched_files claim disagreed with the auto-
    /// diff at the close (`effort.report`) time. Drained by the Stop hook to fire
    /// a one-shot directive prompting the agent to call
    /// `effort.amend` (or silently agree). Cleared after the
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

#[derive(Default)]
pub struct ThreadRuntimeRegistry {
    inner: Mutex<HashMap<ThreadId, ThreadRuntime>>,
}

impl ThreadRuntimeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Convenience for shared ownership.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_page_is_per_thread_and_clearable() {
        let r = ThreadRuntimeRegistry::new();
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
}
