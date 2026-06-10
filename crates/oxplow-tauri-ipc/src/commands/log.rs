use oxplow_git::{CommitDetail, GitLogCommit, GitLogResult};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_git_log(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    limit: Option<u32>,
    all: bool,
) -> Result<GitLogResult, IpcError> {
    oxplow_rpc::commands::log::get_git_log(&state, stream_id, limit, all).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_commit_detail(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    sha: String,
) -> Result<Option<CommitDetail>, IpcError> {
    oxplow_rpc::commands::log::get_commit_detail(&state, stream_id, sha).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_commits_ahead_of(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    base: String,
    head: String,
    limit: u32,
) -> Result<Vec<GitLogCommit>, IpcError> {
    oxplow_rpc::commands::log::get_commits_ahead_of(&state, stream_id, base, head, limit).await
}
