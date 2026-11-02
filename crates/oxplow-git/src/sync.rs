//! Git sync operations: fetch / pull / push / merge / rebase / commit / add.
//!
//! These shell out to `git` rather than going through `git2`, because:
//!
//! - `git2` doesn't drive remote auth credential helpers, so the user's
//!   configured `credential.helper` (Keychain on macOS, libsecret on
//!   Linux, manager-core on Windows) wouldn't be honored. The CLI does
//!   the right thing automatically.
//! - These ops are user-initiated and best-effort; capturing stdout +
//!   stderr in a typed result is enough for the UI.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use specta::Type;

/// Result of a git sync operation.
///
/// `success` is true iff the underlying `git` exited 0. `stdout` /
/// `stderr` are captured verbatim; the UI surfaces them in a toast or
/// the operation log.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GitOpResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub status: Option<i32>,
    /// Number of files the smart-merge pass auto-resolved after git left
    /// them conflicted (0 for non-merge ops). Lets the UI report
    /// "N conflicts auto-resolved".
    #[serde(default)]
    pub auto_resolved: u32,
}

fn run(args: &[&str], cwd: &Path) -> std::io::Result<GitOpResult> {
    let output = Command::new("git").args(args).current_dir(cwd).output()?;
    Ok(GitOpResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code(),
        auto_resolved: 0,
    })
}

pub fn fetch(repo: &Path, remote: Option<&str>) -> std::io::Result<GitOpResult> {
    let r = remote.unwrap_or("origin");
    run(&["fetch", r], repo)
}

pub fn pull(repo: &Path) -> std::io::Result<GitOpResult> {
    run(&["pull", "--ff-only"], repo)
}

pub fn pull_remote_into_current(
    repo: &Path,
    remote: &str,
    branch: &str,
) -> std::io::Result<GitOpResult> {
    run(&["pull", remote, branch], repo)
}

pub fn push(repo: &Path) -> std::io::Result<GitOpResult> {
    run(&["push"], repo)
}

pub fn push_current_to(repo: &Path, remote: &str, branch: &str) -> std::io::Result<GitOpResult> {
    run(&["push", remote, &format!("HEAD:{branch}")], repo)
}

pub fn merge(repo: &Path, source: &str) -> std::io::Result<GitOpResult> {
    run(&["merge", "--no-edit", source], repo)
}

pub fn rebase(repo: &Path, onto: &str) -> std::io::Result<GitOpResult> {
    run(&["rebase", onto], repo)
}

pub fn cherry_pick(repo: &Path, commit: &str) -> std::io::Result<GitOpResult> {
    run(&["cherry-pick", commit], repo)
}

pub fn revert(repo: &Path, commit: &str) -> std::io::Result<GitOpResult> {
    run(&["revert", "--no-edit", commit], repo)
}

/// Stage the workspace's changes — untracked files too when
/// `include_untracked` — and commit them.
pub fn commit(repo: &Path, message: &str, include_untracked: bool) -> std::io::Result<GitOpResult> {
    let staged = run(&["add", if include_untracked { "-A" } else { "-u" }], repo)?;
    if !staged.success {
        return Ok(staged);
    }
    run(&["commit", "-m", message], repo)
}

/// Take back the commit `sha` that `parent` precedes: a soft reset, so
/// the head moves to `parent` and the commit's changes stay staged
/// (index and working tree untouched). Refused, with nothing changed,
/// when the head is no longer `sha`, `parent` isn't its parent, a remote
/// branch holds the commit (it was pushed), or the repository is
/// mid-merge, rebase, cherry-pick or revert, or the working tree has
/// changes (they would be staged with the commit's).
pub fn uncommit(repo: &Path, sha: &str, parent: &str) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    if let Some(op) = crate::conflict::get_repo_conflict_state(repo).operation {
        return Err(format!("a {op:?} is in progress; finish or abort it first").to_lowercase());
    }
    let head = run(&["rev-parse", "--verify", "HEAD^{commit}"], repo).map_err(io)?;
    if !head.success || head.stdout.trim() != sha {
        return Err("the head is no longer that commit (something was committed after it, or the branch moved)".into());
    }
    let actual = run(&["rev-parse", "--verify", &format!("{sha}^")], repo).map_err(io)?;
    if !actual.success || actual.stdout.trim() != parent {
        return Err("the commit's parent is not the one expected".into());
    }
    let remote = run(&["branch", "-r", "--contains", sha], repo).map_err(io)?;
    if !remote.success {
        return Err(remote.stderr.trim().to_string());
    }
    if !remote.stdout.trim().is_empty() {
        return Err("the commit was pushed (a remote branch holds it); revert it instead".into());
    }
    // The commit's changes come back staged; any other change in the tree
    // would be staged with them and could not be told apart.
    let status = run(&["status", "--porcelain"], repo).map_err(io)?;
    if !status.success {
        return Err(status.stderr.trim().to_string());
    }
    if !status.stdout.trim().is_empty() {
        return Err("the working tree has changes since; commit or discard them first".into());
    }
    let reset = run(&["reset", "--soft", parent], repo).map_err(io)?;
    if reset.success {
        Ok(())
    } else {
        Err(reset.stderr.trim().to_string())
    }
}

/// Switch the workspace to branch `name`, creating it at the current
/// head when `create`.
pub fn checkout_branch(repo: &Path, name: &str, create: bool) -> std::io::Result<GitOpResult> {
    if create {
        run(&["checkout", "-b", name], repo)
    } else {
        run(&["checkout", name], repo)
    }
}

/// Settle a conflicted path by taking one side whole and staging it.
pub fn take_conflict_side(repo: &Path, path: &str, ours: bool) -> std::io::Result<GitOpResult> {
    let side = if ours { "--ours" } else { "--theirs" };
    let taken = run(&["checkout", side, "--", path], repo)?;
    if !taken.success {
        return Ok(taken);
    }
    run(&["add", "--", path], repo)
}

pub fn add_path(repo: &Path, path: &str) -> std::io::Result<GitOpResult> {
    run(&["add", "--", path], repo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Cmd;
    use tempfile::tempdir;

    fn init_repo(dir: &Path) {
        Cmd::new("git")
            .args(["init", "-q", "--initial-branch=main"])
            .current_dir(dir)
            .output()
            .unwrap();
        Cmd::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        Cmd::new("git")
            .args(["config", "user.name", "test"])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    #[test]
    fn add_then_commit_creates_commit() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let r = commit(dir.path(), "initial", true).unwrap();
        assert!(r.success, "stderr: {}", r.stderr);
    }

    #[test]
    fn fetch_with_no_remote_fails_gracefully() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let r = fetch(dir.path(), None).unwrap();
        // No `origin` configured → non-zero exit; stderr explains.
        assert!(!r.success);
    }
}
