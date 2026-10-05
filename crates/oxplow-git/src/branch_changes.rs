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

/// The branch context and the working tree's changes. The context is read
/// in-process (`git2`, the repo opened once); the one git process a call
/// spawns is the working tree's `status`, whose untracked walk uses git's
/// own caches (tsk241).
pub fn get_change_scopes(repo: &Path) -> ChangeScopes {
    let git = crate::repo::is_git_repo(repo)
        .then(|| git2::Repository::open(repo).ok())
        .flatten();
    let current_branch = git.as_ref().and_then(current_branch_of);
    let branch_base = git.as_ref().and_then(detect_base_branch);
    let upstream = git.as_ref().and_then(detect_upstream_ref);
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
    // `-z` is not just a separator change: it turns OFF git's C-quoting.
    // Without it, any path needing quotes (a space, any non-ASCII byte)
    // arrives wrapped in `"` with octal escapes, and storing that
    // verbatim produced phantom directories like `"out` (tsk268).
    //
    // It is also unambiguous by construction: the only two bytes a POSIX
    // filename cannot contain are `/` and NUL, which are exactly the
    // component and record separators here. Nothing needs decoding, so
    // nothing can be decoded wrong.
    let raw = match run_capturing(
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        repo,
    ) {
        Some(s) => s,
        None => return (Vec::new(), Vec::new()),
    };
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let records: Vec<&str> = raw.split('\0').filter(|r| !r.is_empty()).collect();
    let mut i = 0;
    while i < records.len() {
        let record = records[i];
        i += 1;
        if record.len() < 4 {
            continue;
        }
        let bytes = record.as_bytes();
        let index_code = bytes[0] as char;
        let worktree_code = bytes[1] as char;
        let path = record[3..].to_string();
        // Under `-z` a rename/copy has no " -> ": the source path is its
        // own following record, and the one carrying the status codes is
        // the destination.
        let original_path = if matches!(index_code, 'R' | 'C') || matches!(worktree_code, 'R' | 'C')
        {
            let source = records.get(i).map(|s| (*s).to_string());
            if source.is_some() {
                i += 1;
            }
            source
        } else {
            None
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

/// The branch `HEAD` is on, when it's on one.
fn current_branch_of(git: &git2::Repository) -> Option<String> {
    let head = git.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    head.shorthand().ok().map(str::to_string)
}

/// The base the branch's changes are against: the first of `origin/main`,
/// `main`, `origin/master`, `master` that exists, else what `origin`'s HEAD
/// points at (`origin/<branch>`).
fn detect_base_branch(git: &git2::Repository) -> Option<String> {
    for candidate in ["origin/main", "main", "origin/master", "master"] {
        if git.revparse_single(candidate).is_ok() {
            return Some(candidate.to_string());
        }
    }
    let origin_head = git.find_reference("refs/remotes/origin/HEAD").ok()?;
    let target = origin_head.symbolic_target().ok()??;
    target
        .strip_prefix("refs/remotes/")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The current branch's upstream, short (`origin/main`).
fn detect_upstream_ref(git: &git2::Repository) -> Option<String> {
    let name = current_branch_of(git)?;
    let branch = git.find_branch(&name, git2::BranchType::Local).ok()?;
    let upstream = branch.upstream().ok()?;
    upstream.name().ok().flatten().map(str::to_string)
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

/// Characterization tests for the two merge steps, pinning the exact
/// semantics of the linear scans they replaced (tsk238).
#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Cmd;
    use tempfile::tempdir;

    /// `-z` changes how renames are encoded, so pin it.
    ///
    /// Porcelain v1 writes `R<sp><sp>old -> new` on one line; under `-z`
    /// there is no arrow — the record carrying the status codes is the
    /// *destination*, and the source follows as its own NUL-separated
    /// record. Reading it the old way would take the destination as the
    /// source and drop a record, desynchronising everything after it
    /// (tsk268).
    #[test]
    fn a_staged_rename_records_both_paths_under_z() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("before.txt"), b"x").unwrap();
        commit(dir.path(), "base");

        Cmd::new("git")
            .args(["mv", "before.txt", "after.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        // A second staged change after the rename: if the rename ate the
        // wrong number of records, this one lands mangled or missing.
        std::fs::write(dir.path().join("zz-later.txt"), b"y").unwrap();
        Cmd::new("git")
            .args(["add", "zz-later.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let (staged, _unstaged) = collect_working_tree_changes(dir.path());
        let renamed = staged
            .iter()
            .find(|e| e.original_path.is_some())
            .unwrap_or_else(|| panic!("no rename recorded, got {staged:?}"));
        assert_eq!(renamed.path, "after.txt", "destination is the entry path");
        assert_eq!(renamed.original_path.as_deref(), Some("before.txt"));
        assert!(
            staged.iter().any(|e| e.path == "zz-later.txt"),
            "the record after a rename must still parse, got {staged:?}"
        );
    }

    /// Paths git has to quote must come back as the real path.
    ///
    /// `git status --porcelain=v1` C-quotes any path that needs it —
    /// wrapping it in `"` and escaping the contents — which happens for
    /// something as ordinary as a space, and for every non-ASCII name.
    /// Taking the porcelain path verbatim stored the quotes and the
    /// escapes, so `out/Internal Metabase Database/x.yaml` surfaced as a
    /// phantom `"out` directory next to the real one, and the recorded
    /// path pointed at nothing on disk (tsk268).
    #[test]
    fn paths_git_quotes_are_recorded_as_the_real_path() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("plain.txt"), b"x").unwrap();
        commit(dir.path(), "base");

        // A space forces quoting; a non-ASCII byte forces quoting *and*
        // octal escaping, which a naive unquote would still get wrong.
        std::fs::create_dir_all(dir.path().join("out/Internal Database")).unwrap();
        std::fs::write(dir.path().join("out/Internal Database/x.yaml"), b"y").unwrap();
        std::fs::write(dir.path().join("caf\u{e9}.txt"), b"z").unwrap();

        let (_staged, unstaged) = collect_working_tree_changes(dir.path());
        let paths: Vec<&str> = unstaged.iter().map(|e| e.path.as_str()).collect();

        assert!(
            paths.contains(&"out/Internal Database/x.yaml"),
            "spaced path should be recorded unquoted, got {paths:?}"
        );
        assert!(
            paths.contains(&"caf\u{e9}.txt"),
            "non-ASCII path should be recorded decoded, got {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains('"')),
            "no recorded path should carry git's quoting: {paths:?}"
        );
    }

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

    /// A repo on `feat`, with a remote `origin` holding `main` and `feat`
    /// tracking `origin/main`.
    fn repo_with_upstream(dir: &Path) {
        let origin = dir.join("origin.git");
        let work = dir.join("work");
        std::fs::create_dir(&work).unwrap();
        init_repo(&work);
        std::fs::write(work.join("a.txt"), "a").unwrap();
        commit(&work, "init");
        let git = |args: &[&str]| {
            let out = Cmd::new("git")
                .args(args)
                .current_dir(&work)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["branch", "-M", "main"]);
        git(&["clone", "-q", "--bare", ".", origin.to_str().unwrap()]);
        git(&["remote", "add", "origin", origin.to_str().unwrap()]);
        git(&["fetch", "-q", "origin"]);
        git(&["checkout", "-q", "-b", "feat", "--track", "origin/main"]);
    }

    /// The branch context: the current branch, the base the diff is
    /// against (`origin/main` first), the upstream, whether it's the base.
    #[test]
    fn change_scopes_name_the_branch_its_base_and_upstream() {
        let dir = tempdir().unwrap();
        repo_with_upstream(dir.path());
        let scopes = get_change_scopes(&dir.path().join("work"));
        assert_eq!(scopes.current_branch.as_deref(), Some("feat"));
        assert_eq!(scopes.branch_base.as_deref(), Some("origin/main"));
        assert_eq!(scopes.upstream.as_deref(), Some("origin/main"));
        assert!(!scopes.on_default_branch);
    }

    /// tsk241: the branch context is read in-process; the one git process a
    /// call spawns is the working tree's `status` (its untracked walk uses
    /// git's own caches). nextest runs each test in its own process, so the
    /// counting `git` on `PATH` is this test's alone.
    #[test]
    fn change_scopes_spawn_only_the_status() {
        let dir = tempdir().unwrap();
        repo_with_upstream(dir.path());
        let real = Cmd::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap();
        let real = String::from_utf8_lossy(&real.stdout).trim().to_string();
        let shim = dir.path().join("bin");
        std::fs::create_dir(&shim).unwrap();
        let count = dir.path().join("spawned");
        std::fs::write(
            shim.join("git"),
            format!(
                "#!/bin/sh\necho \"$1\" >> '{}'\nexec '{real}' \"$@\"\n",
                count.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(
            shim.join("git"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        let path = format!(
            "{}:{}",
            shim.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        // SAFETY: nextest runs this test alone in its process.
        unsafe { std::env::set_var("PATH", path) };
        get_change_scopes(&dir.path().join("work"));
        let spawned = std::fs::read_to_string(&count).unwrap_or_default();
        assert_eq!(spawned.lines().collect::<Vec<_>>(), vec!["status"]);
    }
}
