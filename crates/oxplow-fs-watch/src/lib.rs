//! Filesystem watcher.
//!
//! Wraps `notify::RecommendedWatcher` and exposes a single broadcast
//! channel of `WatchEvent`s. Raw events fire **immediately** — there is
//! no built-in debounce. Reused by oxplow-git for `.git/refs` watching,
//! and by the snapshot-capture and analysis pipelines.
//!
//! Two ways to consume:
//!   - [`FsWatcher::subscribe`] — the raw, immediate stream. Every
//!     OS-level event flows through with no coalescing. The snapshot
//!     singleton uses this so its dirty set is always current the
//!     instant a snapshot is requested.
//!   - [`FsWatcher::subscribe_debounced`] — a coalescing listener that
//!     batches a burst into at most one event per `(path, kind)` pair
//!     per debounce window. This is what most of the system (file
//!     tree, editor prompts, git-context UI) should watch, so a single
//!     `git checkout` or save-storm doesn't spam every subscriber.
//!
//! Replaces the `chokidar` usage scattered through the TS codebase.
//! The watcher is cancelled by dropping the `FsWatcher` handle —
//! internal threads exit cleanly, no zombie threads.

mod filter;
mod registrar;
mod router;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use filter::{
    exclude_file, is_ignore_file, should_ignore_workspace_watch_path, WorkspaceFilter,
};
pub use notify::RecursiveMode;
use notify::{RecommendedWatcher, Watcher};
use router::{FollowMsg, Follower, Route, Router};
use thiserror::Error;
use tokio::sync::broadcast;

#[derive(Debug, Error)]
pub enum FsWatchError {
    #[error("notify error: {0}")]
    Notify(#[from] notify::Error),
}

/// What happened to a watched path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WatchEventKind {
    Created,
    Modified,
    Removed,
    /// `notify` couldn't classify the event but the path is implicated.
    Other,
}

#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub path: PathBuf,
    pub kind: WatchEventKind,
}

/// Active filesystem watcher.
///
/// Hold this for as long as you want to watch. Drop it to cancel.
pub struct FsWatcher {
    // Holding the watcher alive keeps it running. Drop releases all OS
    // handles. Shared (weakly) with a workspace watch's follower, which
    // adds a watch for each top-level directory that appears.
    _watcher: Arc<Mutex<RecommendedWatcher>>,
    sender: broadcast::Sender<WatchEvent>,
    /// A workspace watch's way to its follower.
    follower: Option<std::sync::mpsc::Sender<FollowMsg>>,
}

impl FsWatcher {
    /// Watch `path` recursively. Events fire immediately, with no
    /// coalescing — see [`FsWatcher::subscribe_debounced`] for the
    /// batched view.
    pub fn watch(path: impl AsRef<Path>) -> Result<Self, FsWatchError> {
        Self::watch_paths(vec![(
            path.as_ref().to_path_buf(),
            RecursiveMode::Recursive,
        )])
    }

    /// Watch a set of paths with per-path recursion modes. A single
    /// OS-level watcher is shared across every entry, so an event on
    /// any registered path flows through the same broadcast channel.
    /// Events are emitted immediately as the OS reports them.
    pub fn watch_paths(paths: Vec<(PathBuf, RecursiveMode)>) -> Result<Self, FsWatchError> {
        Self::start(paths, None)
    }

    /// Watch a worktree: its root non-recursively and each top-level
    /// directory `filter` keeps recursively ([`workspace_watch_paths`]) —
    /// pruned at registration, so a gitignored `target/` never floods the
    /// stream (tsk206). It **follows the tree** (tsk227, tsk1051): a kept
    /// top-level directory that appears later gets its recursive watch,
    /// and the files already in it are reported (as `Other`), since
    /// whatever was written before that watch landed no watch saw.
    pub fn watch_workspace(root: &Path, filter: WorkspaceFilter) -> Result<Self, FsWatchError> {
        let mut paths = workspace_watch_paths(root, &filter);
        // The exclude file is one of git's ignore files: an edit to it is
        // heard (and nothing else in the git dir).
        if let Some(info) = exclude_file(root).as_deref().and_then(Path::parent) {
            if info.is_dir() {
                paths.push((info.to_path_buf(), RecursiveMode::NonRecursive));
            }
        }
        let followed = paths
            .iter()
            .filter(|(_, mode)| matches!(mode, RecursiveMode::Recursive))
            .filter_map(|(p, _)| p.file_name().map(|n| n.to_os_string()))
            .collect();
        // The OS may report paths under either spelling of the root (macOS
        // reports `/private/var` for `/var`).
        let mut roots = vec![root.to_path_buf()];
        if let Ok(canonical) = root.canonicalize() {
            if canonical != root {
                roots.push(canonical);
            }
        }
        Self::start(
            paths,
            Some(Follow {
                roots,
                filter,
                followed,
            }),
        )
    }

    fn start(
        paths: Vec<(PathBuf, RecursiveMode)>,
        follow: Option<Follow>,
    ) -> Result<Self, FsWatchError> {
        // Capacity is generous because the raw stream is un-coalesced:
        // a `git checkout` or build can fire hundreds of events before
        // a slow subscriber drains them. Lagged subscribers log and
        // recover; they never block the watcher thread.
        let (tx, _) = broadcast::channel(1024);
        let (follower_tx, follower_rx) = std::sync::mpsc::channel::<FollowMsg>();
        let follower = follow.as_ref().map(|_| follower_tx.clone());
        let router = Router {
            events: tx.clone(),
            route: follow.as_ref().map(|f| Route {
                roots: f.roots.clone(),
                filter: f.filter.clone(),
                follower: follower_tx,
            }),
        };

        let mut watcher = notify::recommended_watcher(move |res| router.handle(res))?;
        for (p, mode) in paths {
            watcher.watch(&p, mode)?;
        }

        let watcher = Arc::new(Mutex::new(watcher));
        if let Some(follow) = follow {
            let weak = Arc::downgrade(&watcher);
            let follower = Follower {
                roots: follow.roots,
                filter: follow.filter,
                followed: follow.followed,
                events: tx.clone(),
            };
            std::thread::spawn(move || follower.run(&weak, &follower_rx));
        }

        Ok(Self {
            _watcher: watcher,
            sender: tx,
            follower,
        })
    }

    /// The filter changed other than by an ignore file (the project's
    /// `generated` lists): a workspace watch moves its watches to match.
    pub fn refilter(&self) {
        if let Some(follower) = &self.follower {
            let _ = follower.send(FollowMsg::Refilter);
        }
    }

    /// Subscribe to the raw, immediate event stream. Every OS-level
    /// event flows through with no coalescing. Subscribers can connect
    /// at any time; events emitted before subscription are dropped.
    pub fn subscribe(&self) -> broadcast::Receiver<WatchEvent> {
        self.sender.subscribe()
    }

    /// Subscribe to a debounced view of the event stream. A burst of
    /// events is collected for `window` after the first one arrives,
    /// then flushed as at most one `WatchEvent` per `(path, kind)`
    /// pair. This is the feed most of the system should watch — it
    /// turns a save-storm or `git checkout` into a single round of
    /// notifications instead of one per touched file.
    ///
    /// The coalescing task lives until the underlying watcher is
    /// dropped (which closes the raw stream); the returned receiver
    /// then closes too.
    pub fn subscribe_debounced(&self, window: Duration) -> broadcast::Receiver<WatchEvent> {
        debounced(self.subscribe(), window)
    }
}

/// See [`FsWatcher::subscribe_debounced`].
fn debounced(
    mut raw: broadcast::Receiver<WatchEvent>,
    window: Duration,
) -> broadcast::Receiver<WatchEvent> {
    let (tx, rx) = broadcast::channel(1024);
    tokio::spawn(async move {
        use broadcast::error::RecvError;
        loop {
            // Block until the first event of a new burst (or the
            // raw stream closes).
            let mut pending: HashSet<(PathBuf, WatchEventKind)> = HashSet::new();
            match raw.recv().await {
                Ok(ev) => {
                    pending.insert((ev.path, ev.kind));
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
            // Accumulate everything that lands within `window`,
            // then flush the de-duplicated set.
            let deadline = tokio::time::sleep(window);
            tokio::pin!(deadline);
            let mut closed = false;
            loop {
                tokio::select! {
                    _ = &mut deadline => break,
                    ev = raw.recv() => match ev {
                        Ok(ev) => { pending.insert((ev.path, ev.kind)); }
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => { closed = true; break; }
                    },
                }
            }
            for (path, kind) in pending.drain() {
                let _ = tx.send(WatchEvent { path, kind });
            }
            if closed {
                break;
            }
        }
    });
    rx
}

/// What a workspace watch registers: the root non-recursively (so a file
/// at the root, or a top-level directory appearing, is still seen) and
/// each top-level directory `filter` keeps, recursively. An unreadable
/// root is watched whole rather than not at all.
pub fn workspace_watch_paths(
    root: &Path,
    filter: &WorkspaceFilter,
) -> Vec<(PathBuf, RecursiveMode)> {
    let mut paths = vec![(root.to_path_buf(), RecursiveMode::NonRecursive)];
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(error = %e, ?root, "cannot enumerate the workspace root; watching it whole");
            return vec![(root.to_path_buf(), RecursiveMode::Recursive)];
        }
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir())
            && !filter.ignore(Path::new(&entry.file_name()), true)
        {
            paths.push((entry.path(), RecursiveMode::Recursive));
        }
    }
    paths
}

/// What a workspace watch's follower starts from.
struct Follow {
    roots: Vec<PathBuf>,
    filter: WorkspaceFilter,
    followed: HashSet<std::ffi::OsString>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::time::timeout;

    /// How long a test waits to hear of a change it made. It only waits this
    /// long if the change never comes. `.config/nextest.toml` says why these
    /// tests run alone.
    const HEARD_WITHIN: Duration = Duration::from_secs(60);

    #[tokio::test]
    async fn detects_new_file() {
        let dir = tempdir().unwrap();
        let watcher = FsWatcher::watch(dir.path()).unwrap();
        let mut rx = watcher.subscribe();

        let target = dir.path().join("hello.txt");
        std::fs::write(&target, b"hello").unwrap();

        // macOS resolves /var/folders to /private/var/folders — compare
        // canonicalized paths so the test is stable across platforms. It
        // can also report the freshly made directory itself first.
        let target = target.canonicalize().unwrap();
        timeout(HEARD_WITHIN, async {
            loop {
                let evt = rx.recv().await.expect("event ok");
                if evt.path.canonicalize().ok().as_ref() == Some(&target) {
                    break;
                }
            }
        })
        .await
        .expect("an event for the new file");
    }

    /// tsk206: the registration set prunes what the filter drops — a
    /// recursive watch on `target/` would hand over every build write —
    /// and keeps the root non-recursive, or the prune buys nothing.
    #[test]
    fn a_workspace_registers_its_kept_top_level_dirs() {
        let project = tempdir().unwrap();
        for d in ["src", "build", "node_modules"] {
            std::fs::create_dir(project.path().join(d)).unwrap();
        }
        std::fs::write(project.path().join("README.md"), b"x").unwrap();
        let names = |paths: &[(PathBuf, RecursiveMode)]| {
            paths
                .iter()
                .filter_map(|(p, _)| p.file_name()?.to_str().map(str::to_string))
                .collect::<Vec<_>>()
        };
        // Nothing is hardcoded: by default `build` is an ordinary dir.
        let all = workspace_watch_paths(project.path(), &WorkspaceFilter::default());
        assert!(names(&all).contains(&"build".to_string()));
        let pruned = workspace_watch_paths(
            project.path(),
            &WorkspaceFilter::with_user_entries(["build", "node_modules"]),
        );
        let got = names(&pruned);
        assert!(
            !got.contains(&"build".to_string()),
            "filtered dir not registered"
        );
        assert!(!got.contains(&"node_modules".to_string()));
        assert!(got.contains(&"src".to_string()));
        let root = pruned
            .iter()
            .find(|(p, _)| p == project.path())
            .expect("the root is registered");
        assert!(matches!(root.1, RecursiveMode::NonRecursive));
    }

    #[tokio::test]
    async fn non_recursive_only_reports_top_level() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("nested");
        std::fs::create_dir(&sub).unwrap();

        let watcher = FsWatcher::watch_paths(vec![(
            dir.path().to_path_buf(),
            RecursiveMode::NonRecursive,
        )])
        .unwrap();
        let mut rx = watcher.subscribe();

        // Write into a subdir — should NOT show up under non-recursive.
        let nested = sub.join("hidden.txt");
        std::fs::write(&nested, b"x").unwrap();

        // Drain for ~600ms; if we see the nested path, fail.
        let drain_deadline = std::time::Instant::now() + Duration::from_millis(600);
        let mut nested_seen = false;
        while std::time::Instant::now() < drain_deadline {
            match timeout(Duration::from_millis(150), rx.recv()).await {
                Ok(Ok(evt)) => {
                    let canon = evt.path.canonicalize().unwrap_or(evt.path.clone());
                    if canon == nested.canonicalize().unwrap() {
                        nested_seen = true;
                        break;
                    }
                }
                _ => break,
            }
        }
        assert!(
            !nested_seen,
            "non-recursive watch should not surface nested writes"
        );

        // Top-level write should be reported.
        let top = dir.path().join("top.txt");
        std::fs::write(&top, b"y").unwrap();
        let mut top_seen = false;
        let deadline = std::time::Instant::now() + HEARD_WITHIN;
        while std::time::Instant::now() < deadline {
            match timeout(Duration::from_millis(500), rx.recv()).await {
                Ok(Ok(evt)) => {
                    let canon = evt.path.canonicalize().unwrap_or(evt.path.clone());
                    if canon == top.canonicalize().unwrap() {
                        top_seen = true;
                        break;
                    }
                }
                Ok(Err(_)) => break,
                Err(_) => {}
            }
        }
        assert!(top_seen, "expected top-level event for {top:?}");
    }

    #[tokio::test]
    async fn drop_cancels_watcher() {
        let dir = tempdir().unwrap();
        let watcher = FsWatcher::watch(dir.path()).unwrap();
        let mut rx = watcher.subscribe();
        drop(watcher);
        // After drop, no further events are emitted; channel closes
        // when the sender is dropped (the watcher held the only sender).
        let target = dir.path().join("ignored.txt");
        std::fs::write(&target, b"x").unwrap();
        let recv = timeout(Duration::from_millis(500), rx.recv()).await;
        // Either a timeout (the channel is silent) or a closed channel.
        match recv {
            Err(_) => {}                                       // timeout — fine
            Ok(Err(broadcast::error::RecvError::Closed)) => {} // closed — fine
            other => panic!("expected closed/timeout, got {other:?}"),
        }
    }

    /// A burst is one round of events: at most one per `(path, kind)`, each
    /// file named. Time is paused, so the window is exact.
    #[tokio::test(start_paused = true)]
    async fn a_debounced_view_coalesces_a_burst_per_path_and_kind() {
        let (raw, raw_rx) = broadcast::channel(1024);
        let mut rx = debounced(raw_rx, Duration::from_millis(150));
        for n in 0..20 {
            let path = PathBuf::from(if n % 2 == 0 { "/a" } else { "/b" });
            raw.send(WatchEvent {
                path,
                kind: WatchEventKind::Modified,
            })
            .unwrap();
        }
        let mut heard = vec![rx.recv().await.unwrap().path, rx.recv().await.unwrap().path];
        heard.sort();
        assert_eq!(heard, [PathBuf::from("/a"), PathBuf::from("/b")]);
        assert!(
            timeout(Duration::from_secs(1), rx.recv()).await.is_err(),
            "one round only"
        );
    }
}
