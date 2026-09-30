//! Git's native reads that have no neutral counterpart yet: change
//! scopes, workspace text search, commit ref labels, recent remote
//! branches, the default branch. Every read shells out live. Mutations are
//! the `vcs.*` / `git.*` bus commands (`commands/vcs.rs`); the rest of
//! this facade folds into the git provider in P5.B7 (`.context/vcs.md`).

use std::path::PathBuf;
use std::sync::Arc;

use oxplow_git::{ChangeScopes, CommitRefLabel, RemoteBranchEntry, TextSearchHit};

use crate::worktrees::WorktreeRouter;

/// Singleton handle. Held inside `Services` as `Arc<GitService>`.
/// Routing a stream to its worktree is the router's; file I/O is
/// `WorkspaceFiles`'; the branch reconciler is its own service.
pub struct GitService {
    router: Arc<WorktreeRouter>,
}

impl GitService {
    pub fn new(router: Arc<WorktreeRouter>) -> Arc<Self> {
        Arc::new(Self { router })
    }

    fn project_dir(&self) -> PathBuf {
        self.router.project_dir().to_path_buf()
    }

    // ---------------------------------------------------------------
    // Reads — every one is a live shell-out via spawn_blocking.
    // ---------------------------------------------------------------

    pub async fn change_scopes(&self, stream_id: Option<&str>) -> ChangeScopes {
        let path = self.router.resolve(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::get_change_scopes(&path))
            .await
            .expect("change_scopes join")
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

    pub async fn detect_default_branch(&self) -> Option<String> {
        let path = self.project_dir();
        tokio::task::spawn_blocking(move || oxplow_git::detect_default_branch(&path))
            .await
            .unwrap_or(None)
    }
}
