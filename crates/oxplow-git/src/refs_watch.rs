//! Watcher for a repo's refs. Emits a domain-level `RefsChangeEvent`
//! when HEAD or any ref changes, so the UI can refresh its branch list
//! and the commit index can catch up without anyone asking.
//!
//! Two sources, one stream: the OS's file events under `refs/` (fast,
//! debounced through `oxplow-fs-watch`), and a cheap check of the refs'
//! state on a timer. File events can come many seconds late or not at
//! all (macOS FSEvents under load, a watch that couldn't start), so the
//! check is what makes a ref move always heard; it reads HEAD and stats
//! the ref files, nothing more.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::broadcast;
use tracing::debug;

use oxplow_fs_watch::{FsWatchError, FsWatcher, WatchEvent};

/// Domain-level event emitted to the UI. Currently only signals
/// "something changed" since the watcher debounces; consumers
/// re-query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RefsChangeEvent {
    /// The repo whose refs changed (absolute path on disk).
    pub repo_path: String,
}

/// How often the refs' state is checked when no file event came.
const CHECK_EVERY: Duration = Duration::from_secs(1);

pub struct GitRefsWatcher {
    _inner: Option<FsWatcher>,
    _stop: tokio::sync::oneshot::Sender<()>,
    sender: broadcast::Sender<RefsChangeEvent>,
}

impl GitRefsWatcher {
    /// Watch the refs of the repo at `repo_path`. Returns a handle whose
    /// Drop cancels the watcher.
    pub fn watch(repo_path: PathBuf, debounce: Duration) -> Result<Self, FsWatchError> {
        let dirs = git_dirs(&repo_path);
        let (sender, _) = broadcast::channel::<RefsChangeEvent>(64);
        // The shared refs (a linked worktree's are its common dir's). A
        // watch that can't start leaves the timer's check.
        let refs_dir = dirs
            .last()
            .cloned()
            .unwrap_or_else(|| repo_path.join(".git"));
        let watcher = match FsWatcher::watch(refs_dir.join("refs")) {
            Ok(w) => Some(w),
            Err(e) if dirs.is_empty() => return Err(e),
            Err(e) => {
                debug!(error = %e, repo = %repo_path.display(), "no refs file watch; checking on a timer only");
                None
            }
        };
        // A single `git commit` writes several ref files in quick
        // succession; debounce so consumers re-query once per burst.
        let events = watcher.as_ref().map(|w| w.subscribe_debounced(debounce));
        let stop = spawn_checks(repo_path, events, CHECK_EVERY, sender.clone());
        Ok(Self {
            _inner: watcher,
            _stop: stop,
            sender,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RefsChangeEvent> {
        self.sender.subscribe()
    }
}

/// Announce on `tx` each burst of `events` and each change of the refs'
/// state seen every `every`, until the returned sender drops. A file
/// event restates what the check compares against, so one move is
/// announced once.
fn spawn_checks(
    repo_path: PathBuf,
    mut events: Option<broadcast::Receiver<WatchEvent>>,
    every: Duration,
    tx: broadcast::Sender<RefsChangeEvent>,
) -> tokio::sync::oneshot::Sender<()> {
    let (stop_tx, mut stop) = tokio::sync::oneshot::channel::<()>();
    let path_str = repo_path.to_string_lossy().into_owned();
    // What a move is a move from: the refs as the watch begins.
    let dirs = git_dirs(&repo_path);
    let mut last = refs_state(&dirs);
    tokio::spawn(async move {
        let state = |dirs: Vec<PathBuf>| async move {
            tokio::task::spawn_blocking(move || refs_state(&dirs))
                .await
                .unwrap_or_default()
        };
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let heard = tokio::select! {
                _ = &mut stop => return,
                ev = async {
                    match events.as_mut() {
                        Some(rx) => rx.recv().await.map(|_| ()),
                        None => std::future::pending().await,
                    }
                } => match ev {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        last = state(dirs.clone()).await;
                        true
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        events = None;
                        false
                    }
                },
                _ = tick.tick() => {
                    let now = state(dirs.clone()).await;
                    let moved = now != last;
                    last = now;
                    moved
                }
            };
            if heard {
                let _ = tx.send(RefsChangeEvent {
                    repo_path: path_str.clone(),
                });
                debug!(repo = %path_str, "refs change");
            }
        }
    });
    stop_tx
}

/// The repo's git dir and, for a linked worktree, its common dir (where
/// the shared refs live) last; none outside a repo.
fn git_dirs(repo_path: &Path) -> Vec<PathBuf> {
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return Vec::new();
    };
    let git_dir = repo.path().to_path_buf();
    let common = repo.commondir().to_path_buf();
    if common == git_dir {
        vec![git_dir]
    } else {
        vec![git_dir, common]
    }
}

/// A fingerprint of where HEAD and the refs are: HEAD's text, and each
/// ref file's (and `packed-refs`') path, size and modification time.
fn refs_state(dirs: &[PathBuf]) -> u64 {
    fn walk(dir: &Path, h: &mut std::collections::hash_map::DefaultHasher) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            match e.metadata() {
                Ok(m) if m.is_dir() => walk(&path, h),
                Ok(m) => stat(&path, &m, h),
                Err(_) => {}
            }
        }
    }
    fn stat(path: &Path, m: &std::fs::Metadata, h: &mut std::collections::hash_map::DefaultHasher) {
        path.hash(h);
        m.len().hash(h);
        m.modified().ok().hash(h);
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for dir in dirs {
        std::fs::read(dir.join("HEAD")).ok().hash(&mut h);
        let packed = dir.join("packed-refs");
        if let Ok(m) = std::fs::metadata(&packed) {
            stat(&packed, &m, &mut h);
        }
        walk(&dir.join("refs"), &mut h);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::time::timeout;

    #[tokio::test]
    async fn writing_a_ref_triggers_event() {
        let dir = tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        // First commit so HEAD resolves and a head ref file exists.
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "t").unwrap();
        config.set_str("user.email", "t@example.com").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let head_oid = repo
            .commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        let watcher =
            GitRefsWatcher::watch(dir.path().to_path_buf(), Duration::from_millis(50)).unwrap();
        let mut rx = watcher.subscribe();

        // Write a new branch ref. libgit2 does this via the refdb,
        // which lands a file on disk under .git/refs/heads/.
        let head = repo.find_commit(head_oid).unwrap();
        repo.branch("feature", &head, false).unwrap();

        // Returns as soon as it lands; the budget allows a loaded machine.
        let evt = timeout(Duration::from_secs(20), rx.recv())
            .await
            .expect("event arrives")
            .expect("recv ok");
        assert!(evt
            .repo_path
            .contains(dir.path().to_string_lossy().as_ref()));
    }

    /// The OS's file events can come late, or not at all (macOS FSEvents
    /// under load): the refs are also checked on a timer, so a commit is
    /// heard with no file event.
    #[tokio::test]
    async fn a_commit_is_heard_without_any_file_event() {
        let dir = tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "t").unwrap();
        config.set_str("user.email", "t@example.com").unwrap();
        let sig = repo.signature().unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        let first = repo
            .commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        let (sender, mut rx) = broadcast::channel(8);
        let _stop = spawn_checks(
            dir.path().to_path_buf(),
            None,
            Duration::from_millis(20),
            sender,
        );
        let parent = repo.find_commit(first).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "second", &tree, &[&parent])
            .unwrap();

        let evt = timeout(Duration::from_secs(20), rx.recv())
            .await
            .expect("the check hears it")
            .expect("recv ok");
        assert!(evt
            .repo_path
            .contains(dir.path().to_string_lossy().as_ref()));
    }

    /// Nothing moved: the check says nothing.
    #[test]
    fn the_refs_state_moves_with_a_commit_and_a_checkout_only() {
        let dir = tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("t", "t@example.com").unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        let first = repo
            .commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        let dirs = git_dirs(dir.path());
        let a = refs_state(&dirs);
        assert_eq!(a, refs_state(&dirs), "nothing moved");
        let parent = repo.find_commit(first).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "second", &tree, &[&parent])
            .unwrap();
        let b = refs_state(&dirs);
        assert_ne!(a, b, "a commit moves it");
        repo.branch("side", &parent, false).unwrap();
        repo.set_head("refs/heads/side").unwrap();
        assert_ne!(b, refs_state(&dirs), "a checkout moves it");
    }
}
