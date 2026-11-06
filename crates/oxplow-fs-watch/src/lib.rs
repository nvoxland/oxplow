//! Filesystem watcher.
//!
//! Wraps `notify::RecommendedWatcher`. Reused by oxplow-git for `.git/refs`
//! watching, and by the snapshot-capture and analysis pipelines.
//!
//! A subscriber reads [`Batch`]es from its [`Subscription`]: a set of what
//! changed, read when it likes, that nothing can overflow. Past a cap, or when
//! the OS says it dropped events, a batch asks for a rescan of a directory
//! instead, and the subscriber checks the disk there. **Events are hints, not
//! the record**: they can come late, and only a rescan makes them whole.
//!
//! Replaces the `chokidar` usage scattered through the TS codebase.
//! The watcher is cancelled by dropping the `FsWatcher` handle —
//! internal threads exit cleanly, no zombie threads.

mod filter;
mod pending;
mod registrar;
mod router;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use filter::{
    exclude_file, is_ignore_file, should_ignore_workspace_watch_path, WorkspaceFilter,
};
pub use notify::RecursiveMode;
use notify::{RecommendedWatcher, Watcher};
use pending::Subscribers;
pub use pending::{Batch, Subscription};
use router::{FollowMsg, Follower, Route, Router};
use thiserror::Error;

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

/// Active filesystem watcher.
///
/// Hold this for as long as you want to watch. Drop it to cancel.
pub struct FsWatcher {
    // Holding the watcher alive keeps it running. Drop releases all OS
    // handles. Shared (weakly) with a workspace watch's follower, which
    // adds a watch for each top-level directory that appears.
    _watcher: Arc<Mutex<RecommendedWatcher>>,
    events: Subscribers,
    /// A workspace watch's way to its follower.
    follower: Option<std::sync::mpsc::Sender<FollowMsg>>,
}

impl FsWatcher {
    /// Watch `path` recursively.
    pub fn watch(path: impl AsRef<Path>) -> Result<Self, FsWatchError> {
        Self::watch_paths(vec![(
            path.as_ref().to_path_buf(),
            RecursiveMode::Recursive,
        )])
    }

    /// Watch a set of paths with per-path recursion modes. A single
    /// OS-level watcher is shared across every entry, so an event on
    /// any registered path reaches the same subscribers.
    pub fn watch_paths(paths: Vec<(PathBuf, RecursiveMode)>) -> Result<Self, FsWatchError> {
        Self::start(paths, None)
    }

    /// Watch a worktree: its root non-recursively and each top-level
    /// directory `filter` keeps recursively ([`workspace_watch_paths`]) —
    /// pruned at registration, so a gitignored `target/` never floods the
    /// stream (tsk206). It **follows the tree** (tsk227, tsk1051): a kept
    /// top-level directory that appears later gets its recursive watch and a
    /// rescan, since whatever was written before that watch landed no watch
    /// saw.
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
        let roots: Vec<PathBuf> = match &follow {
            Some(follow) => follow.roots.clone(),
            None => paths.iter().map(|(p, _)| p.clone()).collect(),
        };
        let whole: Vec<PathBuf> = match &follow {
            Some(follow) => follow.roots.iter().take(1).cloned().collect(),
            None => roots.clone(),
        };
        let events = Subscribers::new(roots, whole);
        let (follower_tx, follower_rx) = std::sync::mpsc::channel::<FollowMsg>();
        let follower = follow.as_ref().map(|_| follower_tx.clone());
        let router = Router {
            events: events.clone(),
            route: follow.as_ref().map(|f| Route {
                roots: f.roots.clone(),
                filter: f.filter.clone(),
                follower: follower_tx,
            }),
        };

        let mut watcher = notify::recommended_watcher(move |res| router.handle(res))?;
        // As one batch: on macOS each `watch` alone stops and restarts the
        // whole FSEvents stream, which took seconds at boot and loaded
        // `fseventsd` for every process on the machine.
        let mut batch = watcher.paths_mut();
        for (p, mode) in paths {
            batch.add(&p, mode)?;
        }
        batch.commit()?;

        let watcher = Arc::new(Mutex::new(watcher));
        if let Some(follow) = follow {
            let weak = Arc::downgrade(&watcher);
            let follower = Follower {
                roots: follow.roots,
                filter: follow.filter,
                followed: follow.followed,
                events: events.clone(),
            };
            std::thread::spawn(move || follower.run(&weak, &follower_rx));
        }

        Ok(Self {
            _watcher: watcher,
            events,
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

    /// What changes from now on: read with [`Subscription::next_batch`] (at
    /// once: the snapshot singleton, whose dirty set must be current the
    /// instant a snapshot is requested) or [`Subscription::next_batch_after`]
    /// (a burst as one batch: most of the system).
    pub fn subscribe(&self) -> Subscription {
        self.events.subscribe()
    }
}

impl Drop for FsWatcher {
    fn drop(&mut self) {
        self.events.close();
    }
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

    /// The batches a subscription reads until `path` is among them, within
    /// `within`.
    async fn heard_within(sub: &mut Subscription, path: &Path, within: Duration) -> bool {
        timeout(within, async {
            while let Some(batch) = sub.next_batch().await {
                let heard = batch
                    .paths
                    .iter()
                    .any(|(p, _)| p.canonicalize().ok().as_deref() == Some(path));
                if heard {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false)
    }

    /// How long a test waits to hear of a change it made. It only waits this
    /// long if the change never comes. `.config/nextest.toml` says why these
    /// tests run alone.
    const HEARD_WITHIN: Duration = Duration::from_secs(60);

    #[tokio::test]
    async fn detects_new_file() {
        let dir = tempdir().unwrap();
        let watcher = FsWatcher::watch(dir.path()).unwrap();
        let mut sub = watcher.subscribe();

        let target = dir.path().join("hello.txt");
        std::fs::write(&target, b"hello").unwrap();

        // macOS resolves /var/folders to /private/var/folders — compare
        // canonicalized paths so the test is stable across platforms. It
        // can also report the freshly made directory itself first.
        let target = target.canonicalize().unwrap();
        assert!(
            heard_within(&mut sub, &target, HEARD_WITHIN).await,
            "an event for the new file"
        );
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
        let sub_dir = dir.path().join("nested");
        std::fs::create_dir(&sub_dir).unwrap();

        let watcher = FsWatcher::watch_paths(vec![(
            dir.path().to_path_buf(),
            RecursiveMode::NonRecursive,
        )])
        .unwrap();
        let mut sub = watcher.subscribe();

        // Write into a subdir — should NOT show up under non-recursive.
        let nested = sub_dir.join("hidden.txt");
        std::fs::write(&nested, b"x").unwrap();
        let nested = nested.canonicalize().unwrap();
        assert!(
            !heard_within(&mut sub, &nested, Duration::from_millis(600)).await,
            "non-recursive watch should not surface nested writes"
        );

        // Top-level write should be reported.
        let top = dir.path().join("top.txt");
        std::fs::write(&top, b"y").unwrap();
        let top = top.canonicalize().unwrap();
        assert!(
            heard_within(&mut sub, &top, HEARD_WITHIN).await,
            "expected top-level event for {top:?}"
        );
    }

    #[tokio::test]
    async fn drop_cancels_watcher() {
        let dir = tempdir().unwrap();
        let watcher = FsWatcher::watch(dir.path()).unwrap();
        let mut sub = watcher.subscribe();
        drop(watcher);
        // After drop, no further events are emitted, and the subscription
        // ends once what it holds is read.
        std::fs::write(dir.path().join("ignored.txt"), b"x").unwrap();
        while sub.next_batch().await.is_some() {}
    }
}
