//! Git's native reads, beyond the VCS capability (`.context/vcs.md`):
//! the git provider's own shapes, kept under `git_` names.

use std::collections::HashMap;

use oxplow_app::vcs::{ChangeScopes, CommitRefLabel, RemoteBranchEntry};
use oxplow_app::Services;
use oxplow_domain::DomainError;

use crate::error::IpcError;

/// Map commit SHAs to their user-facing branch/tag labels. Used by
/// the Local History dashboard to chip each snapshot with its
/// pinned commit's branch/tag name; SHAs that match no ref are absent
/// from the result (caller renders a short-sha fallback).
pub async fn git_resolve_commit_ref_labels(
    svc: &Services,
    shas: Vec<String>,
) -> Result<HashMap<String, Vec<CommitRefLabel>>, IpcError> {
    svc.git
        .commit_ref_labels(&svc.layout.project_dir, shas)
        .await
        .map_err(|e| DomainError::from(e).into())
}

pub async fn git_list_recent_remote_branches(
    svc: &Services,
    limit: Option<usize>,
) -> Result<Vec<RemoteBranchEntry>, IpcError> {
    svc.git
        .recent_remote_branches(&svc.layout.project_dir, limit.unwrap_or(50))
        .await
        .map_err(|e| DomainError::from(e).into())
}

pub async fn git_change_scopes(
    svc: &Services,
    stream_id: Option<String>,
) -> Result<ChangeScopes, IpcError> {
    let ws = svc
        .worktrees
        .resolve(stream_id.as_deref())
        .await
        .into_local_path();
    svc.git
        .change_scopes(&ws)
        .await
        .map_err(|e| DomainError::from(e).into())
}
