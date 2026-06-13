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

pub fn commit_all(repo: &Path, message: &str) -> std::io::Result<GitOpResult> {
    let staged = run(&["add", "-A"], repo)?;
    if !staged.success {
        return Ok(staged);
    }
    run(&["commit", "-m", message], repo)
}

pub fn add_path(repo: &Path, path: &str) -> std::io::Result<GitOpResult> {
    run(&["add", "--", path], repo)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TextSearchHit {
    pub path: String,
    pub line: u32,
    pub snippet: String,
}

/// Run `git grep` against the repo's tracked + untracked files.
/// `query` is treated as a fixed string (`-F`), case-sensitive, with
/// line numbers. Caps results at `limit` (default 200, max 1000).
pub fn search_workspace_text(repo: &Path, query: &str, limit: Option<usize>) -> Vec<TextSearchHit> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return vec![];
    }
    let limit = limit.unwrap_or(200).clamp(1, 1000);
    let output = match Command::new("git")
        .args([
            "grep",
            "--no-color",
            "-n",
            "-I",
            "-F",
            "--untracked",
            "--",
            trimmed,
        ])
        .current_dir(repo)
        .output()
    {
        Ok(o) => o,
        Err(_) => return vec![],
    };
    // `git grep` exits 1 when there are no matches — treat as empty.
    if !output.status.success() && output.stdout.is_empty() {
        return vec![];
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let mut hits = Vec::new();
    for line in raw.lines() {
        if hits.len() >= limit {
            break;
        }
        let mut split = line.splitn(3, ':');
        let path = match split.next() {
            Some(p) => p,
            None => continue,
        };
        let line_no: u32 = match split.next().and_then(|s| s.parse().ok()) {
            Some(n) => n,
            None => continue,
        };
        let snippet = match split.next() {
            Some(s) => s,
            None => continue,
        };
        let snippet = if snippet.len() > 400 {
            format!("{}…", &snippet[..400])
        } else {
            snippet.to_string()
        };
        hits.push(TextSearchHit {
            path: path.to_string(),
            line: line_no,
            snippet,
        });
    }
    hits
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
        let r = commit_all(dir.path(), "initial").unwrap();
        assert!(r.success, "stderr: {}", r.stderr);
    }

    #[test]
    fn search_workspace_text_finds_matches() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("a.txt"), "alpha\nbeta\ngamma\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "alpha bravo\n").unwrap();
        let hits = search_workspace_text(dir.path(), "alpha", None);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|h| h.path == "a.txt" && h.line == 1));
    }

    #[test]
    fn search_workspace_text_returns_empty_on_no_match() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("a.txt"), "alpha\n").unwrap();
        let hits = search_workspace_text(dir.path(), "nothere", None);
        assert!(hits.is_empty());
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
