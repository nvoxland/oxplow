use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::repo::is_git_repo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum GitOperationKind {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RepoConflictState {
    /// Active long-running git op, or `None` when the worktree is clean.
    pub operation: Option<GitOperationKind>,
    /// Number of paths reported as unmerged by `git status --porcelain`.
    pub conflicted_count: u32,
}

/// Snapshot the worktree's conflict state for the rail HUD.
///
/// Cheap — one fs stat for the operation kind plus one git status
/// when an op is active. Conflicts can also exist outside an
/// in-progress op (e.g. after `git stash pop`), so we still scan
/// porcelain for `U`-flag codes when no MERGE_HEAD is present.
pub fn get_repo_conflict_state(worktree_path: impl AsRef<Path>) -> RepoConflictState {
    RepoConflictState {
        operation: detect_git_operation(worktree_path.as_ref()),
        conflicted_count: count_unmerged_paths(worktree_path.as_ref()),
    }
}

fn detect_git_operation(worktree_path: &Path) -> Option<GitOperationKind> {
    let git_dir = resolve_git_dir(worktree_path)?;
    if exists_marker(&git_dir, "MERGE_HEAD") {
        return Some(GitOperationKind::Merge);
    }
    if exists_marker(&git_dir, "CHERRY_PICK_HEAD") {
        return Some(GitOperationKind::CherryPick);
    }
    if exists_marker(&git_dir, "REVERT_HEAD") {
        return Some(GitOperationKind::Revert);
    }
    if exists_marker(&git_dir, "REBASE_HEAD")
        || exists_dir(&git_dir, "rebase-merge")
        || exists_dir(&git_dir, "rebase-apply")
    {
        return Some(GitOperationKind::Rebase);
    }
    None
}

fn resolve_git_dir(worktree_path: &Path) -> Option<PathBuf> {
    let dot_git = worktree_path.join(".git");
    let meta = std::fs::metadata(&dot_git).ok()?;
    if meta.is_dir() {
        return Some(dot_git);
    }
    if meta.is_file() {
        let contents = std::fs::read_to_string(&dot_git).ok()?;
        for line in contents.lines() {
            if let Some(rest) = line.strip_prefix("gitdir:") {
                let target = rest.trim();
                let p = if std::path::Path::new(target).is_absolute() {
                    PathBuf::from(target)
                } else {
                    worktree_path.join(target)
                };
                return Some(p);
            }
        }
    }
    None
}

fn exists_marker(git_dir: &Path, name: &str) -> bool {
    git_dir.join(name).exists()
}

fn exists_dir(git_dir: &Path, name: &str) -> bool {
    std::fs::metadata(git_dir.join(name))
        .map(|m| m.is_dir())
        .unwrap_or(false)
}

fn count_unmerged_paths(worktree_path: &Path) -> u32 {
    if !is_git_repo(worktree_path) {
        return 0;
    }
    list_conflicted_paths(worktree_path).len() as u32
}

/// Every path left unmerged in the workspace, sorted.
pub fn list_conflicted_paths(worktree_path: &Path) -> Vec<String> {
    let Ok(repo) = git2::Repository::open(worktree_path) else {
        return vec![];
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_unmodified(false);
    opts.include_ignored(false);
    let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
        return vec![];
    };
    let mut out: Vec<String> = statuses
        .iter()
        .filter(|s| s.status().contains(git2::Status::CONFLICTED))
        .filter_map(|s| s.path().ok().map(str::to_string))
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
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

    #[test]
    fn clean_repo_has_no_operation() {
        let dir = init_repo();
        let state = get_repo_conflict_state(dir.path());
        assert_eq!(state.operation, None);
        assert_eq!(state.conflicted_count, 0);
    }

    #[test]
    fn merge_marker_reports_merge() {
        let dir = init_repo();
        File::create(dir.path().join(".git/MERGE_HEAD")).unwrap();
        let state = get_repo_conflict_state(dir.path());
        assert_eq!(state.operation, Some(GitOperationKind::Merge));
    }

    #[test]
    fn cherry_pick_marker_reports_cherry_pick() {
        let dir = init_repo();
        File::create(dir.path().join(".git/CHERRY_PICK_HEAD")).unwrap();
        let state = get_repo_conflict_state(dir.path());
        assert_eq!(state.operation, Some(GitOperationKind::CherryPick));
    }

    #[test]
    fn rebase_apply_dir_reports_rebase() {
        let dir = init_repo();
        std::fs::create_dir(dir.path().join(".git/rebase-apply")).unwrap();
        let state = get_repo_conflict_state(dir.path());
        assert_eq!(state.operation, Some(GitOperationKind::Rebase));
    }

    #[test]
    fn revert_marker_reports_revert() {
        let dir = init_repo();
        File::create(dir.path().join(".git/REVERT_HEAD")).unwrap();
        let state = get_repo_conflict_state(dir.path());
        assert_eq!(state.operation, Some(GitOperationKind::Revert));
    }
}
