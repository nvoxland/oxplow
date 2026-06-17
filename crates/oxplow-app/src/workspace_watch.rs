//! Bridges per-stream filesystem and `.git/refs` watchers onto the
//! shared `EventBus`.
//!
//! Spawned at boot. Iterates the stream list, opens an `FsWatcher`
//! against each worktree (ignored paths — `.git`/`.oxplow` +
//! `.gitignore` + the project's `generated.exclude` — are pruned via
//! the shared [`oxplow_fs_watch::WorkspaceFilter`]) and a `GitRefsWatcher`
//! against `<worktree>/.git/refs`. Translates the per-watcher
//! broadcasts into `OxplowEvent::WorkspaceChanged` /
//! `OxplowEvent::GitRefsChanged` so the renderer's existing
//! subscribers fire.
//!
//! Also watches the project root for `.git` appearing/disappearing
//! and emits `OxplowEvent::WorkspaceContextChanged` so the renderer
//! can flip the git-aware UI without polling.
//!
//! The registry holds the watcher handles for the lifetime of the
//! process; dropping the handle cancels the watcher.

use std::path::{Path, PathBuf};
use std::time::Duration;

use oxplow_domain::{Stream, StreamKind};
use oxplow_fs_watch::{FsWatcher, RecursiveMode, WatchEvent, WatchEventKind};
use oxplow_git::GitRefsWatcher;
use oxplow_session::StreamService;
use tracing::{debug, warn};

use crate::events::{EventBus, OxplowEvent, WorkspaceChangeKind};

/// Holds every per-stream watcher handle. Dropping the registry
/// cancels every watcher in lockstep.
pub struct WorkspaceWatchRegistry {
    _watchers: Vec<StreamWatchers>,
    _project_watcher: Option<FsWatcher>,
}

struct StreamWatchers {
    _fs: FsWatcher,
    _refs: Option<GitRefsWatcher>,
}

impl WorkspaceWatchRegistry {
    /// Boot the registry. Looks up every existing stream, spawns a
    /// watcher pair per stream, and starts the project-root `.git`
    /// presence watcher.
    ///
    /// Streams whose `worktree_path` no longer exists on disk
    /// (externally deleted while oxplow was offline) are auto-archived
    /// here and announced via [`OxplowEvent::StreamOrphaned`] so the
    /// renderer can toast. The primary stream is exempt — it points at
    /// the project root itself and a missing project root is a
    /// different failure mode.
    pub async fn spawn(
        streams: StreamService,
        events: EventBus,
        project_dir: PathBuf,
        filter: oxplow_fs_watch::WorkspaceFilter,
    ) -> Self {
        let stream_rows = streams.list_streams().await.unwrap_or_default();
        let mut watchers = Vec::new();
        for s in stream_rows {
            let worktree = PathBuf::from(&s.worktree_path);
            if !worktree.exists() {
                if matches!(s.kind, StreamKind::Worktree) {
                    auto_archive_orphan(&streams, &events, &s).await;
                }
                continue;
            }
            let on_orphan: OnOrphan = {
                let svc = streams.clone();
                let bus = events.clone();
                let stream_for_cb = s.clone();
                Box::new(move || {
                    let svc = svc.clone();
                    let bus = bus.clone();
                    let stream_for_cb = stream_for_cb.clone();
                    Box::pin(async move { auto_archive_orphan(&svc, &bus, &stream_for_cb).await })
                })
            };
            let is_worktree = matches!(s.kind, StreamKind::Worktree);
            if let Some(w) = spawn_for_stream(
                s.id,
                worktree,
                events.clone(),
                is_worktree,
                on_orphan,
                filter.clone(),
            ) {
                watchers.push(w);
            }
        }
        let project_watcher = spawn_project_context(project_dir, events);
        Self {
            _watchers: watchers,
            _project_watcher: project_watcher,
        }
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
    } else {
        events.emit(OxplowEvent::StreamsChanged);
    }
    events.emit(OxplowEvent::StreamOrphaned {
        stream_id: stream.id,
        title: stream.title.clone(),
    });
}

/// Callback invoked once when the watcher detects the worktree root
/// vanished at runtime. Boxed-future so the caller can run async ops
/// (archive_stream + emit) without forcing the closure to be sync.
type OnOrphan =
    Box<dyn FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send>;

fn spawn_for_stream(
    stream_id: oxplow_domain::StreamId,
    worktree: PathBuf,
    events: EventBus,
    is_worktree: bool,
    on_orphan: OnOrphan,
    filter: oxplow_fs_watch::WorkspaceFilter,
) -> Option<StreamWatchers> {
    if !worktree.exists() {
        debug!(?worktree, %stream_id, "skipping watcher — worktree missing");
        return None;
    }
    let mut paths: Vec<(PathBuf, RecursiveMode)> = Vec::new();
    // Top-level non-recursive watch so new files at the worktree root
    // (and the appearance/disappearance of top-level dirs) still fire.
    paths.push((worktree.clone(), RecursiveMode::NonRecursive));
    match std::fs::read_dir(&worktree) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if !file_type.is_dir() {
                    continue;
                }
                let name = entry.file_name();
                // Never register a recursive watch on an ignored
                // top-level dir (the central filter: `.git`/`.oxplow`
                // + `.gitignore` + the project's `generated.exclude`).
                if filter.ignore(Path::new(&name), true) {
                    continue;
                }
                paths.push((entry.path(), RecursiveMode::Recursive));
            }
        }
        Err(e) => {
            warn!(error = %e, %stream_id, ?worktree, "could not enumerate worktree top-level");
        }
    }
    let fs = match FsWatcher::watch_paths(paths) {
        Ok(w) => w,
        Err(e) => {
            warn!(error = %e, %stream_id, ?worktree, "fs watcher failed to start");
            return None;
        }
    };
    {
        // Debounced: a `git checkout` or save-storm should refresh the
        // file tree / editor once, not once per touched file. (The
        // snapshot dirty set listens to the raw stream separately.)
        let mut rx = fs.subscribe_debounced(Duration::from_millis(250));
        let bus = events.clone();
        let id = stream_id;
        let root = worktree.clone();
        let mut on_orphan_slot = Some(on_orphan);
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(WatchEvent { path, kind }) => {
                        // If the worktree root has vanished (user `rm
                        // -rf`'d it, `git worktree remove` ran, etc.),
                        // archive the stream + toast and exit — the
                        // watcher is dead either way. We check on every
                        // event, not just `Removed`: macOS FSEvents
                        // surfaces a directory's own deletion as an
                        // `Updated` of the parent, so keying on the
                        // event kind is unreliable.
                        if is_worktree && !root.exists() {
                            if let Some(cb) = on_orphan_slot.take() {
                                cb().await;
                            }
                            break;
                        }
                        let rel_path = path.strip_prefix(&root).unwrap_or(&path);
                        if filter.ignore(rel_path, path.is_dir()) || is_swap_file(&path) {
                            continue;
                        }
                        let rel = rel_path.to_string_lossy().into_owned();
                        bus.emit(OxplowEvent::WorkspaceChanged {
                            stream_id: id,
                            change_kind: classify(&kind),
                            path: rel,
                        });
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "workspace watcher lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    let refs = match GitRefsWatcher::watch(worktree.clone(), Duration::from_millis(250)) {
        Ok(w) => Some(w),
        Err(e) => {
            // A non-git worktree won't have `.git/refs/`; that's
            // fine — drop the refs watcher silently.
            debug!(error = %e, %stream_id, "git refs watcher unavailable");
            None
        }
    };
    if let Some(refs_handle) = refs.as_ref() {
        let mut rx = refs_handle.subscribe();
        let bus = events.clone();
        let id = stream_id;
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(_) => bus.emit(OxplowEvent::GitRefsChanged { stream_id: id }),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    Some(StreamWatchers {
        _fs: fs,
        _refs: refs,
    })
}

/// Watch the project root for `.git` appearing or disappearing and
/// emit `WorkspaceContextChanged` so the renderer flips the git-aware
/// UI without polling. Initial state is reported on the first emit;
/// callers also `getWorkspaceContext` for the first paint.
fn spawn_project_context(project_dir: PathBuf, events: EventBus) -> Option<FsWatcher> {
    // Non-recursive: we only care about whether `.git` appears or
    // disappears at the project root. A recursive watch here would
    // re-walk the entire .git tree on boot for nothing.
    let watcher =
        match FsWatcher::watch_paths(vec![(project_dir.clone(), RecursiveMode::NonRecursive)]) {
            Ok(w) => w,
            Err(e) => {
                warn!(error = %e, ?project_dir, "project context watcher failed to start");
                return None;
            }
        };
    // Debounced: `git init`/`rm -rf .git` churns the root briefly; we
    // only need to settle on the final `.git` presence state.
    let mut rx = watcher.subscribe_debounced(Duration::from_millis(500));
    let mut last_state = project_dir.join(".git").exists();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(WatchEvent { path, .. }) => {
                    // Only react to events on `.git` itself.
                    let touched_git = path.file_name().map(|n| n == ".git").unwrap_or(false)
                        || path.components().any(|c| c.as_os_str() == ".git");
                    if !touched_git {
                        continue;
                    }
                    let now = project_dir.join(".git").exists();
                    if now != last_state {
                        last_state = now;
                        events.emit(OxplowEvent::WorkspaceContextChanged { git_enabled: now });
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    Some(watcher)
}

fn classify(k: &WatchEventKind) -> WorkspaceChangeKind {
    match k {
        WatchEventKind::Created => WorkspaceChangeKind::Created,
        WatchEventKind::Modified => WorkspaceChangeKind::Updated,
        WatchEventKind::Removed => WorkspaceChangeKind::Deleted,
        WatchEventKind::Other => WorkspaceChangeKind::Updated,
    }
}

/// Editor swap / temp files the renderer never needs to react to.
/// Path-based ignores (`.git`, `.oxplow`, `.gitignore`, the project's
/// `generated.exclude`) are the central [`oxplow_fs_watch::WorkspaceFilter`]'s
/// job — this only catches the editor-noise suffixes the filter
/// doesn't model.
fn is_swap_file(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.ends_with('~') || s.ends_with(".swp") || s.ends_with(".tmp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::time::timeout;

    #[tokio::test]
    async fn spawn_for_stream_skips_target_and_node_modules() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        for sub in ["target", "node_modules", "src"] {
            std::fs::create_dir(root.join(sub)).unwrap();
        }

        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        let stream_id = oxplow_domain::StreamId::new(1);
        let on_orphan: OnOrphan = Box::new(|| Box::pin(async {}));
        // No .gitignore in the tempdir, so exclude the build dirs via the
        // filter's exclude list (mirrors how a real project gitignores them).
        let filter =
            oxplow_fs_watch::WorkspaceFilter::with_user_entries(["target", "node_modules"]);
        let _watchers = spawn_for_stream(
            stream_id,
            root.clone(),
            bus.clone(),
            false,
            on_orphan,
            filter,
        )
        .expect("watchers");

        // Give notify a moment to settle the cache walk before writing.
        tokio::time::sleep(Duration::from_millis(200)).await;

        std::fs::write(root.join("target").join("ignored.txt"), b"x").unwrap();
        std::fs::write(root.join("node_modules").join("ignored.txt"), b"x").unwrap();
        std::fs::write(root.join("src").join("seen.txt"), b"y").unwrap();

        let mut seen_src = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            match timeout(Duration::from_millis(500), rx.recv()).await {
                Ok(Ok(crate::events::OxplowEvent::WorkspaceChanged { path, .. })) => {
                    if path.contains("target") || path.contains("node_modules") {
                        panic!("unexpected event for excluded path: {path}");
                    }
                    if path.contains("seen.txt") {
                        seen_src = true;
                        // Drain a moment longer to ensure no excluded events sneak in.
                    }
                }
                Ok(Ok(_)) => {}
                _ => {
                    if seen_src {
                        break;
                    }
                }
            }
        }
        assert!(seen_src, "expected WorkspaceChanged event for src/seen.txt");

        // Keep arc alive to avoid drop ordering surprises.
        drop(_watchers);
        let _ = Arc::new(());
    }

    #[tokio::test]
    async fn spawn_archives_streams_with_missing_worktree() {
        use oxplow_db::{Database, SqliteStreamStore, SqliteThreadStore};
        use oxplow_session::{StreamService, WorkspaceLayout};
        use std::process::Command;

        // Real git repo with a real worktree, both as siblings under a
        // shared parent so the StreamService's "<parent>/<basename>-<slug>"
        // layout works.
        let parent = tempdir().unwrap();
        let project = parent.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut opts = git2::RepositoryInitOptions::new();
        opts.initial_head("main");
        let repo = git2::Repository::init_opts(&project, &opts).unwrap();
        // Seed an initial commit so `git worktree add` has a HEAD.
        {
            let sig = git2::Signature::now("Test", "test@example.com").unwrap();
            let tree_id = {
                let mut idx = repo.index().unwrap();
                idx.write_tree().unwrap()
            };
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }
        let layout = WorkspaceLayout::for_project(&project);
        let db = Database::in_memory();
        let stream_store = Arc::new(SqliteStreamStore::new(db.clone()));
        let thread_store = Arc::new(SqliteThreadStore::new(db));
        let svc = StreamService::new(layout, stream_store, thread_store);
        svc.ensure_primary().await.unwrap();
        // Use the real `create_worktree` path so the on-disk dir exists
        // before we delete it.
        let orphan = svc
            .create_worktree("ghost", "Ghost", "ghost-branch", "main")
            .await
            .unwrap();
        // Simulate the user `rm -rf`-ing the worktree directory while
        // oxplow was offline. Don't bother running `git worktree prune`;
        // the watcher only checks `worktree_path.exists()`.
        std::fs::remove_dir_all(&orphan.worktree_path).unwrap();
        // Workspace watcher needs git installed for the project_dir
        // refs watcher; skip that on hosts without git on PATH.
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }

        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        let _registry = WorkspaceWatchRegistry::spawn(
            svc.clone(),
            bus.clone(),
            project.clone(),
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .await;

        // Drain events until we see StreamOrphaned for the right id.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut saw_orphan = false;
        while std::time::Instant::now() < deadline {
            match timeout(Duration::from_millis(250), rx.recv()).await {
                Ok(Ok(OxplowEvent::StreamOrphaned { stream_id, title })) => {
                    assert_eq!(stream_id, orphan.id);
                    assert_eq!(title, "Ghost");
                    saw_orphan = true;
                    break;
                }
                Ok(Ok(_)) => continue,
                _ => break,
            }
        }
        assert!(
            saw_orphan,
            "expected StreamOrphaned event for missing worktree"
        );

        // And the row is now archived in the store.
        let surviving = svc.list_streams().await.unwrap();
        assert!(
            surviving.iter().all(|s| s.id != orphan.id),
            "orphaned stream should be archived",
        );
    }

    #[tokio::test]
    async fn runtime_worktree_deletion_triggers_orphan() {
        use oxplow_db::{Database, SqliteStreamStore, SqliteThreadStore};
        use oxplow_session::{StreamService, WorkspaceLayout};
        use std::process::Command;

        if Command::new("git").arg("--version").output().is_err() {
            return;
        }

        let parent = tempdir().unwrap();
        let project = parent.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut opts = git2::RepositoryInitOptions::new();
        opts.initial_head("main");
        let repo = git2::Repository::init_opts(&project, &opts).unwrap();
        {
            let sig = git2::Signature::now("Test", "test@example.com").unwrap();
            let tree_id = {
                let mut idx = repo.index().unwrap();
                idx.write_tree().unwrap()
            };
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }
        let layout = WorkspaceLayout::for_project(&project);
        let db = Database::in_memory();
        let stream_store = Arc::new(SqliteStreamStore::new(db.clone()));
        let thread_store = Arc::new(SqliteThreadStore::new(db));
        let svc = StreamService::new(layout, stream_store, thread_store);
        svc.ensure_primary().await.unwrap();
        let stream = svc
            .create_worktree("ghost-rt", "GhostRT", "ghost-rt", "main")
            .await
            .unwrap();
        // Seed a real file inside the worktree so the recursive
        // sub-watchers have content to fire Removed events on when
        // we wipe the dir. (`git worktree add` from an empty initial
        // commit yields a near-empty dir, and platform watchers can
        // miss the root-self deletion.)
        let seeded_dir = std::path::Path::new(&stream.worktree_path).join("seed");
        std::fs::create_dir(&seeded_dir).unwrap();
        std::fs::write(seeded_dir.join("hello.txt"), b"hi").unwrap();

        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        let _registry = WorkspaceWatchRegistry::spawn(
            svc.clone(),
            bus.clone(),
            project.clone(),
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .await;

        // Let notify settle its initial cache walk before the delete.
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Now delete the worktree dir while the watcher is live.
        std::fs::remove_dir_all(&stream.worktree_path).unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut saw_orphan = false;
        while std::time::Instant::now() < deadline {
            match timeout(Duration::from_millis(300), rx.recv()).await {
                Ok(Ok(OxplowEvent::StreamOrphaned { stream_id, .. })) => {
                    if stream_id == stream.id {
                        saw_orphan = true;
                        break;
                    }
                }
                Ok(Ok(_)) => continue,
                _ => continue,
            }
        }
        assert!(
            saw_orphan,
            "expected StreamOrphaned event after runtime worktree deletion",
        );

        let surviving = svc.list_streams().await.unwrap();
        assert!(surviving.iter().all(|s| s.id != stream.id));
    }
}
