use std::path::Path;

/// True when `path` is the top-level of a git repository.
///
/// Matches the TS implementation: a `.git` directory or file plus a
/// matching `git rev-parse --show-toplevel`.
pub fn is_git_repo(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    let Ok(repo) = git2::Repository::open(path) else {
        return false;
    };
    // git2 considers a worktree to be a repo too; we want to be more
    // strict and only return true for the *toplevel* dir of a repo.
    if let Some(workdir) = repo.workdir() {
        let canon_target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let canon_repo = std::fs::canonicalize(workdir).unwrap_or_else(|_| workdir.to_path_buf());
        canon_target == canon_repo
    } else {
        false
    }
}

/// Make `path` a new repository with one empty root commit on its
/// default branch (HEAD born, so a workspace can be opened over it).
pub fn init_repository(path: impl AsRef<Path>) -> Result<(), git2::Error> {
    let repo = git2::Repository::init(path.as_ref())?;
    let sig = git2::Signature::now("oxplow", "oxplow@localhost")?;
    let tree = repo.find_tree(repo.index()?.write_tree()?)?;
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "oxplow: initial commit",
        &tree,
        &[],
    )?;
    Ok(())
}

/// True when `path` is a secondary git worktree (its `.git` is a
/// regular file pointing at the main repo's worktrees/ dir).
///
/// Oxplow refuses to start in such a worktree because it creates its
/// own worktrees as siblings of the main repo, and nesting one inside
/// a user-created worktree makes pane/stream accounting incoherent.
pub fn is_git_worktree(path: impl AsRef<Path>) -> bool {
    let dot_git = path.as_ref().join(".git");
    match std::fs::metadata(&dot_git) {
        Ok(m) => m.is_file(),
        Err(_) => false,
    }
}

/// Current branch name, or `None` when HEAD is detached or not a
/// git repo.
pub fn detect_current_branch(path: impl AsRef<Path>) -> Option<String> {
    let repo = git2::Repository::open(path.as_ref()).ok()?;
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    head.shorthand().ok().map(|s| s.to_string())
}

/// The full commit id `rev` names (a sha or its prefix, a branch, a tag,
/// `HEAD`).
pub fn resolve_revision(path: impl AsRef<Path>, rev: &str) -> Result<String, git2::Error> {
    let repo = git2::Repository::open(path.as_ref())?;
    let commit = repo.revparse_single(rev)?.peel_to_commit()?;
    Ok(commit.id().to_string())
}

/// Where the histories of `a` and `b` fork; `None` when they share no
/// history.
pub fn merge_base(path: impl AsRef<Path>, a: &str, b: &str) -> Result<Option<String>, git2::Error> {
    let repo = git2::Repository::open(path.as_ref())?;
    let a = repo.revparse_single(a)?.peel_to_commit()?.id();
    let b = repo.revparse_single(b)?.peel_to_commit()?.id();
    match repo.merge_base(a, b) {
        Ok(id) => Ok(Some(id.to_string())),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        // Stage an initial commit so HEAD is a real branch.
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
    fn is_git_repo_recognizes_init() {
        let dir = make_repo();
        assert!(is_git_repo(dir.path()));
    }

    #[test]
    fn is_git_repo_false_for_plain_dir() {
        let dir = tempdir().unwrap();
        assert!(!is_git_repo(dir.path()));
    }

    #[test]
    fn is_git_worktree_false_for_main_repo() {
        let dir = make_repo();
        assert!(!is_git_worktree(dir.path()));
    }

    #[test]
    fn detect_current_branch_returns_some() {
        let dir = make_repo();
        let head = detect_current_branch(dir.path());
        // git2 default depends on git config; either "main" or "master"
        // is acceptable. The point is we got a name.
        assert!(matches!(head.as_deref(), Some("main") | Some("master")));
    }

    #[test]
    fn detect_current_branch_none_for_non_repo() {
        let dir = tempdir().unwrap();
        assert!(detect_current_branch(dir.path()).is_none());
    }
}
