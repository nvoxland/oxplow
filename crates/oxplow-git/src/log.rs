//! `git log` + commit detail surface. Replaces the
//! `getGitLog` / `getCommitDetail` exports from
//! `src/git/git.ts`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct GitLogCommit {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    pub email: String,
    pub timestamp_secs: i64,
    pub subject: String,
    pub parents: Vec<String>,
}

/// Minimal commit pointer carried by ref overlays. The renderer only
/// reads `.sha` to bucket refs into the per-row badge map, so the
/// extra fields on `GitLogCommit` would be dead weight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct GitLogRefCommit {
    pub sha: String,
}

/// One branch head or tag tied to a commit. Surfaced on `GitLogResult`
/// so `CommitGraphTable` (and its dashboard re-use in the
/// recent-commits card) can render branch/tag badges next to each row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct GitLogRef {
    pub name: String,
    pub commit: GitLogRefCommit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct GitLogResult {
    pub commits: Vec<GitLogCommit>,
    /// Local branch heads keyed by the commit they point at.
    #[serde(rename = "branchHeads")]
    pub branch_heads: Vec<GitLogRef>,
    /// Tags (lightweight + annotated, dereferenced to their commit).
    pub tags: Vec<GitLogRef>,
}

#[derive(Debug, Clone, Default)]
pub struct GitLogOptions {
    pub limit: Option<usize>,
    /// When true, walks every ref (`--all`); otherwise walks `from`, else HEAD.
    pub all: bool,
    /// Where the walk starts: any revision (a branch, a tag, a sha);
    /// HEAD when absent. Ignored with `all`.
    pub from: Option<String>,
    /// Only the commits that changed this repo-relative path.
    pub path: Option<String>,
    /// Resume after this commit: the walk's commits up to and including it
    /// are skipped (the last sha of the previous page).
    pub after: Option<String>,
}

pub fn get_git_log(repo_path: &Path, options: GitLogOptions) -> GitLogResult {
    let empty = || GitLogResult {
        commits: vec![],
        branch_heads: vec![],
        tags: vec![],
    };
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return empty();
    };
    let mut walk = match repo.revwalk() {
        Ok(w) => w,
        Err(_) => return empty(),
    };
    if options.all {
        let _ = walk.push_glob("refs/*");
    } else if let Some(from) = &options.from {
        let Some(oid) = repo
            .revparse_single(from)
            .ok()
            .and_then(|o| o.peel_to_commit().ok())
            .map(|c| c.id())
        else {
            return empty();
        };
        if walk.push(oid).is_err() {
            return empty();
        }
    } else if walk.push_head().is_err() {
        return empty();
    }
    let _ = walk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL);
    let limit = options.limit.unwrap_or(200);

    let mut skipping = options.after.is_some();
    let mut commits = Vec::with_capacity(limit);
    for oid in walk.flatten() {
        if commits.len() >= limit {
            break;
        }
        if skipping {
            skipping = options.after.as_deref() != Some(oid.to_string().as_str());
            continue;
        }
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        if let Some(path) = &options.path {
            if !touches(&repo, &commit, path) {
                continue;
            }
        }
        let author = commit.author();
        commits.push(GitLogCommit {
            sha: oid.to_string(),
            short_sha: oid.to_string()[..7].to_string(),
            author: author.name().unwrap_or("").to_string(),
            email: author.email().unwrap_or("").to_string(),
            timestamp_secs: commit.time().seconds(),
            subject: commit.summary().ok().flatten().unwrap_or("").to_string(),
            parents: commit.parent_ids().map(|p| p.to_string()).collect(),
        });
    }
    let (branch_heads, tags) = collect_log_refs(&repo);
    GitLogResult {
        commits,
        branch_heads,
        tags,
    }
}

/// Whether `commit` changed `path` against its first parent (a root
/// commit: whether it has the path).
fn touches(repo: &git2::Repository, commit: &git2::Commit, path: &str) -> bool {
    let Ok(tree) = commit.tree() else {
        return false;
    };
    let parent = commit.parent(0).ok().and_then(|p| p.tree().ok());
    let mut opts = git2::DiffOptions::new();
    opts.pathspec(path);
    repo.diff_tree_to_tree(parent.as_ref(), Some(&tree), Some(&mut opts))
        .map(|d| d.deltas().len() > 0)
        .unwrap_or(false)
}

/// Every tag with the commit it points at (annotated tags peeled),
/// sorted by name. Empty when not a repo.
pub fn list_tags(repo_path: &Path) -> Vec<(String, String)> {
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return vec![];
    };
    let mut tags: Vec<(String, String)> = collect_log_refs(&repo)
        .1
        .into_iter()
        .map(|t| (t.name, t.commit.sha))
        .collect();
    tags.sort();
    tags
}

/// Walk every local branch + tag and emit `GitLogRef` entries pointing
/// at the commit each ref resolves to. The renderer indexes these by
/// sha so refs whose target sha isn't in `commits[]` simply don't
/// render — no need to filter here.
fn collect_log_refs(repo: &git2::Repository) -> (Vec<GitLogRef>, Vec<GitLogRef>) {
    let mut branch_heads = Vec::new();
    let mut tags = Vec::new();
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
            branch_heads.push(GitLogRef {
                name: name.to_string(),
                commit: GitLogRefCommit {
                    sha: oid.to_string(),
                },
            });
        }
    }
    if let Ok(tag_names) = repo.tag_names(None) {
        for name in tag_names.iter().filter_map(|n| n.ok().flatten()) {
            let full = format!("refs/tags/{name}");
            let Ok(reference) = repo.find_reference(&full) else {
                continue;
            };
            // Annotated tags are tag objects — peel to the commit they
            // point at; lightweight tags resolve directly.
            let target_oid = match reference.peel_to_commit() {
                Ok(commit) => commit.id(),
                Err(_) => match reference.target() {
                    Some(oid) => oid,
                    None => continue,
                },
            };
            tags.push(GitLogRef {
                name: name.to_string(),
                commit: GitLogRefCommit {
                    sha: target_oid.to_string(),
                },
            });
        }
    }
    (branch_heads, tags)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CommitDetailFile {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CommitDetail {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    pub email: String,
    pub timestamp_secs: i64,
    pub subject: String,
    pub body: String,
    pub parents: Vec<String>,
    pub files: Vec<CommitDetailFile>,
}

pub fn get_commit_detail(repo_path: &Path, sha: &str) -> Option<CommitDetail> {
    let repo = git2::Repository::open(repo_path).ok()?;
    // `Oid::from_str` zero-pads anything shorter than 40 hex chars into a
    // syntactically-valid-but-nonexistent OID, so it can't resolve abbreviated
    // shas — only trust it for full-length hashes and let `revparse_single`
    // (which expands against the object DB) handle everything else, including
    // the abbreviated shas that Activity-feed commit links carry.
    let oid = git2::Oid::from_str(sha)
        .ok()
        .filter(|_| sha.len() == 40)
        .or_else(|| repo.revparse_single(sha).ok().map(|obj| obj.id()))?;
    let commit = repo.find_commit(oid).ok()?;
    let author = commit.author();
    let parents: Vec<String> = commit.parent_ids().map(|p| p.to_string()).collect();

    // Diff against the first parent (or empty tree if root).
    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
    let commit_tree = commit.tree().ok()?;
    let diff = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&commit_tree), None)
        .ok()?;

    let mut files: Vec<CommitDetailFile> = Vec::new();
    diff.foreach(
        &mut |delta, _| {
            let path = delta
                .new_file()
                .path()
                .or(delta.old_file().path())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let status = match delta.status() {
                git2::Delta::Added => "added",
                git2::Delta::Deleted => "deleted",
                git2::Delta::Modified => "modified",
                git2::Delta::Renamed => "renamed",
                git2::Delta::Copied => "copied",
                _ => "other",
            };
            files.push(CommitDetailFile {
                path,
                additions: 0,
                deletions: 0,
                status: status.into(),
            });
            true
        },
        None,
        None,
        None,
    )
    .ok()?;

    // Hunk-level walk to attribute additions/deletions to files.
    let stats = diff.stats().ok();
    if let Some(stats) = stats {
        // libgit2 returns aggregate stats, not per-file; for per-file
        // we use the line callback.
        let _ = stats;
    }
    diff.print(git2::DiffFormat::NameStatus, |_d, _h, _l| true)
        .ok();
    diff.foreach(
        &mut |_d, _| true,
        None,
        None,
        Some(&mut |delta, _h, line| {
            let new_path = delta
                .new_file()
                .path()
                .or(delta.old_file().path())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(file) = files.iter_mut().find(|f| f.path == new_path) {
                match line.origin() {
                    '+' => file.additions += 1,
                    '-' => file.deletions += 1,
                    _ => {}
                }
            }
            true
        }),
    )
    .ok()?;

    Some(CommitDetail {
        sha: oid.to_string(),
        short_sha: oid.to_string()[..7].to_string(),
        author: author.name().unwrap_or("").to_string(),
        email: author.email().unwrap_or("").to_string(),
        timestamp_secs: commit.time().seconds(),
        subject: commit.summary().ok().flatten().unwrap_or("").to_string(),
        body: commit.body().ok().flatten().unwrap_or("").to_string(),
        parents,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_repo_with_commits(n: usize) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "t@example.com").unwrap();
        let sig = repo.signature().unwrap();
        let mut parents: Vec<git2::Commit> = vec![];
        for i in 0..n {
            let path = format!("f{i}.txt");
            std::fs::write(dir.path().join(&path), format!("v{i}")).unwrap();
            let mut idx = repo.index().unwrap();
            idx.add_path(std::path::Path::new(&path)).unwrap();
            idx.write().unwrap();
            let tree_id = idx.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
            let new_oid = repo
                .commit(
                    Some("HEAD"),
                    &sig,
                    &sig,
                    &format!("commit {i}"),
                    &tree,
                    &parent_refs,
                )
                .unwrap();
            parents = vec![repo.find_commit(new_oid).unwrap()];
        }
        dir
    }

    #[test]
    fn log_returns_commits_in_order() {
        let dir = make_repo_with_commits(3);
        let result = get_git_log(dir.path(), GitLogOptions::default());
        assert_eq!(result.commits.len(), 3);
        // newest commit first
        assert_eq!(result.commits[0].subject, "commit 2");
    }

    #[test]
    fn log_respects_limit() {
        let dir = make_repo_with_commits(5);
        let result = get_git_log(
            dir.path(),
            GitLogOptions {
                limit: Some(2),
                ..Default::default()
            },
        );
        assert_eq!(result.commits.len(), 2);
    }

    /// History starts from any revision, is filtered to a path, and
    /// resumes after the last commit of a page, with no horizon.
    #[test]
    fn log_starts_anywhere_filters_by_path_and_pages_by_the_last_sha() {
        let dir = make_repo_with_commits(5);
        let repo = git2::Repository::open(dir.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let mid = head.parent(0).unwrap().parent(0).unwrap();
        repo.branch("old", &mid, false).unwrap();
        let from_old = get_git_log(
            dir.path(),
            GitLogOptions {
                from: Some("old".into()),
                ..Default::default()
            },
        );
        assert_eq!(from_old.commits.len(), 3);
        assert_eq!(from_old.commits[0].subject, "commit 2");

        let touching = get_git_log(
            dir.path(),
            GitLogOptions {
                path: Some("f1.txt".into()),
                ..Default::default()
            },
        );
        assert_eq!(touching.commits.len(), 1);
        assert_eq!(touching.commits[0].subject, "commit 1");

        let page = |after: Option<String>| {
            get_git_log(
                dir.path(),
                GitLogOptions {
                    limit: Some(2),
                    after,
                    ..Default::default()
                },
            )
            .commits
        };
        let first = page(None);
        let second = page(Some(first[1].sha.clone()));
        let third = page(Some(second[1].sha.clone()));
        let subjects: Vec<&str> = first
            .iter()
            .chain(&second)
            .chain(&third)
            .map(|c| c.subject.as_str())
            .collect();
        assert_eq!(
            subjects,
            ["commit 4", "commit 3", "commit 2", "commit 1", "commit 0"]
        );
    }

    #[test]
    fn commit_detail_returns_files() {
        let dir = make_repo_with_commits(2);
        let log = get_git_log(dir.path(), GitLogOptions::default());
        let detail = get_commit_detail(dir.path(), &log.commits[0].sha).expect("detail");
        assert_eq!(detail.subject, "commit 1");
        assert_eq!(detail.files.len(), 1);
        assert_eq!(detail.files[0].path, "f1.txt");
        assert_eq!(detail.files[0].status, "added");
    }

    #[test]
    fn commit_detail_unknown_sha_returns_none() {
        let dir = make_repo_with_commits(1);
        assert!(get_commit_detail(dir.path(), "deadbeef").is_none());
    }

    #[test]
    fn commit_detail_resolves_abbreviated_sha() {
        // Activity-feed commit links carry abbreviated shas (e.g. the 7-char
        // prefixes GitHub-style UIs show). A valid short prefix of a real
        // commit must resolve — `Oid::from_str` alone zero-pads it into a
        // nonexistent OID, so this exercises the `revparse_single` fallback.
        let dir = make_repo_with_commits(2);
        let log = get_git_log(dir.path(), GitLogOptions::default());
        let full = &log.commits[0].sha;
        let short = &full[..7];
        let detail = get_commit_detail(dir.path(), short).expect("short sha resolves");
        assert_eq!(&detail.sha, full);
        assert_eq!(detail.subject, "commit 1");
    }

    #[test]
    fn log_carries_branch_heads_and_tags() {
        let dir = make_repo_with_commits(2);
        let repo = git2::Repository::open(dir.path()).unwrap();
        // Add a second branch pointing at the older commit and a
        // lightweight tag at HEAD so we have one of each to assert on.
        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        let older_oid = head_commit.parent(0).unwrap().id();
        let older_commit = repo.find_commit(older_oid).unwrap();
        repo.branch("feature", &older_commit, false).unwrap();
        repo.tag_lightweight("v0.1", head_commit.as_object(), false)
            .unwrap();

        let result = get_git_log(dir.path(), GitLogOptions::default());
        let head_sha = head_commit.id().to_string();
        let older_sha = older_oid.to_string();

        assert!(
            result
                .branch_heads
                .iter()
                .any(|b| b.name == "feature" && b.commit.sha == older_sha),
            "expected `feature` branch head at older commit, got {:?}",
            result.branch_heads,
        );
        assert!(
            result
                .tags
                .iter()
                .any(|t| t.name == "v0.1" && t.commit.sha == head_sha),
            "expected `v0.1` tag at HEAD, got {:?}",
            result.tags,
        );
    }
}
