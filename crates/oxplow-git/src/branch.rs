use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum BranchRefKind {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BranchRef {
    pub kind: BranchRefKind,
    pub name: String,
    /// Full ref name (e.g. `refs/heads/main`).
    pub ref_: String,
    /// Remote name for `kind = Remote`; `None` for locals.
    pub remote: Option<String>,
    /// The commit the branch points at (full sha), when it resolves.
    pub head: Option<String>,
    /// For a local branch that tracks one, the remote-tracking branch
    /// (`origin/main`); `None` otherwise.
    pub upstream: Option<String>,
    /// Commits the branch has that its upstream lacks; 0 without one.
    pub ahead: u32,
    /// Commits its upstream has that the branch lacks; 0 without one.
    pub behind: u32,
}

/// List local + remote branches in a repo. Empty list when not a repo.
pub fn list_branches(path: impl AsRef<Path>) -> Vec<BranchRef> {
    let Ok(repo) = git2::Repository::open(path.as_ref()) else {
        return Vec::new();
    };
    let Ok(branches) = repo.branches(None) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for branch in branches.flatten() {
        let (b, ty) = branch;
        let Ok(Some(short)) = b.name() else { continue };
        let short = short.to_string();
        // A local branch's upstream, and how far the two have parted.
        let upstream = if ty == git2::BranchType::Local {
            b.upstream().ok().and_then(|u| {
                let name = u.name().ok().flatten()?.to_string();
                let theirs = u.get().peel_to_commit().ok()?.id();
                Some((name, theirs))
            })
        } else {
            None
        };
        let r = b.into_reference();
        let full = r.name().unwrap_or("").to_string();
        let head = r.peel_to_commit().ok().map(|c| c.id().to_string());
        let (ahead, behind, upstream) = match (&upstream, r.peel_to_commit().ok()) {
            (Some((name, theirs)), Some(mine)) => {
                let (a, b) = repo
                    .graph_ahead_behind(mine.id(), *theirs)
                    .unwrap_or((0, 0));
                (a as u32, b as u32, Some(name.clone()))
            }
            (Some((name, _)), None) => (0, 0, Some(name.clone())),
            _ => (0, 0, None),
        };
        match ty {
            git2::BranchType::Local => out.push(BranchRef {
                kind: BranchRefKind::Local,
                name: short,
                ref_: full,
                remote: None,
                head,
                upstream,
                ahead,
                behind,
            }),
            git2::BranchType::Remote => {
                let (remote, name) = split_remote(&short);
                out.push(BranchRef {
                    kind: BranchRefKind::Remote,
                    name: name.to_string(),
                    ref_: full,
                    remote: Some(remote.to_string()),
                    head,
                    upstream: None,
                    ahead: 0,
                    behind: 0,
                });
            }
        }
    }
    out
}

fn split_remote(short: &str) -> (&str, &str) {
    match short.split_once('/') {
        Some((r, n)) => (r, n),
        None => ("", short),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn branches_carry_their_head_commit() {
        let dir = init_repo();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let head = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id()
            .to_string();
        let b = list_branches(dir.path());
        assert!(!b.is_empty());
        assert!(
            b.iter().all(|r| r.head.as_deref() == Some(head.as_str())),
            "{b:?}"
        );
    }

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

    /// A local branch with an upstream names it and says how far the two have
    /// parted; one without, and a remote branch, say nothing.
    #[test]
    fn a_local_branch_carries_its_upstream_and_how_far_they_have_parted() {
        let dir = init_repo();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let sig = repo.signature().unwrap();
        let commit_on = |parent: &git2::Commit, msg: &str, update: &str| {
            let tree = parent.tree().unwrap();
            repo.commit(Some(update), &sig, &sig, msg, &tree, &[parent])
                .unwrap()
        };
        let base = repo.head().unwrap().peel_to_commit().unwrap();
        let local = repo.head().unwrap().shorthand().unwrap().to_string();
        // The remote branch is one commit ahead of the base; the local one two ahead of it
        // (so: parted by 2 ahead, 1 behind).
        let remote_tip = commit_on(&base, "remote", "refs/remotes/origin/up");
        let local_one = commit_on(&base, "local 1", &format!("refs/heads/{local}"));
        let local_one = repo.find_commit(local_one).unwrap();
        commit_on(&local_one, "local 2", &format!("refs/heads/{local}"));
        let _ = remote_tip;
        let mut config = repo.config().unwrap();
        config
            .set_str(&format!("branch.{local}.remote"), "origin")
            .unwrap();
        config
            .set_str(&format!("branch.{local}.merge"), "refs/heads/up")
            .unwrap();
        config
            .set_str("remote.origin.url", "file:///nowhere")
            .unwrap();
        config
            .set_str("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")
            .unwrap();
        repo.branch("lonely", &base, false).unwrap();

        let branches = list_branches(dir.path());
        let get = |name: &str, kind: BranchRefKind| {
            branches
                .iter()
                .find(|b| b.name == name && b.kind == kind)
                .unwrap()
        };
        let mine = get(&local, BranchRefKind::Local);
        assert_eq!(mine.upstream.as_deref(), Some("origin/up"), "{branches:?}");
        assert_eq!((mine.ahead, mine.behind), (2, 1));
        let lonely = get("lonely", BranchRefKind::Local);
        assert_eq!(
            (lonely.upstream.as_deref(), lonely.ahead, lonely.behind),
            (None, 0, 0)
        );
        let remote = get("up", BranchRefKind::Remote);
        assert_eq!(
            (remote.upstream.as_deref(), remote.ahead, remote.behind),
            (None, 0, 0)
        );
    }

    #[test]
    fn lists_default_local_branch() {
        let dir = init_repo();
        let branches = list_branches(dir.path());
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].kind, BranchRefKind::Local);
    }

    #[test]
    fn lists_added_branch() {
        let dir = init_repo();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();
        let names: Vec<_> = list_branches(dir.path())
            .into_iter()
            .map(|b| b.name)
            .collect();
        assert!(names.contains(&"feature".to_string()));
    }

    #[test]
    fn empty_for_non_repo() {
        let dir = tempdir().unwrap();
        assert!(list_branches(dir.path()).is_empty());
    }
}
