//! Per-thread follow-up reminders the agent posts before stopping.
//!
//! Direct port of `src/electron/followup-store.ts`. In-memory only —
//! follow-ups are intentionally ephemeral; UserPromptSubmit clears
//! them.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::broadcast;

use oxplow_domain::{FollowupId, ThreadId};

/// Process-local counter for ephemeral follow-up ids. Follow-ups never
/// hit the DB, so there's no rowid to allocate — a monotonic counter
/// gives each the canonical `fup<int>` shape.
static NEXT_FOLLOWUP_ID: AtomicI64 = AtomicI64::new(1);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Followup {
    pub id: String,
    pub thread_id: ThreadId,
    pub body: String,
    pub created_at: i64,
}

#[derive(Clone)]
pub struct FollowupStore {
    inner: Arc<Mutex<indexmap::IndexMap<String, Followup>>>,
    events: broadcast::Sender<ThreadId>,
}

impl Default for FollowupStore {
    fn default() -> Self {
        Self::new()
    }
}

impl FollowupStore {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(Mutex::new(indexmap::IndexMap::new())),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ThreadId> {
        self.events.subscribe()
    }

    pub fn add(&self, thread_id: ThreadId, body: String) -> Followup {
        let id = FollowupId::new(NEXT_FOLLOWUP_ID.fetch_add(1, Ordering::Relaxed)).to_string();
        let item = Followup {
            id: id.clone(),
            thread_id,
            body,
            created_at: unix_ms(),
        };
        self.inner.lock().insert(id, item.clone());
        let _ = self.events.send(thread_id);
        item
    }

    pub fn list_for_thread(&self, thread_id: &ThreadId) -> Vec<Followup> {
        self.inner
            .lock()
            .values()
            .filter(|f| &f.thread_id == thread_id)
            .cloned()
            .collect()
    }

    pub fn remove(&self, id: &str) -> Option<Followup> {
        let removed = self.inner.lock().shift_remove(id);
        if let Some(ref item) = removed {
            let _ = self.events.send(item.thread_id);
        }
        removed
    }

    /// Clear every follow-up for `thread_id` — fired from
    /// UserPromptSubmit so each new turn starts fresh.
    pub fn clear_for_thread(&self, thread_id: &ThreadId) {
        let mut map = self.inner.lock();
        let to_remove: Vec<String> = map
            .iter()
            .filter(|(_, f)| &f.thread_id == thread_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in to_remove {
            map.shift_remove(&id);
        }
        let _ = self.events.send(*thread_id);
    }
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_list() {
        let store = FollowupStore::new();
        let tid = ThreadId::new(1);
        store.add(tid, "remember to verify".into());
        store.add(tid, "and the other thing".into());
        let list = store.list_for_thread(&tid);
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn remove_returns_item() {
        let store = FollowupStore::new();
        let tid = ThreadId::new(1);
        let item = store.add(tid, "x".into());
        let removed = store.remove(&item.id).unwrap();
        assert_eq!(removed.id, item.id);
    }

    #[test]
    fn clear_for_thread_only_clears_that_thread() {
        let store = FollowupStore::new();
        let a = ThreadId::new(1);
        let b = ThreadId::new(2);
        store.add(a, "x".into());
        store.add(b, "y".into());
        store.clear_for_thread(&a);
        assert!(store.list_for_thread(&a).is_empty());
        assert_eq!(store.list_for_thread(&b).len(), 1);
    }

    #[test]
    fn add_emits_event() {
        let store = FollowupStore::new();
        let mut rx = store.subscribe();
        let tid = ThreadId::new(1);
        store.add(tid, "x".into());
        let evt = rx.try_recv().unwrap();
        assert_eq!(evt, tid);
    }
}
