//! Singleton git access surface.
//!
//! `GitService` is the one place anything in the app reaches when it
//! needs git data. It's a **thin facade** over `oxplow_git::*`: every
//! read shells out live and every write passes through to the
//! corresponding `oxplow_git::*` op and then emits the renderer-facing
//! `OxplowEvent` so panels refetch.
//!
//! There is no shared mutable cache here. The previous design cached
//! statuses / branches / log / ahead-behind / remote-branches and
//! subscribed to its own invalidation triggers (`WorkspaceChanged` /
//! `GitRefsChanged`). That made cache invalidation race other bus
//! subscribers on the same event — readers landing on the cache before
//! the invalidation hop could see stale data. The git ops we wrap
//! (`list_git_statuses`, `list_branches`, etc.) are sub-10ms libgit2
//! calls; the cache wasn't carrying its weight against the correctness
//! cost.
//!
//! If a future hotspot warrants caching, add it **inside** the facade
//! (per-method memo, request coalescer, whatever) — never let cached
//! state leak through the API. Callers shouldn't be able to tell.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_domain::StreamId;
use oxplow_git::{
    AheadBehind, BranchRef, ChangeScopes, CommitRefLabel, Divergence, GitLogCommit, GitLogOptions,
    GitLogResult, GitOpResult, GitWorktreeEntry, GroupedGitRefs, RemoteBranchEntry, TextSearchHit,
};
use tracing::warn;

use crate::events::{EventBus, OxplowEvent, WorkspaceChangeKind};
use crate::worktrees::WorktreeRouter;

/// Singleton handle. Held inside `Services` as `Arc<GitService>`.
/// Routing a stream to its worktree is the router's; file I/O is
/// `WorkspaceFiles`'; the branch reconciler is its own service.
pub struct GitService {
    router: Arc<WorktreeRouter>,
    events: EventBus,
}

impl GitService {
    pub fn new(router: Arc<WorktreeRouter>, events: EventBus) -> Arc<Self> {
        Arc::new(Self { router, events })
    }

    fn project_dir(&self) -> PathBuf {
        self.router.project_dir().to_path_buf()
    }

    /// The worktree of a **stream-scoped destructive op** (merge /
    /// rebase / commit), refusing rather than falling back to the
    /// primary (`WorktreeRouter::resolve_strict`).
    async fn strict(&self, stream_id: Option<&str>) -> std::io::Result<PathBuf> {
        self.router
            .resolve_strict(stream_id)
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()))
    }

    /// Emit the renderer-facing events for a write that touched
    /// `stream_id`'s worktree. Pass `refs_changed=true` when the op
    /// may have moved HEAD or any ref so refs subscribers fire too.
    fn announce_write(&self, stream_id: Option<&StreamId>, refs_changed: bool) {
        if let Some(id) = stream_id {
            self.events.emit(OxplowEvent::WorkspaceChanged {
                stream_id: *id,
                change_kind: WorkspaceChangeKind::Updated,
                path: String::new(),
            });
            if refs_changed {
                self.events
                    .emit(OxplowEvent::GitRefsChanged { stream_id: *id });
            }
        }
    }

    // ---------------------------------------------------------------
    // Reads — every one is a live shell-out via spawn_blocking.
    // ---------------------------------------------------------------

    pub async fn branches_for(&self, stream_id: Option<&str>) -> Vec<BranchRef> {
        let path = self.router.resolve(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::list_branches(path))
            .await
            .unwrap_or_default()
    }

    /// `list_branches` against the project root — used by the shared
    /// branch picker that doesn't sit inside a specific stream.
    pub async fn list_branches_project(&self) -> Vec<BranchRef> {
        let path = self.project_dir();
        tokio::task::spawn_blocking(move || oxplow_git::list_branches(path))
            .await
            .unwrap_or_default()
    }

    pub async fn ahead_behind(
        &self,
        stream_id: Option<&str>,
        base: String,
        head: String,
    ) -> AheadBehind {
        let path = self.router.resolve(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::get_ahead_behind(&path, &base, &head))
            .await
            .expect("ahead_behind join")
    }

    /// Divergence + merge-readiness of `head` vs `base`, resolved
    /// against `stream_id`'s repo (defaults to the project root, where
    /// every worktree branch is visible since they share `.git`).
    pub async fn divergence(
        &self,
        stream_id: Option<&str>,
        base: String,
        head: String,
    ) -> Divergence {
        let path = self.router.resolve(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::compute_divergence(&path, &base, &head))
            .await
            .expect("divergence join")
    }

    /// Resolved 40-char sha for HEAD in `stream_id`'s worktree.
    /// Returns `None` when the directory isn't a git repo or HEAD
    /// is unborn.
    pub async fn head_commit_sha(&self, stream_id: Option<&str>) -> Option<String> {
        let path = self.router.resolve(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::head_commit_sha(&path))
            .await
            .ok()
            .flatten()
    }

    pub async fn change_scopes(&self, stream_id: Option<&str>) -> ChangeScopes {
        let path = self.router.resolve(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::get_change_scopes(&path))
            .await
            .expect("change_scopes join")
    }

    pub async fn git_log(&self, stream_id: Option<&str>, opts: GitLogOptions) -> GitLogResult {
        let path = self.router.resolve(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::get_git_log(&path, opts))
            .await
            .expect("git_log join")
    }

    pub async fn commits_ahead_of(
        &self,
        stream_id: Option<&str>,
        base: String,
        head: String,
        limit: usize,
    ) -> Vec<GitLogCommit> {
        let path = self.router.resolve(stream_id).await;
        tokio::task::spawn_blocking(move || {
            oxplow_git::get_commits_ahead_of(&path, &base, &head, limit)
        })
        .await
        .unwrap_or_default()
    }

    pub async fn list_file_commits(
        &self,
        stream_id: Option<&str>,
        path: String,
        limit: usize,
    ) -> Vec<GitLogCommit> {
        let dir = self.router.resolve(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::list_file_commits(&dir, &path, limit))
            .await
            .unwrap_or_default()
    }

    pub async fn search_workspace_text(
        &self,
        stream_id: Option<&str>,
        query: String,
        limit: Option<usize>,
    ) -> Vec<TextSearchHit> {
        let dir = self.router.resolve(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::search_workspace_text(&dir, &query, limit))
            .await
            .unwrap_or_default()
    }

    pub async fn list_all_refs(&self) -> GroupedGitRefs {
        let path = self.project_dir();
        tokio::task::spawn_blocking(move || oxplow_git::list_all_refs(&path))
            .await
            .expect("list_all_refs join")
    }

    /// Map commit SHAs to every branch + tag pointing at them.
    /// Branches come first, then tags. Shas without a matching ref
    /// are absent — caller falls back to a short-sha chip.
    pub async fn resolve_commit_ref_labels(
        &self,
        shas: Vec<String>,
    ) -> std::collections::HashMap<String, Vec<CommitRefLabel>> {
        let path = self.project_dir();
        tokio::task::spawn_blocking(move || oxplow_git::resolve_commit_ref_labels(&path, &shas))
            .await
            .unwrap_or_default()
    }

    pub async fn list_recent_remote_branches(&self, limit: usize) -> Vec<RemoteBranchEntry> {
        let path = self.project_dir();
        tokio::task::spawn_blocking(move || oxplow_git::list_recent_remote_branches(&path, limit))
            .await
            .unwrap_or_default()
    }

    pub async fn list_existing_worktrees(&self) -> Vec<GitWorktreeEntry> {
        let path = self.project_dir();
        tokio::task::spawn_blocking(move || oxplow_git::list_existing_worktrees(&path))
            .await
            .unwrap_or_default()
    }

    pub async fn list_adoptable_worktrees(&self, registered: Vec<String>) -> Vec<GitWorktreeEntry> {
        let path = self.project_dir();
        tokio::task::spawn_blocking(move || {
            oxplow_git::list_adoptable_worktrees(&path, &registered)
        })
        .await
        .unwrap_or_default()
    }

    pub async fn detect_default_branch(&self) -> Option<String> {
        let path = self.project_dir();
        tokio::task::spawn_blocking(move || oxplow_git::detect_default_branch(&path))
            .await
            .unwrap_or(None)
    }

    // ---------------------------------------------------------------
    // Mutating ops — pass through to oxplow_git, then emit events.
    // ---------------------------------------------------------------

    pub async fn commit_all(
        &self,
        stream_id: Option<&str>,
        message: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.strict(stream_id).await?;
        let result = run_blocking(move || oxplow_git::commit(&path, &message, true)).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    pub async fn add_path(
        &self,
        stream_id: Option<&str>,
        relpath: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.router.resolve(stream_id).await;
        let result = run_blocking(move || oxplow_git::add_path(&path, &relpath)).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), false);
        Ok(result)
    }

    pub async fn restore_path(
        &self,
        stream_id: Option<&str>,
        relpath: String,
    ) -> std::io::Result<()> {
        let path = self.router.resolve(stream_id).await;
        run_blocking(move || oxplow_git::restore_path(&path, &relpath)).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), false);
        Ok(())
    }

    pub async fn fetch(
        &self,
        stream_id: Option<&str>,
        remote: Option<String>,
    ) -> std::io::Result<GitOpResult> {
        let path = self.router.resolve(stream_id).await;
        let result = run_blocking(move || oxplow_git::fetch(&path, remote.as_deref())).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    pub async fn pull(&self, stream_id: Option<&str>) -> std::io::Result<GitOpResult> {
        let path = self.router.resolve(stream_id).await;
        let result = run_blocking(move || oxplow_git::pull(&path)).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    pub async fn pull_remote_into_current(
        &self,
        stream_id: Option<&str>,
        remote: String,
        branch: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.router.resolve(stream_id).await;
        let result =
            run_blocking(move || oxplow_git::pull_remote_into_current(&path, &remote, &branch))
                .await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    pub async fn push(&self, stream_id: Option<&str>) -> std::io::Result<GitOpResult> {
        let path = self.router.resolve(stream_id).await;
        let result = run_blocking(move || oxplow_git::push(&path)).await?;
        // Push doesn't change local refs but ahead/behind shifts.
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    pub async fn push_current_to(
        &self,
        stream_id: Option<&str>,
        remote: String,
        branch: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.router.resolve(stream_id).await;
        let result =
            run_blocking(move || oxplow_git::push_current_to(&path, &remote, &branch)).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    pub async fn merge(
        &self,
        stream_id: Option<&str>,
        source: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.strict(stream_id).await?;
        let result =
            run_blocking(move || Ok(with_auto_resolve(oxplow_git::merge(&path, &source)?, &path)))
                .await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    pub async fn rebase(
        &self,
        stream_id: Option<&str>,
        onto: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.strict(stream_id).await?;
        let result =
            run_blocking(move || Ok(with_auto_resolve(oxplow_git::rebase(&path, &onto)?, &path)))
                .await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    /// Cherry-pick `commit` into the stream's worktree, then run the
    /// smart-merge pass over any conflicts git leaves. Stream-scoped and
    /// destructive, so it resolves the worktree strictly (no primary
    /// fallback).
    pub async fn cherry_pick(
        &self,
        stream_id: Option<&str>,
        commit: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.strict(stream_id).await?;
        let result = run_blocking(move || {
            Ok(with_auto_resolve(
                oxplow_git::cherry_pick(&path, &commit)?,
                &path,
            ))
        })
        .await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    /// Revert `commit` in the stream's worktree, then run the smart-merge
    /// pass over any conflicts git leaves.
    pub async fn revert(
        &self,
        stream_id: Option<&str>,
        commit: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.strict(stream_id).await?;
        let result = run_blocking(move || {
            Ok(with_auto_resolve(
                oxplow_git::revert(&path, &commit)?,
                &path,
            ))
        })
        .await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    pub async fn rename_branch(
        &self,
        from: String,
        to: String,
    ) -> Result<(), oxplow_git::BranchOpError> {
        let path = self.project_dir();
        run_blocking_branch(move || oxplow_git::rename_branch(&path, &from, &to)).await?;
        self.broadcast_refs_change_all().await;
        Ok(())
    }

    pub async fn delete_branch(
        &self,
        branch: String,
        force: bool,
    ) -> Result<(), oxplow_git::BranchOpError> {
        let path = self.project_dir();
        run_blocking_branch(move || oxplow_git::delete_branch(&path, &branch, force)).await?;
        self.broadcast_refs_change_all().await;
        Ok(())
    }

    pub async fn append_to_gitignore(
        &self,
        stream_id: Option<&str>,
        entry: String,
    ) -> std::io::Result<()> {
        let path = self.router.resolve(stream_id).await;
        run_blocking(move || oxplow_git::append_to_gitignore(&path, &entry)).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), false);
        Ok(())
    }

    /// Project-wide refs change — fan out a `GitRefsChanged` for
    /// every registered stream so per-stream subscribers (snapshot
    /// capture, history panel, branch picker) all refresh.
    async fn broadcast_refs_change_all(&self) {
        match self.router.all().await {
            Ok(all) => {
                for (id, _) in all {
                    self.events
                        .emit(OxplowEvent::GitRefsChanged { stream_id: id });
                }
            }
            Err(error) => warn!(%error, "couldn't list the streams to announce a refs change"),
        }
    }
}

fn stream_id_from(s: Option<&str>) -> Option<StreamId> {
    s.and_then(StreamId::try_from_str)
}

/// After a long-running git op (merge / rebase / cherry-pick / revert)
/// leaves conflicts, run the token-level smart-merge pass and record how
/// many files it cleanly auto-resolved. A no-op when the op succeeded.
/// Operation-agnostic: it reads whatever unmerged paths sit in the index,
/// regardless of which op produced them, and only resolves the current
/// step's conflicts (it never `--continue`s a paused rebase/cherry-pick —
/// the user/UI drives continuation).
fn with_auto_resolve(mut result: GitOpResult, path: &Path) -> GitOpResult {
    if !result.success {
        result.auto_resolved = oxplow_git::auto_resolve_conflicts(path).resolved.len() as u32;
    }
    result
}

async fn run_blocking<R>(
    f: impl FnOnce() -> std::io::Result<R> + Send + 'static,
) -> std::io::Result<R>
where
    R: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "git op join failed");
            Err(std::io::Error::other(e.to_string()))
        }
    }
}

async fn run_blocking_branch<R>(
    f: impl FnOnce() -> Result<R, oxplow_git::BranchOpError> + Send + 'static,
) -> Result<R, oxplow_git::BranchOpError>
where
    R: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .expect("branch op join")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteStreamStore};
    use oxplow_domain::stores::StreamStore;
    use oxplow_domain::{Stream, StreamKind, Timestamp};
    use std::process::Command as Cmd;

    fn run_git(dir: &Path, args: &[&str]) {
        let out = Cmd::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} in {dir:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn write_commit(dir: &Path, file: &str, contents: &str, msg: &str) {
        std::fs::write(dir.join(file), contents).unwrap();
        run_git(dir, &["add", "-A"]);
        run_git(dir, &["commit", "-q", "-m", msg]);
    }

    fn git_sha(dir: &Path, rev: &str) -> String {
        let out = Cmd::new("git")
            .args(["rev-parse", rev])
            .current_dir(dir)
            .output()
            .expect("rev-parse");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Build a primary repo on `main` plus a sibling worktree on
    /// `feature` that is one commit behind `main`. Returns the service,
    /// the sibling worktree path, the StreamId registered for it, and
    /// the tempdirs to keep alive.
    async fn setup_behind_worktree() -> (
        Arc<GitService>,
        PathBuf,
        StreamId,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let primary = tempfile::tempdir().unwrap();
        let p = primary.path();
        run_git(p, &["init", "-q", "--initial-branch=main"]);
        run_git(p, &["config", "user.email", "test@example.com"]);
        run_git(p, &["config", "user.name", "test"]);
        write_commit(p, "base.txt", "base", "base");
        run_git(p, &["branch", "feature"]);

        // Sibling worktree checked out on `feature`, then advance main.
        let wt_parent = tempfile::tempdir().unwrap();
        let sib = wt_parent.path().join("feature-wt");
        run_git(
            p,
            &["worktree", "add", "-q", sib.to_str().unwrap(), "feature"],
        );
        write_commit(p, "adv.txt", "advanced", "advance main");

        let db = Database::in_memory();
        let store = Arc::new(SqliteStreamStore::new(db));
        let stream_id = StreamId::new(2);
        let now = Timestamp::now();
        store
            .upsert(&Stream {
                id: stream_id,
                kind: StreamKind::Worktree,
                title: "feature".into(),
                branch: "feature".into(),
                branch_ref: "refs/heads/feature".into(),
                branch_source: "main".into(),
                worktree_path: sib.to_string_lossy().into_owned(),
                working_pane: String::new(),
                talking_pane: String::new(),
                working_session_id: String::new(),
                talking_session_id: String::new(),
                custom_prompt: None,
                created_at: now,
                updated_at: now,
                archived_at: None,
            })
            .await
            .unwrap();

        let svc = GitService::new(
            Arc::new(WorktreeRouter::new(p.to_path_buf(), store)),
            EventBus::new(),
        );
        (svc, sib, stream_id, primary, wt_parent)
    }

    #[tokio::test]
    async fn merge_for_non_primary_stream_targets_that_worktree() {
        let (svc, sib, stream_id, _primary, _wt) = setup_behind_worktree().await;

        // Sanity: the advance commit only exists on main, not yet in the
        // feature worktree.
        assert!(!sib.join("adv.txt").exists());

        let res = svc
            .merge(Some(&stream_id.to_string()), "main".into())
            .await
            .unwrap();
        assert!(res.success, "merge failed: {}", res.stderr);

        // The merge ran in the FEATURE worktree, not the primary: main's
        // advance commit is now present there. Under the old silent-
        // primary fallback this would have no-opped ("Already up to date")
        // and adv.txt would still be missing.
        assert!(
            sib.join("adv.txt").exists(),
            "merge should have advanced the feature worktree"
        );
    }

    #[tokio::test]
    async fn merge_with_unresolved_stream_errors_instead_of_no_opping_primary() {
        let (svc, _sib, _stream_id, _primary, _wt) = setup_behind_worktree().await;

        // None (omitted / field didn't bind) must error, not silently
        // merge into the primary worktree.
        assert!(svc.merge(None, "main".into()).await.is_err());
        // A syntactically-invalid stream id errors.
        assert!(svc
            .merge(Some("not-a-stream"), "main".into())
            .await
            .is_err());
        // A well-formed but unknown stream id errors.
        assert!(svc.merge(Some("str999"), "main".into()).await.is_err());
    }

    #[tokio::test]
    async fn rebase_and_commit_all_reject_unresolved_streams() {
        let (svc, _sib, _stream_id, _primary, _wt) = setup_behind_worktree().await;
        assert!(svc.rebase(None, "main".into()).await.is_err());
        assert!(svc.rebase(Some("str999"), "main".into()).await.is_err());
        assert!(svc.commit_all(None, "msg".into()).await.is_err());
        assert!(svc.commit_all(Some("str999"), "msg".into()).await.is_err());
    }

    /// Build a primary repo on `main` + a sibling `feature` worktree where
    /// `main` and `feature` each committed a *different* version of the
    /// same line in `cfg.txt`. Merging main into feature makes git report a
    /// conflict (line-level), which the smart-merge pass may or may not be
    /// able to auto-resolve depending on whether the edits truly overlap.
    async fn setup_conflicting_worktree(
        base: &str,
        main_v: &str,
        feature_v: &str,
    ) -> (
        Arc<GitService>,
        PathBuf,
        StreamId,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let primary = tempfile::tempdir().unwrap();
        let p = primary.path();
        run_git(p, &["init", "-q", "--initial-branch=main"]);
        run_git(p, &["config", "user.email", "test@example.com"]);
        run_git(p, &["config", "user.name", "test"]);
        write_commit(p, "cfg.txt", base, "base");
        run_git(p, &["branch", "feature"]);

        let wt_parent = tempfile::tempdir().unwrap();
        let sib = wt_parent.path().join("feature-wt");
        run_git(
            p,
            &["worktree", "add", "-q", sib.to_str().unwrap(), "feature"],
        );
        // Divergent edits to the same line on each branch.
        write_commit(p, "cfg.txt", main_v, "main edit");
        write_commit(&sib, "cfg.txt", feature_v, "feature edit");

        let db = Database::in_memory();
        let store = Arc::new(SqliteStreamStore::new(db));
        let stream_id = StreamId::new(2);
        let now = Timestamp::now();
        store
            .upsert(&Stream {
                id: stream_id,
                kind: StreamKind::Worktree,
                title: "feature".into(),
                branch: "feature".into(),
                branch_ref: "refs/heads/feature".into(),
                branch_source: "main".into(),
                worktree_path: sib.to_string_lossy().into_owned(),
                working_pane: String::new(),
                talking_pane: String::new(),
                working_session_id: String::new(),
                talking_session_id: String::new(),
                custom_prompt: None,
                created_at: now,
                updated_at: now,
                archived_at: None,
            })
            .await
            .unwrap();

        let svc = GitService::new(
            Arc::new(WorktreeRouter::new(p.to_path_buf(), store)),
            EventBus::new(),
        );
        (svc, sib, stream_id, primary, wt_parent)
    }

    #[tokio::test]
    async fn merge_auto_resolves_same_line_different_word_conflict() {
        // main changes the first word, feature changes the last word of the
        // same line. Git's line-based merge conflicts; the smart-merge pass
        // recognises the edits don't overlap and resolves them.
        let (svc, sib, stream_id, _primary, _wt) = setup_conflicting_worktree(
            "alpha beta gamma\n",
            "ALPHA beta gamma\n",
            "alpha beta GAMMA\n",
        )
        .await;

        let res = svc
            .merge(Some(&stream_id.to_string()), "main".into())
            .await
            .unwrap();

        // Git itself reported a conflict, but we auto-resolved one file.
        assert_eq!(res.auto_resolved, 1, "stderr: {}", res.stderr);

        let merged = std::fs::read_to_string(sib.join("cfg.txt")).unwrap();
        assert_eq!(merged, "ALPHA beta GAMMA\n");
        assert!(
            !merged.contains("<<<<<<<"),
            "no conflict markers should remain: {merged:?}"
        );
        // No unmerged paths left in the worktree.
        let state = oxplow_git::get_repo_conflict_state(&sib);
        assert_eq!(state.conflicted_count, 0);
    }

    #[tokio::test]
    async fn merge_leaves_true_overlap_conflicted() {
        // Both branches change the *same* word differently — a real overlap
        // the smart pass must refuse to auto-resolve.
        let (svc, sib, stream_id, _primary, _wt) = setup_conflicting_worktree(
            "let timeout = 10;\n",
            "let timeout = 20;\n",
            "let timeout = 30;\n",
        )
        .await;

        let res = svc
            .merge(Some(&stream_id.to_string()), "main".into())
            .await
            .unwrap();

        assert_eq!(res.auto_resolved, 0);
        let contents = std::fs::read_to_string(sib.join("cfg.txt")).unwrap();
        assert!(
            contents.contains("<<<<<<<"),
            "git's conflict markers must be left intact: {contents:?}"
        );
        let state = oxplow_git::get_repo_conflict_state(&sib);
        assert_eq!(state.conflicted_count, 1);
    }

    #[tokio::test]
    async fn rebase_auto_resolves_same_line_different_word_conflict() {
        // The auto-resolve pass is operation-agnostic: a rebase that git
        // leaves conflicted on a same-line/different-word edit resolves the
        // same way a merge does.
        let (svc, sib, stream_id, _primary, _wt) = setup_conflicting_worktree(
            "alpha beta gamma\n",
            "ALPHA beta gamma\n",
            "alpha beta GAMMA\n",
        )
        .await;

        let res = svc
            .rebase(Some(&stream_id.to_string()), "main".into())
            .await
            .unwrap();

        assert_eq!(res.auto_resolved, 1, "stderr: {}", res.stderr);
        let merged = std::fs::read_to_string(sib.join("cfg.txt")).unwrap();
        assert_eq!(merged, "ALPHA beta GAMMA\n");
        assert!(!merged.contains("<<<<<<<"));
        let state = oxplow_git::get_repo_conflict_state(&sib);
        assert_eq!(state.conflicted_count, 0);
    }

    #[tokio::test]
    async fn cherry_pick_auto_resolves_same_line_different_word_conflict() {
        // Cherry-pick main's edit onto feature, which touched a different
        // word of the same line — git conflicts, the smart pass resolves.
        let (svc, sib, stream_id, primary, _wt) = setup_conflicting_worktree(
            "alpha beta gamma\n",
            "ALPHA beta gamma\n",
            "alpha beta GAMMA\n",
        )
        .await;
        let main_sha = git_sha(primary.path(), "main");

        let res = svc
            .cherry_pick(Some(&stream_id.to_string()), main_sha)
            .await
            .unwrap();

        assert_eq!(res.auto_resolved, 1, "stderr: {}", res.stderr);
        let merged = std::fs::read_to_string(sib.join("cfg.txt")).unwrap();
        assert_eq!(merged, "ALPHA beta GAMMA\n");
        assert!(!merged.contains("<<<<<<<"));
        let state = oxplow_git::get_repo_conflict_state(&sib);
        assert_eq!(state.conflicted_count, 0);
    }
}
