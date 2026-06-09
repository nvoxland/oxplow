//! Comments — threaded annotations anchored to a text selection on any
//! page. Each mutation emits `CommentsChanged` so the renderer (and any
//! other window) refetches the affected page's comments + the inbox.

use oxplow_app::OxplowEvent;
use oxplow_domain::stores::CommentStore;
use oxplow_domain::{
    Comment, CommentId, CommentIntent, CommentMessage, CommentStatus, CommentTarget, CommentThread,
    StreamId, ThreadId,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::IpcError;
use crate::state::AppState;

fn emit_changed(state: &tauri::State<'_, AppState>, comment: &Comment) {
    state.events.emit(OxplowEvent::CommentsChanged {
        stream_id: comment.stream_id,
        target_kind: comment.target_kind.clone(),
        target_id: comment.target_id.clone(),
    });
}

/// Bundled args for [`create_comment`]. A single struct keeps the
/// command under tauri-specta's argument-count cap and reads better
/// than a dozen positional params. `selectors` is the W3C selectors
/// array (opaque JSON); `context_chain` / `referenced_refs` are the
/// typed context (see [`Comment`]).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateCommentRequest {
    pub stream_id: StreamId,
    pub thread_id: Option<ThreadId>,
    pub target_kind: String,
    pub target_id: String,
    pub quote: String,
    pub selectors_json: String,
    pub context_chain: Vec<CommentTarget>,
    pub referenced_refs: Vec<CommentTarget>,
    pub intent: CommentIntent,
    pub author: String,
    pub body: String,
}

#[tauri::command]
#[specta::specta]
pub async fn create_comment(
    state: tauri::State<'_, AppState>,
    req: CreateCommentRequest,
) -> Result<CommentThread, IpcError> {
    let target = CommentTarget {
        kind: req.target_kind,
        id: req.target_id,
    };
    let thread = state
        .comment_store
        .create(
            &req.stream_id,
            req.thread_id.as_ref(),
            &target,
            &req.quote,
            &req.selectors_json,
            &req.context_chain,
            &req.referenced_refs,
            req.intent,
            &req.author,
            &req.body,
        )
        .await?;
    emit_changed(&state, &thread.comment);
    Ok(thread)
}

#[tauri::command]
#[specta::specta]
pub async fn add_comment_message(
    state: tauri::State<'_, AppState>,
    comment_id: CommentId,
    author: String,
    body: String,
) -> Result<CommentMessage, IpcError> {
    let message = state
        .comment_store
        .add_message(comment_id, &author, &body)
        .await?;
    if let Some(thread) = state.comment_store.get(comment_id).await? {
        emit_changed(&state, &thread.comment);
    }
    Ok(message)
}

#[tauri::command]
#[specta::specta]
pub async fn list_comments_for_target(
    state: tauri::State<'_, AppState>,
    target_kind: String,
    target_id: String,
) -> Result<Vec<CommentThread>, IpcError> {
    let target = CommentTarget {
        kind: target_kind,
        id: target_id,
    };
    Ok(state.comment_store.list_for_target(&target).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_comments_for_stream(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<Vec<CommentThread>, IpcError> {
    Ok(state.comment_store.list_for_stream(&stream_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn set_comment_intent(
    state: tauri::State<'_, AppState>,
    comment_id: CommentId,
    intent: CommentIntent,
) -> Result<(), IpcError> {
    state.comment_store.set_intent(comment_id, intent).await?;
    if let Some(thread) = state.comment_store.get(comment_id).await? {
        emit_changed(&state, &thread.comment);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_comment_status(
    state: tauri::State<'_, AppState>,
    comment_id: CommentId,
    status: CommentStatus,
) -> Result<(), IpcError> {
    state.comment_store.set_status(comment_id, status).await?;
    if let Some(thread) = state.comment_store.get(comment_id).await? {
        emit_changed(&state, &thread.comment);
    }
    Ok(())
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
    Ok(state
        .comment_store
        .set_anchor(comment_id, &selectors_json, orphaned)
        .await?)
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
    state
        .comment_store
        .relink(comment_id, &quote, &selectors_json)
        .await?;
    if let Some(thread) = state.comment_store.get(comment_id).await? {
        emit_changed(&state, &thread.comment);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_comment(
    state: tauri::State<'_, AppState>,
    comment_id: CommentId,
) -> Result<(), IpcError> {
    // Fetch before deleting so we can emit with the right target.
    let target = state.comment_store.get(comment_id).await?;
    state.comment_store.delete(comment_id).await?;
    if let Some(thread) = target {
        emit_changed(&state, &thread.comment);
    }
    Ok(())
}
