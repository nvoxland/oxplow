//! The git provider of the VCS capability (`oxplow_domain::vcs::Vcs`,
//! `.context/vcs.md`): the trait over `oxplow_git`, stateless and
//! path-based. libgit2 and the git CLI block, so every call runs under
//! `spawn_blocking`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use oxplow_domain::vcs::{
    BlameLine, Branch, Checkout, CleanBaseline, CommitRequest, ConflictChoice, Divergence,
    FileStatus, Head, InProgressOp, LogQuery, MergeReadiness, ObjectId, ObjectStore, OpOutcome,
    RemoteBranch, RevisionDetail, RevisionFile, RevisionGraph, RevisionInfo, StatusEntry, Tag, Vcs,
    VcsError, VcsFeatures, VcsWorkspace, WorkspaceStatus,
};
use oxplow_domain::FileChange;

/// Git, as the VCS provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitProvider;

/// Git's own shapes for its native reads.
pub use oxplow_git::{ChangeScopes, CommitRefLabel, RemoteBranchEntry};

/// Git's own operations, beyond the VCS capability: history rewriting
/// and `.gitignore` (the `git.*` commands, `commands/vcs.rs`, run them),
/// and the git-native reads behind the `git_*` RPCs.
impl GitProvider {
    /// Make `path` a new repository with an empty root commit (a
    /// throwaway project, such as the conformance kit's host).
    pub async fn init_repository(&self, path: &Path) -> Result<(), VcsError> {
        let path = path.to_path_buf();
        blocking(move || {
            oxplow_git::init_repository(&path).map_err(|e| VcsError::Failed(e.to_string()))
        })
        .await
    }

    /// Replay the workspace's branch onto `onto`; oxplow's smart merge
    /// then settles the conflicts it can.
    pub async fn rebase(&self, ws: &Path, onto: &str) -> Result<OpOutcome, VcsError> {
        not_an_option("revision", onto)?;
        let (ws, onto) = (ws.to_path_buf(), onto.to_string());
        blocking(move || {
            repo_check(&ws)?;
            let r = oxplow_git::rebase(&ws, &onto).map_err(|e| VcsError::Failed(e.to_string()))?;
            Ok(with_auto_resolve(&ws, r))
        })
        .await
    }

    pub async fn cherry_pick(&self, ws: &Path, rev: &str) -> Result<OpOutcome, VcsError> {
        not_an_option("revision", rev)?;
        let (ws, rev) = (ws.to_path_buf(), rev.to_string());
        blocking(move || {
            repo_check(&ws)?;
            let r =
                oxplow_git::cherry_pick(&ws, &rev).map_err(|e| VcsError::Failed(e.to_string()))?;
            Ok(with_auto_resolve(&ws, r))
        })
        .await
    }

    pub async fn revert(&self, ws: &Path, rev: &str) -> Result<OpOutcome, VcsError> {
        not_an_option("revision", rev)?;
        let (ws, rev) = (ws.to_path_buf(), rev.to_string());
        blocking(move || {
            repo_check(&ws)?;
            let r = oxplow_git::revert(&ws, &rev).map_err(|e| VcsError::Failed(e.to_string()))?;
            Ok(with_auto_resolve(&ws, r))
        })
        .await
    }

    /// The workspace's changes grouped the way git sees them: staged,
    /// unstaged, and the branch against its base and upstream.
    pub async fn change_scopes(&self, ws: &Path) -> Result<ChangeScopes, VcsError> {
        let ws = ws.to_path_buf();
        blocking(move || {
            repo_check(&ws)?;
            Ok(oxplow_git::get_change_scopes(&ws))
        })
        .await
    }

    /// Every branch and tag pointing at each of `shas` (branches first);
    /// a sha no ref points at is absent.
    pub async fn commit_ref_labels(
        &self,
        ws: &Path,
        shas: Vec<String>,
    ) -> Result<HashMap<String, Vec<CommitRefLabel>>, VcsError> {
        let ws = ws.to_path_buf();
        blocking(move || {
            repo_check(&ws)?;
            Ok(oxplow_git::resolve_commit_ref_labels(&ws, &shas))
        })
        .await
    }

    /// Remote-tracking branches, most recently committed first.
    pub async fn recent_remote_branches(
        &self,
        ws: &Path,
        limit: usize,
    ) -> Result<Vec<RemoteBranchEntry>, VcsError> {
        let ws = ws.to_path_buf();
        blocking(move || {
            repo_check(&ws)?;
            Ok(oxplow_git::list_recent_remote_branches(&ws, limit))
        })
        .await
    }

    /// Append `entry` to the workspace's `.gitignore` (once).
    pub async fn ignore(&self, ws: &Path, entry: &str) -> Result<(), VcsError> {
        let (ws, entry) = (ws.to_path_buf(), entry.to_string());
        blocking(move || {
            repo_check(&ws)?;
            oxplow_git::append_to_gitignore(&ws, &entry)
                .map_err(|e| VcsError::Failed(e.to_string()))
        })
        .await
    }
}

/// The commit graph of a repository (`None`: it couldn't be opened).
struct GitGraph(Option<git2::Repository>);

impl RevisionGraph for GitGraph {
    fn resolve(&self, rev: &str) -> Option<String> {
        // revparse, not Oid::from_str: a short id must expand against the
        // object database (from_str zero-pads it into a nonexistent id).
        let commit = self
            .0
            .as_ref()?
            .revparse_single(rev)
            .ok()?
            .peel_to_commit()
            .ok()?;
        Some(commit.id().to_string())
    }

    fn is_ancestor_or_equal(&self, ancestor: &str, descendant: &str) -> Option<bool> {
        if ancestor == descendant {
            return Some(true);
        }
        let repo = self.0.as_ref()?;
        let anc = git2::Oid::from_str(ancestor).ok()?;
        let desc = git2::Oid::from_str(descendant).ok()?;
        // graph_descendant_of takes (descendant, ancestor).
        repo.graph_descendant_of(desc, anc).ok()
    }

    fn time_of(&self, rev: &str) -> Option<oxplow_domain::Timestamp> {
        let repo = self.0.as_ref()?;
        let commit = repo.find_commit(git2::Oid::from_str(rev).ok()?).ok()?;
        Some(oxplow_domain::Timestamp::from_unix_ms(
            commit.time().seconds() * 1000,
        ))
    }
}

/// The object database of the repository at `.0` (any of its worktrees).
struct GitObjects(PathBuf);

impl ObjectStore for GitObjects {
    fn read(&self, id: &ObjectId) -> Option<Vec<u8>> {
        oxplow_git::read_blob(&self.0, &id.0)
    }

    fn id_of(&self, bytes: &[u8]) -> ObjectId {
        ObjectId(oxplow_git::git_blob_oid(bytes).unwrap_or_default())
    }
}

/// HEAD's blobs, vouched for by the git index's cached stat.
struct GitBaseline(oxplow_git::GitCleanBaseline);

impl CleanBaseline for GitBaseline {
    fn clean_object(&self, path: &str, size: u64, mtime: (i64, u32)) -> Option<ObjectId> {
        self.0
            .clean_head_oid(path, size, mtime)
            .map(|oid| ObjectId(oid.to_string()))
    }

    fn candidates(&self) -> usize {
        self.0.candidate_count()
    }
}

/// Run `f` on the blocking pool; a panicked task is a failure, not a
/// crash of the caller.
/// Refuse a rev, branch or remote name that would read as an option on
/// git's command line (`--upload-pack=…`, `--contents=…`, `-b`). Every
/// caller-supplied name the CLI sees passes through here.
fn not_an_option(what: &str, value: &str) -> Result<(), VcsError> {
    if value.starts_with('-') {
        return Err(VcsError::Failed(format!(
            "{what} `{value}` would be read as a git option; names can't start with `-`"
        )));
    }
    Ok(())
}

async fn blocking<R: Send + 'static>(
    f: impl FnOnce() -> Result<R, VcsError> + Send + 'static,
) -> Result<R, VcsError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| VcsError::Failed(format!("git call didn't finish: {e}")))?
}

fn repo_check(ws: &Path) -> Result<(), VcsError> {
    if git2::Repository::open(ws).is_ok() {
        Ok(())
    } else {
        Err(VcsError::NotARepository(ws.display().to_string()))
    }
}

fn info(c: oxplow_git::GitLogCommit) -> RevisionInfo {
    RevisionInfo {
        id: c.sha,
        short_id: c.short_sha,
        author: c.author,
        email: c.email,
        time: c.timestamp_secs,
        subject: c.subject,
        parents: c.parents,
    }
}

fn outcome(ws: &Path, r: oxplow_git::GitOpResult) -> OpOutcome {
    let log = [r.stdout.trim(), r.stderr.trim()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    OpOutcome {
        success: r.success,
        log,
        conflicts: oxplow_git::list_conflicted_paths(ws),
        auto_resolved: r.auto_resolved,
    }
}

/// A CLI step that must succeed; its output is the error otherwise.
fn cli(r: std::io::Result<oxplow_git::GitOpResult>) -> Result<oxplow_git::GitOpResult, VcsError> {
    let r = r.map_err(|e| VcsError::Failed(e.to_string()))?;
    if r.success {
        Ok(r)
    } else {
        let msg = if r.stderr.trim().is_empty() {
            r.stdout.trim()
        } else {
            r.stderr.trim()
        };
        Err(VcsError::Failed(msg.to_string()))
    }
}

/// A merge-like op, then oxplow's smart merge over what it left
/// conflicted.
fn with_auto_resolve(ws: &Path, mut r: oxplow_git::GitOpResult) -> OpOutcome {
    if !r.success {
        r.auto_resolved = oxplow_git::auto_resolve_conflicts(ws).resolved.len() as u32;
    }
    outcome(ws, r)
}

fn branch_err(e: oxplow_git::BranchOpError) -> VcsError {
    VcsError::Failed(e.to_string())
}

#[async_trait]
impl Vcs for GitProvider {
    fn rev_kind(&self) -> &'static str {
        "git"
    }

    fn features(&self) -> VcsFeatures {
        VcsFeatures {
            isolated_workspaces: true,
            remotes: true,
            // `diff` compares content trees (`oxplow_domain::diff_trees`).
            rename_detection: false,
        }
    }

    async fn detect(&self, root: &Path) -> Option<Checkout> {
        let root = root.to_path_buf();
        blocking(move || {
            Ok(if !oxplow_git::is_git_repo(&root) {
                None
            } else if oxplow_git::is_git_worktree(&root) {
                Some(Checkout::Secondary)
            } else {
                Some(Checkout::Primary)
            })
        })
        .await
        .unwrap_or(None)
    }

    async fn head(&self, ws: &Path) -> Result<Head, VcsError> {
        let ws = ws.to_path_buf();
        blocking(move || {
            repo_check(&ws)?;
            Ok(Head {
                revision: oxplow_git::head_commit_sha(&ws),
                branch: oxplow_git::detect_current_branch(&ws),
            })
        })
        .await
    }

    async fn resolve(&self, ws: &Path, rev: &str) -> Result<String, VcsError> {
        let (ws, rev) = (ws.to_path_buf(), rev.to_string());
        blocking(move || {
            repo_check(&ws)?;
            oxplow_git::resolve_revision(&ws, &rev).map_err(|_| VcsError::UnknownRevision(rev))
        })
        .await
    }

    async fn log(&self, ws: &Path, query: LogQuery) -> Result<Vec<RevisionInfo>, VcsError> {
        let ws = ws.to_path_buf();
        blocking(move || {
            repo_check(&ws)?;
            let log = oxplow_git::get_git_log(
                &ws,
                oxplow_git::GitLogOptions {
                    limit: query.limit.map(|n| n as usize),
                    all: query.all,
                },
            );
            Ok(log.commits.into_iter().map(info).collect())
        })
        .await
    }

    async fn revision(&self, ws: &Path, rev: &str) -> Result<Option<RevisionDetail>, VcsError> {
        let id = match self.resolve(ws, rev).await {
            Ok(id) => id,
            Err(VcsError::UnknownRevision(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let ws = ws.to_path_buf();
        blocking(move || {
            Ok(
                oxplow_git::get_commit_detail(&ws, &id).map(|d| RevisionDetail {
                    info: RevisionInfo {
                        id: d.sha,
                        short_id: d.short_sha,
                        author: d.author,
                        email: d.email,
                        time: d.timestamp_secs,
                        subject: d.subject,
                        parents: d.parents,
                    },
                    body: d.body,
                    files: d
                        .files
                        .into_iter()
                        .map(|f| RevisionFile {
                            status: match f.status.as_str() {
                                "added" | "copied" => FileStatus::Added,
                                "deleted" => FileStatus::Deleted,
                                "renamed" => FileStatus::Renamed,
                                _ => FileStatus::Modified,
                            },
                            path: f.path,
                            additions: f.additions,
                            deletions: f.deletions,
                        })
                        .collect(),
                }),
            )
        })
        .await
    }

    async fn revisions_between(
        &self,
        ws: &Path,
        base: &str,
        head: &str,
        limit: u32,
    ) -> Result<Vec<RevisionInfo>, VcsError> {
        let base = self.resolve(ws, base).await?;
        let head = self.resolve(ws, head).await?;
        let ws = ws.to_path_buf();
        blocking(move || {
            Ok(
                oxplow_git::get_commits_ahead_of(&ws, &base, &head, limit as usize)
                    .into_iter()
                    .map(info)
                    .collect(),
            )
        })
        .await
    }

    async fn file_history(
        &self,
        ws: &Path,
        path: &str,
        limit: u32,
    ) -> Result<Vec<RevisionInfo>, VcsError> {
        let (ws, path) = (ws.to_path_buf(), path.to_string());
        blocking(move || {
            repo_check(&ws)?;
            Ok(oxplow_git::list_file_commits(&ws, &path, limit as usize)
                .into_iter()
                .map(info)
                .collect())
        })
        .await
    }

    async fn files_at(&self, ws: &Path, rev: &str) -> Result<BTreeMap<String, ObjectId>, VcsError> {
        let (ws, rev) = (ws.to_path_buf(), rev.to_string());
        blocking(move || {
            repo_check(&ws)?;
            let tree = oxplow_git::tree_at_commit(&ws, &rev)
                .map_err(|_| VcsError::UnknownRevision(rev))?;
            Ok(tree.into_iter().map(|(p, id)| (p, ObjectId(id))).collect())
        })
        .await
    }

    fn object_store(&self, ws: &Path) -> Arc<dyn ObjectStore> {
        Arc::new(GitObjects(ws.to_path_buf()))
    }

    fn clean_baseline(&self, ws: &Path) -> Box<dyn CleanBaseline> {
        Box::new(GitBaseline(oxplow_git::GitCleanBaseline::build(ws)))
    }

    fn revision_graph(&self, ws: &Path) -> Box<dyn RevisionGraph> {
        Box::new(GitGraph(git2::Repository::open(ws).ok()))
    }

    fn watch_refs(
        &self,
        ws: &Path,
        on_change: Box<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn Send>, VcsError> {
        let watcher = oxplow_git::GitRefsWatcher::watch(
            ws.to_path_buf(),
            std::time::Duration::from_millis(250),
        )
        .map_err(|e| VcsError::Failed(e.to_string()))?;
        let mut rx = watcher.subscribe();
        tokio::spawn(async move {
            use tokio::sync::broadcast::error::RecvError;
            // Runs until the watcher (the returned guard) is dropped.
            while let Ok(_) | Err(RecvError::Lagged(_)) = rx.recv().await {
                on_change();
            }
        });
        Ok(Box::new(watcher))
    }

    async fn diff(&self, ws: &Path, a: &str, b: &str) -> Result<Vec<FileChange>, VcsError> {
        let (ws, a, b) = (ws.to_path_buf(), a.to_string(), b.to_string());
        blocking(move || {
            repo_check(&ws)?;
            oxplow_git::diff_commits(&ws, &a, &b).map_err(|e| VcsError::Failed(e.to_string()))
        })
        .await
    }

    async fn status(&self, ws: &Path) -> Result<WorkspaceStatus, VcsError> {
        let ws = ws.to_path_buf();
        blocking(move || {
            repo_check(&ws)?;
            let conflicted = oxplow_git::list_conflicted_paths(&ws);
            let mut entries: Vec<StatusEntry> = oxplow_git::list_git_statuses(&ws)
                .into_iter()
                .filter(|(p, _)| !conflicted.contains(p))
                .map(|(path, s)| StatusEntry {
                    path,
                    status: match s {
                        oxplow_git::GitFileStatus::Added => FileStatus::Added,
                        oxplow_git::GitFileStatus::Modified => FileStatus::Modified,
                        oxplow_git::GitFileStatus::Deleted => FileStatus::Deleted,
                        oxplow_git::GitFileStatus::Renamed => FileStatus::Renamed,
                        oxplow_git::GitFileStatus::Untracked => FileStatus::Untracked,
                    },
                })
                .chain(conflicted.iter().map(|p| StatusEntry {
                    path: p.clone(),
                    status: FileStatus::Conflicted,
                }))
                .collect();
            entries.sort_by(|a, b| a.path.cmp(&b.path));
            let in_progress =
                oxplow_git::get_repo_conflict_state(&ws)
                    .operation
                    .map(|op| match op {
                        oxplow_git::GitOperationKind::Merge => InProgressOp::Merge,
                        oxplow_git::GitOperationKind::Rebase => InProgressOp::Rebase,
                        oxplow_git::GitOperationKind::CherryPick => InProgressOp::CherryPick,
                        oxplow_git::GitOperationKind::Revert => InProgressOp::Revert,
                    });
            Ok(WorkspaceStatus {
                entries,
                in_progress,
            })
        })
        .await
    }

    async fn branches(&self, ws: &Path) -> Result<Vec<Branch>, VcsError> {
        let ws = ws.to_path_buf();
        blocking(move || {
            repo_check(&ws)?;
            let default = oxplow_git::detect_default_branch(&ws);
            Ok(oxplow_git::list_branches(&ws)
                .into_iter()
                .map(|b| Branch {
                    is_default: b.remote.is_none() && default.as_deref() == Some(&b.name),
                    name: b.name,
                    remote: b.remote,
                    head: b.head,
                })
                .collect())
        })
        .await
    }

    async fn tags(&self, ws: &Path) -> Result<Vec<Tag>, VcsError> {
        let ws = ws.to_path_buf();
        blocking(move || {
            repo_check(&ws)?;
            Ok(oxplow_git::list_tags(&ws)
                .into_iter()
                .map(|(name, revision)| Tag { name, revision })
                .collect())
        })
        .await
    }

    async fn divergence(&self, ws: &Path, base: &str, head: &str) -> Result<Divergence, VcsError> {
        let (ws, base, head) = (ws.to_path_buf(), base.to_string(), head.to_string());
        blocking(move || {
            repo_check(&ws)?;
            let d = oxplow_git::compute_divergence(&ws, &base, &head);
            Ok(Divergence {
                ahead: d.ahead,
                behind: d.behind,
                overlapping_files: d.overlapping_files,
                readiness: match d.readiness {
                    oxplow_git::MergeReadiness::AlreadyIntegrated => {
                        MergeReadiness::AlreadyIntegrated
                    }
                    oxplow_git::MergeReadiness::Clean => MergeReadiness::Clean,
                    oxplow_git::MergeReadiness::Conflict => MergeReadiness::Conflict,
                },
            })
        })
        .await
    }

    async fn blame(
        &self,
        ws: &Path,
        path: &str,
        rev: Option<&str>,
    ) -> Result<Vec<BlameLine>, VcsError> {
        if let Some(r) = rev {
            not_an_option("revision", r)?;
        }
        let (ws, path, rev) = (ws.to_path_buf(), path.to_string(), rev.map(str::to_string));
        blocking(move || {
            repo_check(&ws)?;
            let lines =
                oxplow_git::git_blame(&ws, rev.as_deref(), &path).map_err(VcsError::Failed)?;
            Ok(lines
                .into_iter()
                .map(|l| BlameLine {
                    line: l.line,
                    revision: (l.sha != oxplow_git::BLAME_ZERO_SHA).then_some(l.sha),
                    author: l.author,
                    email: l.author_mail,
                    time: l.author_time,
                    summary: l.summary,
                })
                .collect())
        })
        .await
    }

    async fn merge_base(&self, ws: &Path, a: &str, b: &str) -> Result<Option<String>, VcsError> {
        let (ws, a, b) = (ws.to_path_buf(), a.to_string(), b.to_string());
        blocking(move || {
            repo_check(&ws)?;
            oxplow_git::merge_base(&ws, &a, &b).map_err(|e| VcsError::Failed(e.to_string()))
        })
        .await
    }

    async fn commit(&self, ws: &Path, req: CommitRequest) -> Result<String, VcsError> {
        let ws = ws.to_path_buf();
        blocking(move || {
            repo_check(&ws)?;
            cli(oxplow_git::commit(&ws, &req.message, req.include_untracked))?;
            oxplow_git::head_commit_sha(&ws)
                .ok_or_else(|| VcsError::Failed("the commit left no head".into()))
        })
        .await
    }

    async fn stage(&self, ws: &Path, paths: &[String]) -> Result<(), VcsError> {
        let (ws, paths) = (ws.to_path_buf(), paths.to_vec());
        blocking(move || {
            repo_check(&ws)?;
            for p in &paths {
                cli(oxplow_git::add_path(&ws, p))?;
            }
            Ok(())
        })
        .await
    }

    async fn discard(&self, ws: &Path, paths: &[String]) -> Result<(), VcsError> {
        let (ws, paths) = (ws.to_path_buf(), paths.to_vec());
        blocking(move || {
            repo_check(&ws)?;
            for p in &paths {
                oxplow_git::restore_path(&ws, p).map_err(|e| VcsError::Failed(e.to_string()))?;
            }
            Ok(())
        })
        .await
    }

    async fn fetch(&self, ws: &Path, remote: Option<&str>) -> Result<OpOutcome, VcsError> {
        if let Some(r) = remote {
            not_an_option("remote", r)?;
        }
        let (ws, remote) = (ws.to_path_buf(), remote.map(str::to_string));
        blocking(move || {
            repo_check(&ws)?;
            let r = oxplow_git::fetch(&ws, remote.as_deref())
                .map_err(|e| VcsError::Failed(e.to_string()))?;
            Ok(outcome(&ws, r))
        })
        .await
    }

    async fn pull(&self, ws: &Path, from: Option<RemoteBranch>) -> Result<OpOutcome, VcsError> {
        if let Some(rb) = &from {
            not_an_option("remote", &rb.remote)?;
            not_an_option("branch", &rb.branch)?;
        }
        let ws = ws.to_path_buf();
        blocking(move || {
            repo_check(&ws)?;
            let r = match &from {
                Some(rb) => oxplow_git::pull_remote_into_current(&ws, &rb.remote, &rb.branch),
                None => oxplow_git::pull(&ws),
            }
            .map_err(|e| VcsError::Failed(e.to_string()))?;
            Ok(with_auto_resolve(&ws, r))
        })
        .await
    }

    async fn push(&self, ws: &Path, to: Option<RemoteBranch>) -> Result<OpOutcome, VcsError> {
        if let Some(rb) = &to {
            not_an_option("remote", &rb.remote)?;
            not_an_option("branch", &rb.branch)?;
        }
        let ws = ws.to_path_buf();
        blocking(move || {
            repo_check(&ws)?;
            let r = match &to {
                Some(rb) => oxplow_git::push_current_to(&ws, &rb.remote, &rb.branch),
                None => oxplow_git::push(&ws),
            }
            .map_err(|e| VcsError::Failed(e.to_string()))?;
            Ok(outcome(&ws, r))
        })
        .await
    }

    async fn merge(&self, ws: &Path, rev: &str) -> Result<OpOutcome, VcsError> {
        not_an_option("revision", rev)?;
        let (ws, rev) = (ws.to_path_buf(), rev.to_string());
        blocking(move || {
            repo_check(&ws)?;
            let r = oxplow_git::merge(&ws, &rev).map_err(|e| VcsError::Failed(e.to_string()))?;
            Ok(with_auto_resolve(&ws, r))
        })
        .await
    }

    async fn checkout_branch(&self, ws: &Path, name: &str, create: bool) -> Result<(), VcsError> {
        not_an_option("branch", name)?;
        let (ws, name) = (ws.to_path_buf(), name.to_string());
        blocking(move || {
            repo_check(&ws)?;
            cli(oxplow_git::checkout_branch(&ws, &name, create)).map(|_| ())
        })
        .await
    }

    async fn rename_branch(&self, ws: &Path, from: &str, to: &str) -> Result<(), VcsError> {
        not_an_option("branch", from)?;
        not_an_option("branch", to)?;
        let (ws, from, to) = (ws.to_path_buf(), from.to_string(), to.to_string());
        blocking(move || oxplow_git::rename_branch(&ws, &from, &to).map_err(branch_err)).await
    }

    async fn delete_branch(&self, ws: &Path, name: &str, force: bool) -> Result<(), VcsError> {
        not_an_option("branch", name)?;
        let (ws, name) = (ws.to_path_buf(), name.to_string());
        blocking(move || oxplow_git::delete_branch(&ws, &name, force).map_err(branch_err)).await
    }

    async fn resolve_conflict(
        &self,
        ws: &Path,
        path: &str,
        choice: ConflictChoice,
    ) -> Result<(), VcsError> {
        let (ws, path) = (ws.to_path_buf(), path.to_string());
        blocking(move || {
            repo_check(&ws)?;
            match choice {
                ConflictChoice::Ours | ConflictChoice::Theirs => cli(
                    oxplow_git::take_conflict_side(&ws, &path, choice == ConflictChoice::Ours),
                )
                .map(|_| ()),
                ConflictChoice::Auto => {
                    let report = oxplow_git::auto_resolve_conflicts(&ws);
                    if report.resolved.contains(&path)
                        || !oxplow_git::list_conflicted_paths(&ws).contains(&path)
                    {
                        Ok(())
                    } else {
                        Err(VcsError::Failed(format!(
                            "{path}: the edits overlap; choose a side"
                        )))
                    }
                }
            }
        })
        .await
    }

    async fn create_workspace(
        &self,
        repo: &Path,
        at: &Path,
        branch: &str,
        from: &str,
    ) -> Result<(), VcsError> {
        let (repo, at): (PathBuf, PathBuf) = (repo.to_path_buf(), at.to_path_buf());
        let (branch, from) = (branch.to_string(), from.to_string());
        blocking(move || {
            oxplow_git::ensure_worktree(&repo, &at, &branch, &from).map_err(|e| match e {
                oxplow_git::EnsureWorktreeError::NotRepo(p) => {
                    VcsError::NotARepository(p.display().to_string())
                }
                e => VcsError::Failed(e.to_string()),
            })
        })
        .await
    }

    async fn remove_workspace(&self, repo: &Path, at: &Path) -> Result<(), VcsError> {
        let (repo, at) = (repo.to_path_buf(), at.to_path_buf());
        blocking(move || {
            repo_check(&repo)?;
            oxplow_git::remove_worktree(&repo, &at).map_err(VcsError::Failed)
        })
        .await
    }

    async fn list_workspaces(&self, repo: &Path) -> Result<Vec<VcsWorkspace>, VcsError> {
        let repo = repo.to_path_buf();
        blocking(move || {
            repo_check(&repo)?;
            Ok(oxplow_git::list_existing_worktrees(&repo)
                .into_iter()
                .map(|w| VcsWorkspace {
                    path: w.path,
                    branch: w.branch,
                    head: w.head_sha,
                    is_main: w.is_main,
                })
                .collect())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} in {dir:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn write_commit(dir: &Path, contents: &str, message: &str) {
        std::fs::write(dir.join("cfg.txt"), contents).unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", message]);
    }

    /// tsk550: a rev or name that reads as an option never reaches git's
    /// command line, whoever passes it.
    #[tokio::test]
    async fn options_are_refused_where_revs_and_names_go() {
        let (_p, wt, _dirs) = diverged("a\n", "b\n", "c\n");
        let bad = "--contents=/etc/hosts";
        let refused = |r: Result<(), VcsError>| {
            let e = r.expect_err("refused");
            assert!(e.to_string().contains("option"), "{e}");
        };
        refused(GitProvider.merge(&wt, bad).await.map(|_| ()));
        refused(GitProvider.rebase(&wt, bad).await.map(|_| ()));
        refused(GitProvider.cherry_pick(&wt, bad).await.map(|_| ()));
        refused(GitProvider.revert(&wt, bad).await.map(|_| ()));
        refused(
            GitProvider
                .blame(&wt, "cfg.txt", Some(bad))
                .await
                .map(|_| ()),
        );
        refused(GitProvider.checkout_branch(&wt, "-b", false).await);
        refused(
            GitProvider
                .fetch(&wt, Some("--upload-pack=x"))
                .await
                .map(|_| ()),
        );
        refused(
            GitProvider
                .push(
                    &wt,
                    Some(RemoteBranch {
                        remote: "origin".into(),
                        branch: "--force".into(),
                    }),
                )
                .await
                .map(|_| ()),
        );
    }

    /// A repo on `main` and a `feature` worktree beside it that each
    /// committed a different version of the one line in `cfg.txt`, so
    /// bringing main into feature conflicts line-wise. Returns the
    /// primary checkout, the feature worktree, and their tempdirs.
    fn diverged(
        base: &str,
        main: &str,
        feature: &str,
    ) -> (PathBuf, PathBuf, [tempfile::TempDir; 2]) {
        let primary = tempfile::tempdir().unwrap();
        let p = primary.path();
        git(p, &["init", "-q", "--initial-branch=main"]);
        git(p, &["config", "user.email", "test@example.com"]);
        git(p, &["config", "user.name", "test"]);
        write_commit(p, base, "base");
        git(p, &["branch", "feature"]);
        let parent = tempfile::tempdir().unwrap();
        let wt = parent.path().join("feature-wt");
        git(
            p,
            &["worktree", "add", "-q", wt.to_str().unwrap(), "feature"],
        );
        write_commit(p, main, "main edit");
        write_commit(&wt, feature, "feature edit");
        (p.to_path_buf(), wt, [primary, parent])
    }

    fn conflicted(ws: &Path) -> usize {
        oxplow_git::get_repo_conflict_state(ws).conflicted_count as usize
    }

    #[tokio::test]
    async fn a_merge_auto_resolves_edits_to_different_words_of_one_line() {
        let (_p, wt, _dirs) = diverged(
            "alpha beta gamma\n",
            "ALPHA beta gamma\n",
            "alpha beta GAMMA\n",
        );
        let out = GitProvider.merge(&wt, "main").await.unwrap();
        assert_eq!(out.auto_resolved, 1, "{}", out.log);
        assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
        assert_eq!(
            std::fs::read_to_string(wt.join("cfg.txt")).unwrap(),
            "ALPHA beta GAMMA\n"
        );
        assert_eq!(conflicted(&wt), 0);
    }

    #[tokio::test]
    async fn a_merge_leaves_a_true_overlap_conflicted() {
        let (_p, wt, _dirs) = diverged(
            "let timeout = 10;\n",
            "let timeout = 20;\n",
            "let timeout = 30;\n",
        );
        let out = GitProvider.merge(&wt, "main").await.unwrap();
        assert_eq!(out.auto_resolved, 0);
        assert_eq!(out.conflicts, vec!["cfg.txt".to_string()]);
        assert!(std::fs::read_to_string(wt.join("cfg.txt"))
            .unwrap()
            .contains("<<<<<<<"));
        assert_eq!(conflicted(&wt), 1);
    }

    #[tokio::test]
    async fn rebase_and_cherry_pick_auto_resolve_the_same_way() {
        let (_p, wt, _dirs) = diverged(
            "alpha beta gamma\n",
            "ALPHA beta gamma\n",
            "alpha beta GAMMA\n",
        );
        let out = GitProvider.rebase(&wt, "main").await.unwrap();
        assert_eq!(out.auto_resolved, 1, "{}", out.log);
        assert_eq!(
            std::fs::read_to_string(wt.join("cfg.txt")).unwrap(),
            "ALPHA beta GAMMA\n"
        );
        assert_eq!(conflicted(&wt), 0);

        let (p, wt, _dirs) = diverged(
            "alpha beta gamma\n",
            "ALPHA beta gamma\n",
            "alpha beta GAMMA\n",
        );
        let main = git(&p, &["rev-parse", "main"]);
        let out = GitProvider.cherry_pick(&wt, &main).await.unwrap();
        assert_eq!(out.auto_resolved, 1, "{}", out.log);
        assert_eq!(conflicted(&wt), 0);
    }
}
