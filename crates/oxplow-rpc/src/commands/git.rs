//! Cores for the `git` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use std::collections::HashMap;

use oxplow_app::Services;
use oxplow_domain::stores::StreamStore;
use oxplow_git::{
    AheadBehind, BlameLine, BranchChanges, ChangeScopes, CommitRefLabel, GitOpResult,
    GitWorktreeEntry, GroupedGitRefs, LocalBlameEntry, RemoteBranchEntry, RepoConflictState,
    TextSearchHit,
};

use crate::error::IpcError;

pub async fn get_repo_conflict_state(
    svc: &Services,
    stream_id: Option<String>,
) -> Result<RepoConflictState, IpcError> {
    Ok(svc.git.conflict_state(stream_id.as_deref()).await)
}

pub async fn get_ahead_behind(
    svc: &Services,
    stream_id: Option<String>,
    base: String,
    head: String,
) -> Result<AheadBehind, IpcError> {
    Ok(svc.git.ahead_behind(stream_id.as_deref(), base, head).await)
}

pub async fn append_to_gitignore(
    svc: &Services,
    stream_id: Option<String>,
    entry: String,
) -> Result<(), IpcError> {
    svc.git
        .append_to_gitignore(stream_id.as_deref(), entry)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn restore_path(
    svc: &Services,
    stream_id: Option<String>,
    path: String,
) -> Result<(), IpcError> {
    svc.git
        .restore_path(stream_id.as_deref(), path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn git_fetch(
    svc: &Services,
    stream_id: Option<String>,
    remote: Option<String>,
) -> Result<GitOpResult, IpcError> {
    svc.git
        .fetch(stream_id.as_deref(), remote)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn git_pull(svc: &Services, stream_id: Option<String>) -> Result<GitOpResult, IpcError> {
    svc.git
        .pull(stream_id.as_deref())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn git_pull_remote_into_current(
    svc: &Services,
    stream_id: Option<String>,
    remote: String,
    branch: String,
) -> Result<GitOpResult, IpcError> {
    svc.git
        .pull_remote_into_current(stream_id.as_deref(), remote, branch)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn git_push(svc: &Services, stream_id: Option<String>) -> Result<GitOpResult, IpcError> {
    svc.git
        .push(stream_id.as_deref())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn git_push_current_to(
    svc: &Services,
    stream_id: Option<String>,
    remote: String,
    branch: String,
) -> Result<GitOpResult, IpcError> {
    svc.git
        .push_current_to(stream_id.as_deref(), remote, branch)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn git_merge_into(
    svc: &Services,
    stream_id: Option<String>,
    source: String,
) -> Result<GitOpResult, IpcError> {
    svc.git
        .merge(stream_id.as_deref(), source)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn git_rebase_onto(
    svc: &Services,
    stream_id: Option<String>,
    onto: String,
) -> Result<GitOpResult, IpcError> {
    svc.git
        .rebase(stream_id.as_deref(), onto)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn git_commit_all(
    svc: &Services,
    stream_id: Option<String>,
    message: String,
) -> Result<GitOpResult, IpcError> {
    svc.git
        .commit_all(stream_id.as_deref(), message)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn git_add_path(
    svc: &Services,
    stream_id: Option<String>,
    path: String,
) -> Result<GitOpResult, IpcError> {
    svc.git
        .add_path(stream_id.as_deref(), path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn list_all_refs(svc: &Services) -> Result<GroupedGitRefs, IpcError> {
    Ok(svc.git.list_all_refs().await)
}

/// Map commit SHAs to a single user-facing branch/tag label. Used by
/// the Local History dashboard to chip each snapshot with its
/// pinned commit's branch/tag name; SHAs that match no ref are absent
/// from the result (caller renders a short-sha fallback).
pub async fn resolve_commit_ref_labels(
    svc: &Services,
    shas: Vec<String>,
) -> Result<HashMap<String, Vec<CommitRefLabel>>, IpcError> {
    Ok(svc.git.resolve_commit_ref_labels(shas).await)
}

pub async fn list_recent_remote_branches(
    svc: &Services,
    limit: Option<usize>,
) -> Result<Vec<RemoteBranchEntry>, IpcError> {
    Ok(svc
        .git
        .list_recent_remote_branches(limit.unwrap_or(50))
        .await)
}

pub async fn list_file_commits(
    svc: &Services,
    stream_id: Option<String>,
    path: String,
    limit: Option<usize>,
) -> Result<Vec<oxplow_git::GitLogCommit>, IpcError> {
    Ok(svc
        .git
        .list_file_commits(stream_id.as_deref(), path, limit.unwrap_or(50))
        .await)
}

pub async fn git_blame(
    svc: &Services,
    stream_id: Option<String>,
    path: String,
) -> Result<Vec<BlameLine>, IpcError> {
    Ok(svc.git.blame(stream_id.as_deref(), path).await)
}

pub async fn local_blame(
    svc: &Services,
    stream_id: Option<String>,
    path: String,
    disk_text: String,
) -> Result<Vec<LocalBlameEntry>, IpcError> {
    Ok(svc
        .git
        .local_blame(stream_id.as_deref(), path, disk_text)
        .await)
}

pub async fn get_change_scopes(
    svc: &Services,
    stream_id: Option<String>,
) -> Result<ChangeScopes, IpcError> {
    Ok(svc.git.change_scopes(stream_id.as_deref()).await)
}

pub async fn get_branch_changes(
    svc: &Services,
    stream_id: Option<String>,
    base_ref: String,
) -> Result<BranchChanges, IpcError> {
    Ok(svc.git.branch_changes(stream_id.as_deref(), base_ref).await)
}

pub async fn list_existing_worktrees(svc: &Services) -> Result<Vec<GitWorktreeEntry>, IpcError> {
    Ok(svc.git.list_existing_worktrees().await)
}

pub async fn list_adoptable_worktrees(svc: &Services) -> Result<Vec<GitWorktreeEntry>, IpcError> {
    let store = oxplow_db::SqliteStreamStore::new(svc.db.clone());
    let registered: Vec<String> = store
        .list()
        .await?
        .into_iter()
        .map(|s| s.worktree_path)
        .collect();
    Ok(svc.git.list_adoptable_worktrees(registered).await)
}

pub async fn search_workspace_text(
    svc: &Services,
    stream_id: Option<String>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<TextSearchHit>, IpcError> {
    Ok(svc
        .git
        .search_workspace_text(stream_id.as_deref(), query, limit)
        .await)
}

pub async fn read_file_at_ref(
    svc: &Services,
    r#ref: String,
    path: String,
) -> Result<Option<String>, IpcError> {
    Ok(svc.git.read_file_at_ref(r#ref, path).await)
}

/// One stream's divergence row for the Git Dashboard "Streams" panel.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct StreamDivergenceRow {
    pub stream_id: String,
    pub title: String,
    pub branch: String,
    pub is_primary: bool,
    pub ahead: u32,
    pub behind: u32,
    pub overlapping_files: Vec<String>,
    pub readiness: oxplow_git::MergeReadiness,
}

/// Cross-stream divergence report: each stream/worktree's ahead/behind
/// and merge-readiness vs the integration branch `base`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct StreamDivergenceReport {
    pub base: String,
    pub rows: Vec<StreamDivergenceRow>,
}

/// Compute divergence + merge-readiness for every stream against the
/// integration branch. `base` defaults to the detected default branch
/// (`main`/`master`), then `"main"` if detection fails.
pub async fn list_stream_divergences(
    svc: &Services,
    base: Option<String>,
) -> Result<StreamDivergenceReport, IpcError> {
    let base = match base {
        Some(b) if !b.trim().is_empty() => b,
        _ => svc
            .git
            .detect_default_branch()
            .await
            .unwrap_or_else(|| "main".to_string()),
    };

    let streams = svc.streams.list_streams().await?;
    let mut rows = Vec::with_capacity(streams.len());
    for s in streams {
        let d = svc
            .git
            .divergence(None, base.clone(), s.branch.clone())
            .await;
        rows.push(StreamDivergenceRow {
            stream_id: s.id.to_string(),
            title: s.title,
            branch: s.branch,
            is_primary: matches!(s.kind, oxplow_domain::StreamKind::Primary),
            ahead: d.ahead,
            behind: d.behind,
            overlapping_files: d.overlapping_files,
            readiness: d.readiness,
        });
    }
    Ok(StreamDivergenceReport { base, rows })
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn get_repo_conflict_state_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "get_repo_conflict_state",
            serde_json::json!({ "streamId": null }),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_object(), "expected a JSON object, got {out}");
    }

    #[tokio::test]
    async fn list_stream_divergences_dispatches_and_returns_report() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_stream_divergences",
            serde_json::json!({ "base": null }),
            &svc,
        )
        .await
        .unwrap();
        assert!(
            out.get("base").is_some(),
            "expected a base field, got {out}"
        );
        assert!(
            out.get("rows").unwrap().is_array(),
            "rows should be an array"
        );
    }

    #[tokio::test]
    async fn read_file_at_ref_dispatches_and_returns_null_for_missing_path() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "read_file_at_ref",
            serde_json::json!({ "ref": "HEAD", "path": "no/such/file.txt" }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(out, serde_json::json!(null));
    }
}
