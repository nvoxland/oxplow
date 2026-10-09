//! The snapshot capability's implementations core ships
//! (`.context/work-tracking.md` "Capabilities", `.context/data-model.md`
//! "Content policy"): [`CoreSnapshots`] is the [`SnapshotProvider`] for
//! both built-ins — `oxplow:snapshots` ("Keep every version", `contents`)
//! and `oxplow:snapshot-hashes` ("Track changes only") — the same capture
//! pipeline (`SnapshotCaptureService`, one per stream) under a
//! [`ContentPolicy`]: the first keeps every file's bytes, the second
//! records identities and keeps none. Both write core's record (`snapshot`,
//! `file_snapshot`, `snapshot_op`), so what changed reads the same either
//! way.
//!
//! The active implementation's `contents` sets the capture policy of
//! every stream's service: at each mark, and on each `capability.switched`
//! ([`SnapshotSwitch`]), which covers the service's own quiet and
//! git-refs takes.

use std::sync::Arc;

use async_trait::async_trait;
use oxplow_domain::events::schema::{CapabilitySwitched, EventType as _};
use oxplow_domain::snapshot::{
    MarkRequest, Marked, SnapshotError, SnapshotProvider, SnapshotRegistry,
};
use oxplow_domain::tree_diff::FileChange;
use oxplow_domain::{DomainError, StreamId};

use crate::capabilities::{Implementation, Source};
use crate::snapshot_capture::{ContentPolicy, TakeRequest};
use crate::snapshot_capture_registry::SnapshotCaptureRegistry;
use crate::snapshot_files::{SnapshotFileError, SnapshotFiles};

/// The capability.
pub const CAPABILITY: &str = "snapshots";
/// The built-in that keeps every version.
pub const KEEP: &str = "oxplow:snapshots";
/// The built-in that tracks changes only.
pub const HASHES: &str = "oxplow:snapshot-hashes";

/// Both built-ins: the capture pipeline under a content policy.
pub struct CoreSnapshots {
    pub id: String,
    pub policy: ContentPolicy,
    pub captures: SnapshotCaptureRegistry,
    pub files: SnapshotFiles,
}

#[async_trait]
impl SnapshotProvider for CoreSnapshots {
    fn id(&self) -> &str {
        &self.id
    }

    fn contents(&self) -> bool {
        self.policy.keeps()
    }

    async fn mark(&self, request: &MarkRequest) -> Result<Option<Marked>, SnapshotError> {
        // A stream with no capture service (no worktree on this machine)
        // has nothing to record.
        let Some(capture) = self.captures.get(&request.stream) else {
            return Ok(None);
        };
        capture.set_content_policy(self.policy);
        let outcome = capture
            .request_take(TakeRequest {
                trigger: request.trigger,
                thread_id: request.thread,
                turn_id: request.turn,
                effort_id: request.effort,
                budget: request.budget,
                provider: Some(self.id.clone()),
            })
            .await
            .map_err(|e| SnapshotError::Storage(e.to_string()))?;
        Ok(outcome.map(|o| Marked {
            snapshot: o.snapshot_id,
            parent: o.parent_snapshot_id,
            unchanged: o.unchanged,
            file_count: u64::from(o.file_count),
            over_budget: o.over_budget,
        }))
    }

    async fn changed(
        &self,
        _stream: StreamId,
        from: Option<i64>,
        to: i64,
    ) -> Result<Vec<FileChange>, SnapshotError> {
        self.files
            .snapshots
            .diff_snapshots(from, to)
            .await
            .map_err(|e| SnapshotError::Storage(e.to_string()))
    }

    async fn read_at(&self, snapshot: i64, path: &str) -> Result<Vec<u8>, SnapshotError> {
        match self.files.read_file_at_snapshot(snapshot, path).await {
            Ok(Some(bytes)) => Ok(bytes),
            Ok(None) => Err(SnapshotError::NotFound(format!(
                "{path} at snapshot {snapshot}"
            ))),
            Err(e) => Err(snapshot_error(e)),
        }
    }
}

/// A file read's failure, as the interface says it.
pub fn snapshot_error(e: SnapshotFileError) -> SnapshotError {
    match e {
        SnapshotFileError::NoContents { snapshot, provider } => {
            SnapshotError::NoContents { snapshot, provider }
        }
        SnapshotFileError::Expired => SnapshotError::Expired,
        SnapshotFileError::NotFound => SnapshotError::NotFound("the snapshot".into()),
        other => SnapshotError::Storage(other.to_string()),
    }
}

/// The policy a built-in `entry` captures under; `None` for an entry that
/// isn't one of the snapshot built-ins.
fn policy_of(entry: &str) -> Option<ContentPolicy> {
    match entry {
        KEEP => Some(ContentPolicy::Keep),
        HASHES => Some(ContentPolicy::HashesOnly),
        _ => None,
    }
}

/// Register the snapshot built-ins `declared` names, and core's own
/// default (`oxplow`, which keeps every version: it is always there, a
/// required capability's fallback).
pub fn register_built_ins(
    registry: &SnapshotRegistry,
    declared: &[Implementation],
    captures: &SnapshotCaptureRegistry,
    files: &SnapshotFiles,
) {
    let core_default = oxplow_domain::capability::spec(CAPABILITY)
        .map(|s| s.default)
        .unwrap_or_default();
    let built = |id: &str, policy: ContentPolicy| -> Arc<dyn SnapshotProvider> {
        Arc::new(CoreSnapshots {
            id: id.to_string(),
            policy,
            captures: captures.clone(),
            files: files.clone(),
        })
    };
    let mut all = vec![built(core_default, ContentPolicy::Keep)];
    all.extend(
        declared
            .iter()
            .filter(|i| i.capability == CAPABILITY)
            .filter_map(|i| match i.source {
                Source::BuiltIn(entry) => policy_of(entry).map(|p| built(&i.id, p)),
                _ => None,
            }),
    );
    captures.set_serves(all.iter().map(|p| p.id().to_string()));
    registry.set_declared(all);
}

/// Set every stream's capture policy to what the active implementation
/// keeps, and idle the captures while it's one they don't serve (a
/// provider process, which marks the worktree itself).
pub fn apply_active_policy(registry: &SnapshotRegistry, captures: &SnapshotCaptureRegistry) {
    let active = registry.active();
    let policy = match &active {
        Some(p) if !p.contents() => ContentPolicy::HashesOnly,
        _ => ContentPolicy::Keep,
    };
    captures.set_content_policy(policy);
    let id = active.map(|p| p.id().to_string());
    captures.follow(id.as_deref());
    captures.set_provider(id);
}

/// On `capability.switched` for snapshots: the captures follow the newly
/// active implementation's policy. Nothing already taken changes.
pub struct SnapshotSwitch {
    pub registry: Arc<SnapshotRegistry>,
    pub captures: SnapshotCaptureRegistry,
}

#[async_trait]
impl crate::event_pump::AsyncEventConsumer for SnapshotSwitch {
    fn name(&self) -> &'static str {
        "snapshots.switch"
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == CapabilitySwitched::TYPE
    }

    async fn handle(&self, event: &oxplow_domain::StoredEvent) -> Result<(), DomainError> {
        if event.envelope.payload["capability"] == CAPABILITY {
            apply_active_policy(&self.registry, &self.captures);
        }
        Ok(())
    }
}

/// A registry over `captures` with core's default (every version) active:
/// what a test that builds its own captures marks through.
#[cfg(test)]
pub(crate) fn registry_over(
    captures: &SnapshotCaptureRegistry,
    store: Arc<oxplow_db::SqliteSnapshotStore>,
    db: &oxplow_db::Database,
    blobs: crate::blob_store::BlobStore,
    project_dir: &std::path::Path,
) -> Arc<SnapshotRegistry> {
    let files = SnapshotFiles {
        snapshots: store,
        streams: Arc::new(oxplow_db::SqliteStreamStore::new(db.clone())),
        content: crate::snapshot_content::SnapshotContent::new(
            blobs,
            oxplow_domain::vcs::Vcs::object_store(&crate::vcs::GitProvider, project_dir),
        ),
        project_dir: project_dir.to_path_buf(),
    };
    let default = oxplow_domain::capability::spec(CAPABILITY)
        .map(|s| s.default.to_string())
        .unwrap_or_default();
    let registry = Arc::new(SnapshotRegistry::new(Arc::new(move || default.clone())));
    register_built_ins(&registry, &[], captures, &files);
    registry
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use oxplow_domain::snapshot::{MarkRequest, SnapshotError, SnapshotTrigger};
    use oxplow_fs_watch::WatchEventKind;

    use super::*;
    use crate::snapshot_capture::SnapshotCaptureService;
    use crate::test_fixtures::{services_with_effort, EffortFixture};

    /// The fixture's stream with a capture service whose gates are off, so
    /// a mark sees what was just written.
    fn stream_of(f: &EffortFixture) -> (StreamId, Arc<SnapshotCaptureService>) {
        let stream = StreamId::new(1);
        let svc = Arc::new(
            SnapshotCaptureService::new(
                f.svc.snapshot_store.clone(),
                f.svc.blobs.clone(),
                f.svc.layout.project_dir.clone(),
                Arc::new(crate::vcs::GitProvider),
                stream,
                1_000_000,
                oxplow_fs_watch::WorkspaceFilter::default(),
            )
            .with_settle_duration(Duration::ZERO)
            .with_predrain_delay(Duration::ZERO),
        );
        f.svc.snapshot_captures.insert_for_test(stream, svc.clone());
        (stream, svc)
    }

    fn write(f: &EffortFixture, svc: &SnapshotCaptureService, path: &str, body: &str) {
        let full = f.svc.layout.project_dir.join(path);
        std::fs::write(&full, body).unwrap();
        svc.mark_dirty(full, WatchEventKind::Other);
    }

    /// `id` as the project's snapshot implementation, its switch applied.
    async fn choose(f: &EffortFixture, id: &str) {
        f.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert(CAPABILITY.into(), id.into());
        let config = crate::config_service::read_config(&f.svc.config);
        f.svc
            .capabilities
            .publish(&config, &f.svc.db)
            .await
            .unwrap();
        f.svc
            .event_pump
            .settle(&["snapshots.switch"], Duration::from_secs(10))
            .await;
    }

    async fn mark(f: &EffortFixture, stream: StreamId) -> i64 {
        f.svc
            .snapshots
            .active()
            .expect("an active implementation")
            .mark(&MarkRequest::new(stream, SnapshotTrigger::Manual))
            .await
            .unwrap()
            .expect("a snapshot")
            .snapshot
    }

    /// An implementation the capture pipeline doesn't serve (a provider
    /// process) idles the pipeline while it's active; choosing one it
    /// serves again resumes it. Decided by what the pipeline serves, never
    /// by core's id.
    #[tokio::test]
    async fn an_implementation_the_pipeline_doesnt_serve_idles_it() {
        struct Elsewhere;
        #[async_trait]
        impl SnapshotProvider for Elsewhere {
            fn id(&self) -> &str {
                "elsewhere"
            }
            fn contents(&self) -> bool {
                false
            }
            async fn mark(&self, _: &MarkRequest) -> Result<Option<Marked>, SnapshotError> {
                Ok(None)
            }
            async fn changed(
                &self,
                _: StreamId,
                _: Option<i64>,
                _: i64,
            ) -> Result<Vec<FileChange>, SnapshotError> {
                Ok(Vec::new())
            }
            async fn read_at(&self, _: i64, _: &str) -> Result<Vec<u8>, SnapshotError> {
                Err(SnapshotError::NotFound("nothing".into()))
            }
        }
        let f = services_with_effort().await;
        let (_, svc) = stream_of(&f);
        let chosen = Arc::new(std::sync::Mutex::new("elsewhere".to_string()));
        let registry = SnapshotRegistry::new({
            let chosen = chosen.clone();
            Arc::new(move || chosen.lock().unwrap().clone())
        });
        register_built_ins(
            &registry,
            &[],
            &f.svc.snapshot_captures,
            &f.svc.snapshot_files(),
        );
        registry.register(Arc::new(Elsewhere));

        apply_active_policy(&registry, &f.svc.snapshot_captures);
        assert!(svc.is_idle());
        assert_eq!(
            f.svc.snapshot_captures.no_contents_reason(),
            Some(crate::snapshot_capture_registry::NO_CONTENTS_REASON)
        );

        *chosen.lock().unwrap() = "oxplow".into();
        apply_active_policy(&registry, &f.svc.snapshot_captures);
        assert!(!svc.is_idle());
        assert_eq!(f.svc.snapshot_captures.no_contents_reason(), None);
    }

    /// "Track changes only" records what changed and keeps no bytes; its
    /// takes say who took them; choosing "Keep every version" again keeps
    /// the next take's bytes. Both are offered, core's own always.
    #[tokio::test]
    async fn tracking_changes_only_keeps_no_bytes_until_switched_back() {
        let f = services_with_effort().await;
        let ids = f.svc.snapshots.ids();
        assert!(
            ids.contains(&"oxplow".to_string()) && ids.contains(&"hashes".to_string()),
            "{ids:?}"
        );
        let (stream, svc) = stream_of(&f);

        choose(&f, "hashes").await;
        write(&f, &svc, "a.txt", "alpha");
        let s1 = mark(&f, stream).await;
        let active = f.svc.snapshots.active().unwrap();
        assert!(!active.contents());
        assert!(matches!(
            active.read_at(s1, "a.txt").await,
            Err(SnapshotError::NoContents { .. })
        ));
        let rows = f
            .svc
            .snapshot_store
            .list_files_for_snapshot(s1)
            .await
            .unwrap();
        let row = rows
            .iter()
            .find(|r| r.path == "a.txt")
            .expect("a.txt recorded");
        let hash = row.blob_hash.clone().expect("its identity");
        assert!(!f.svc.blobs.has(&hash), "no bytes kept");
        let changed = active.changed(stream, None, s1).await.unwrap();
        assert!(changed.iter().any(|c| c.path == "a.txt"));
        let ops = f.svc.snapshot_store.list_ops(stream, 10).await.unwrap();
        let op = ops.iter().find(|o| o.snapshot_id == s1).unwrap();
        assert_eq!(
            (op.provider.as_deref(), op.contents),
            (Some("hashes"), false)
        );

        choose(&f, "oxplow").await;
        write(&f, &svc, "b.txt", "beta");
        let s2 = mark(&f, stream).await;
        let active = f.svc.snapshots.active().unwrap();
        assert!(active.contents());
        assert_eq!(active.read_at(s2, "b.txt").await.unwrap(), b"beta");
        let ops = f.svc.snapshot_store.list_ops(stream, 10).await.unwrap();
        let op = ops.iter().find(|o| o.snapshot_id == s2).unwrap();
        assert_eq!(
            (op.provider.as_deref(), op.contents),
            (Some("oxplow"), true)
        );
    }
}
