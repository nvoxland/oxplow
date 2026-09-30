//! Cores for the `git` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use std::collections::HashMap;

use oxplow_app::Services;
use oxplow_git::{ChangeScopes, CommitRefLabel, GitOpResult, RemoteBranchEntry, TextSearchHit};

use crate::error::IpcError;

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

pub async fn git_cherry_pick(
    svc: &Services,
    stream_id: Option<String>,
    commit: String,
) -> Result<GitOpResult, IpcError> {
    svc.git
        .cherry_pick(stream_id.as_deref(), commit)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn git_revert(
    svc: &Services,
    stream_id: Option<String>,
    commit: String,
) -> Result<GitOpResult, IpcError> {
    svc.git
        .revert(stream_id.as_deref(), commit)
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

/// Map commit SHAs to a single user-facing branch/tag label. Used by
/// the Local History dashboard to chip each snapshot with its
/// pinned commit's branch/tag name; SHAs that match no ref are absent
/// from the result (caller renders a short-sha fallback).
pub async fn git_resolve_commit_ref_labels(
    svc: &Services,
    shas: Vec<String>,
) -> Result<HashMap<String, Vec<CommitRefLabel>>, IpcError> {
    Ok(svc.git.resolve_commit_ref_labels(shas).await)
}

pub async fn git_list_recent_remote_branches(
    svc: &Services,
    limit: Option<usize>,
) -> Result<Vec<RemoteBranchEntry>, IpcError> {
    Ok(svc
        .git
        .list_recent_remote_branches(limit.unwrap_or(50))
        .await)
}

pub async fn git_change_scopes(
    svc: &Services,
    stream_id: Option<String>,
) -> Result<ChangeScopes, IpcError> {
    Ok(svc.git.change_scopes(stream_id.as_deref()).await)
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

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn git_cherry_pick_dispatches_and_returns_op_result() {
        let (svc, _dir) = crate::test_support::services();
        let primary = svc.streams.list_streams().await.unwrap();
        let stream_id = primary.first().expect("primary stream").id.to_string();
        // A bogus commit can't be picked, but routing through the strict
        // stream resolution + service still yields a GitOpResult (failure),
        // proving the command is registered and wired.
        let out = crate::dispatch(
            "git_cherry_pick",
            serde_json::json!({ "streamId": stream_id, "commit": "0000000000000000000000000000000000000000" }),
            &svc,
        )
        .await
        .unwrap();
        assert!(
            out.get("success").is_some(),
            "expected GitOpResult, got {out}"
        );
        assert!(
            out.get("auto_resolved").is_some(),
            "expected auto_resolved field, got {out}"
        );
    }

    #[tokio::test]
    async fn git_revert_dispatches_and_returns_op_result() {
        let (svc, _dir) = crate::test_support::services();
        let primary = svc.streams.list_streams().await.unwrap();
        let stream_id = primary.first().expect("primary stream").id.to_string();
        let out = crate::dispatch(
            "git_revert",
            serde_json::json!({ "streamId": stream_id, "commit": "0000000000000000000000000000000000000000" }),
            &svc,
        )
        .await
        .unwrap();
        assert!(
            out.get("success").is_some(),
            "expected GitOpResult, got {out}"
        );
        assert!(
            out.get("auto_resolved").is_some(),
            "expected auto_resolved field, got {out}"
        );
    }
}
