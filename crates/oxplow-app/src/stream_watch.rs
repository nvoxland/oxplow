//! Each stream's one file watch, which starts and stops with the stream.
//!
//! [`StreamWatchRegistry::register`] opens one `FsWatcher` on the stream's
//! worktree, judging by the stream's one filter, and hangs off it everything
//! that follows that worktree: the snapshot capture's dirty set, the
//! announcements on the `EventBus` (`OxplowEvent::WorkspaceChanged`), and the
//! refs watch through the VCS (`Vcs::watch_refs`, onto `RefMoves`). What follows
//! a few paths of the primary worktree (the wiki, the config, version control
//! appearing) subscribes to its watch ([`StreamWatchRegistry::subscribe_primary`]).
//!
//! Dropping a stream's entry ([`StreamWatchRegistry::unregister`]) ends its
//! watch and everything hung off it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use oxplow_domain::vcs::Vcs;
use oxplow_domain::{Stream, StreamId, StreamKind};
use oxplow_fs_watch::{Batch, FsWatcher, Scope, Subscription};
use oxplow_session::StreamService;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::events::{EventBus, OxplowEvent};
use crate::snapshot_capture_registry::SnapshotCaptureRegistry;

/// How long a burst of file changes collects before it is announced.
const ANNOUNCE_AFTER: Duration = Duration::from_millis(250);
/// How many paths one announcement names: past this, it says so, and an open
/// editor reads everything it holds.
const MAX_ANNOUNCED_PATHS: usize = 200;

/// Every live stream's watch.
#[derive(Clone)]
pub struct StreamWatchRegistry {
    inner: Arc<Inner>,
}

struct Inner {
    watches: Mutex<HashMap<StreamId, StreamWatch>>,
    primary: Mutex<Option<StreamId>>,
    streams: StreamService,
    captures: SnapshotCaptureRegistry,
    vcs: Arc<dyn Vcs>,
    events: EventBus,
    ref_moves: crate::ref_moves::RefMoves,
}

/// One stream's watch and what hangs off it. Dropping it ends them all.
struct StreamWatch {
    fs: Arc<FsWatcher>,
    _refs: Option<Box<dyn Send>>,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for StreamWatch {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl StreamWatchRegistry {
    pub fn new(
        streams: StreamService,
        captures: SnapshotCaptureRegistry,
        vcs: Arc<dyn Vcs>,
        events: EventBus,
        ref_moves: crate::ref_moves::RefMoves,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                watches: Mutex::default(),
                primary: Mutex::default(),
                streams,
                captures,
                vcs,
                events,
                ref_moves,
            }),
        }
    }

    /// Watch every existing stream. One whose worktree no longer exists
    /// (externally deleted while oxplow was offline) is auto-archived here and
    /// announced via [`OxplowEvent::StreamOrphaned`] so the renderer can toast.
    /// The primary stream is exempt — it points at the project root itself and
    /// a missing project root is a different failure mode.
    pub async fn register_all(&self) {
        let streams = self.inner.streams.list_streams().await.unwrap_or_default();
        for stream in streams {
            if !Path::new(&stream.worktree_path).exists() {
                if matches!(stream.kind, StreamKind::Worktree) {
                    auto_archive_orphan(&self.inner.streams, &self.inner.events, &stream).await;
                }
                continue;
            }
            self.register(&stream);
        }
    }

    /// Start `stream`'s watch, its capture with it. Idempotent. Nothing
    /// starts for a worktree that isn't on disk, and a watch that can't start
    /// is logged: the stream still works, without following its files.
    pub fn register(&self, stream: &Stream) {
        let inner = &self.inner;
        let mut watches = inner.watches.lock().unwrap_or_else(|e| e.into_inner());
        if watches.contains_key(&stream.id) {
            return;
        }
        let Some(capture) = inner.captures.register(stream) else {
            debug!(stream = %stream.id, "no watch: worktree missing");
            return;
        };
        let worktree = PathBuf::from(&stream.worktree_path);
        // The way this OS watches a tree, following new directories.
        let fs = match FsWatcher::watch_workspace(&worktree, capture.workspace_filter()) {
            Ok(fs) => Arc::new(fs),
            Err(e) => {
                warn!(error = %e, stream = %stream.id, ?worktree, "fs watcher failed to start");
                return;
            }
        };
        let refs = {
            let ref_moves = inner.ref_moves.clone();
            let id = stream.id;
            let on_change = Box::new(move || ref_moves.moved(id));
            match inner.vcs.watch_refs(&worktree, on_change) {
                Ok(guard) => Some(guard),
                Err(e) => {
                    // A workspace outside version control has no refs to
                    // watch; that's fine.
                    debug!(error = %e, stream = %stream.id, "refs watcher unavailable");
                    None
                }
            }
        };
        let tasks = vec![
            capture.spawn_watcher(fs.clone()),
            capture.spawn_git_refs_listener(&inner.ref_moves),
            self.spawn_announcer(stream.clone(), fs.subscribe()),
        ];
        if matches!(stream.kind, StreamKind::Primary) {
            *inner.primary.lock().unwrap_or_else(|e| e.into_inner()) = Some(stream.id);
        }
        watches.insert(
            stream.id,
            StreamWatch {
                fs,
                _refs: refs,
                tasks,
            },
        );
    }

    /// End `stream`'s watch and everything hung off it.
    pub fn unregister(&self, stream: &StreamId) {
        let removed = self
            .inner
            .watches
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(stream);
        drop(removed);
    }

    /// What `scope` names of the primary worktree. Nothing ever comes if it
    /// isn't watched.
    pub fn subscribe_primary(&self, scope: Scope) -> Subscription {
        let primary = *self.inner.primary.lock().unwrap_or_else(|e| e.into_inner());
        let watches = self.inner.watches.lock().unwrap_or_else(|e| e.into_inner());
        match primary.and_then(|id| watches.get(&id)) {
            Some(watch) => watch.fs.subscribe_to(scope),
            None => Subscription::idle(),
        }
    }

    /// Announce each batch until the watch ends, or the worktree has gone. If
    /// the worktree root has vanished (user `rm -rf`'d it, `git worktree
    /// remove` ran, etc.), archive the stream + toast — the watch is dead
    /// either way. We check on every batch, not just a removal: macOS FSEvents
    /// surfaces a directory's own deletion as an `Updated` of the parent, so
    /// keying on the event kind is unreliable.
    fn spawn_announcer(&self, stream: Stream, mut changes: Subscription) -> JoinHandle<()> {
        let this = self.clone();
        let root = PathBuf::from(&stream.worktree_path);
        tokio::spawn(async move {
            let inner = &this.inner;
            while let Some(batch) = changes.next_batch_after(ANNOUNCE_AFTER).await {
                if matches!(stream.kind, StreamKind::Worktree) && !root.exists() {
                    auto_archive_orphan(&inner.streams, &inner.events, &stream).await;
                    inner.captures.unregister(&stream.id);
                    this.unregister(&stream.id);
                    return;
                }
                announce(&inner.events, &inner.ref_moves, stream.id, &root, batch);
            }
        })
    }
}

/// Archive a stream whose worktree directory has been deleted out
/// from under us. Best-effort: archive failures are logged and the
/// orphan event is still emitted (so the user at least sees the
/// notification) — the alternative is silently leaving a dead row in
/// the rail.
async fn auto_archive_orphan(streams: &StreamService, events: &EventBus, stream: &Stream) {
    warn!(
        stream_id = %stream.id,
        title = %stream.title,
        worktree = %stream.worktree_path,
        "stream worktree missing on disk; auto-archiving",
    );
    if let Err(e) = streams.archive_stream(&stream.id, false).await {
        warn!(error = %e, stream_id = %stream.id, "failed to archive orphaned stream");
    }
    events.emit(OxplowEvent::StreamOrphaned {
        stream_id: stream.id,
        title: stream.title.clone(),
    });
}

/// What the watch heard (judged by the stream's filter), as one announcement.
/// The working tree's file list follows the edit itself, not the next snapshot
/// take. Changes it lost, or more paths than one announcement names, say so:
/// everything is read again.
fn announce(
    events: &EventBus,
    ref_moves: &crate::ref_moves::RefMoves,
    stream_id: StreamId,
    root: &Path,
    batch: Batch,
) {
    let mut paths: Vec<String> = batch
        .paths
        .iter()
        .map(|(path, _)| path)
        .filter(|path| !is_swap_file(path))
        .map(|path| {
            path.strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    if paths.is_empty() && batch.rescans.is_empty() {
        return;
    }
    let overflow = !batch.rescans.is_empty() || paths.len() > MAX_ANNOUNCED_PATHS;
    if overflow {
        paths.clear();
    }
    ref_moves.index_moved(stream_id);
    events.emit(OxplowEvent::WorkspaceChanged {
        stream_id,
        paths,
        overflow,
    });
}

/// Follow the project root coming under (or leaving) version control, and emit
/// `WorkspaceContextChanged` so the renderer flips its VCS-aware UI without
/// polling. The VCS says what counts (`Vcs::detect`), re-asked after each
/// settled change at the root; callers also `getWorkspaceContext` for the
/// first paint.
pub fn follow_project_context(
    mut changes: Subscription,
    project_dir: PathBuf,
    vcs: Arc<dyn Vcs>,
    events: EventBus,
) {
    tokio::spawn(async move {
        let mut last_state = vcs.detect(&project_dir).await.is_some();
        // An init or a removal churns the root briefly; we only need the
        // settled state.
        while changes
            .next_batch_after(Duration::from_millis(500))
            .await
            .is_some()
        {
            let now = vcs.detect(&project_dir).await.is_some();
            if now != last_state {
                last_state = now;
                events.emit(OxplowEvent::WorkspaceContextChanged { vcs_enabled: now });
            }
        }
    });
}

/// Editor swap / temp files the renderer never needs to react to.
/// Path-based ignores (`.git`, `.oxplow`, `.gitignore`, the project's
/// `generated.exclude`) are the central [`oxplow_fs_watch::WorkspaceFilter`]'s
/// job — this only catches the editor-noise suffixes the filter
/// doesn't dataset.
fn is_swap_file(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.ends_with('~') || s.ends_with(".swp") || s.ends_with(".tmp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteStreamStore, SqliteThreadStore};
    use oxplow_fs_watch::{Feed, WatchEventKind};
    use oxplow_session::WorkspaceLayout;
    use tempfile::{tempdir, TempDir};
    use tokio::time::timeout;

    /// A real repo with the registry over it. Nothing here starts a watch
    /// unless a test registers a stream.
    struct Rig {
        _parent: TempDir,
        project: PathBuf,
        svc: StreamService,
        registry: StreamWatchRegistry,
        bus: EventBus,
    }

    async fn rig() -> Rig {
        // A real git repo with a real worktree, both as siblings under a shared
        // parent so the StreamService's "<parent>/<basename>-<slug>" layout works.
        let parent = tempdir().unwrap();
        let project = parent.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut opts = git2::RepositoryInitOptions::new();
        opts.initial_head("main");
        let repo = git2::Repository::init_opts(&project, &opts).unwrap();
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        let db = Database::in_memory();
        let svc = StreamService::new(
            WorkspaceLayout::for_project(&project),
            Arc::new(crate::vcs::GitProvider),
            Arc::new(SqliteStreamStore::new(db.clone())),
            Arc::new(SqliteThreadStore::new(db.clone())),
        );
        svc.ensure_primary().await.unwrap();
        let captures = SnapshotCaptureRegistry::new(
            crate::snapshot_capture_registry::SnapshotCaptureRegistryConfig {
                vcs: Arc::new(crate::vcs::GitProvider),
                snapshot_store: Arc::new(oxplow_db::SqliteSnapshotStore::new(db)),
                blobs: crate::blob_store::BlobStore::new(project.join(".oxplow/snapshots")),
                max_file_bytes: 1_000_000,
                generated: Default::default(),
                open_turn_probe: None,
            },
        );
        let bus = EventBus::new();
        let registry = StreamWatchRegistry::new(
            svc.clone(),
            captures,
            Arc::new(crate::vcs::GitProvider),
            bus.clone(),
            crate::ref_moves::RefMoves::new(bus.clone()),
        );
        Rig {
            _parent: parent,
            project,
            svc,
            registry,
            bus,
        }
    }

    impl Rig {
        fn watched(&self, stream: &StreamId) -> bool {
            self.registry
                .inner
                .watches
                .lock()
                .unwrap()
                .contains_key(stream)
        }
    }

    /// A stream made after boot is followed like one that was there, and an
    /// archived one stops: its watch and what hangs off it end.
    #[tokio::test]
    async fn a_watch_starts_and_stops_with_its_stream() {
        let rig = rig().await;
        rig.registry.register_all().await;
        let primary = rig.svc.list_streams().await.unwrap()[0].clone();
        assert!(rig.watched(&primary.id));

        let later = rig
            .svc
            .create_worktree("later", "Later", "later", "main")
            .await
            .unwrap();
        assert!(!rig.watched(&later.id));
        rig.registry.register(&later);
        rig.registry.register(&later);
        assert!(rig.watched(&later.id), "made after boot");
        let tasks: Vec<_> = rig
            .registry
            .inner
            .watches
            .lock()
            .unwrap()
            .get(&later.id)
            .unwrap()
            .tasks
            .iter()
            .map(JoinHandle::abort_handle)
            .collect();
        rig.registry.unregister(&later.id);
        assert!(!rig.watched(&later.id));
        for task in tasks {
            timeout(Duration::from_secs(5), async {
                while !task.is_finished() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("what hung off the watch ended with it");
        }
    }

    /// A stream whose worktree went while oxplow was off is archived at boot.
    #[tokio::test]
    async fn a_worktree_gone_at_boot_archives_its_stream() {
        let rig = rig().await;
        let orphan = rig
            .svc
            .create_worktree("ghost", "Ghost", "ghost-branch", "main")
            .await
            .unwrap();
        std::fs::remove_dir_all(&orphan.worktree_path).unwrap();
        let mut rx = rig.bus.subscribe_ui();
        rig.registry.register_all().await;
        let mut saw = false;
        while let Ok(event) = rx.try_recv() {
            if let OxplowEvent::StreamOrphaned { stream_id, title } = event {
                assert_eq!((stream_id, title.as_str()), (orphan.id, "Ghost"));
                saw = true;
            }
        }
        assert!(saw, "expected StreamOrphaned");
        let left = rig.svc.list_streams().await.unwrap();
        assert!(left.iter().all(|s| s.id != orphan.id));
        assert!(!rig.watched(&orphan.id));
        assert!(rig.project.exists());
    }

    /// A worktree deleted while watched archives its stream, on whatever
    /// event comes next, and the watch ends.
    #[tokio::test]
    async fn a_worktree_deleted_while_watched_archives_its_stream() {
        let rig = rig().await;
        let stream = rig
            .svc
            .create_worktree("ghost-rt", "GhostRT", "ghost-rt", "main")
            .await
            .unwrap();
        let mut rx = rig.bus.subscribe_ui();
        let feed = Feed::default();
        let announcer = rig
            .registry
            .spawn_announcer(stream.clone(), feed.subscribe());
        std::fs::remove_dir_all(&stream.worktree_path).unwrap();
        feed.change(
            &Path::new(&stream.worktree_path).join("gone.txt"),
            &WatchEventKind::Removed,
        );
        timeout(Duration::from_secs(5), announcer)
            .await
            .expect("the announcer ends")
            .unwrap();
        let mut saw = false;
        while let Ok(event) = rx.try_recv() {
            saw |= matches!(event, OxplowEvent::StreamOrphaned { stream_id, .. } if stream_id == stream.id);
        }
        assert!(saw, "expected StreamOrphaned");
    }

    fn modified(path: &str) -> (PathBuf, WatchEventKind) {
        (PathBuf::from(path), WatchEventKind::Modified)
    }

    /// A burst is one announcement and one signal to the working-files
    /// listener, however many paths it holds; swap files alone say nothing;
    /// more paths than it names, or lost changes, say the list isn't whole.
    #[tokio::test]
    async fn a_batch_is_one_announcement() {
        let bus = EventBus::new();
        let mut ui = bus.subscribe_ui();
        let ref_moves = crate::ref_moves::RefMoves::new(bus.clone());
        let mut index = ref_moves.subscribe_index();
        let stream = StreamId::new(4);
        let root = Path::new("/r");
        let mut said = |batch: Batch| {
            announce(&bus, &ref_moves, stream, root, batch);
            let mut out = Vec::new();
            while let Ok(event) = ui.try_recv() {
                if let OxplowEvent::WorkspaceChanged {
                    paths, overflow, ..
                } = event
                {
                    out.push((paths, overflow));
                }
            }
            (out, index.pending().len())
        };

        let swap = Batch {
            paths: vec![modified("/r/a.swp")],
            rescans: Vec::new(),
        };
        assert_eq!(said(swap), (vec![], 0), "a swap file says nothing");

        let two = Batch {
            paths: vec![modified("/r/src/a.rs"), modified("/r/a.swp")],
            rescans: Vec::new(),
        };
        assert_eq!(said(two), (vec![(vec!["src/a.rs".to_string()], false)], 1));

        let burst = Batch {
            paths: (0..=MAX_ANNOUNCED_PATHS)
                .map(|i| modified(&format!("/r/{i}")))
                .collect(),
            rescans: Vec::new(),
        };
        assert_eq!(said(burst), (vec![(vec![], true)], 1), "one, however many");

        let lost = Batch {
            paths: Vec::new(),
            rescans: vec![PathBuf::from("/r/src")],
        };
        assert_eq!(said(lost), (vec![(vec![], true)], 1));
    }

    /// The one test here that needs the OS: an edit made outside oxplow reaches
    /// the working-tree file list at once, through a real watch.
    #[tokio::test]
    async fn an_edit_on_disk_tells_the_working_files_listener() {
        let rig = rig().await;
        let mut index = rig.registry.inner.ref_moves.subscribe_index();
        rig.registry.register_all().await;
        let primary = rig.svc.list_streams().await.unwrap()[0].id;
        std::fs::create_dir(rig.project.join("src")).unwrap();
        let mut heard = None;
        for n in 0..240 {
            std::fs::write(rig.project.join("src").join("edit.txt"), n.to_string()).unwrap();
            if let Ok(moved) = timeout(Duration::from_millis(500), index.recv()).await {
                heard = moved;
                break;
            }
        }
        assert_eq!(heard, Some(crate::ref_moves::Moved::Stream(primary)));
    }
}
