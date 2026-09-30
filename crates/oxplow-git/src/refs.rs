//! Refs + file-at-ref reads.
//!
//! Powers ref-pickers (branch / tag / remote dropdowns), file-history
//! views, and the diff editor's "open this file at HEAD/<sha>" path.

use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::log::GitLogCommit;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RefOption {
    pub label: String,
    pub r#ref: String,
    pub kind: RefKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    Local,
    Remote,
    Tag,
    Head,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GroupedGitRefs {
    pub locals: Vec<RefOption>,
    pub remotes: Vec<RefOption>,
    pub tags: Vec<RefOption>,
}

/// Returns every ref grouped into local branches, remote branches and
/// tags. Omits the `HEAD` symbolic ref since it's never useful as a
/// picker option.
pub fn list_all_refs(repo_path: &Path) -> GroupedGitRefs {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => {
            return GroupedGitRefs {
                locals: vec![],
                remotes: vec![],
                tags: vec![],
            }
        }
    };
    let mut locals = vec![];
    let mut remotes = vec![];
    let mut tags = vec![];
    if let Ok(refs) = repo.references() {
        for r in refs.flatten() {
            let name = match r.name() {
                Ok(n) => n.to_string(),
                Err(_) => continue,
            };
            if name == "HEAD" {
                continue;
            }
            if let Some(short) = name.strip_prefix("refs/heads/") {
                locals.push(RefOption {
                    label: short.to_string(),
                    r#ref: name.clone(),
                    kind: RefKind::Local,
                });
            } else if let Some(short) = name.strip_prefix("refs/remotes/") {
                if short == "origin/HEAD" {
                    continue;
                }
                remotes.push(RefOption {
                    label: short.to_string(),
                    r#ref: name.clone(),
                    kind: RefKind::Remote,
                });
            } else if let Some(short) = name.strip_prefix("refs/tags/") {
                tags.push(RefOption {
                    label: short.to_string(),
                    r#ref: name.clone(),
                    kind: RefKind::Tag,
                });
            }
        }
    }
    locals.sort_by(|a, b| a.label.cmp(&b.label));
    remotes.sort_by(|a, b| a.label.cmp(&b.label));
    tags.sort_by(|a, b| b.label.cmp(&a.label));
    GroupedGitRefs {
        locals,
        remotes,
        tags,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommitRefLabelKind {
    Branch,
    Tag,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct CommitRefLabel {
    pub kind: CommitRefLabelKind,
    pub name: String,
}

/// Given a set of commit SHAs, return every branch + tag that points
/// directly at each sha. Branches come first (sorted by name), then
/// tags (sorted by name). SHAs with no matching ref are omitted —
/// callers fall back to a short-sha chip.
pub fn resolve_commit_ref_labels(
    repo_path: &Path,
    shas: &[String],
) -> std::collections::HashMap<String, Vec<CommitRefLabel>> {
    let mut out: std::collections::HashMap<String, Vec<CommitRefLabel>> =
        std::collections::HashMap::new();
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return out,
    };
    let want: std::collections::HashSet<&str> = shas.iter().map(|s| s.as_str()).collect();
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
        for entry in branches.flatten() {
            let (branch, _) = entry;
            let Ok(Some(name)) = branch.name() else {
                continue;
            };
            let Ok(reference) = branch.get().resolve() else {
                continue;
            };
            let Some(oid) = reference.target() else {
                continue;
            };
            let sha = oid.to_string();
            if want.contains(sha.as_str()) {
                out.entry(sha).or_default().push(CommitRefLabel {
                    kind: CommitRefLabelKind::Branch,
                    name: name.to_string(),
                });
            }
        }
    }
    if let Ok(tag_names) = repo.tag_names(None) {
        for name in tag_names.iter().filter_map(|n| n.ok().flatten()) {
            let full = format!("refs/tags/{name}");
            let Ok(reference) = repo.find_reference(&full) else {
                continue;
            };
            let target_oid = match reference.peel_to_commit() {
                Ok(commit) => commit.id(),
                Err(_) => match reference.target() {
                    Some(oid) => oid,
                    None => continue,
                },
            };
            let sha = target_oid.to_string();
            if want.contains(sha.as_str()) {
                out.entry(sha).or_default().push(CommitRefLabel {
                    kind: CommitRefLabelKind::Tag,
                    name: name.to_string(),
                });
            }
        }
    }
    // Stable ordering: branches (already alphabetical within their
    // pass), then tags. Sort each kind alphabetically to keep the
    // chip order deterministic across calls.
    for labels in out.values_mut() {
        labels.sort_by(|a, b| match (&a.kind, &b.kind) {
            (CommitRefLabelKind::Branch, CommitRefLabelKind::Tag) => std::cmp::Ordering::Less,
            (CommitRefLabelKind::Tag, CommitRefLabelKind::Branch) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });
    }
    out
}

/// List the most recent commits that touched `path`, up to `limit`.
pub fn list_file_commits(repo_path: &Path, path: &str, limit: usize) -> Vec<GitLogCommit> {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    let mut walker = match repo.revwalk() {
        Ok(w) => w,
        Err(_) => return vec![],
    };
    if walker.push_head().is_err() {
        return vec![];
    }
    walker.set_sorting(git2::Sort::TIME).ok();

    let target = Path::new(path);
    let mut out = Vec::new();
    for oid_res in walker {
        let oid = match oid_res {
            Ok(o) => o,
            Err(_) => continue,
        };
        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };
        // Compare with first parent (or empty tree if no parents).
        let touches = touches_path(&repo, &commit, target).unwrap_or(false);
        if !touches {
            continue;
        }
        let sha = commit.id().to_string();
        let short_sha = sha[..7.min(sha.len())].to_string();
        out.push(GitLogCommit {
            sha,
            short_sha,
            author: commit.author().name().unwrap_or("").to_string(),
            email: commit.author().email().unwrap_or("").to_string(),
            timestamp_secs: commit.time().seconds(),
            subject: commit.summary().ok().flatten().unwrap_or("").to_string(),
            parents: commit.parent_ids().map(|p| p.to_string()).collect(),
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn touches_path(
    repo: &git2::Repository,
    commit: &git2::Commit<'_>,
    path: &Path,
) -> Result<bool, git2::Error> {
    let tree = commit.tree()?;
    let parent_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };
    let mut diff_opts = git2::DiffOptions::new();
    diff_opts.pathspec(path);
    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut diff_opts))?;
    Ok(diff.deltas().any(|_| true))
}

/// List recent remote branches sorted by author commit time. Returns
/// up to `limit` entries.
pub fn list_recent_remote_branches(repo_path: &Path, limit: usize) -> Vec<RemoteBranchEntry> {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    let mut entries = Vec::new();
    if let Ok(refs) = repo.references_glob("refs/remotes/*/*") {
        for r in refs.flatten() {
            let name = match r.shorthand() {
                Ok(n) => n.to_string(),
                Err(_) => continue,
            };
            if name.ends_with("/HEAD") {
                continue;
            }
            let commit = match r.peel_to_commit() {
                Ok(c) => c,
                Err(_) => continue,
            };
            entries.push(RemoteBranchEntry {
                ref_name: r.name().unwrap_or("").to_string(),
                short_name: name,
                last_commit_at: commit.time().seconds(),
                last_commit_subject: commit.summary().ok().flatten().unwrap_or("").to_string(),
            });
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.last_commit_at));
    entries.truncate(limit);
    entries
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RemoteBranchEntry {
    pub ref_name: String,
    pub short_name: String,
    pub last_commit_at: i64,
    pub last_commit_subject: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn init_with_commit(dir: &Path, file: &str, content: &str) {
        Command::new("git")
            .args(["init", "-q", "--initial-branch=main"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "t@e.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "t"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::fs::write(dir.join(file), content).unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "msg"])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    #[test]
    fn resolve_commit_ref_labels_lists_branch_and_tag_on_same_sha() {
        let dir = tempdir().unwrap();
        init_with_commit(dir.path(), "a.txt", "x");
        Command::new("git")
            .args(["tag", "v1"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        // A second branch on the same sha so we can verify multiple
        // branches list together.
        Command::new("git")
            .args(["branch", "release"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let head_out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let head = String::from_utf8(head_out.stdout)
            .unwrap()
            .trim()
            .to_string();
        let labels = resolve_commit_ref_labels(dir.path(), std::slice::from_ref(&head));
        let labels = labels.get(&head).expect("head has labels");
        // Branches before tags, alphabetical within each kind.
        assert_eq!(labels.len(), 3);
        assert_eq!(labels[0].kind, CommitRefLabelKind::Branch);
        assert_eq!(labels[0].name, "main");
        assert_eq!(labels[1].kind, CommitRefLabelKind::Branch);
        assert_eq!(labels[1].name, "release");
        assert_eq!(labels[2].kind, CommitRefLabelKind::Tag);
        assert_eq!(labels[2].name, "v1");
    }

    #[test]
    fn resolve_commit_ref_labels_returns_tag_when_no_branch() {
        let dir = tempdir().unwrap();
        init_with_commit(dir.path(), "a.txt", "x");
        Command::new("git")
            .args(["tag", "v1"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let head_out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let head = String::from_utf8(head_out.stdout)
            .unwrap()
            .trim()
            .to_string();
        // Make a second commit so the tag stays on the older sha
        // without a branch.
        Command::new("git")
            .args(["checkout", "-b", "feat"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::fs::write(dir.path().join("b.txt"), "y").unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "msg2"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        // Now delete `main` so the original sha has only the tag.
        Command::new("git")
            .args(["branch", "-D", "main"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let labels = resolve_commit_ref_labels(dir.path(), std::slice::from_ref(&head));
        let labels = labels.get(&head).expect("head has labels");
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].kind, CommitRefLabelKind::Tag);
        assert_eq!(labels[0].name, "v1");
    }

    #[test]
    fn list_all_refs_returns_local_main() {
        let dir = tempdir().unwrap();
        init_with_commit(dir.path(), "a.txt", "x");
        let refs = list_all_refs(dir.path());
        assert!(refs.locals.iter().any(|r| r.label == "main"));
    }

    #[test]
    fn list_file_commits_finds_commit_touching_path() {
        let dir = tempdir().unwrap();
        init_with_commit(dir.path(), "a.txt", "hello");
        let commits = list_file_commits(dir.path(), "a.txt", 10);
        assert_eq!(commits.len(), 1);
    }
}
