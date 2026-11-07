//! `git status`-like inspection via libgit2. Returns a path → status
//! map (and the status counts the rail shows).

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum GitFileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
}

/// What one pass over the working tree says of its files.
#[derive(Debug, Default, Clone)]
pub struct WorktreeStatus {
    /// Every changed/untracked path and its classification.
    pub statuses: HashMap<String, GitFileStatus>,
    /// Every path with a change in the index (`git add`ed).
    pub staged: std::collections::HashSet<String>,
    /// Every path left unmerged, sorted.
    pub conflicted: Vec<String>,
    /// Both ends of every rename. An entry names only one.
    pub renamed: std::collections::HashSet<String>,
}

/// The statuses, the staged paths and the unmerged paths of `repo_path` in
/// one scan of its tree (a scan is the cost: the three were once three
/// scans, 30 ms apiece here). Empty when not a git repo.
pub fn worktree_status(repo_path: &Path) -> WorktreeStatus {
    let mut opts = scan_options();
    opts.renames_head_to_index(true)
        .renames_index_to_workdir(true);
    scan(repo_path, &mut opts)
}

/// [`worktree_status`] of only `paths` (from the root, as git names them):
/// what the scan says of each, and of nothing else. Renames are not looked
/// for, since a rename needs both of its ends in view: a renamed file shows
/// as a deletion and an addition. Costs what the paths ask, not the size of
/// the tree: git is told which files to look at, and looks at those. A
/// folder named as a path brings what changed under it.
pub fn worktree_status_of(repo_path: &Path, paths: &[String]) -> WorktreeStatus {
    if paths.is_empty() {
        return WorktreeStatus::default();
    }
    let mut opts = scan_options();
    // The paths as they are written, not as patterns, so a folder asked
    // about names none of what is inside it.
    opts.disable_pathspec_match(true);
    for path in paths {
        opts.pathspec(path);
    }
    scan(repo_path, &mut opts)
}

/// What every scan looks for.
fn scan_options() -> git2::StatusOptions {
    let mut opts = git2::StatusOptions::new();
    // A new folder's files by their own paths, not the folder (`lib/`).
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .include_unmodified(false);
    opts
}

/// One scan of `repo_path` with `opts`, classified.
fn scan(repo_path: &Path, opts: &mut git2::StatusOptions) -> WorktreeStatus {
    let mut out = WorktreeStatus::default();
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return out;
    };
    let Ok(statuses) = repo.statuses(Some(opts)) else {
        return out;
    };
    let in_index = git2::Status::INDEX_NEW
        | git2::Status::INDEX_MODIFIED
        | git2::Status::INDEX_DELETED
        | git2::Status::INDEX_RENAMED
        | git2::Status::INDEX_TYPECHANGE;
    for entry in statuses.iter() {
        let Ok(path) = entry.path().map(|s| s.to_string()) else {
            continue;
        };
        let s = entry.status();
        if s.contains(git2::Status::CONFLICTED) {
            out.conflicted.push(path);
            continue;
        }
        if s.intersects(in_index) {
            out.staged.insert(path.clone());
        }
        if s.intersects(git2::Status::INDEX_RENAMED | git2::Status::WT_RENAMED) {
            for delta in [entry.head_to_index(), entry.index_to_workdir()]
                .into_iter()
                .flatten()
            {
                for file in [delta.old_file(), delta.new_file()] {
                    if let Some(end) = file.path().and_then(Path::to_str) {
                        out.renamed.insert(end.to_string());
                    }
                }
            }
        }
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
        out.statuses.insert(path, classification);
    }
    out.conflicted.sort();
    out
}

/// Map every changed/untracked path under `repo_path` to its
/// classification. Fast-path: empty map if not a git repo.
pub fn list_git_statuses(repo_path: &Path) -> HashMap<String, GitFileStatus> {
    worktree_status(repo_path).statuses
}

/// Every path with a change in the index (`git add`ed): new, modified,
/// deleted or renamed against HEAD. Empty when not a repo.
pub fn list_staged_paths(repo_path: &Path) -> std::collections::HashSet<String> {
    worktree_status(repo_path).staged
}

/// mtime as `(unix seconds, nanoseconds)` — compared at nanosecond
/// precision (like git) so a checkout that lands files and the index in
/// the same wall-clock second isn't spuriously treated as racy.
type Mtime = (i64, u32);

/// A git-clean decision oracle built once per snapshot sweep, then
/// queried per file from the stat the sweep's walk already computed —
/// so determining "is this committed file still byte-clean?" costs no
/// extra `stat`/`status` pass (that's the fusion that replaces a full
/// libgit2 `statuses()` scan of every tracked file).
///
/// It reuses git's own stat-shortcut: a working file is clean vs the
/// index when its `(size, mtime)` matches the index's cached stat, and
/// the file's mtime is strictly older than the index file itself (git's
/// "racy-clean" rule — a file touched in the same second the index was
/// written can't be trusted by stat alone). "Clean vs HEAD" additionally
/// requires the index entry's OID to equal HEAD's (not staged-modified).
///
/// Conservative by construction: any uncertainty returns `None`, so the
/// caller falls back to reading + hashing the content. A genuinely
/// modified file is therefore never mistaken for clean.
#[derive(Default)]
pub struct GitCleanBaseline {
    /// path → HEAD blob OID hex, for tracked files whose index entry
    /// matches HEAD (the only git-back candidates).
    head_oids: HashMap<String, String>,
    /// path → (index `file_size`, index cached mtime).
    index_stat: HashMap<String, (u64, Mtime)>,
    /// mtime of the `.git/index` file — the racy-clean threshold. A
    /// working file whose mtime is `>=` this can't be trusted by stat
    /// alone. `(0, 0)` disables git-backing (treated as "always racy").
    index_mtime: Mtime,
}

impl GitCleanBaseline {
    /// Build from the repo's HEAD tree + index. In-memory tree walk plus
    /// one index read — no working-tree `stat`s. Empty (backs nothing)
    /// when not a git repo or HEAD is unborn.
    pub fn build(repo_path: &Path) -> Self {
        let mut out = GitCleanBaseline::default();
        let Ok(repo) = git2::Repository::open(repo_path) else {
            return out;
        };
        let Ok(commit) = repo.head().and_then(|h| h.peel_to_commit()) else {
            return out;
        };
        let Ok(tree) = commit.tree() else {
            return out;
        };
        // HEAD tree → path → OID (in-memory).
        let mut head_tree: HashMap<String, String> = HashMap::new();
        let _ = tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
            if entry.kind() == Some(git2::ObjectType::Blob) {
                if let Ok(name) = entry.name() {
                    head_tree.insert(format!("{root}{name}"), entry.id().to_string());
                }
            }
            git2::TreeWalkResult::Ok
        });
        // Index: keep only stage-0 entries whose OID matches HEAD (i.e.
        // not staged-modified) — those are the candidates for clean-vs-HEAD.
        if let Ok(index) = repo.index() {
            for e in index.iter() {
                let stage = (e.flags >> 12) & 0x3;
                if stage != 0 {
                    continue;
                }
                let path = String::from_utf8_lossy(&e.path).into_owned();
                let oid = e.id.to_string();
                if head_tree.get(&path) == Some(&oid) {
                    let mtime = (e.mtime.seconds() as i64, e.mtime.nanoseconds());
                    out.index_stat
                        .insert(path.clone(), (e.file_size as u64, mtime));
                    out.head_oids.insert(path, oid);
                }
            }
        }
        // mtime of the index file itself (`.git[/worktrees/<n>]/index`).
        if let Ok(md) = std::fs::metadata(repo.path().join("index")) {
            if let Ok(mt) = md.modified() {
                if let Ok(d) = mt.duration_since(std::time::UNIX_EPOCH) {
                    out.index_mtime = (d.as_secs() as i64, d.subsec_nanos());
                }
            }
        }
        out
    }

    /// HEAD blob OID for `rel` when the working file (given its already-
    /// computed `size` + `mtime`) is confidently byte-clean vs HEAD.
    /// `None` whenever the stat-shortcut can't be trusted — the caller
    /// then reads + hashes the bytes instead.
    pub fn clean_head_oid(&self, rel: &str, size: u64, mtime: Mtime) -> Option<&str> {
        if self.index_mtime == (0, 0) {
            return None;
        }
        let head = self.head_oids.get(rel)?;
        let (isize, imtime) = self.index_stat.get(rel)?;
        // Stat must match the index's cached stat exactly...
        if *isize != size || *imtime != mtime {
            return None;
        }
        // ...and not be racy (mtime strictly older than the index write,
        // compared at nanosecond precision).
        if mtime >= self.index_mtime {
            return None;
        }
        Some(head.as_str())
    }

    /// Number of git-back candidate paths (clean-in-index tracked files).
    pub fn candidate_count(&self) -> usize {
        self.head_oids.len()
    }
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
            cache.insert(
                repo_path.to_path_buf(),
                git2::Repository::open(repo_path).ok()?,
            );
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

    /// Paths whose change is in the index (`git add`ed) are listed; an
    /// edit not yet added, and an untracked file, are not.
    #[test]
    fn staged_paths_are_those_with_a_change_in_the_index() {
        let dir = init_repo();
        let repo = git2::Repository::open(dir.path()).unwrap();
        std::fs::write(dir.path().join("added.txt"), "a").unwrap();
        std::fs::write(dir.path().join("loose.txt"), "l").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("added.txt")).unwrap();
        index.write().unwrap();
        let staged = list_staged_paths(dir.path());
        assert!(staged.contains("added.txt"), "{staged:?}");
        assert!(!staged.contains("loose.txt"), "{staged:?}");
    }

    /// One scan says what three scans did: the statuses, the staged paths
    /// and the unmerged ones.
    #[test]
    fn one_scan_gives_statuses_staged_and_conflicted() {
        let dir = init_repo();
        let repo = git2::Repository::open(dir.path()).unwrap();
        std::fs::write(dir.path().join("added.txt"), "a").unwrap();
        std::fs::write(dir.path().join("loose.txt"), "l").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("added.txt")).unwrap();
        index.write().unwrap();
        let scan = worktree_status(dir.path());
        assert_eq!(scan.statuses.get("added.txt"), Some(&GitFileStatus::Added));
        assert_eq!(
            scan.statuses.get("loose.txt"),
            Some(&GitFileStatus::Untracked)
        );
        assert_eq!(scan.staged, ["added.txt".to_string()].into());
        assert!(scan.conflicted.is_empty());
        assert!(worktree_status(&dir.path().join("nowhere"))
            .statuses
            .is_empty());
    }

    #[test]
    fn untracked_file_classified_as_untracked() {
        let dir = init_repo();
        std::fs::write(dir.path().join("new.txt"), "x").unwrap();
        let s = list_git_statuses(dir.path());
        assert_eq!(s.get("new.txt"), Some(&GitFileStatus::Untracked));
    }

    /// A file in a folder git doesn't track yet is listed by its own path,
    /// not as the folder (`lib/`), which no caller can open or diff.
    #[test]
    fn an_untracked_folders_files_are_listed_by_their_paths() {
        let dir = init_repo();
        std::fs::create_dir_all(dir.path().join("lib/deep")).unwrap();
        std::fs::write(dir.path().join("lib/deep/b.rs"), "b").unwrap();
        let s = list_git_statuses(dir.path());
        assert_eq!(
            s.get("lib/deep/b.rs"),
            Some(&GitFileStatus::Untracked),
            "{s:?}"
        );
        assert!(!s.contains_key("lib/"), "{s:?}");
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

    /// A repo with every kind of state: an edited, a staged, a deleted and
    /// a clean tracked file, an untracked file, one in an untracked folder,
    /// and an edit and an untracked file outside what is asked about.
    fn varied_repo() -> tempfile::TempDir {
        let dir = init_repo();
        commit_files(
            dir.path(),
            &[
                ("edited.txt", "1"),
                ("deleted.txt", "1"),
                ("clean.txt", "1"),
                ("other_edit.txt", "1"),
                ("staged_edit.txt", "1"),
            ],
        );
        std::fs::write(dir.path().join("edited.txt"), "2").unwrap();
        std::fs::write(dir.path().join("other_edit.txt"), "2").unwrap();
        std::fs::remove_file(dir.path().join("deleted.txt")).unwrap();
        std::fs::write(dir.path().join("untracked.txt"), "u").unwrap();
        std::fs::write(dir.path().join("other_untracked.txt"), "u").unwrap();
        std::fs::create_dir_all(dir.path().join("lib/deep")).unwrap();
        std::fs::write(dir.path().join("lib/deep/b.rs"), "b").unwrap();
        std::fs::write(dir.path().join("lib/deep/c.rs"), "c").unwrap();
        std::fs::write(dir.path().join("staged_new.txt"), "s").unwrap();
        std::fs::write(dir.path().join("staged_edit.txt"), "2").unwrap();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("staged_new.txt")).unwrap();
        idx.add_path(Path::new("staged_edit.txt")).unwrap();
        idx.write().unwrap();
        dir
    }

    /// Only the paths asked about are named, each as `worktree_status` names
    /// it: an edit, a staged new file and a staged edit, a deletion, an
    /// untracked file, one in an untracked folder; a clean file is absent.
    #[test]
    fn status_of_names_only_those_paths() {
        let dir = varied_repo();
        let asked: Vec<String> = [
            "edited.txt",
            "staged_new.txt",
            "staged_edit.txt",
            "deleted.txt",
            "untracked.txt",
            "lib/deep/b.rs",
            "clean.txt",
            "never_existed.txt",
        ]
        .map(String::from)
        .to_vec();
        let all = worktree_status(dir.path());
        let some = worktree_status_of(dir.path(), &asked);
        for path in &asked {
            assert_eq!(some.statuses.get(path), all.statuses.get(path), "{path}");
            assert_eq!(
                some.staged.contains(path),
                all.staged.contains(path),
                "{path}"
            );
        }
        assert_eq!(
            some.statuses.get("edited.txt"),
            Some(&GitFileStatus::Modified)
        );
        assert_eq!(
            some.statuses.get("staged_new.txt"),
            Some(&GitFileStatus::Added)
        );
        assert_eq!(
            some.statuses.get("deleted.txt"),
            Some(&GitFileStatus::Deleted)
        );
        assert_eq!(
            some.statuses.get("untracked.txt"),
            Some(&GitFileStatus::Untracked)
        );
        assert_eq!(
            some.statuses.get("lib/deep/b.rs"),
            Some(&GitFileStatus::Untracked)
        );
        assert!(!some.statuses.contains_key("clean.txt"));
        assert!(some.staged.contains("staged_new.txt") && some.staged.contains("staged_edit.txt"));
        // Nothing outside the paths: not the other edit, the other untracked
        // files, nor the folder's other file.
        let mut named: Vec<&String> = some.statuses.keys().collect();
        named.sort();
        assert_eq!(
            named,
            vec![
                "deleted.txt",
                "edited.txt",
                "lib/deep/b.rs",
                "staged_edit.txt",
                "staged_new.txt",
                "untracked.txt"
            ]
        );
        assert!(some.conflicted.is_empty() && some.renamed.is_empty());
    }

    /// No paths, no answer; a repository that isn't one, no answer; and a
    /// folder named as a path brings what changed under it, as in git.
    #[test]
    fn status_of_nothing_names_nothing() {
        let dir = varied_repo();
        assert!(worktree_status_of(dir.path(), &[]).statuses.is_empty());
        assert!(
            worktree_status_of(&dir.path().join("nowhere"), &["a".to_string()])
                .statuses
                .is_empty()
        );
        let folder = worktree_status_of(dir.path(), &["lib".to_string()]);
        let mut named: Vec<&String> = folder.statuses.keys().collect();
        named.sort();
        assert_eq!(named, vec!["lib/deep/b.rs", "lib/deep/c.rs"]);
    }

    #[test]
    fn read_blob_round_trips_clean_content() {
        let dir = init_repo();
        commit_files(dir.path(), &[("doc.md", "# hello\nworld\n")]);
        let oid = crate::tree::git_blob_oid(b"# hello\nworld\n").unwrap();
        let bytes = read_blob(dir.path(), &oid).expect("blob readable");
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
    fn git_clean_baseline_decision_logic() {
        // Construct directly to test the pure stat-shortcut decision
        // deterministically (no filesystem mtime races).
        let mut b = GitCleanBaseline::default();
        b.head_oids.insert("a.txt".into(), "OID_A".into());
        b.index_stat.insert("a.txt".into(), (5, (100, 500)));
        b.index_mtime = (200, 0);

        // Exact stat match, not racy → clean.
        assert_eq!(b.clean_head_oid("a.txt", 5, (100, 500)), Some("OID_A"));
        // Size changed → not clean.
        assert_eq!(b.clean_head_oid("a.txt", 6, (100, 500)), None);
        // mtime differs from index cache (even by a nanosecond) → not clean.
        assert_eq!(b.clean_head_oid("a.txt", 5, (100, 501)), None);
        // Unknown path → not clean.
        assert_eq!(b.clean_head_oid("other.txt", 5, (100, 500)), None);

        // Racy: file mtime >= index file mtime → can't trust stat. Here the
        // nanosecond pushes it past the threshold even though seconds match.
        let mut racy = GitCleanBaseline::default();
        racy.head_oids.insert("a.txt".into(), "OID_A".into());
        racy.index_stat.insert("a.txt".into(), (5, (200, 5)));
        racy.index_mtime = (200, 0);
        assert_eq!(racy.clean_head_oid("a.txt", 5, (200, 5)), None);

        // No index mtime (not a git repo) → never clean.
        let empty = GitCleanBaseline::default();
        assert_eq!(empty.clean_head_oid("a.txt", 5, (100, 500)), None);
    }

    #[test]
    fn git_clean_baseline_build_indexes_clean_files() {
        let dir = init_repo();
        commit_files(dir.path(), &[("a.txt", "alpha"), ("sub/b.txt", "beta")]);
        // Stage a modification to a.txt (index OID != HEAD) → excluded.
        std::fs::write(dir.path().join("a.txt"), "alpha-2").unwrap();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("a.txt")).unwrap();
        idx.write().unwrap();

        let b = GitCleanBaseline::build(dir.path());
        // b.txt is committed + unmodified in index → candidate with HEAD OID.
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
        assert_eq!(b.head_oids.get("sub/b.txt"), Some(&expect_b));
        // a.txt is staged-modified (index OID != HEAD) → not a candidate.
        assert!(!b.head_oids.contains_key("a.txt"));
        assert!(b.index_mtime.0 > 0, "index mtime should be populated");
    }
}
