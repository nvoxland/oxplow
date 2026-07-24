//! Branch-changes diff: what's different between HEAD and a base
//! ref, including any working-tree wip edits on top.
//!
//! Mirrors the original TS surface: returns a flat list of files
//! with adds/dels counts, and includes untracked files from
//! `status --porcelain`.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use specta::Type;

/// "Where am I?" branch context the UI shows above the diff/log
/// views, plus the live working-tree changeset split by staging
/// state so the renderer can show a unified files-changed list
/// without making a second IPC call.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ChangeScopes {
    pub current_branch: Option<String>,
    pub branch_base: Option<String>,
    pub upstream: Option<String>,
    pub on_default_branch: bool,
    /// Files in the index (staged for commit). Empty when nothing is
    /// staged.
    pub staged: Vec<BranchChangeEntry>,
    /// Files modified or untracked in the working tree relative to
    /// the index. Empty in a clean tree.
    pub unstaged: Vec<BranchChangeEntry>,
}

pub fn get_change_scopes(repo: &Path) -> ChangeScopes {
    let current_branch = crate::repo::detect_current_branch(repo);
    let branch_base = detect_base_branch(repo);
    let upstream = detect_upstream_ref(repo);
    let base_name = branch_base
        .as_deref()
        .and_then(|b| b.strip_prefix("origin/").or(Some(b)));
    let on_default_branch = match (&current_branch, base_name) {
        (Some(cur), Some(base)) => cur == base,
        _ => false,
    };
    let (staged, unstaged) = collect_working_tree_changes(repo);
    ChangeScopes {
        current_branch,
        branch_base,
        upstream,
        on_default_branch,
        staged,
        unstaged,
    }
}

/// Parse `git status --porcelain=v1 --untracked-files=all` into two
/// lists. The first column is index status, the second is worktree
/// status; either non-space puts the file in the matching bucket.
fn collect_working_tree_changes(repo: &Path) -> (Vec<BranchChangeEntry>, Vec<BranchChangeEntry>) {
    if !crate::repo::is_git_repo(repo) {
        return (Vec::new(), Vec::new());
    }
    let raw = match run_capturing(&["status", "--porcelain=v1", "--untracked-files=all"], repo) {
        Some(s) => s,
        None => return (Vec::new(), Vec::new()),
    };
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    for line in raw.lines() {
        if line.len() < 4 {
            continue;
        }
        let bytes = line.as_bytes();
        let index_code = bytes[0] as char;
        let worktree_code = bytes[1] as char;
        let rest = &line[3..];
        // Renames in porcelain v1 read "R<sp><sp>old -> new"; we
        // record the new path and stash the old as original_path.
        let (path, original_path) = if let Some(idx) = rest.find(" -> ") {
            (rest[idx + 4..].to_string(), Some(rest[..idx].to_string()))
        } else {
            (rest.to_string(), None)
        };
        if index_code != ' ' && index_code != '?' {
            staged.push(BranchChangeEntry {
                path: path.clone(),
                original_path: original_path.clone(),
                change: classify(index_code),
                additions: 0,
                deletions: 0,
            });
        }
        if worktree_code != ' ' {
            unstaged.push(BranchChangeEntry {
                path,
                original_path,
                change: if worktree_code == '?' {
                    ChangeKind::Untracked
                } else {
                    classify(worktree_code)
                },
                additions: 0,
                deletions: 0,
            });
        }
    }
    (staged, unstaged)
}

fn classify(code: char) -> ChangeKind {
    match code {
        'A' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'R' => ChangeKind::Renamed,
        'C' => ChangeKind::Copied,
        '?' => ChangeKind::Untracked,
        _ => ChangeKind::Modified,
    }
}

fn detect_base_branch(repo: &Path) -> Option<String> {
    if !crate::repo::is_git_repo(repo) {
        return None;
    }
    for candidate in ["origin/main", "main", "origin/master", "master"] {
        if ref_exists(repo, candidate) {
            return Some(candidate.to_string());
        }
    }
    let out = Command::new("git")
        .args(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .current_dir(repo)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn detect_upstream_ref(repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .current_dir(repo)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn ref_exists(repo: &Path, r#ref: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", r#ref])
        .current_dir(repo)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Untracked,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BranchChangeEntry {
    pub path: String,
    pub original_path: Option<String>,
    pub change: ChangeKind,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BranchChanges {
    pub base_ref: String,
    pub merge_base: Option<String>,
    pub files: Vec<BranchChangeEntry>,
}

fn run_capturing(args: &[&str], cwd: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn list_branch_changes(repo: &Path, base_ref: &str) -> BranchChanges {
    if !crate::repo::is_git_repo(repo) {
        return BranchChanges {
            base_ref: base_ref.to_string(),
            merge_base: None,
            files: vec![],
        };
    }
    let merge_base = run_capturing(&["merge-base", "HEAD", base_ref], repo)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let merge_base_str = match &merge_base {
        Some(s) => s.clone(),
        None => {
            return BranchChanges {
                base_ref: base_ref.to_string(),
                merge_base: None,
                files: vec![],
            }
        }
    };

    let name_status =
        run_capturing(&["diff", "--name-status", "-z", &merge_base_str], repo).unwrap_or_default();
    let numstat =
        run_capturing(&["diff", "--numstat", "-z", &merge_base_str], repo).unwrap_or_default();

    let mut entries = parse_name_status_z(&name_status);
    let counts = parse_numstat_z(&numstat);

    apply_numstat(&mut entries, &counts);

    // Untracked files via status --porcelain
    if let Some(status) = run_capturing(&["status", "--porcelain", "--untracked-files=all"], repo) {
        append_untracked(&mut entries, &status);
    }

    BranchChanges {
        base_ref: base_ref.to_string(),
        merge_base,
        files: entries,
    }
}

/// Attach `--numstat` add/delete counts to the `--name-status` entries.
///
/// Indexed rather than scanned per entry: this ran as a linear `find_map`
/// over `counts` for every entry, which is O(entries × counts) on a diff
/// where both lists are the same size (tsk238). **First occurrence of a
/// path wins**, preserving the original scan's semantics — a plain
/// `HashMap::insert` would flip that to last-wins.
fn apply_numstat(entries: &mut [BranchChangeEntry], counts: &[(String, u32, u32)]) {
    let mut by_path: std::collections::HashMap<&str, (u32, u32)> =
        std::collections::HashMap::with_capacity(counts.len());
    for (path, adds, dels) in counts {
        by_path.entry(path.as_str()).or_insert((*adds, *dels));
    }
    for entry in entries.iter_mut() {
        if let Some(&(adds, dels)) = by_path.get(entry.path.as_str()) {
            entry.additions = adds;
            entry.deletions = dels;
        }
    }
}

/// Append `?? ` paths from `git status --porcelain` that aren't already
/// listed. The membership test was a linear scan of `entries` per line,
/// which `--untracked-files=all` can make expensive on a repo with many
/// untracked files (tsk238). Paths are added to the seen set as they're
/// pushed, so duplicate `??` lines still collapse.
fn append_untracked(entries: &mut Vec<BranchChangeEntry>, status: &str) {
    let mut seen: std::collections::HashSet<String> =
        entries.iter().map(|e| e.path.clone()).collect();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("?? ") {
            if seen.insert(rest.to_string()) {
                entries.push(BranchChangeEntry {
                    path: rest.to_string(),
                    original_path: None,
                    change: ChangeKind::Untracked,
                    additions: 0,
                    deletions: 0,
                });
            }
        }
    }
}

fn parse_name_status_z(raw: &str) -> Vec<BranchChangeEntry> {
    let mut out = Vec::new();
    let mut iter = raw.split('\0').peekable();
    while let Some(status) = iter.next() {
        if status.is_empty() {
            continue;
        }
        let kind_char = status.chars().next().unwrap_or(' ');
        match kind_char {
            'A' | 'M' | 'D' | 'T' => {
                let path = match iter.next() {
                    Some(p) if !p.is_empty() => p,
                    _ => continue,
                };
                out.push(BranchChangeEntry {
                    path: path.to_string(),
                    original_path: None,
                    change: match kind_char {
                        'A' => ChangeKind::Added,
                        'D' => ChangeKind::Deleted,
                        _ => ChangeKind::Modified,
                    },
                    additions: 0,
                    deletions: 0,
                });
            }
            'R' | 'C' => {
                let from = iter.next().unwrap_or("").to_string();
                let to = iter.next().unwrap_or("").to_string();
                if from.is_empty() || to.is_empty() {
                    continue;
                }
                out.push(BranchChangeEntry {
                    path: to,
                    original_path: Some(from),
                    change: if kind_char == 'R' {
                        ChangeKind::Renamed
                    } else {
                        ChangeKind::Copied
                    },
                    additions: 0,
                    deletions: 0,
                });
            }
            _ => {}
        }
    }
    out
}

fn parse_numstat_z(raw: &str) -> Vec<(String, u32, u32)> {
    let mut out = Vec::new();
    let mut iter = raw.split('\0').peekable();
    while let Some(line) = iter.next() {
        if line.is_empty() {
            continue;
        }
        // Format: "<adds>\t<dels>\t<path>" or for renames "<adds>\t<dels>\t" then two NUL-separated paths.
        let mut parts = line.splitn(3, '\t');
        let adds: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let dels: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let path = parts.next().unwrap_or("").to_string();
        if path.is_empty() {
            // Rename: next two NUL-separated tokens are old/new paths.
            let _old = iter.next();
            let new_path = iter.next().unwrap_or("").to_string();
            if !new_path.is_empty() {
                out.push((new_path, adds, dels));
            }
        } else {
            out.push((path, adds, dels));
        }
    }
    out
}

/// Characterization tests for the two merge steps, pinning the exact
/// semantics of the linear scans they replaced (tsk238).
#[cfg(test)]
mod merge_tests {
    use super::*;

    fn entry(path: &str) -> BranchChangeEntry {
        BranchChangeEntry {
            path: path.to_string(),
            original_path: None,
            change: ChangeKind::Modified,
            additions: 0,
            deletions: 0,
        }
    }

    #[test]
    fn numstat_counts_land_on_matching_entries() {
        let mut entries = vec![entry("a.rs"), entry("b.rs")];
        apply_numstat(
            &mut entries,
            &[("b.rs".into(), 3, 4), ("a.rs".into(), 1, 2)],
        );
        assert_eq!((entries[0].additions, entries[0].deletions), (1, 2));
        assert_eq!((entries[1].additions, entries[1].deletions), (3, 4));
    }

    #[test]
    fn entries_without_a_numstat_row_keep_zero_counts() {
        let mut entries = vec![entry("a.rs")];
        apply_numstat(&mut entries, &[("other.rs".into(), 9, 9)]);
        assert_eq!((entries[0].additions, entries[0].deletions), (0, 0));
    }

    /// The linear `find_map` took the FIRST match. A plain
    /// `HashMap::insert` would take the last — this pins the difference.
    #[test]
    fn duplicate_numstat_paths_keep_the_first_occurrence() {
        let mut entries = vec![entry("a.rs")];
        apply_numstat(
            &mut entries,
            &[("a.rs".into(), 1, 1), ("a.rs".into(), 99, 99)],
        );
        assert_eq!((entries[0].additions, entries[0].deletions), (1, 1));
    }

    #[test]
    fn untracked_lines_are_appended_as_untracked_entries() {
        let mut entries = vec![entry("a.rs")];
        append_untracked(&mut entries, "?? new.rs\n M a.rs\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].path, "new.rs");
        assert!(matches!(entries[1].change, ChangeKind::Untracked));
    }

    #[test]
    fn untracked_does_not_duplicate_an_existing_entry() {
        let mut entries = vec![entry("a.rs")];
        append_untracked(&mut entries, "?? a.rs\n");
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].change, ChangeKind::Modified));
    }

    /// The old scan re-checked `entries` as it pushed, so repeated `??`
    /// lines collapsed; the seen-set must too.
    #[test]
    fn repeated_untracked_lines_collapse() {
        let mut entries = vec![];
        append_untracked(&mut entries, "?? dup.rs\n?? dup.rs\n");
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn non_untracked_status_lines_are_ignored() {
        let mut entries = vec![];
        append_untracked(&mut entries, " M a.rs\nA  b.rs\nUU c.rs\n");
        assert!(entries.is_empty());
    }
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
            .args(["config", "user.email", "t@e.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        Cmd::new("git")
            .args(["config", "user.name", "t"])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    fn commit(dir: &Path, msg: &str) {
        Cmd::new("git")
            .args(["add", "-A"])
            .current_dir(dir)
            .output()
            .unwrap();
        Cmd::new("git")
            .args(["commit", "-m", msg])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    #[test]
    fn detects_added_file_against_base() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        commit(dir.path(), "init");
        // Create a feature branch and add a file.
        Cmd::new("git")
            .args(["checkout", "-b", "feat"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::fs::write(dir.path().join("b.txt"), "b").unwrap();
        commit(dir.path(), "add b");
        let changes = list_branch_changes(dir.path(), "main");
        assert!(changes
            .files
            .iter()
            .any(|f| f.path == "b.txt" && f.change == ChangeKind::Added));
    }

    #[test]
    fn detects_untracked_file() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        commit(dir.path(), "init");
        std::fs::write(dir.path().join("u.txt"), "u").unwrap();
        let changes = list_branch_changes(dir.path(), "main");
        assert!(changes
            .files
            .iter()
            .any(|f| f.path == "u.txt" && f.change == ChangeKind::Untracked));
    }

    #[test]
    fn change_scopes_buckets_staged_and_unstaged() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        commit(dir.path(), "init");
        // Stage one file, modify another in the worktree, leave a
        // third untracked.
        std::fs::write(dir.path().join("staged.txt"), "stage").unwrap();
        Cmd::new("git")
            .args(["add", "staged.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::fs::write(dir.path().join("a.txt"), "a-modified").unwrap();
        std::fs::write(dir.path().join("untracked.txt"), "u").unwrap();

        let scopes = get_change_scopes(dir.path());
        assert!(
            scopes.staged.iter().any(|e| e.path == "staged.txt"),
            "staged.txt should appear in `staged`, got {:?}",
            scopes.staged
        );
        assert!(
            scopes.unstaged.iter().any(|e| e.path == "a.txt"),
            "a.txt modification should appear in `unstaged`, got {:?}",
            scopes.unstaged
        );
        assert!(
            scopes
                .unstaged
                .iter()
                .any(|e| e.path == "untracked.txt" && e.change == ChangeKind::Untracked),
            "untracked.txt should appear in `unstaged` as Untracked, got {:?}",
            scopes.unstaged
        );
    }
}
