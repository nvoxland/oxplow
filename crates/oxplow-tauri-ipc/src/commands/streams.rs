use oxplow_domain::{Stream, StreamId};

use crate::error::IpcError;
use crate::state::AppState;

pub use oxplow_rpc::commands::streams::{
    AdoptWorktreeRequest, CreateWorktreeRequest, RenameStreamRequest, SetStreamPromptRequest,
};

#[tauri::command]
#[specta::specta]
pub async fn list_streams(state: tauri::State<'_, AppState>) -> Result<Vec<Stream>, IpcError> {
    oxplow_rpc::commands::streams::list_streams(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_worktree(
    state: tauri::State<'_, AppState>,
    req: CreateWorktreeRequest,
) -> Result<Stream, IpcError> {
    oxplow_rpc::commands::streams::create_worktree(&state, req).await
}

/// Register an on-disk git worktree as a new stream without
/// running `git worktree add`. Source of valid paths is
/// `list_adoptable_worktrees`; the renderer's New Stream form's
/// "worktree" mode dispatches here.
#[tauri::command]
#[specta::specta]
pub async fn adopt_worktree(
    state: tauri::State<'_, AppState>,
    req: AdoptWorktreeRequest,
) -> Result<Stream, IpcError> {
    oxplow_rpc::commands::streams::adopt_worktree(&state, req).await
}

/// Soft-delete a stream and every thread under it via `archived_at`.
/// Refuses if any thread in the stream has a pane currently in the
/// `Running` state — the user must wait for the agent to settle (or
/// stop it) before removing the stream. When `delete_worktree` is
/// true the on-disk worktree directory is also removed.
#[tauri::command]
#[specta::specta]
pub async fn archive_stream(
    state: tauri::State<'_, AppState>,
    id: StreamId,
    delete_worktree: bool,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::streams::archive_stream(&state, id, delete_worktree).await
}

/// Returns the primary stream — the project root. Useful for any UI
/// path that needs to know "what does the user think of as 'this'
/// project?" without enumerating the full list.
#[tauri::command]
#[specta::specta]
pub async fn get_primary_stream(
    state: tauri::State<'_, AppState>,
) -> Result<Option<Stream>, IpcError> {
    oxplow_rpc::commands::streams::get_primary_stream(&state).await
}

/// Currently-selected stream (None falls back to primary in the UI).
#[tauri::command]
#[specta::specta]
pub async fn get_current_stream(
    state: tauri::State<'_, AppState>,
) -> Result<Option<Stream>, IpcError> {
    oxplow_rpc::commands::streams::get_current_stream(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn switch_stream(
    state: tauri::State<'_, AppState>,
    id: Option<StreamId>,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::streams::switch_stream(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn rename_stream(
    state: tauri::State<'_, AppState>,
    req: RenameStreamRequest,
) -> Result<Stream, IpcError> {
    oxplow_rpc::commands::streams::rename_stream(&state, req).await
}

/// Per-stream custom prompt — appended to every agent system prompt
/// when this stream is active. `None` (or empty) clears it.
#[tauri::command]
#[specta::specta]
pub async fn set_stream_prompt(
    state: tauri::State<'_, AppState>,
    req: SetStreamPromptRequest,
) -> Result<Stream, IpcError> {
    oxplow_rpc::commands::streams::set_stream_prompt(&state, req).await
}

/// Switch the worktree's HEAD branch. Updates the stream row and runs
/// `git checkout` inside the worktree.
#[tauri::command]
#[specta::specta]
pub async fn reorder_streams(
    state: tauri::State<'_, AppState>,
    order: Vec<StreamId>,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::streams::reorder_streams(&state, order).await
}

#[tauri::command]
#[specta::specta]
pub async fn checkout_stream_branch(
    state: tauri::State<'_, AppState>,
    id: StreamId,
    branch: String,
) -> Result<Stream, IpcError> {
    oxplow_rpc::commands::streams::checkout_stream_branch(&state, id, branch).await
}
