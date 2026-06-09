//! Per-stream snapshot capture services.
//!
//! Snapshot capture is per-worktree: each `Stream` watches its own
//! worktree path and writes `file_snapshot` rows tagged with its
//! `stream_id`. The registry is the single lookup point — callers
//! resolve the right service via `get(&stream_id)` and use it for
//! `request_snapshot`, blob reads, etc.
//!
//! All services share the same `SqliteSnapshotStore` + `BlobStore` so
//! the on-disk representation is unified; only the in-memory dirty
//! set, settle gate, and fs-watch listener are per-stream.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use oxplow_db::SqliteSnapshotStore;
use oxplow_domain::{Stream, StreamId};
use oxplow_fs_watch::WorkspaceFilter;

use crate::blob_store::BlobStore;
use crate::events::EventBus;
use crate::snapshot_capture::SnapshotCaptureService;

/// Build parameters shared across every per-stream service. The
/// registry holds these so `register()` can construct fresh services
/// at runtime (e.g. when a new stream is created) without callers
/// having to re-thread the storage handles each time.
#[derive(Clone)]
pub struct SnapshotCaptureRegistryConfig {
    pub snapshot_store: Arc<SqliteSnapshotStore>,
    pub blobs: BlobStore,
    pub max_file_bytes: u64,
    pub workspace_filter: WorkspaceFilter,
    pub events: EventBus,
}

/// Per-stream registry of [`SnapshotCaptureService`]s. Look up by
/// `StreamId`; mutate via `register` / `unregister`. Cheap to clone —
/// the underlying map sits behind an `Arc<RwLock<…>>`.
#[derive(Clone)]
pub struct SnapshotCaptureRegistry {
    services: Arc<RwLock<HashMap<StreamId, Arc<SnapshotCaptureService>>>>,
    /// `primary_id` lets legacy single-stream callers fetch the
    /// primary service without already knowing its id — the wiki
    /// watcher, MCP file-ref-version resolver, and bootloader logs
    /// all need it. Set once at boot.
    primary_id: Arc<RwLock<Option<StreamId>>>,
    /// Live workspace filter, seeded from `config.workspace_filter`.
    /// Kept separate (and behind a lock) so [`Self::set_workspace_filter`]
    /// can update the filter handed to *future* `register()` calls when
    /// the project's `generated` config changes at runtime.
    workspace_filter: Arc<RwLock<WorkspaceFilter>>,
    config: SnapshotCaptureRegistryConfig,
}

impl SnapshotCaptureRegistry {
    pub fn new(config: SnapshotCaptureRegistryConfig) -> Self {
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
            primary_id: Arc::new(RwLock::new(None)),
            workspace_filter: Arc::new(RwLock::new(config.workspace_filter.clone())),
            config,
        }
    }

    /// Build + insert a service for `stream`. Idempotent: if a service
    /// already exists for this stream id the existing handle is
    /// returned unchanged (caller stays observer of the same fs-watch
    /// pipeline). Streams whose worktree path doesn't exist on disk
    /// (archived, orphaned) are silently skipped — returns `None`.
    pub fn register(&self, stream: &Stream) -> Option<Arc<SnapshotCaptureService>> {
        let worktree = PathBuf::from(&stream.worktree_path);
        if !worktree.is_dir() {
            tracing::debug!(
                stream_id = %stream.id,
                worktree = %stream.worktree_path,
                "snapshot registry: skipping stream — worktree not on disk",
            );
            return None;
        }
        // Fast-path: already registered.
        {
            let services = self.services.read().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = services.get(&stream.id) {
                return Some(existing.clone());
            }
        }
        let svc = Arc::new(
            SnapshotCaptureService::new(
                self.config.snapshot_store.clone(),
                self.config.blobs.clone(),
                worktree,
                stream.id,
                self.config.max_file_bytes,
                self.workspace_filter
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone(),
            )
            .with_events(self.config.events.clone()),
        );
        let mut services = self.services.write().unwrap_or_else(|e| e.into_inner());
        // Double-check after acquiring the write lock — a concurrent
        // register for the same id may have raced us.
        if let Some(existing) = services.get(&stream.id) {
            return Some(existing.clone());
        }
        services.insert(stream.id, svc.clone());
        Some(svc)
    }

    /// Drop the service for `id` and tear down its watcher. The
    /// `spawn_watcher` task holds its own clone of the service and its
    /// `FsWatcher` is a task-local, so dropping the registry's `Arc`
    /// alone never wakes it — we must `shutdown()` to signal the task
    /// to exit. Done outside the write lock so we don't hold it across
    /// the notify.
    pub fn unregister(&self, id: &StreamId) {
        let removed = {
            let mut services = self.services.write().unwrap_or_else(|e| e.into_inner());
            services.remove(id)
        };
        if let Some(svc) = removed {
            svc.shutdown();
        }
    }

    /// Swap the workspace path filter for every live service AND the
    /// copy used to build future ones. Called when the project's
    /// `generated` config changes (via the `set_generated` IPC) so
    /// snapshot include/exclude edits take effect without a restart.
    pub fn set_workspace_filter(&self, filter: WorkspaceFilter) {
        *self
            .workspace_filter
            .write()
            .unwrap_or_else(|e| e.into_inner()) = filter.clone();
        for svc in self.list() {
            svc.set_workspace_filter(filter.clone());
        }
    }

    pub fn get(&self, id: &StreamId) -> Option<Arc<SnapshotCaptureService>> {
        self.services
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// Snapshot of the currently-registered services. Order is not
    /// guaranteed; callers needing primary-first should query
    /// [`Self::primary`] explicitly.
    pub fn list(&self) -> Vec<Arc<SnapshotCaptureService>> {
        self.services
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }

    /// Mark `id` as the primary stream — used by legacy callers that
    /// haven't migrated to a stream-aware lookup yet (wiki watcher,
    /// MCP file-ref-version resolver). Must be called after the
    /// corresponding `register` so the lookup actually resolves.
    pub fn set_primary(&self, id: StreamId) {
        *self.primary_id.write().unwrap_or_else(|e| e.into_inner()) = Some(id);
    }

    /// Fetch the primary stream's service. `None` only before boot
    /// has called [`Self::set_primary`].
    pub fn primary(&self) -> Option<Arc<SnapshotCaptureService>> {
        let id = (*self
            .primary_id
            .read()
            .unwrap_or_else(|e| e.into_inner()))?;
        self.get(&id)
    }

    /// Spawn the fs-watch listener for every currently-registered
    /// service. Safe to call multiple times — each service
    /// idempotently spawns at most one watcher per `Arc` (in practice
    /// only main.rs calls this, once at boot).
    pub fn spawn_all_watchers(&self) -> Vec<tokio::task::JoinHandle<()>> {
        let services = self.services.read().unwrap_or_else(|e| e.into_inner());
        services.values().map(|s| s.spawn_watcher()).collect()
    }

    /// Test-only: insert a pre-built service under `id`, replacing any
    /// existing entry. Lets tests override the default builder (with
    /// custom settle / predrain durations, for example) without
    /// re-implementing the full `register` path.
    pub fn insert_for_test(&self, id: StreamId, svc: Arc<SnapshotCaptureService>) {
        let mut services = self.services.write().unwrap_or_else(|e| e.into_inner());
        services.insert(id, svc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;
    use oxplow_domain::{StreamKind, Timestamp};
    use std::time::Duration;

    fn config_for(project: &std::path::Path) -> SnapshotCaptureRegistryConfig {
        let db = Database::in_memory();
        SnapshotCaptureRegistryConfig {
            snapshot_store: Arc::new(SqliteSnapshotStore::new(db)),
            blobs: BlobStore::new(project.join(".oxplow/snapshots")),
            max_file_bytes: 1_000_000,
            workspace_filter: WorkspaceFilter::default(),
            events: EventBus::new(),
        }
    }

    fn stream_at(path: &std::path::Path) -> Stream {
        Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: path.to_string_lossy().into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        }
    }

    #[tokio::test]
    async fn unregister_tears_down_the_watcher() {
        let project = tempfile::tempdir().unwrap();
        let reg = SnapshotCaptureRegistry::new(config_for(project.path()));
        let stream = stream_at(project.path());
        let svc = reg.register(&stream).expect("worktree exists");
        let handle = svc.spawn_watcher();
        // Let the watcher park on rx.recv().
        tokio::time::sleep(Duration::from_millis(50)).await;

        reg.unregister(&stream.id);
        assert!(reg.get(&stream.id).is_none(), "service should be removed");

        let joined = tokio::time::timeout(Duration::from_secs(5), handle).await;
        assert!(
            joined.is_ok(),
            "unregister must shut the watcher down, but it stayed parked",
        );
    }
}
