//! Cross-stream divergence / merge-readiness computation.
//!
//! Given an integration branch (`base`, usually `main`) and a stream's
//! branch (`head`), compute how far the two have diverged and whether a
//! merge of `head` into `base` is likely to be clean. This is the
//! backing computation for the Git Dashboard's "Streams" panel — the
//! thing a developer consults to decide *when to consolidate* parallel
//! streams.
//!
//! The conflict-risk signal is a **file-overlap heuristic**: if any
//! file was changed on both sides since their merge-base, a line-level
//! merge can collide there. It deliberately does not run a trial merge
//! (expensive, and the smart-merge pass already handles the line-noise
//! case at merge time) — naming the overlapping files is enough to make
//! the consolidation call.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

/// Whether merging `head` into `base` looks safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum MergeReadiness {
    /// `head` has no commits beyond `base` — nothing to merge.
    AlreadyIntegrated,
    /// `head` is ahead and no file was touched on both sides since the
    /// merge-base — a merge should apply cleanly.
    Clean,
    /// `head` is ahead and at least one file was touched on both sides —
    /// a merge will likely conflict (see `overlapping_files`).
    Conflict,
}

/// Divergence of a single stream branch (`head`) against the
/// integration branch (`base`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Divergence {
    /// Commits on `head` not reachable from `base`.
    pub ahead: u32,
    /// Commits on `base` not reachable from `head`.
    pub behind: u32,
    /// Files changed on **both** sides since the merge-base, sorted.
    /// Empty unless `readiness` is `Conflict`.
    pub overlapping_files: Vec<String>,
    pub readiness: MergeReadiness,
}

impl Divergence {
    fn integrated() -> Self {
        Divergence {
            ahead: 0,
            behind: 0,
            overlapping_files: vec![],
            readiness: MergeReadiness::AlreadyIntegrated,
        }
    }
}

/// Paths touched between `from` (optional — `None` means the empty
/// tree, i.e. an unrelated history) and `to`.
fn changed_paths(
    repo: &git2::Repository,
    from: Option<git2::Oid>,
    to: git2::Oid,
) -> Result<BTreeSet<String>, git2::Error> {
    let from_tree = match from {
        Some(oid) => Some(repo.find_commit(oid)?.tree()?),
        None => None,
    };
    let to_tree = repo.find_commit(to)?.tree()?;
    let diff = repo.diff_tree_to_tree(from_tree.as_ref(), Some(&to_tree), None)?;
    let mut out = BTreeSet::new();
    for delta in diff.deltas() {
        if let Some(p) = delta.new_file().path().and_then(|p| p.to_str()) {
            out.insert(p.to_string());
        }
        if let Some(p) = delta.old_file().path().and_then(|p| p.to_str()) {
            out.insert(p.to_string());
        }
    }
    Ok(out)
}

/// Compute divergence + merge-readiness of `head` vs `base`. Both are
/// any revspec libgit2 resolves (branch name, sha, tag). Returns
/// "already integrated" zeros on any lookup failure so a missing /
/// unresolvable branch never breaks the dashboard row.
pub fn compute_divergence(repo_path: &Path, base: &str, head: &str) -> Divergence {
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return Divergence::integrated();
    };
    let resolve =
        |name: &str| -> Option<git2::Oid> { repo.revparse_single(name).ok().map(|o| o.id()) };
    let (Some(base_oid), Some(head_oid)) = (resolve(base), resolve(head)) else {
        return Divergence::integrated();
    };

    let (ahead, behind) = repo
        .graph_ahead_behind(head_oid, base_oid)
        .map(|(a, b)| (a as u32, b as u32))
        .unwrap_or((0, 0));

    if ahead == 0 {
        return Divergence {
            ahead: 0,
            behind,
            overlapping_files: vec![],
            readiness: MergeReadiness::AlreadyIntegrated,
        };
    }

    // Files changed on each side since the common ancestor. Unrelated
    // histories (no merge-base) diff against the empty tree on both
    // sides, so every shared path counts as an overlap.
    let merge_base = repo.merge_base(base_oid, head_oid).ok();
    let head_files = changed_paths(&repo, merge_base, head_oid).unwrap_or_default();
    let base_files = changed_paths(&repo, merge_base, base_oid).unwrap_or_default();

    let overlapping_files: Vec<String> = head_files.intersection(&base_files).cloned().collect();
    let readiness = if overlapping_files.is_empty() {
        MergeReadiness::Clean
    } else {
        MergeReadiness::Conflict
    };

    Divergence {
        ahead,
        behind,
        overlapping_files,
        readiness,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Cmd;
    use tempfile::tempdir;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Cmd::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    fn init_repo(dir: &Path) {
        git(dir, &["init", "-q", "--initial-branch=main"]);
        git(dir, &["config", "user.email", "t@e.com"]);
        git(dir, &["config", "user.name", "t"]);
    }

    fn write(dir: &Path, path: &str, body: &str) {
        std::fs::write(dir.join(path), body).unwrap();
    }

    fn commit(dir: &Path, msg: &str) {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", msg]);
    }

    #[test]
    fn already_integrated_when_head_equals_base() {
        let dir = tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        write(p, "a.txt", "a");
        commit(p, "init");

        let d = compute_divergence(p, "main", "main");
        assert_eq!(d.ahead, 0);
        assert_eq!(d.readiness, MergeReadiness::AlreadyIntegrated);
    }

    #[test]
    fn clean_when_disjoint_files() {
        let dir = tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        write(p, "shared.txt", "base");
        commit(p, "init");

        // feature branch edits a new file only.
        git(p, &["checkout", "-q", "-b", "feature"]);
        write(p, "feature.txt", "f");
        commit(p, "feature work");

        // main advances on a different file.
        git(p, &["checkout", "-q", "main"]);
        write(p, "main-only.txt", "m");
        commit(p, "main work");

        let d = compute_divergence(p, "main", "feature");
        assert_eq!(d.ahead, 1, "feature is 1 commit ahead of main");
        assert_eq!(d.behind, 1, "feature is 1 commit behind main");
        assert_eq!(d.readiness, MergeReadiness::Clean);
        assert!(d.overlapping_files.is_empty());
    }

    #[test]
    fn conflict_names_overlapping_files() {
        let dir = tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        write(p, "shared.txt", "v0");
        write(p, "other.txt", "x");
        commit(p, "init");

        git(p, &["checkout", "-q", "-b", "feature"]);
        write(p, "shared.txt", "feature-edit");
        write(p, "feature.txt", "f");
        commit(p, "feature edits shared");

        git(p, &["checkout", "-q", "main"]);
        write(p, "shared.txt", "main-edit");
        commit(p, "main edits shared");

        let d = compute_divergence(p, "main", "feature");
        assert_eq!(d.ahead, 1);
        assert_eq!(d.behind, 1);
        assert_eq!(d.readiness, MergeReadiness::Conflict);
        assert_eq!(d.overlapping_files, vec!["shared.txt".to_string()]);
    }

    #[test]
    fn behind_only_is_already_integrated() {
        // head is an ancestor of base: base advanced, head didn't.
        let dir = tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        write(p, "a.txt", "a");
        commit(p, "init");
        git(p, &["checkout", "-q", "-b", "feature"]);
        // feature stays put; main advances.
        git(p, &["checkout", "-q", "main"]);
        write(p, "b.txt", "b");
        commit(p, "main advances");

        let d = compute_divergence(p, "main", "feature");
        assert_eq!(d.ahead, 0);
        assert_eq!(d.behind, 1);
        assert_eq!(d.readiness, MergeReadiness::AlreadyIntegrated);
    }

    #[test]
    fn unresolvable_head_is_integrated_zeros() {
        let dir = tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        write(p, "a.txt", "a");
        commit(p, "init");

        let d = compute_divergence(p, "main", "does-not-exist");
        assert_eq!(d, Divergence::integrated());
    }
}
