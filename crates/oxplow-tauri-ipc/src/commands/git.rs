use oxplow_git::{
    AheadBehind, BlameLine, BranchChanges, ChangeScopes, CommitRefLabel, GitOpResult,
    GitOperationKind, GitWorktreeEntry, GroupedGitRefs, LocalBlameEntry, RemoteBranchEntry,
    RepoConflictState, TextSearchHit,
};
use std::collections::HashMap;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_repo_conflict_state(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<RepoConflictState, IpcError> {
    oxplow_rpc::commands::git::get_repo_conflict_state(&state, stream_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_ahead_behind(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    base: String,
    head: String,
) -> Result<AheadBehind, IpcError> {
    oxplow_rpc::commands::git::get_ahead_behind(&state, stream_id, base, head).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_stream_divergences(
    state: tauri::State<'_, AppState>,
    base: Option<String>,
) -> Result<oxplow_rpc::commands::git::StreamDivergenceReport, IpcError> {
    oxplow_rpc::commands::git::list_stream_divergences(&state, base).await
}

#[tauri::command]
#[specta::specta]
pub async fn append_to_gitignore(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    entry: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::git::append_to_gitignore(&state, stream_id, entry).await
}

#[tauri::command]
#[specta::specta]
pub async fn restore_path(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    path: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::git::restore_path(&state, stream_id, path).await
}

/// Re-export the operation kind so the TS bindings include it.
pub fn _capture_git_operation_kind() -> GitOperationKind {
    GitOperationKind::Merge
}

#[tauri::command]
#[specta::specta]
pub async fn git_fetch(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    remote: Option<String>,
) -> Result<GitOpResult, IpcError> {
    oxplow_rpc::commands::git::git_fetch(&state, stream_id, remote).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_pull(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<GitOpResult, IpcError> {
    oxplow_rpc::commands::git::git_pull(&state, stream_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_pull_remote_into_current(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    remote: String,
    branch: String,
) -> Result<GitOpResult, IpcError> {
    oxplow_rpc::commands::git::git_pull_remote_into_current(&state, stream_id, remote, branch).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_push(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<GitOpResult, IpcError> {
    oxplow_rpc::commands::git::git_push(&state, stream_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_push_current_to(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    remote: String,
    branch: String,
) -> Result<GitOpResult, IpcError> {
    oxplow_rpc::commands::git::git_push_current_to(&state, stream_id, remote, branch).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_merge_into(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    source: String,
) -> Result<GitOpResult, IpcError> {
    oxplow_rpc::commands::git::git_merge_into(&state, stream_id, source).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_rebase_onto(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    onto: String,
) -> Result<GitOpResult, IpcError> {
    oxplow_rpc::commands::git::git_rebase_onto(&state, stream_id, onto).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_commit_all(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    message: String,
) -> Result<GitOpResult, IpcError> {
    oxplow_rpc::commands::git::git_commit_all(&state, stream_id, message).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_add_path(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    path: String,
) -> Result<GitOpResult, IpcError> {
    oxplow_rpc::commands::git::git_add_path(&state, stream_id, path).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_all_refs(state: tauri::State<'_, AppState>) -> Result<GroupedGitRefs, IpcError> {
    oxplow_rpc::commands::git::list_all_refs(&state).await
}

/// Map commit SHAs to a single user-facing branch/tag label. Used by
/// the Local History dashboard to chip each snapshot with its
/// pinned commit's branch/tag name; SHAs that match no ref are absent
/// from the result (caller renders a short-sha fallback).
#[tauri::command]
#[specta::specta]
pub async fn resolve_commit_ref_labels(
    state: tauri::State<'_, AppState>,
    shas: Vec<String>,
) -> Result<HashMap<String, Vec<CommitRefLabel>>, IpcError> {
    oxplow_rpc::commands::git::resolve_commit_ref_labels(&state, shas).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_recent_remote_branches(
    state: tauri::State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<RemoteBranchEntry>, IpcError> {
    oxplow_rpc::commands::git::list_recent_remote_branches(&state, limit).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_file_commits(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    path: String,
    limit: Option<usize>,
) -> Result<Vec<oxplow_git::GitLogCommit>, IpcError> {
    oxplow_rpc::commands::git::list_file_commits(&state, stream_id, path, limit).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_blame(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    path: String,
) -> Result<Vec<BlameLine>, IpcError> {
    oxplow_rpc::commands::git::git_blame(&state, stream_id, path).await
}

#[tauri::command]
#[specta::specta]
pub async fn local_blame(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    path: String,
    disk_text: String,
) -> Result<Vec<LocalBlameEntry>, IpcError> {
    oxplow_rpc::commands::git::local_blame(&state, stream_id, path, disk_text).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_change_scopes(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<ChangeScopes, IpcError> {
    oxplow_rpc::commands::git::get_change_scopes(&state, stream_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_branch_changes(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    base_ref: String,
) -> Result<BranchChanges, IpcError> {
    oxplow_rpc::commands::git::get_branch_changes(&state, stream_id, base_ref).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_existing_worktrees(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GitWorktreeEntry>, IpcError> {
    oxplow_rpc::commands::git::list_existing_worktrees(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_adoptable_worktrees(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GitWorktreeEntry>, IpcError> {
    oxplow_rpc::commands::git::list_adoptable_worktrees(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn search_workspace_text(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<TextSearchHit>, IpcError> {
    oxplow_rpc::commands::git::search_workspace_text(&state, stream_id, query, limit).await
}

#[tauri::command]
#[specta::specta]
pub async fn read_file_at_ref(
    state: tauri::State<'_, AppState>,
    r#ref: String,
    path: String,
) -> Result<Option<String>, IpcError> {
    oxplow_rpc::commands::git::read_file_at_ref(&state, r#ref, path).await
}
