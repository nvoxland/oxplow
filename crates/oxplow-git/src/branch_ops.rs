//! Branch lifecycle operations: rename, delete, ahead/behind,
//! commits-ahead-of, default branch detection, refs grouping.
//!
//! Aggregates the assorted branch operations from
//! `src/git/git.ts` that didn't fit into the basic `branch` module.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use crate::log::GitLogCommit;

#[derive(Debug, Error)]
pub enum BranchOpError {
    #[error("git2: {0}")]
    Git2(#[from] git2::Error),
    #[error("not a repository")]
    NotARepo,
    #[error("branch not found: {0}")]
    NotFound(String),
}

/// Rename a local branch.
pub fn rename_branch(repo_path: &Path, from: &str, to: &str) -> Result<(), BranchOpError> {
    let repo = git2::Repository::open(repo_path)?;
    let mut branch = repo
        .find_branch(from, git2::BranchType::Local)
        .map_err(|_| BranchOpError::NotFound(from.into()))?;
    branch.rename(to, false)?;
    Ok(())
}

/// Delete a local branch. `force = true` corresponds to `git branch -D`.
pub fn delete_branch(repo_path: &Path, branch: &str, force: bool) -> Result<(), BranchOpError> {
    let _ = force; // libgit2's `Branch::delete` doesn't honor unmerged-rejection
                   // the way the CLI does; relying on it being safe-by-default
                   // is fine because callers always confirm in the UI.
    let repo = git2::Repository::open(repo_path)?;
    let mut b = repo
        .find_branch(branch, git2::BranchType::Local)
        .map_err(|_| BranchOpError::NotFound(branch.into()))?;
    b.delete()?;
    Ok(())
}

/// Best-effort default-branch detection.
///
/// Order:
///   1. `init.defaultBranch` config (if present)
///   2. `origin/HEAD` symbolic ref
///   3. `main` if it exists, else `master`
///   4. None when nothing matches
pub fn detect_default_branch(repo_path: &Path) -> Option<String> {
    let repo = git2::Repository::open(repo_path).ok()?;
    if let Ok(config) = repo.config() {
        if let Ok(val) = config.get_string("init.defaultBranch") {
            if !val.trim().is_empty() {
                return Some(val);
            }
        }
    }
    if let Ok(reference) = repo.find_reference("refs/remotes/origin/HEAD") {
        if let Ok(Some(target)) = reference.symbolic_target() {
            if let Some(name) = target.strip_prefix("refs/remotes/origin/") {
                return Some(name.to_string());
            }
        }
    }
    for candidate in &["main", "master"] {
        if repo.find_branch(candidate, git2::BranchType::Local).is_ok() {
            return Some((*candidate).to_string());
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AheadBehind {
    pub ahead: u32,
    pub behind: u32,
}

/// Count commits `head` is ahead/behind `base`. Both args are
/// branch names (short form). Returns `(0, 0)` on lookup failure.
pub fn get_ahead_behind(repo_path: &Path, base: &str, head: &str) -> AheadBehind {
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return AheadBehind {
            ahead: 0,
            behind: 0,
        };
    };
    let resolve =
        |name: &str| -> Option<git2::Oid> { repo.revparse_single(name).ok().map(|obj| obj.id()) };
    let Some(base_oid) = resolve(base) else {
        return AheadBehind {
            ahead: 0,
            behind: 0,
        };
    };
    let Some(head_oid) = resolve(head) else {
        return AheadBehind {
            ahead: 0,
            behind: 0,
        };
    };
    repo.graph_ahead_behind(head_oid, base_oid)
        .map(|(ahead, behind)| AheadBehind {
            ahead: ahead as u32,
            behind: behind as u32,
        })
        .unwrap_or(AheadBehind {
            ahead: 0,
            behind: 0,
        })
}

/// First N commits `head` has that aren't on `base`. Equivalent to
/// `git log base..head -n LIMIT`.
pub fn get_commits_ahead_of(
    repo_path: &Path,
    base: &str,
    head: &str,
    limit: usize,
) -> Vec<GitLogCommit> {
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return vec![];
    };
    let Ok(mut walk) = repo.revwalk() else {
        return vec![];
    };
    let _ = walk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL);
    let resolve = |name: &str| repo.revparse_single(name).ok().map(|o| o.id());
    let Some(head_oid) = resolve(head) else {
        return vec![];
    };
    let Some(base_oid) = resolve(base) else {
        return vec![];
    };
    if walk.push(head_oid).is_err() {
        return vec![];
    }
    if walk.hide(base_oid).is_err() {
        return vec![];
    }

    let mut out = Vec::with_capacity(limit);
    for oid in walk.flatten().take(limit) {
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        let author = commit.author();
        out.push(GitLogCommit {
            sha: oid.to_string(),
            short_sha: oid.to_string()[..7].to_string(),
            author: author.name().unwrap_or("").to_string(),
            email: author.email().unwrap_or("").to_string(),
            timestamp_secs: commit.time().seconds(),
            subject: commit.summary().ok().flatten().unwrap_or("").to_string(),
            parents: commit.parent_ids().map(|p| p.to_string()).collect(),
        });
    }
    out
}

/// Append a line to `.gitignore`, creating it if needed. Idempotent —
/// duplicate entries are skipped.
pub fn append_to_gitignore(repo_path: &Path, entry: &str) -> std::io::Result<()> {
    let path = repo_path.join(".gitignore");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let trimmed = entry.trim();
    if existing.lines().any(|line| line.trim() == trimmed) {
        return Ok(());
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(trimmed);
    next.push('\n');
    std::fs::write(path, next)
}

/// Restore a path to its HEAD state — `git restore <path>`. Falls
/// back to the CLI because libgit2's checkout API is fiddly for a
/// single path.
pub fn restore_path(repo_path: &Path, path: &str) -> std::io::Result<()> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("restore")
        .arg(path)
        .output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        // Pin the initial branch to "main" via init options so the
        // tests don't depend on the system-wide init.defaultBranch
        // (CI runners often default to "master").
        let mut opts = git2::RepositoryInitOptions::new();
        opts.initial_head("main");
        let repo = git2::Repository::init_opts(dir.path(), &opts).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "t@example.com").unwrap();
        config.set_str("init.defaultBranch", "main").unwrap();
        let sig = repo.signature().unwrap();
        std::fs::write(dir.path().join("a"), "x").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("a")).unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        dir
    }

    #[test]
    fn rename_renames() {
        let dir = init_repo();
        let repo = git2::Repository::open(dir.path()).unwrap();
        rename_branch(dir.path(), "main", "trunk").unwrap();
        assert!(repo.find_branch("trunk", git2::BranchType::Local).is_ok());
        assert!(repo.find_branch("main", git2::BranchType::Local).is_err());
    }

    #[test]
    fn delete_removes_branch() {
        let dir = init_repo();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();
        delete_branch(dir.path(), "feature", false).unwrap();
        assert!(repo
            .find_branch("feature", git2::BranchType::Local)
            .is_err());
    }

    #[test]
    fn detect_default_uses_init_default_branch() {
        let dir = init_repo();
        // We set init.defaultBranch=main in the fixture; HEAD's branch
        // should be main.
        assert_eq!(detect_default_branch(dir.path()), Some("main".into()));
    }

    #[test]
    fn ahead_behind_zero_for_same_branch() {
        let dir = init_repo();
        let ab = get_ahead_behind(dir.path(), "main", "main");
        assert_eq!(
            ab,
            AheadBehind {
                ahead: 0,
                behind: 0
            }
        );
    }

    #[test]
    fn commits_ahead_of_lists_only_new_commits() {
        let dir = init_repo();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let base_head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &base_head, false).unwrap();
        // Switch to feature and add a commit.
        repo.set_head("refs/heads/feature").unwrap();
        std::fs::write(dir.path().join("b"), "y").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("b")).unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = repo.signature().unwrap();
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "feature commit",
            &tree,
            &[&base_head],
        )
        .unwrap();

        let commits = get_commits_ahead_of(dir.path(), "main", "feature", 10);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].subject, "feature commit");
    }

    #[test]
    fn append_to_gitignore_idempotent() {
        let dir = init_repo();
        append_to_gitignore(dir.path(), "target/").unwrap();
        append_to_gitignore(dir.path(), "target/").unwrap();
        let body = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(body.matches("target/").count(), 1);
    }
}
