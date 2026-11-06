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
use crate::snapshot_capture::{ContentPolicy, SnapshotCaptureService};

/// Why work that needs a snapshot's file bytes is skipped under "Track
/// changes only" (the `snapshots.contents` feature is unmet).
pub const NO_CONTENTS_REASON: &str = "the active snapshot implementation keeps no file contents; \
     choose Keep every version in Settings → Capabilities to keep them";

/// Build parameters shared across every per-stream service. The
/// registry holds these so `register()` can construct fresh services
/// at runtime (e.g. when a new stream is created) without callers
/// having to re-thread the storage handles each time.
#[derive(Clone)]
pub struct SnapshotCaptureRegistryConfig {
    pub snapshot_store: Arc<SqliteSnapshotStore>,
    pub blobs: BlobStore,
    /// The VCS whose object store backs clean files.
    pub vcs: Arc<dyn oxplow_domain::vcs::Vcs>,
    pub max_file_bytes: u64,
    /// The project's `generated` lists. Each service's filter is its own,
    /// rooted at its stream's worktree.
    pub generated: oxplow_config::GeneratedConfig,
    /// Lets each service's quiet trigger yield to an open agent turn.
    pub open_turn_probe: Option<crate::snapshot_capture::OpenTurnProbe>,
}

/// Per-stream registry of [`SnapshotCaptureService`]s. Look up by
/// `StreamId`; mutate via `register` / `unregister`. Cheap to clone —
/// the underlying map sits behind an `Arc<RwLock<…>>`.
#[derive(Clone)]
pub struct SnapshotCaptureRegistry {
    services: Arc<RwLock<HashMap<StreamId, Arc<SnapshotCaptureService>>>>,
    /// `primary_id` lets a caller about the primary stream fetch its
    /// service without already knowing its id — the wiki watcher, MCP
    /// file-ref-version resolver, and bootloader logs. Set once at boot.
    primary_id: Arc<RwLock<Option<StreamId>>>,
    /// The `generated` lists now, for *future* `register()` calls.
    generated: Arc<RwLock<oxplow_config::GeneratedConfig>>,
    /// Whether takes keep file bytes: every live service has it, and a
    /// service registered later starts with it.
    content_policy: Arc<RwLock<ContentPolicy>>,
    /// The active snapshot implementation's id, which its takes record.
    provider: Arc<RwLock<Option<String>>>,
    /// The snapshot implementations these services take for (the
    /// built-ins over this pipeline), set as they're registered.
    serves: Arc<RwLock<std::collections::BTreeSet<String>>>,
    /// Set while the active implementation is one it doesn't serve: every
    /// service, and a later one, takes nothing of its own.
    idle: Arc<std::sync::atomic::AtomicBool>,
    config: SnapshotCaptureRegistryConfig,
}

impl SnapshotCaptureRegistry {
    pub fn new(config: SnapshotCaptureRegistryConfig) -> Self {
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
            primary_id: Arc::new(RwLock::new(None)),
            generated: Arc::new(RwLock::new(config.generated.clone())),
            content_policy: Arc::new(RwLock::new(ContentPolicy::default())),
            provider: Arc::default(),
            serves: Arc::default(),
            idle: Arc::default(),
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
        let filter = {
            let generated = self.generated.read().unwrap_or_else(|e| e.into_inner());
            WorkspaceFilter::for_project(&worktree, &generated.exclude, &generated.include)
        };
        let mut svc = SnapshotCaptureService::new(
            self.config.snapshot_store.clone(),
            self.config.blobs.clone(),
            worktree,
            self.config.vcs.clone(),
            stream.id,
            self.config.max_file_bytes,
            filter,
        );
        if let Some(probe) = &self.config.open_turn_probe {
            svc = svc.with_open_turn_probe(probe.clone());
        }
        let svc = Arc::new(svc);
        svc.set_content_policy(self.content_policy());
        svc.set_idle(self.idle.load(std::sync::atomic::Ordering::SeqCst));
        svc.set_provider(
            self.provider
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
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

    /// The project's `generated` lists changed, for every live service and
    /// the ones registered later, so include/exclude edits take effect
    /// without a restart.
    pub fn set_generated(&self, generated: &oxplow_config::GeneratedConfig) {
        generated.clone_into(&mut self.generated.write().unwrap_or_else(|e| e.into_inner()));
        for svc in self.list() {
            svc.set_generated(generated);
        }
    }

    /// Set whether takes keep file bytes on every live service and on the
    /// ones registered later (the active snapshot implementation's choice).
    pub fn set_content_policy(&self, policy: ContentPolicy) {
        *self
            .content_policy
            .write()
            .unwrap_or_else(|e| e.into_inner()) = policy;
        for svc in self.list() {
            svc.set_content_policy(policy);
        }
    }

    /// Record every live service's takes, and later ones', as `provider`.
    pub fn set_provider(&self, provider: Option<String>) {
        *self.provider.write().unwrap_or_else(|e| e.into_inner()) = provider.clone();
        for svc in self.list() {
            svc.set_provider(provider.clone());
        }
    }

    /// The size above which a file's bytes aren't kept (`oversize`).
    pub fn max_file_bytes(&self) -> u64 {
        self.config.max_file_bytes
    }

    /// The snapshot implementations this pipeline takes for: the rest are
    /// processes that mark the worktree themselves.
    pub fn set_serves(&self, ids: impl IntoIterator<Item = String>) {
        *self.serves.write().unwrap_or_else(|e| e.into_inner()) = ids.into_iter().collect();
    }

    /// Follow the active snapshot implementation `id`: when it's one this
    /// pipeline serves the services take as usual, otherwise they idle
    /// ([`SnapshotCaptureService::set_idle`]).
    pub fn follow(&self, id: Option<&str>) {
        let idle = id.is_some_and(|id| {
            !self
                .serves
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .contains(id)
        });
        self.idle.store(idle, std::sync::atomic::Ordering::SeqCst);
        for svc in self.list() {
            svc.set_idle(idle);
        }
    }

    /// Why work that reads file bytes of snapshots can't run: the active
    /// snapshot implementation keeps none ("Track changes only"). `None`
    /// when it keeps them. Such work skips with this reason, never errors.
    pub fn no_contents_reason(&self) -> Option<&'static str> {
        (!self.content_policy().keeps()).then_some(NO_CONTENTS_REASON)
    }

    /// Whether takes keep file bytes now.
    pub fn content_policy(&self) -> ContentPolicy {
        *self
            .content_policy
            .read()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// `stream`'s filter, when it has a capture.
    pub fn workspace_filter(&self, stream: &StreamId) -> Option<WorkspaceFilter> {
        self.get(stream).map(|svc| svc.workspace_filter())
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

    /// Mark `id` as the primary stream, for callers about it (the wiki
    /// watcher, the MCP file-ref-version resolver). Must be called after
    /// the corresponding `register` so the lookup actually resolves.
    pub fn set_primary(&self, id: StreamId) {
        *self.primary_id.write().unwrap_or_else(|e| e.into_inner()) = Some(id);
    }

    /// Fetch the primary stream's service. `None` only before boot
    /// has called [`Self::set_primary`].
    pub fn primary(&self) -> Option<Arc<SnapshotCaptureService>> {
        let id = (*self.primary_id.read().unwrap_or_else(|e| e.into_inner()))?;
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
            vcs: std::sync::Arc::new(crate::vcs::GitProvider),
            snapshot_store: Arc::new(SqliteSnapshotStore::new(db)),
            blobs: BlobStore::new(project.join(".oxplow/snapshots")),
            max_file_bytes: 1_000_000,
            generated: Default::default(),
            open_turn_probe: None,
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
            host: oxplow_domain::HostId::LOCAL,
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

    /// Each stream is judged by its own worktree's ignore files, and a
    /// `generated` change reaches every stream and one registered later.
    #[tokio::test]
    async fn each_stream_has_its_own_filter_and_all_follow_generated() {
        let main = tempfile::tempdir().unwrap();
        let tree = tempfile::tempdir().unwrap();
        std::fs::write(main.path().join(".gitignore"), "only-in-main\n").unwrap();
        std::fs::write(tree.path().join(".gitignore"), "only-in-tree\n").unwrap();
        let reg = SnapshotCaptureRegistry::new(config_for(main.path()));
        let of = |svc: &SnapshotCaptureService, path: &str| {
            svc.workspace_filter()
                .ignore(std::path::Path::new(path), false)
        };
        let primary = reg.register(&stream_at(main.path())).unwrap();
        let mut second = stream_at(tree.path());
        second.id = StreamId::new(2);
        let worktree = reg.register(&second).unwrap();
        assert!(of(&primary, "only-in-main"));
        assert!(!of(&primary, "only-in-tree"));
        assert!(of(&worktree, "only-in-tree"));
        assert!(!of(&worktree, "only-in-main"), "not the primary's rules");

        reg.set_generated(&oxplow_config::GeneratedConfig {
            exclude: vec!["out".into()],
            ..Default::default()
        });
        assert!(of(&primary, "out/a.js"));
        assert!(of(&worktree, "out/a.js"));
        let later = tempfile::tempdir().unwrap();
        let mut third = stream_at(later.path());
        third.id = StreamId::new(3);
        assert!(of(&reg.register(&third).unwrap(), "out/a.js"));
    }
}
