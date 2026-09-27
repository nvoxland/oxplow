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
        let r = b.into_reference();
        let full = r.name().unwrap_or("").to_string();
        let head = r.peel_to_commit().ok().map(|c| c.id().to_string());
        match ty {
            git2::BranchType::Local => out.push(BranchRef {
                kind: BranchRefKind::Local,
                name: short,
                ref_: full,
                remote: None,
                head,
            }),
            git2::BranchType::Remote => {
                let (remote, name) = split_remote(&short);
                out.push(BranchRef {
                    kind: BranchRefKind::Remote,
                    name: name.to_string(),
                    ref_: full,
                    remote: Some(remote.to_string()),
                    head,
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
