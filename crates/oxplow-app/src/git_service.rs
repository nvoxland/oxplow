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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_domain::stores::StreamStore;
use oxplow_domain::{StreamId, Timestamp};
use oxplow_git::{
    detect_current_branch, AheadBehind, BlameLine, BranchChanges, BranchRef, ChangeScopes,
    CommitDetail, CommitRefLabel, GitFileStatus, GitLogCommit, GitLogOptions, GitLogResult,
    GitOpResult, GitWorktreeEntry, GroupedGitRefs, LocalBlameEntry, RemoteBranchEntry,
    RepoConflictState, TextSearchHit, WorkspaceEntry, WorkspaceFile, WorkspaceIndexedFile,
    WorkspaceStatusSummary,
};
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::events::{EventBus, OxplowEvent, WorkspaceChangeKind};

/// Singleton handle. Held inside `Services` as `Arc<GitService>`.
pub struct GitService {
    project_dir: PathBuf,
    streams: Arc<dyn StreamStore>,
    events: EventBus,
    /// stream_id → worktree path. The only mutable state we keep —
    /// it's a routing table, not a cache. `register/deregister`
    /// maintain it as streams come and go.
    worktrees: Arc<RwLock<HashMap<StreamId, PathBuf>>>,
}

impl GitService {
    /// Build the service. Existing streams are seeded asynchronously
    /// from `streams.list()` so callers don't have to await; reads
    /// against unseeded streams fall back to the project root via
    /// `resolve_repo_dir`.
    ///
    /// A small bus listener is spawned to keep the per-stream `branch`
    /// field reconciled against the live HEAD whenever refs move.
    /// That's persistent state in the stream record — not a cache.
    pub fn spawn(
        project_dir: PathBuf,
        streams: Arc<dyn StreamStore>,
        events: EventBus,
    ) -> Arc<Self> {
        let svc = Arc::new(Self {
            project_dir,
            streams: streams.clone(),
            events: events.clone(),
            worktrees: Arc::new(RwLock::new(HashMap::new())),
        });

        // Reconcile-branch listener.
        Self::spawn_branch_reconciler(svc.clone());

        // Seed known streams + reconcile each one's branch from HEAD.
        let seed_svc = svc.clone();
        tokio::spawn(async move {
            if let Ok(rows) = streams.list().await {
                for s in rows {
                    let path = resolve_worktree(&seed_svc.project_dir, &s.worktree_path);
                    seed_svc.register(&s.id, path.clone()).await;
                    seed_svc.reconcile_branch(&s.id, &path).await;
                }
            }
        });

        svc
    }

    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    /// Resolve a stream id (or `None` → project root) to a worktree
    /// path.
    pub async fn resolve_repo_dir(&self, stream_id: Option<&str>) -> PathBuf {
        let Some(id) = stream_id else {
            return self.project_dir.clone();
        };
        let Some(id) = StreamId::try_from_str(id) else {
            return self.project_dir.clone();
        };
        let map = self.worktrees.read().await;
        if let Some(p) = map.get(&id) {
            return p.clone();
        }
        drop(map);
        // Stream may exist in the store but not yet registered (e.g.
        // an IPC call landing before boot finished seeding). Look it
        // up directly so we still hand back a useful path.
        if let Ok(rows) = self.streams.list().await {
            if let Some(s) = rows.into_iter().find(|s| s.id == id) {
                return resolve_worktree(&self.project_dir, &s.worktree_path);
            }
        }
        self.project_dir.clone()
    }

    /// Resolve a stream id to its worktree path for a **stream-scoped
    /// destructive op** (merge / rebase / commit_all), erroring rather
    /// than silently falling back to the primary worktree.
    ///
    /// `resolve_repo_dir` treats an absent or unresolvable `stream_id`
    /// as "use the project root". That's fine for reads and for ops
    /// that genuinely default to primary, but for a merge/rebase/commit
    /// it's a footgun: a caller that meant stream B but sent a field
    /// that didn't bind (e.g. snake_case `stream_id` where the wire
    /// field is camelCase `streamId`, so it arrives as `None`) would
    /// run the op against the PRIMARY worktree and get a misleading
    /// "Already up to date" success on the wrong branch. These ops must
    /// name a stream that actually resolves; anything else is an error.
    async fn resolve_stream_worktree_strict(
        &self,
        stream_id: Option<&str>,
    ) -> std::io::Result<PathBuf> {
        let Some(id_str) = stream_id else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "this git operation is stream-scoped and requires a stream id; \
                 refusing to fall back to the primary worktree",
            ));
        };
        let Some(id) = StreamId::try_from_str(id_str) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{id_str:?} is not a valid stream id"),
            ));
        };
        {
            let map = self.worktrees.read().await;
            if let Some(p) = map.get(&id) {
                return Ok(p.clone());
            }
        }
        // Not yet registered (e.g. an IPC call landing before boot
        // finished seeding) — look the stream up directly before giving
        // up, mirroring `resolve_repo_dir`.
        if let Ok(rows) = self.streams.list().await {
            if let Some(s) = rows.into_iter().find(|s| s.id == id) {
                return Ok(resolve_worktree(&self.project_dir, &s.worktree_path));
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no stream found for id {id_str:?}"),
        ))
    }

    /// Register a stream's worktree with the service. Idempotent.
    pub async fn register(&self, stream_id: &StreamId, worktree: PathBuf) {
        let mut map = self.worktrees.write().await;
        map.insert(*stream_id, worktree);
    }

    /// Drop a stream from the service. Used when a stream is deleted.
    pub async fn deregister(&self, stream_id: &StreamId) {
        let mut map = self.worktrees.write().await;
        map.remove(stream_id);
    }

    fn spawn_branch_reconciler(svc: Arc<Self>) {
        let mut rx = svc.events.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(OxplowEvent::GitRefsChanged { stream_id }) => {
                        let path = svc.resolve_repo_dir(Some(&stream_id.to_string())).await;
                        svc.reconcile_branch(&stream_id, &path).await;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// Compare the worktree's live HEAD against the Stream record's
    /// stored `branch` and persist the new value if they diverged.
    ///
    /// External `git checkout`s — run while oxplow is off, or run from
    /// a terminal during a session — never touch the Stream record on
    /// their own; without this step the bottom-bar branch chip and any
    /// other consumer of `stream.branch` keep showing the branch the
    /// stream was created on.
    async fn reconcile_branch(&self, stream_id: &StreamId, worktree: &Path) {
        let Some(detected) = detect_current_branch(worktree) else {
            return;
        };
        let Ok(Some(mut stored)) = self.streams.get(stream_id).await else {
            return;
        };
        if stored.branch == detected {
            return;
        }
        stored.branch = detected.clone();
        stored.branch_ref = format!("refs/heads/{detected}");
        stored.updated_at = Timestamp::now();
        if let Err(e) = self.streams.upsert(&stored).await {
            warn!(stream_id = %stream_id, error = %e, "failed to persist reconciled branch");
            return;
        }
        debug!(stream_id = %stream_id, branch = %detected, "reconciled stream branch from HEAD");
        self.events.emit(OxplowEvent::StreamsChanged);
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

    pub async fn status_summary(&self, stream_id: Option<&str>) -> WorkspaceStatusSummary {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || {
            let map = oxplow_git::list_git_statuses(&path);
            oxplow_git::summarize_git_statuses(&map)
        })
        .await
        .unwrap_or_default()
    }

    pub async fn statuses(&self, stream_id: Option<&str>) -> HashMap<String, GitFileStatus> {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::list_git_statuses(&path))
            .await
            .unwrap_or_default()
    }

    pub async fn branches_for(&self, stream_id: Option<&str>) -> Vec<BranchRef> {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::list_branches(path))
            .await
            .unwrap_or_default()
    }

    /// `list_branches` against the project root — used by the shared
    /// branch picker that doesn't sit inside a specific stream.
    pub async fn list_branches_project(&self) -> Vec<BranchRef> {
        let path = self.project_dir.clone();
        tokio::task::spawn_blocking(move || oxplow_git::list_branches(path))
            .await
            .unwrap_or_default()
    }

    pub async fn conflict_state(&self, stream_id: Option<&str>) -> RepoConflictState {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::get_repo_conflict_state(&path))
            .await
            .expect("conflict_state join")
    }

    pub async fn ahead_behind(
        &self,
        stream_id: Option<&str>,
        base: String,
        head: String,
    ) -> AheadBehind {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::get_ahead_behind(&path, &base, &head))
            .await
            .expect("ahead_behind join")
    }

    /// Resolved 40-char sha for HEAD in `stream_id`'s worktree.
    /// Returns `None` when the directory isn't a git repo or HEAD
    /// is unborn.
    pub async fn head_commit_sha(&self, stream_id: Option<&str>) -> Option<String> {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::head_commit_sha(&path))
            .await
            .ok()
            .flatten()
    }

    pub async fn change_scopes(&self, stream_id: Option<&str>) -> ChangeScopes {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::get_change_scopes(&path))
            .await
            .expect("change_scopes join")
    }

    pub async fn branch_changes(&self, stream_id: Option<&str>, base_ref: String) -> BranchChanges {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::list_branch_changes(&path, &base_ref))
            .await
            .expect("branch_changes join")
    }

    pub async fn git_log(&self, stream_id: Option<&str>, opts: GitLogOptions) -> GitLogResult {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::get_git_log(&path, opts))
            .await
            .expect("git_log join")
    }

    pub async fn commit_detail(
        &self,
        stream_id: Option<&str>,
        sha: String,
    ) -> Option<CommitDetail> {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::get_commit_detail(&path, &sha))
            .await
            .unwrap_or(None)
    }

    pub async fn commits_ahead_of(
        &self,
        stream_id: Option<&str>,
        base: String,
        head: String,
        limit: usize,
    ) -> Vec<GitLogCommit> {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || {
            oxplow_git::get_commits_ahead_of(&path, &base, &head, limit)
        })
        .await
        .unwrap_or_default()
    }

    pub async fn blame(&self, stream_id: Option<&str>, path: String) -> Vec<BlameLine> {
        let dir = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::git_blame(&dir, &path))
            .await
            .unwrap_or_default()
    }

    pub async fn local_blame(
        &self,
        stream_id: Option<&str>,
        path: String,
        disk_text: String,
    ) -> Vec<LocalBlameEntry> {
        let dir = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::local_blame(&dir, &path, &disk_text))
            .await
            .unwrap_or_default()
    }

    pub async fn list_file_commits(
        &self,
        stream_id: Option<&str>,
        path: String,
        limit: usize,
    ) -> Vec<GitLogCommit> {
        let dir = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::list_file_commits(&dir, &path, limit))
            .await
            .unwrap_or_default()
    }

    pub async fn read_file_at_ref(&self, r#ref: String, path: String) -> Option<String> {
        let project = self.project_dir.clone();
        tokio::task::spawn_blocking(move || oxplow_git::read_file_at_ref(&project, &r#ref, &path))
            .await
            .unwrap_or(None)
    }

    pub async fn search_workspace_text(
        &self,
        stream_id: Option<&str>,
        query: String,
        limit: Option<usize>,
    ) -> Vec<TextSearchHit> {
        let dir = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::search_workspace_text(&dir, &query, limit))
            .await
            .unwrap_or_default()
    }

    pub async fn list_all_refs(&self) -> GroupedGitRefs {
        let path = self.project_dir.clone();
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
        let path = self.project_dir.clone();
        tokio::task::spawn_blocking(move || oxplow_git::resolve_commit_ref_labels(&path, &shas))
            .await
            .unwrap_or_default()
    }

    pub async fn list_recent_remote_branches(&self, limit: usize) -> Vec<RemoteBranchEntry> {
        let path = self.project_dir.clone();
        tokio::task::spawn_blocking(move || oxplow_git::list_recent_remote_branches(&path, limit))
            .await
            .unwrap_or_default()
    }

    pub async fn list_existing_worktrees(&self) -> Vec<GitWorktreeEntry> {
        let path = self.project_dir.clone();
        tokio::task::spawn_blocking(move || oxplow_git::list_existing_worktrees(&path))
            .await
            .unwrap_or_default()
    }

    pub async fn list_adoptable_worktrees(&self, registered: Vec<String>) -> Vec<GitWorktreeEntry> {
        let path = self.project_dir.clone();
        tokio::task::spawn_blocking(move || {
            oxplow_git::list_adoptable_worktrees(&path, &registered)
        })
        .await
        .unwrap_or_default()
    }

    pub async fn detect_default_branch(&self) -> Option<String> {
        let path = self.project_dir.clone();
        tokio::task::spawn_blocking(move || oxplow_git::detect_default_branch(&path))
            .await
            .unwrap_or(None)
    }

    // ---------------------------------------------------------------
    // Workspace-file pass-throughs.
    // ---------------------------------------------------------------

    pub async fn list_workspace_entries(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
    ) -> Result<Vec<WorkspaceEntry>, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        let statuses = self.statuses(stream_id).await;
        tokio::task::spawn_blocking(move || {
            oxplow_git::list_workspace_entries(&root, &relative_path, &statuses)
        })
        .await
        .expect("workspace entries join")
    }

    /// `filter` carries the project's `generated:` exclusions (built by
    /// the caller from config) — excluded directories are pruned from
    /// the walk, keeping build/vendor trees out of the quick-open index.
    pub async fn list_workspace_files(
        &self,
        stream_id: Option<&str>,
        filter: oxplow_fs_watch::WorkspaceFilter,
    ) -> Result<Vec<WorkspaceIndexedFile>, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        let statuses = self.statuses(stream_id).await;
        tokio::task::spawn_blocking(move || {
            oxplow_git::list_workspace_files(&root, &statuses, "", &|path| {
                filter.ignore(std::path::Path::new(path))
            })
        })
        .await
        .expect("workspace files join")
    }

    pub async fn read_workspace_file(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
    ) -> Result<WorkspaceFile, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::read_workspace_file(&root, &relative_path))
            .await
            .expect("read workspace file join")
    }

    pub async fn write_workspace_file(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
        content: String,
    ) -> Result<WorkspaceFile, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        let result = tokio::task::spawn_blocking(move || {
            oxplow_git::write_workspace_file(&root, &relative_path, &content)
        })
        .await
        .expect("write workspace file join")?;
        self.announce_write(stream_id_from(stream_id).as_ref(), false);
        Ok(result)
    }

    pub async fn create_workspace_file(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
        content: String,
    ) -> Result<WorkspaceFile, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        let result = tokio::task::spawn_blocking(move || {
            oxplow_git::create_workspace_file(&root, &relative_path, &content)
        })
        .await
        .expect("create workspace file join")?;
        self.announce_write(stream_id_from(stream_id).as_ref(), false);
        Ok(result)
    }

    pub async fn create_workspace_directory(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
    ) -> Result<String, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || {
            oxplow_git::create_workspace_directory(&root, &relative_path)
        })
        .await
        .expect("create workspace dir join")
    }

    pub async fn rename_workspace_path(
        &self,
        stream_id: Option<&str>,
        from_path: String,
        to_path: String,
    ) -> Result<(String, String), oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        let result = tokio::task::spawn_blocking(move || {
            oxplow_git::rename_workspace_path(&root, &from_path, &to_path)
        })
        .await
        .expect("rename workspace path join")?;
        self.announce_write(stream_id_from(stream_id).as_ref(), false);
        Ok(result)
    }

    pub async fn delete_workspace_path(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
    ) -> Result<String, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        let result = tokio::task::spawn_blocking(move || {
            oxplow_git::delete_workspace_path(&root, &relative_path)
        })
        .await
        .expect("delete workspace path join")?;
        self.announce_write(stream_id_from(stream_id).as_ref(), false);
        Ok(result)
    }

    // ---------------------------------------------------------------
    // Mutating ops — pass through to oxplow_git, then emit events.
    // ---------------------------------------------------------------

    pub async fn commit_all(
        &self,
        stream_id: Option<&str>,
        message: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.resolve_stream_worktree_strict(stream_id).await?;
        let result = run_blocking(move || oxplow_git::commit_all(&path, &message)).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    pub async fn add_path(
        &self,
        stream_id: Option<&str>,
        relpath: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.resolve_repo_dir(stream_id).await;
        let result = run_blocking(move || oxplow_git::add_path(&path, &relpath)).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), false);
        Ok(result)
    }

    pub async fn restore_path(
        &self,
        stream_id: Option<&str>,
        relpath: String,
    ) -> std::io::Result<()> {
        let path = self.resolve_repo_dir(stream_id).await;
        run_blocking(move || oxplow_git::restore_path(&path, &relpath)).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), false);
        Ok(())
    }

    pub async fn fetch(
        &self,
        stream_id: Option<&str>,
        remote: Option<String>,
    ) -> std::io::Result<GitOpResult> {
        let path = self.resolve_repo_dir(stream_id).await;
        let result = run_blocking(move || oxplow_git::fetch(&path, remote.as_deref())).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    pub async fn pull(&self, stream_id: Option<&str>) -> std::io::Result<GitOpResult> {
        let path = self.resolve_repo_dir(stream_id).await;
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
        let path = self.resolve_repo_dir(stream_id).await;
        let result =
            run_blocking(move || oxplow_git::pull_remote_into_current(&path, &remote, &branch))
                .await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    pub async fn push(&self, stream_id: Option<&str>) -> std::io::Result<GitOpResult> {
        let path = self.resolve_repo_dir(stream_id).await;
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
        let path = self.resolve_repo_dir(stream_id).await;
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
        let path = self.resolve_stream_worktree_strict(stream_id).await?;
        let result = run_blocking(move || {
            let mut r = oxplow_git::merge(&path, &source)?;
            if !r.success {
                // Git left conflicts (or refused) — run the smart-merge
                // pass to clear the ones that are only line-level conflicts.
                r.auto_resolved = oxplow_git::auto_resolve_conflicts(&path).resolved.len() as u32;
            }
            Ok(r)
        })
        .await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), true);
        Ok(result)
    }

    pub async fn rebase(
        &self,
        stream_id: Option<&str>,
        onto: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.resolve_stream_worktree_strict(stream_id).await?;
        let result = run_blocking(move || {
            let mut r = oxplow_git::rebase(&path, &onto)?;
            if !r.success {
                r.auto_resolved = oxplow_git::auto_resolve_conflicts(&path).resolved.len() as u32;
            }
            Ok(r)
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
        let path = self.project_dir.clone();
        run_blocking_branch(move || oxplow_git::rename_branch(&path, &from, &to)).await?;
        self.broadcast_refs_change_all().await;
        Ok(())
    }

    pub async fn delete_branch(
        &self,
        branch: String,
        force: bool,
    ) -> Result<(), oxplow_git::BranchOpError> {
        let path = self.project_dir.clone();
        run_blocking_branch(move || oxplow_git::delete_branch(&path, &branch, force)).await?;
        self.broadcast_refs_change_all().await;
        Ok(())
    }

    pub async fn append_to_gitignore(
        &self,
        stream_id: Option<&str>,
        entry: String,
    ) -> std::io::Result<()> {
        let path = self.resolve_repo_dir(stream_id).await;
        run_blocking(move || oxplow_git::append_to_gitignore(&path, &entry)).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), false);
        Ok(())
    }

    /// Project-wide refs change — fan out a `GitRefsChanged` for
    /// every registered stream so per-stream subscribers (snapshot
    /// capture, history panel, branch picker) all refresh.
    async fn broadcast_refs_change_all(&self) {
        let ids: Vec<StreamId> = {
            let map = self.worktrees.read().await;
            map.keys().cloned().collect()
        };
        for id in ids {
            self.events
                .emit(OxplowEvent::GitRefsChanged { stream_id: id });
        }
    }
}

fn stream_id_from(s: Option<&str>) -> Option<StreamId> {
    s.and_then(StreamId::try_from_str)
}

fn resolve_worktree(project_dir: &Path, recorded: &str) -> PathBuf {
    let raw = PathBuf::from(recorded);
    if raw.is_absolute() {
        raw
    } else {
        project_dir.join(raw)
    }
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
    use oxplow_domain::{Stream, StreamKind};
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

        let svc = GitService::spawn(p.to_path_buf(), store, EventBus::new());
        svc.register(&stream_id, sib.clone()).await;
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

        let svc = GitService::spawn(p.to_path_buf(), store, EventBus::new());
        svc.register(&stream_id, sib.clone()).await;
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
}
