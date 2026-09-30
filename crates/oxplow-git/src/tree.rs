//! Content-tree view of a git commit, for the shared
//! [`oxplow_domain::diff_trees`] comparison.
//!
//! This produces the same `path -> content-id` shape the snapshot store
//! produces (`SqliteSnapshotStore::tree_at`), so snapshot-to-snapshot
//! and commit-to-commit diffs run through one comparison instead of
//! `git diff`. The content id here is the git blob oid. (This is a pure
//! content-identity diff — add / modified / deleted; it does not do
//! git's rename detection.)

use std::collections::BTreeMap;
use std::path::Path;

/// The content tree of `rev` (any revspec libgit2 resolves — sha,
/// branch, tag): `path -> blob oid`. Walks the commit's tree
/// recursively; blobs only (sub-trees are traversed, submodules
/// skipped).
pub fn tree_at_commit(
    repo_path: impl AsRef<Path>,
    rev: &str,
) -> Result<BTreeMap<String, String>, git2::Error> {
    let repo = git2::Repository::open(repo_path.as_ref())?;
    let commit = repo.revparse_single(rev)?.peel_to_commit()?;
    let tree = commit.tree()?;

    let mut out: BTreeMap<String, String> = BTreeMap::new();
    tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
        if entry.kind() == Some(git2::ObjectType::Blob) {
            if let Ok(name) = entry.name() {
                // `root` is "" at the top level, else "dir/.../".
                out.insert(format!("{root}{name}"), entry.id().to_string());
            }
        }
        git2::TreeWalkResult::Ok
    })?;
    Ok(out)
}

/// The blob at `path` in `rev`'s tree — a path lookup, not a walk;
/// `None` for a missing path or a directory.
pub fn blob_at(
    repo_path: impl AsRef<Path>,
    rev: &str,
    path: &str,
) -> Result<Option<String>, git2::Error> {
    let repo = git2::Repository::open(repo_path.as_ref())?;
    let tree = repo.revparse_single(rev)?.peel_to_commit()?.tree()?;
    match tree.get_path(Path::new(path)) {
        Ok(entry) if entry.kind() == Some(git2::ObjectType::Blob) => {
            Ok(Some(entry.id().to_string()))
        }
        Ok(_) => Ok(None),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The git blob oid a byte slice would hash to, *without* writing it to
/// the odb. Lets snapshot-store content (xxh3-keyed) and live
/// working-tree bytes be normalized into the same git-oid identity
/// space a commit tree uses, so a mixed snapshot↔commit (or
/// working-tree) diff compares like-for-like. `None` only if libgit2
/// rejects the hash (it shouldn't for a blob).
pub fn git_blob_oid(bytes: &[u8]) -> Option<String> {
    git2::Oid::hash_object(git2::ObjectType::Blob, bytes)
        .ok()
        .map(|oid| oid.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::ChangeStatus;
    use oxplow_domain::FileChange;
    use std::process::Command as Cmd;
    use tempfile::tempdir;

    fn git(dir: &Path, args: &[&str]) {
        Cmd::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
    }

    fn init_repo(dir: &Path) {
        git(dir, &["init", "-q", "--initial-branch=main"]);
        git(dir, &["config", "user.email", "t@e.com"]);
        git(dir, &["config", "user.name", "t"]);
    }

    fn commit_all(dir: &Path, msg: &str) -> String {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", msg]);
        let out = Cmd::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    #[test]
    fn two_commits_trees_differ_by_add_modify_delete() {
        let dir = tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        std::fs::write(p.join("keep.txt"), "keep").unwrap();
        std::fs::write(p.join("mod.txt"), "v1").unwrap();
        std::fs::write(p.join("gone.txt"), "bye").unwrap();
        let c1 = commit_all(p, "first");

        std::fs::write(p.join("mod.txt"), "v2").unwrap(); // modified
        std::fs::remove_file(p.join("gone.txt")).unwrap(); // deleted
        std::fs::create_dir(p.join("sub")).unwrap();
        std::fs::write(p.join("sub/new.txt"), "new").unwrap(); // added (nested)
        let c2 = commit_all(p, "second");

        let changes = oxplow_domain::diff_trees(
            &tree_at_commit(p, &c1).unwrap(),
            &tree_at_commit(p, &c2).unwrap(),
        );
        assert_eq!(
            changes,
            vec![
                FileChange {
                    path: "gone.txt".into(),
                    status: ChangeStatus::Deleted
                },
                FileChange {
                    path: "mod.txt".into(),
                    status: ChangeStatus::Modified
                },
                FileChange {
                    path: "sub/new.txt".into(),
                    status: ChangeStatus::Added
                },
            ]
        );
        // keep.txt unchanged → omitted.
        assert!(!changes.iter().any(|c| c.path == "keep.txt"));
    }

    #[test]
    fn git_blob_oid_matches_libgit2_blob_write() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let repo = git2::Repository::open(dir.path()).unwrap();
        let bytes = b"hello world\n";
        // Writing the blob to the odb yields the same oid hash_object
        // computes — so git_blob_oid must equal it.
        let written = repo.blob(bytes).unwrap().to_string();
        assert_eq!(git_blob_oid(bytes), Some(written));
    }
}
