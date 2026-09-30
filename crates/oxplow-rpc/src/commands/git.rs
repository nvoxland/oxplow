//! Cores for the `git` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use std::collections::HashMap;

use oxplow_app::Services;
use oxplow_git::{ChangeScopes, CommitRefLabel, RemoteBranchEntry, TextSearchHit};

use crate::error::IpcError;

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
