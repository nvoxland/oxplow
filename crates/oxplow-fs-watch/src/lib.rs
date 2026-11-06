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

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use filter::{
    exclude_file, is_ignore_file, should_ignore_workspace_watch_path, WorkspaceFilter,
};
pub use notify::RecursiveMode;
use notify::{RecommendedWatcher, Watcher};
use pending::Subscribers;
pub use pending::{Batch, Feed, Scope, Subscription};
pub use router::Uncovered;
use router::{FollowMsg, Follower, Registration, Route, Router};
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
    // handles. Shared (weakly) with a per-directory watch's follower.
    _watcher: Arc<Mutex<RecommendedWatcher>>,
    events: Subscribers,
    /// A workspace watch's root and filter, and its way to its follower.
    workspace: Option<Workspace>,
}

struct Workspace {
    root: PathBuf,
    filter: WorkspaceFilter,
    follower: Option<std::sync::mpsc::Sender<FollowMsg>>,
    uncovered: Uncovered,
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
        let events = Subscribers::new(
            paths.iter().map(|(p, _)| p.clone()).collect(),
            paths.iter().map(|(p, _)| p.clone()).collect(),
        );
        let router = Router {
            events: events.clone(),
            route: None,
        };
        let mut watcher = notify::recommended_watcher(move |res| router.handle(res))?;
        register(&mut watcher, &paths)?;
        Ok(Self {
            _watcher: Arc::new(Mutex::new(watcher)),
            events,
            workspace: None,
        })
    }

    /// Watch a worktree, reporting only what `filter` keeps, the way this OS
    /// watches a tree (see [`Registration`]).
    pub fn watch_workspace(root: &Path, filter: WorkspaceFilter) -> Result<Self, FsWatchError> {
        // The OS may report paths under either spelling of the root (macOS
        // reports `/private/var` for `/var`).
        let mut roots = vec![root.to_path_buf()];
        if let Ok(canonical) = root.canonicalize() {
            if canonical != root {
                roots.push(canonical);
            }
        }
        let registration = Registration::for_this_os();
        let per_directory = registration == Registration::PerDirectory;
        let events = Subscribers::new(roots.clone(), vec![root.to_path_buf()]);
        let (follower_tx, follower_rx) = std::sync::mpsc::channel::<FollowMsg>();
        let follower = per_directory.then_some(follower_tx);
        let router = Router {
            events: events.clone(),
            route: Some(Route {
                roots,
                filter: filter.clone(),
                follower: follower.clone(),
            }),
        };
        let mut watcher = notify::recommended_watcher(move |res| router.handle(res))?;
        register(&mut watcher, &workspace_watch_paths(root, registration))?;

        let watcher = Arc::new(Mutex::new(watcher));
        let uncovered = Uncovered::default();
        if per_directory {
            let mut follow = Follower {
                root: root.to_path_buf(),
                filter: filter.clone(),
                watched: std::collections::HashSet::new(),
                uncovered: uncovered.clone(),
                events: events.clone(),
            };
            follow.cover(root, &*watcher);
            let weak = Arc::downgrade(&watcher);
            std::thread::spawn(move || follow.run(&weak, &follower_rx));
        }
        Ok(Self {
            _watcher: watcher,
            events,
            workspace: Some(Workspace {
                root: root.to_path_buf(),
                filter,
                follower,
                uncovered,
            }),
        })
    }

    /// The filter changed other than by an ignore file (the project's
    /// `generated` lists): what it stopped ignoring is looked for on the disk,
    /// and a per-directory watch moves to match.
    pub fn refilter(&self) {
        let Some(workspace) = &self.workspace else {
            return;
        };
        workspace.filter.forget_rules();
        self.events.rescan(&workspace.root);
        if let Some(follower) = &workspace.follower {
            let _ = follower.send(FollowMsg::Refilter(workspace.root.clone()));
        }
    }

    /// The directories this watch could not cover (the OS's limit on watches).
    pub fn uncovered(&self) -> Uncovered {
        self.workspace
            .as_ref()
            .map(|w| w.uncovered.clone())
            .unwrap_or_default()
    }

    /// What changes from now on: read with [`Subscription::next_batch`] (at
    /// once: the snapshot singleton, whose dirty set must be current the
    /// instant a snapshot is requested) or [`Subscription::next_batch_after`]
    /// (a burst as one batch: most of the system).
    pub fn subscribe(&self) -> Subscription {
        self.events.subscribe(None)
    }

    /// [`Self::subscribe`] for the paths `scope` names, kept by the filter or
    /// not, and no others.
    pub fn subscribe_to(&self, scope: Scope) -> Subscription {
        self.events.subscribe(Some(scope))
    }
}

impl Drop for FsWatcher {
    fn drop(&mut self) {
        self.events.close();
    }
}

/// What a workspace watch registers up front. A per-directory watch's follower
/// adds the rest as it walks.
fn workspace_watch_paths(root: &Path, registration: Registration) -> Vec<(PathBuf, RecursiveMode)> {
    let mut paths = Vec::new();
    if registration == Registration::Root {
        paths.push((root.to_path_buf(), RecursiveMode::Recursive));
    }
    // The exclude file is one of git's ignore files: an edit to it is heard
    // (and nothing else in the git dir). A recursive root already covers its
    // own.
    if let Some(info) = exclude_file(root).as_deref().and_then(Path::parent) {
        let covered = registration == Registration::Root && info.starts_with(root);
        if info.is_dir() && !covered {
            paths.push((info.to_path_buf(), RecursiveMode::NonRecursive));
        }
    }
    paths
}

/// As one batch: on macOS each `watch` alone stops and restarts the whole
/// FSEvents stream, which took seconds at boot and loaded `fseventsd` for every
/// process on the machine.
fn register(
    watcher: &mut RecommendedWatcher,
    paths: &[(PathBuf, RecursiveMode)],
) -> Result<(), FsWatchError> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut batch = watcher.paths_mut();
    for (path, mode) in paths {
        batch.add(path, *mode)?;
    }
    batch.commit()?;
    Ok(())
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

    /// What a workspace registers up front: on this OS's kind of recursive
    /// watch the root alone, however many folders it has; per directory, nothing
    /// yet (the follower walks). A non-git worktree has no exclude file to hear.
    #[test]
    fn a_workspace_registers_one_root_or_leaves_it_to_the_follower() {
        let project = tempdir().unwrap();
        for d in ["src", "build", "node_modules"] {
            std::fs::create_dir(project.path().join(d)).unwrap();
        }
        let root = project.path();
        let paths = workspace_watch_paths(root, Registration::Root);
        assert_eq!(paths.len(), 1);
        assert!(matches!(paths[0].1, RecursiveMode::Recursive));
        assert!(workspace_watch_paths(root, Registration::PerDirectory).is_empty());

        // A linked worktree hears its main checkout's exclude file.
        std::fs::create_dir_all(root.join("main/.git/info")).unwrap();
        std::fs::write(
            root.join("src/.git"),
            format!("gitdir: {}\n", root.join("main/.git").display()),
        )
        .unwrap();
        let paths = workspace_watch_paths(&root.join("src"), Registration::Root);
        assert_eq!(paths.len(), 2, "{paths:?}");
        assert_eq!(paths[1].0, root.join("main/.git/info"));
    }

    /// A folder made after the watch started: what is in it is heard, or the
    /// watch asks for it to be looked for.
    #[tokio::test]
    async fn a_new_folders_files_reach_a_workspace_subscriber() {
        let project = tempdir().unwrap();
        let root = project.path().canonicalize().unwrap();
        let watcher = FsWatcher::watch_workspace(&root, WorkspaceFilter::default()).unwrap();
        let mut sub = watcher.subscribe();
        std::fs::create_dir_all(root.join("new/deep")).unwrap();
        let file = root.join("new/deep/f.txt");
        std::fs::write(&file, b"x").unwrap();
        let found = timeout(HEARD_WITHIN, async {
            while let Some(batch) = sub.next_batch().await {
                if batch.paths.iter().any(|(p, _)| p == &file)
                    || batch.rescans.iter().any(|dir| file.starts_with(dir))
                {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        assert!(found, "the file in the new folder");
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
