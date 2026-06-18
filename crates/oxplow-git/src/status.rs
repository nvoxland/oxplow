//! `git status`-like inspection via libgit2. Returns a path → status
//! map shaped to feed `workspace::list_workspace_entries`.

use std::collections::HashMap;
use std::path::Path;

use crate::workspace::GitFileStatus;

/// Map every changed/untracked path under `repo_path` to its
/// classification. Fast-path: empty map if not a git repo.
pub fn list_git_statuses(repo_path: &Path) -> HashMap<String, GitFileStatus> {
    let mut out = HashMap::new();
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return out;
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .include_ignored(false)
        .include_unmodified(false)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);
    let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
        return out;
    };
    for entry in statuses.iter() {
        let Some(path) = entry.path().map(|s| s.to_string()) else {
            continue;
        };
        let s = entry.status();
        // Classify in priority order matching the TS contract.
        // Untracked vs Added: Untracked = worktree-only-new (no index
        // entry). Added = staged-new (INDEX_NEW set). Modified covers
        // both index-modified and worktree-modified cases. Deleted
        // and Renamed surface their own flags.
        let classification = if s.contains(git2::Status::INDEX_NEW) {
            GitFileStatus::Added
        } else if s.contains(git2::Status::WT_NEW) {
            GitFileStatus::Untracked
        } else if s.contains(git2::Status::WT_DELETED) || s.contains(git2::Status::INDEX_DELETED) {
            GitFileStatus::Deleted
        } else if s.contains(git2::Status::WT_RENAMED) || s.contains(git2::Status::INDEX_RENAMED) {
            GitFileStatus::Renamed
        } else if s.contains(git2::Status::WT_MODIFIED) || s.contains(git2::Status::INDEX_MODIFIED)
        {
            GitFileStatus::Modified
        } else {
            continue;
        };
        out.insert(path, classification);
    }
    out
}

/// Single-path status lookup. Convenience wrapper around the bulk
/// call; not optimized for hot-path use.
pub fn status_for_path(repo_path: &Path, target: &str) -> Option<GitFileStatus> {
    list_git_statuses(repo_path).get(target).copied()
}

/// Map every **working-tree-clean tracked file** to its HEAD blob OID
/// (40-char hex). A path is included only when it exists in the HEAD
/// tree as a blob AND git reports no change/untracked status for it —
/// i.e. the bytes on disk are byte-identical to the committed blob.
///
/// This powers git-sourced snapshot baselines: the capture path can
/// record `storage = 'git'` with this OID instead of copying the bytes
/// into oxplow's blob store, because they're already in the git object
/// db and recoverable via `find_blob`. Empty map when not a git repo or
/// HEAD is unborn (a repo with no commits backs everything the old way).
pub fn clean_head_blob_oids(repo_path: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return out;
    };
    // Unborn branch / no HEAD commit → nothing committed to lean on.
    let Ok(commit) = repo.head().and_then(|h| h.peel_to_commit()) else {
        return out;
    };
    let Ok(tree) = commit.tree() else {
        return out;
    };
    // Walk the whole HEAD tree once, collecting blob OIDs per path.
    // `root` is the directory prefix ending in `/` ("" at the top).
    let _ = tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
        if entry.kind() == Some(git2::ObjectType::Blob) {
            if let Some(name) = entry.name() {
                out.insert(format!("{root}{name}"), entry.id().to_string());
            }
        }
        git2::TreeWalkResult::Ok
    });
    // Drop any *tracked* path that differs from HEAD on disk. We
    // deliberately do NOT enumerate untracked files: they're never in the
    // HEAD tree (so never in `out`), and `include_untracked(true)` would
    // force libgit2 to scan the entire untracked working tree — on a big
    // repo that's hundreds of thousands of files of pure overhead. With
    // untracked+ignored off, status only diffs tracked entries, which is
    // all we need to know "is this committed file still byte-clean."
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(false)
        .include_ignored(false)
        .renames_head_to_index(false)
        .renames_index_to_workdir(false);
    if let Ok(statuses) = repo.statuses(Some(&mut opts)) {
        for entry in statuses.iter() {
            if let Some(path) = entry.path() {
                out.remove(path);
            }
        }
    }
    out
}

thread_local! {
    /// Per-thread cache of opened repositories keyed by repo path. A loop
    /// reader — chiefly the search indexer materializing thousands of
    /// git-backed snapshot rows — would otherwise pay a fresh
    /// `Repository::open` (config read + repo discovery) per file. The
    /// handle is only ever borrowed *within* a single synchronous
    /// `read_blob` call, never held across an `.await`, so it stays off
    /// the `Send` futures and never crosses threads.
    static REPO_CACHE: std::cell::RefCell<HashMap<std::path::PathBuf, git2::Repository>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Read the raw bytes of a git blob by its OID (40-char hex). `None`
/// when not a git repo, the OID doesn't parse, or the object is absent
/// from the odb (e.g. history was rewritten and the blob got GC'd —
/// the bytes are genuinely unrecoverable, which the caller surfaces as
/// "content unavailable"). The object db is shared across all worktrees
/// of a repo, so any worktree's `repo_path` resolves a committed blob.
///
/// The opened repository is cached per thread (see `REPO_CACHE`) so
/// repeated reads against the same repo don't re-open it. Git-backed
/// snapshot rows only ever reference boot-era committed blobs, so a
/// repo opened during the session always sees them.
pub fn read_blob(repo_path: &Path, oid_hex: &str) -> Option<Vec<u8>> {
    let oid = git2::Oid::from_str(oid_hex).ok()?;
    REPO_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(repo_path) {
            cache.insert(repo_path.to_path_buf(), git2::Repository::open(repo_path).ok()?);
        }
        let blob = cache.get(repo_path)?.find_blob(oid).ok()?;
        Some(blob.content().to_vec())
    })
}

/// Resolved 40-char sha for `HEAD`. `None` when not a git repo or
/// when HEAD is unborn / detached at no commit.
pub fn head_commit_sha(repo_path: &Path) -> Option<String> {
    let repo = git2::Repository::open(repo_path).ok()?;
    let head = repo.head().ok()?;
    let oid = head.target()?;
    Some(oid.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "t@example.com").unwrap();
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
    fn untracked_file_classified_as_untracked() {
        let dir = init_repo();
        std::fs::write(dir.path().join("new.txt"), "x").unwrap();
        let s = list_git_statuses(dir.path());
        assert_eq!(s.get("new.txt"), Some(&GitFileStatus::Untracked));
    }

    #[test]
    fn modified_file_classified_as_modified() {
        let dir = init_repo();
        let repo = git2::Repository::open(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.txt"), "v1").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("a.txt")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = repo.signature().unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add a", &tree, &[&head])
            .unwrap();
        // Refresh the in-memory index from disk so the post-commit
        // state matches the file system before we compare.
        let mut idx = repo.index().unwrap();
        idx.read(true).unwrap();
        std::fs::write(dir.path().join("a.txt"), "v2").unwrap();
        let s = list_git_statuses(dir.path());
        assert_eq!(s.get("a.txt"), Some(&GitFileStatus::Modified));
    }

    /// Commit `files` (path, content) onto HEAD in `dir`.
    fn commit_files(dir: &Path, files: &[(&str, &str)]) {
        let repo = git2::Repository::open(dir).unwrap();
        let mut idx = repo.index().unwrap();
        for (path, content) in files {
            let full = dir.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, content).unwrap();
            idx.add_path(Path::new(path)).unwrap();
        }
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = repo.signature().unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "more", &tree, &[&head])
            .unwrap();
    }

    #[test]
    fn clean_head_blob_oids_includes_clean_excludes_dirty_and_untracked() {
        let dir = init_repo();
        commit_files(dir.path(), &[("a.txt", "alpha"), ("sub/b.txt", "beta")]);

        // Modify a.txt on disk (dirty), add an untracked file.
        std::fs::write(dir.path().join("a.txt"), "alpha-changed").unwrap();
        std::fs::write(dir.path().join("c.txt"), "gamma").unwrap();

        let oids = clean_head_blob_oids(dir.path());

        // Clean nested tracked file → present, with its real blob OID.
        let repo = git2::Repository::open(dir.path()).unwrap();
        let expect_b = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .tree()
            .unwrap()
            .get_path(Path::new("sub/b.txt"))
            .unwrap()
            .id()
            .to_string();
        assert_eq!(oids.get("sub/b.txt"), Some(&expect_b));

        // Dirty tracked file and untracked file → excluded.
        assert!(!oids.contains_key("a.txt"), "dirty file must be excluded");
        assert!(
            !oids.contains_key("c.txt"),
            "untracked file must be excluded"
        );
    }

    #[test]
    fn read_blob_round_trips_clean_content() {
        let dir = init_repo();
        commit_files(dir.path(), &[("doc.md", "# hello\nworld\n")]);
        let oids = clean_head_blob_oids(dir.path());
        let oid = oids.get("doc.md").expect("clean file present");
        let bytes = read_blob(dir.path(), oid).expect("blob readable");
        assert_eq!(bytes, b"# hello\nworld\n");
    }

    #[test]
    fn read_blob_none_for_missing_oid() {
        let dir = init_repo();
        // Well-formed but absent OID → None, not a panic.
        assert!(read_blob(dir.path(), "0123456789abcdef0123456789abcdef01234567").is_none());
        assert!(read_blob(dir.path(), "not-a-hash").is_none());
    }

    #[test]
    fn clean_head_blob_oids_empty_for_unborn_repo() {
        let dir = tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("x.txt"), "x").unwrap();
        assert!(clean_head_blob_oids(dir.path()).is_empty());
    }
}
