//! In-memory store for "what's running right now" rows shown in the
//! status bar.
//!
//! Direct port of `src/electron/background-task-store.ts`. Lives in
//! `oxplow-app` (not `oxplow-db`) because it's intentionally
//! non-persistent — restart drops the running set on the floor, the
//! UI re-discovers tasks as new ones start.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::broadcast;

const DEFAULT_GRACE_MS: i64 = 4_000;
const SNAPSHOT_RETENTION_MS: i64 = 5 * 60_000;
const SNAPSHOT_MAX_ENTRIES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum BackgroundTaskKind {
    #[default]
    Git,
    CodeQuality,
    Lsp,
    NotesResync,
    Snapshot,
    /// Recomputing code metrics over the whole tree (the metric baseline, tsk48).
    /// Slow and CPU-hungry — it must be visible, or "why is oxplow pegging a core?"
    /// has no answer.
    Metrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundTaskStatus {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct BackgroundTask {
    pub id: String,
    pub kind: BackgroundTaskKind,
    pub label: String,
    pub detail: Option<String>,
    /// 0..=1 for determinate, `None` for indeterminate.
    pub progress: Option<f64>,
    pub status: BackgroundTaskStatus,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub error: Option<String>,
    /// Producer-supplied opaque JSON attached at complete/fail.
    /// Stored as a string so the bindings stay simple — the producer
    /// is expected to JSON-stringify before passing.
    pub result_json: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundTaskChangeKind {
    Started,
    Updated,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct BackgroundTaskChange {
    pub kind: BackgroundTaskChangeKind,
    pub id: String,
}

#[derive(Debug, Clone, Default)]
pub struct StartInput {
    pub kind: BackgroundTaskKind,
    pub label: String,
    pub detail: Option<String>,
    pub progress: Option<f64>,
}

#[derive(Default, Clone)]
pub struct UpdateInput {
    pub label: Option<String>,
    pub detail: Option<Option<String>>, // outer Some = "set"; inner None = "clear"
    pub progress: Option<Option<f64>>,
}

struct State {
    tasks: indexmap::IndexMap<String, BackgroundTask>,
    snapshots: VecDeque<BackgroundTask>,
}

#[derive(Clone)]
pub struct BackgroundTaskStore {
    state: Arc<Mutex<State>>,
    events: broadcast::Sender<BackgroundTaskChange>,
}

impl Default for BackgroundTaskStore {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundTaskStore {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            state: Arc::new(Mutex::new(State {
                tasks: indexmap::IndexMap::new(),
                snapshots: VecDeque::new(),
            })),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BackgroundTaskChange> {
        self.events.subscribe()
    }

    pub fn list_running(&self) -> Vec<BackgroundTask> {
        self.gc();
        self.state.lock().tasks.values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<BackgroundTask> {
        self.gc();
        let s = self.state.lock();
        if let Some(t) = s.tasks.get(id) {
            return Some(t.clone());
        }
        s.snapshots.iter().find(|t| t.id == id).cloned()
    }

    pub fn start(&self, input: StartInput) -> BackgroundTask {
        let id = format!("bg-{}", uuid::Uuid::new_v4().simple());
        let now = unix_ms();
        let task = BackgroundTask {
            id: id.clone(),
            kind: input.kind,
            label: input.label,
            detail: input.detail,
            progress: input.progress,
            status: BackgroundTaskStatus::Running,
            started_at: now,
            ended_at: None,
            error: None,
            result_json: None,
        };
        self.state.lock().tasks.insert(id.clone(), task.clone());
        let _ = self.events.send(BackgroundTaskChange {
            kind: BackgroundTaskChangeKind::Started,
            id,
        });
        task
    }

    pub fn update(&self, id: &str, patch: UpdateInput) {
        let mut state = self.state.lock();
        let Some(task) = state.tasks.get_mut(id) else {
            return;
        };
        if let Some(label) = patch.label {
            task.label = label;
        }
        if let Some(detail) = patch.detail {
            task.detail = detail;
        }
        if let Some(progress) = patch.progress {
            task.progress = progress;
        }
        let _ = self.events.send(BackgroundTaskChange {
            kind: BackgroundTaskChangeKind::Updated,
            id: id.to_string(),
        });
    }

    pub fn complete(&self, id: &str, result: Option<serde_json::Value>) {
        let result_json = result.and_then(|v| serde_json::to_string(&v).ok());
        self.end(id, BackgroundTaskStatus::Done, None, result_json);
    }

    pub fn fail(&self, id: &str, error: String, result: Option<serde_json::Value>) {
        let result_json = result.and_then(|v| serde_json::to_string(&v).ok());
        self.end(id, BackgroundTaskStatus::Failed, Some(error), result_json);
    }

    fn end(
        &self,
        id: &str,
        status: BackgroundTaskStatus,
        error: Option<String>,
        result_json: Option<String>,
    ) {
        let mut state = self.state.lock();
        let Some(task) = state.tasks.get_mut(id) else {
            return;
        };
        task.status = status;
        task.ended_at = Some(unix_ms());
        task.error = error;
        task.result_json = result_json;
        let snapshot = task.clone();
        // Move into the snapshot ring so awaiters can still resolve
        // by id after the grace window ticks the row off the bar.
        state.snapshots.push_back(snapshot);
        while state.snapshots.len() > SNAPSHOT_MAX_ENTRIES {
            state.snapshots.pop_front();
        }
        let _ = self.events.send(BackgroundTaskChange {
            kind: BackgroundTaskChangeKind::Ended,
            id: id.to_string(),
        });
    }

    /// Drop completed-but-still-visible tasks past the grace window
    /// and stale snapshot entries.
    fn gc(&self) {
        let now = unix_ms();
        let mut state = self.state.lock();
        let to_drop: Vec<String> = state
            .tasks
            .iter()
            .filter_map(|(id, t)| match (t.status, t.ended_at) {
                (BackgroundTaskStatus::Running, _) => None,
                (_, Some(ended)) if now - ended > DEFAULT_GRACE_MS => Some(id.clone()),
                _ => None,
            })
            .collect();
        for id in to_drop {
            state.tasks.shift_remove(&id);
        }
        // Snapshot eviction by retention window.
        while let Some(front) = state.snapshots.front() {
            let ended = front.ended_at.unwrap_or(now);
            if now - ended > SNAPSHOT_RETENTION_MS {
                state.snapshots.pop_front();
            } else {
                break;
            }
        }
    }
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_then_complete_keeps_task_until_grace_expires() {
        let store = BackgroundTaskStore::new();
        let task = store.start(StartInput {
            kind: BackgroundTaskKind::Git,
            label: "git push".into(),
            ..Default::default()
        });
        store.complete(&task.id, None);
        assert!(store.get(&task.id).is_some());
        // The task should still be in the running list because it
        // hasn't been GC'd yet (gc() is called on read but the grace
        // window of 4s hasn't elapsed).
        let running = store.list_running();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].status, BackgroundTaskStatus::Done);
    }

    #[test]
    fn fail_carries_error_and_result() {
        let store = BackgroundTaskStore::new();
        let task = store.start(StartInput::default());
        store.fail(
            &task.id,
            "boom".into(),
            Some(serde_json::json!({"code": 1})),
        );
        let got = store.get(&task.id).unwrap();
        assert_eq!(got.status, BackgroundTaskStatus::Failed);
        assert_eq!(got.error.as_deref(), Some("boom"));
        assert_eq!(got.result_json.as_deref(), Some("{\"code\":1}"));
    }

    #[test]
    fn update_patches_fields() {
        let store = BackgroundTaskStore::new();
        let task = store.start(StartInput {
            label: "v1".into(),
            ..Default::default()
        });
        store.update(
            &task.id,
            UpdateInput {
                label: Some("v2".into()),
                progress: Some(Some(0.5)),
                ..Default::default()
            },
        );
        let got = store.get(&task.id).unwrap();
        assert_eq!(got.label, "v2");
        assert_eq!(got.progress, Some(0.5));
    }

    #[test]
    fn events_fire_in_order() {
        let store = BackgroundTaskStore::new();
        let mut rx = store.subscribe();
        let task = store.start(StartInput::default());
        store.update(&task.id, UpdateInput::default());
        store.complete(&task.id, None);
        let started = rx.try_recv().unwrap();
        assert_eq!(started.kind, BackgroundTaskChangeKind::Started);
        let updated = rx.try_recv().unwrap();
        assert_eq!(updated.kind, BackgroundTaskChangeKind::Updated);
        let ended = rx.try_recv().unwrap();
        assert_eq!(ended.kind, BackgroundTaskChangeKind::Ended);
    }

    #[test]
    fn snapshot_resolves_by_id_after_grace_eviction() {
        let store = BackgroundTaskStore::new();
        let task = store.start(StartInput::default());
        store.complete(&task.id, Some(serde_json::json!({"ok": true})));
        // Manually pretend the grace window expired by setting the
        // task's ended_at to far in the past.
        {
            let mut state = store.state.lock();
            if let Some(t) = state.tasks.get_mut(&task.id) {
                t.ended_at = Some(unix_ms() - DEFAULT_GRACE_MS - 1);
            }
        }
        let _ = store.list_running(); // triggers gc
                                      // The running map dropped the task but the snapshot remains.
        let snap = store.get(&task.id).unwrap();
        assert_eq!(snap.status, BackgroundTaskStatus::Done);
        assert_eq!(snap.result_json.as_deref(), Some("{\"ok\":true}"));
    }
}
