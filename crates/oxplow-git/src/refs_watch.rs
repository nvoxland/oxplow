//! Watcher for `.git/refs/`. Emits a domain-level `RefsChangeEvent`
//! when any ref under `refs/heads/` changes, so the UI can refresh
//! its branch list without polling.
//!
//! Wraps `oxplow-fs-watch` rather than re-implementing the
//! debouncer.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::broadcast;
use tracing::debug;

use oxplow_fs_watch::{FsWatchError, FsWatcher};

/// Domain-level event emitted to the UI. Currently only signals
/// "something changed" since the watcher debounces; consumers
/// re-query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RefsChangeEvent {
    /// The repo whose refs changed (absolute path on disk).
    pub repo_path: String,
}

pub struct GitRefsWatcher {
    _inner: FsWatcher,
    sender: broadcast::Sender<RefsChangeEvent>,
}

impl GitRefsWatcher {
    /// Watch the `.git/refs/` directory under `repo_path`. Returns a
    /// handle whose Drop cancels the watcher.
    pub fn watch(repo_path: PathBuf, debounce: Duration) -> Result<Self, FsWatchError> {
        let refs_dir = repo_path.join(".git").join("refs");
        let (sender, _) = broadcast::channel::<RefsChangeEvent>(64);

        let watcher = FsWatcher::watch(&refs_dir)?;
        // A single `git commit` writes several ref files in quick
        // succession; debounce so consumers re-query once per burst.
        let mut sub = watcher.subscribe_debounced(debounce);
        let tx = sender.clone();
        let path_str = repo_path.to_string_lossy().into_owned();
        tokio::spawn(async move {
            while let Ok(_evt) = sub.recv().await {
                let _ = tx.send(RefsChangeEvent {
                    repo_path: path_str.clone(),
                });
                debug!(repo = %path_str, "refs change");
            }
        });

        Ok(Self {
            _inner: watcher,
            sender,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RefsChangeEvent> {
        self.sender.subscribe()
    }
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
}
