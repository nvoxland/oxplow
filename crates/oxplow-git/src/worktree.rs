use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GitWorktreeEntry {
    pub path: String,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub is_main: bool,
    pub is_detached: bool,
    pub is_locked: bool,
    pub is_prunable: bool,
}

/// Parses `git worktree list --porcelain` output. Records are
/// blank-line-separated; each record is line-oriented `key value`
/// pairs.
pub fn list_existing_worktrees(repo_root: impl AsRef<Path>) -> Vec<GitWorktreeEntry> {
    let repo_root = repo_root.as_ref();
    if !crate::repo::is_git_repo(repo_root) {
        return vec![];
    }
    let output = match Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_root)
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };
    let raw = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    let mut current: Option<GitWorktreeEntry> = None;
    let mut is_first = true;
    for line in raw.split('\n') {
        if line.is_empty() {
            if let Some(mut e) = current.take() {
                e.is_main = is_first;
                is_first = false;
                out.push(e);
            }
            continue;
        }
        let entry = current.get_or_insert_with(|| GitWorktreeEntry {
            path: String::new(),
            branch: None,
            head_sha: None,
            is_main: false,
            is_detached: false,
            is_locked: false,
            is_prunable: false,
        });
        if let Some(rest) = line.strip_prefix("worktree ") {
            entry.path = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            entry.head_sha = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("branch ") {
            entry.branch = Some(rest.strip_prefix("refs/heads/").unwrap_or(rest).to_string());
        } else if line == "detached" {
            entry.is_detached = true;
        } else if line == "locked" || line.starts_with("locked ") {
            entry.is_locked = true;
        } else if line == "prunable" || line.starts_with("prunable ") {
            entry.is_prunable = true;
        }
    }
    if let Some(mut e) = current.take() {
        e.is_main = is_first;
        out.push(e);
    }
    out
}

/// Existing worktrees on disk that aren't yet registered as oxplow
/// streams. Caller filters against the StreamStore. Path comparison
/// is canonicalized so `/var/...` vs `/private/var/...` (macOS) and
/// trailing-slash quirks don't cause false negatives.
pub fn list_adoptable_worktrees(
    repo_root: impl AsRef<Path>,
    registered_paths: &[String],
) -> Vec<GitWorktreeEntry> {
    let canon_registered: Vec<PathBuf> = registered_paths
        .iter()
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .collect();
    list_existing_worktrees(repo_root)
        .into_iter()
        .filter(|e| {
            if e.is_main {
                return false;
            }
            let canon = std::fs::canonicalize(&e.path).ok();
            match canon {
                Some(c) => !canon_registered.iter().any(|r| r == &c),
                None => !registered_paths.iter().any(|p| p == &e.path),
            }
        })
        .collect()
}

#[derive(Debug, Error)]
pub enum EnsureWorktreeError {
    #[error("not a git repo: {0}")]
    NotRepo(PathBuf),
    #[error("git worktree add failed: {0}")]
    WorktreeAdd(String),
}

/// Ensure that a worktree exists at `worktree_path` checked out to
/// `branch`, creating the branch from `branch_source` if needed.
///
/// Idempotent: if the worktree already exists, this is a no-op.
/// Uses `git worktree add` shell-out because libgit2's worktree
/// support is incomplete for the cases we care about (e.g.
/// `--no-track` flags, `-B` to force re-create).
pub fn ensure_worktree(
    repo_root: impl AsRef<Path>,
    worktree_path: impl AsRef<Path>,
    branch: &str,
    branch_source: &str,
) -> Result<(), EnsureWorktreeError> {
    let repo_root = repo_root.as_ref();
    let worktree_path = worktree_path.as_ref();

    if !crate::repo::is_git_repo(repo_root) {
        return Err(EnsureWorktreeError::NotRepo(repo_root.to_path_buf()));
    }

    if worktree_path.exists() {
        info!(
            path = %worktree_path.display(),
            "worktree already exists; skipping ensure"
        );
        return Ok(());
    }

    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            EnsureWorktreeError::WorktreeAdd(format!(
                "create worktree parent dir {}: {e}",
                parent.display()
            ))
        })?;
    }

    // Create the branch from source if it doesn't exist; otherwise
    // just attach.
    let branch_exists = {
        let repo = git2::Repository::open(repo_root)
            .map_err(|e| EnsureWorktreeError::WorktreeAdd(e.message().to_string()))?;
        let exists = repo.find_branch(branch, git2::BranchType::Local).is_ok();
        exists
    };

    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo_root).arg("worktree").arg("add");
    if !branch_exists {
        cmd.arg("-b")
            .arg(branch)
            .arg(worktree_path)
            .arg(branch_source);
    } else {
        cmd.arg(worktree_path).arg(branch);
    }

    let out = cmd
        .output()
        .map_err(|e| EnsureWorktreeError::WorktreeAdd(format!("spawn: {e}")))?;
    if !out.status.success() {
        return Err(EnsureWorktreeError::WorktreeAdd(format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    info!(
        path = %worktree_path.display(),
        branch,
        "worktree created"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        // Set init.defaultBranch so the branch source matches.
        config.set_str("init.defaultBranch", "main").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = {
            let mut idx = repo.index().unwrap();
            idx.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        dir
    }

    fn current_branch(repo: &git2::Repository) -> String {
        repo.head()
            .unwrap()
            .shorthand()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "main".into())
    }

    #[test]
    fn creates_new_worktree_with_new_branch() {
        let repo_dir = init_repo();
        let repo = git2::Repository::open(repo_dir.path()).unwrap();
        let main = current_branch(&repo);

        let wt_root = tempdir().unwrap();
        let wt_path = wt_root.path().join("feature-wt");
        let result = ensure_worktree(repo_dir.path(), &wt_path, "feature", &main);
        assert!(result.is_ok(), "{result:?}");
        assert!(wt_path.exists());
        // The new branch should now exist in the main repo.
        assert!(repo.find_branch("feature", git2::BranchType::Local).is_ok());
    }

    #[test]
    fn idempotent_when_path_exists() {
        let repo_dir = init_repo();
        let repo = git2::Repository::open(repo_dir.path()).unwrap();
        let main = current_branch(&repo);

        let wt_root = tempdir().unwrap();
        let wt_path = wt_root.path().join("dup");
        ensure_worktree(repo_dir.path(), &wt_path, "feature", &main).unwrap();
        // Second call should be a no-op (and therefore Ok), even
        // though `git worktree add` itself would fail loudly.
        ensure_worktree(repo_dir.path(), &wt_path, "feature", &main).unwrap();
    }

    #[test]
    fn rejects_non_repo() {
        let dir = tempdir().unwrap();
        let result = ensure_worktree(dir.path(), dir.path().join("wt"), "x", "y");
        assert!(matches!(result, Err(EnsureWorktreeError::NotRepo(_))));
    }

    #[test]
    fn list_existing_worktrees_returns_main() {
        let repo_dir = init_repo();
        let entries = list_existing_worktrees(repo_dir.path());
        assert!(entries.iter().any(|e| e.is_main));
    }

    #[test]
    fn list_existing_worktrees_includes_added_one() {
        let repo_dir = init_repo();
        let repo = git2::Repository::open(repo_dir.path()).unwrap();
        let main = current_branch(&repo);
        let wt_root = tempdir().unwrap();
        let wt_path = wt_root.path().join("feat");
        ensure_worktree(repo_dir.path(), &wt_path, "feat", &main).unwrap();
        let entries = list_existing_worktrees(repo_dir.path());
        assert!(entries.iter().any(|e| e.branch.as_deref() == Some("feat")));
    }

    #[test]
    fn list_adoptable_excludes_registered_paths() {
        let repo_dir = init_repo();
        let repo = git2::Repository::open(repo_dir.path()).unwrap();
        let main = current_branch(&repo);
        let wt_root = tempdir().unwrap();
        let wt_path = wt_root.path().join("feat");
        ensure_worktree(repo_dir.path(), &wt_path, "feat", &main).unwrap();
        let registered = vec![wt_path.to_string_lossy().into_owned()];
        let adoptable = list_adoptable_worktrees(repo_dir.path(), &registered);
        assert!(adoptable.is_empty());
        let none_registered: Vec<String> = vec![];
        let adoptable = list_adoptable_worktrees(repo_dir.path(), &none_registered);
        assert!(adoptable
            .iter()
            .any(|e| e.branch.as_deref() == Some("feat")));
    }
}
