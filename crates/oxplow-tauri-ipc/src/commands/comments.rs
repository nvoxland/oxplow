//! Comments — threaded annotations anchored to a text selection on any
//! page. Each mutation emits `CommentsChanged` so the renderer (and any
//! other window) refetches the affected page's comments + the inbox.

use oxplow_domain::{
    CommentId, CommentIntent, CommentMessage, CommentStatus, CommentThread, StreamId,
};

use crate::error::IpcError;
use crate::state::AppState;

pub use oxplow_rpc::commands::comments::CreateCommentRequest;

#[tauri::command]
#[specta::specta]
pub async fn create_comment(
    state: tauri::State<'_, AppState>,
    req: CreateCommentRequest,
) -> Result<CommentThread, IpcError> {
    oxplow_rpc::commands::comments::create_comment(&state, req).await
}

#[tauri::command]
#[specta::specta]
pub async fn add_comment_message(
    state: tauri::State<'_, AppState>,
    comment_id: CommentId,
    author: String,
    body: String,
) -> Result<CommentMessage, IpcError> {
    oxplow_rpc::commands::comments::add_comment_message(&state, comment_id, author, body).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_comments_for_target(
    state: tauri::State<'_, AppState>,
    target_kind: String,
    target_id: String,
) -> Result<Vec<CommentThread>, IpcError> {
    oxplow_rpc::commands::comments::list_comments_for_target(&state, target_kind, target_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_comments_for_stream(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<Vec<CommentThread>, IpcError> {
    oxplow_rpc::commands::comments::list_comments_for_stream(&state, stream_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_comment_intent(
    state: tauri::State<'_, AppState>,
    comment_id: CommentId,
    intent: CommentIntent,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::comments::set_comment_intent(&state, comment_id, intent).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_comment_status(
    state: tauri::State<'_, AppState>,
    comment_id: CommentId,
    status: CommentStatus,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::comments::set_comment_status(&state, comment_id, status).await
}

/// Persist a re-resolved anchor hint (and orphan flag) after the
/// renderer re-locates — or fails to re-locate — the quote in current
/// content. No event: this is a passive sync, not a user mutation.
#[tauri::command]
#[specta::specta]
pub async fn set_comment_anchor(
    state: tauri::State<'_, AppState>,
    comment_id: CommentId,
    selectors_json: String,
    orphaned: bool,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::comments::set_comment_anchor(&state, comment_id, selectors_json, orphaned)
        .await
}

/// Re-attach an orphaned comment to a freshly-selected span: rewrite
/// both quote + anchor and clear the orphan flag. A user mutation, so it
/// emits a changed event (unlike the passive `set_comment_anchor`).
#[tauri::command]
#[specta::specta]
pub async fn relink_comment(
    state: tauri::State<'_, AppState>,
    comment_id: CommentId,
    quote: String,
    selectors_json: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::comments::relink_comment(&state, comment_id, quote, selectors_json).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_comment(
    state: tauri::State<'_, AppState>,
    comment_id: CommentId,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::comments::delete_comment(&state, comment_id).await
}
