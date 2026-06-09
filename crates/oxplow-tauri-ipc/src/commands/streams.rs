use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::OxplowEvent;
use oxplow_domain::{Stream, StreamId};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_streams(state: tauri::State<'_, AppState>) -> Result<Vec<Stream>, IpcError> {
    Ok(state.streams.list_streams().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn ensure_primary(state: tauri::State<'_, AppState>) -> Result<Stream, IpcError> {
    Ok(state.streams.ensure_primary().await?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CreateWorktreeRequest {
    pub slug: String,
    pub title: String,
    pub branch: String,
    #[serde(rename = "branchSource")]
    pub branch_source: String,
}

#[tauri::command]
#[specta::specta]
pub async fn create_worktree(
    state: tauri::State<'_, AppState>,
    req: CreateWorktreeRequest,
) -> Result<Stream, IpcError> {
    let stream = state
        .streams
        .create_worktree(&req.slug, req.title, req.branch, req.branch_source)
        .await?;
    state
        .git
        .register(&stream.id, std::path::PathBuf::from(&stream.worktree_path))
        .await;
    // Spin up the per-stream snapshot capture service + its fs-watch
    // and git-refs listeners so edits in the new worktree start
    // landing in `file_snapshot` immediately.
    if let Some(svc) = state.snapshot_captures.register(&stream) {
        svc.spawn_watcher();
        svc.spawn_git_refs_listener();
    }
    state.events.emit(OxplowEvent::StreamsChanged);
    Ok(stream)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AdoptWorktreeRequest {
    pub path: String,
    pub title: String,
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
    let stream = state
        .streams
        .adopt_worktree(std::path::PathBuf::from(&req.path), req.title)
        .await?;
    state
        .git
        .register(&stream.id, std::path::PathBuf::from(&stream.worktree_path))
        .await;
    if let Some(svc) = state.snapshot_captures.register(&stream) {
        svc.spawn_watcher();
        svc.spawn_git_refs_listener();
    }
    state.events.emit(OxplowEvent::StreamsChanged);
    Ok(stream)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_stream(
    state: tauri::State<'_, AppState>,
    id: StreamId,
) -> Result<(), IpcError> {
    state.streams.delete_stream(&id).await?;
    // Drop the stream's file rows from the search index.
    let _ = state.search_store.purge_stream(&id.to_string()).await;
    state.git.deregister(&id).await;
    // Drop the per-stream snapshot service from the registry. The
    // spawned watcher task continues until process exit (it holds an
    // internal Arc to keep the service alive); follow-up to add
    // graceful shutdown is filed separately.
    state.snapshot_captures.unregister(&id);
    state.events.emit(OxplowEvent::StreamsChanged);
    Ok(())
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
    use oxplow_domain::stores::{AgentStatusStore, ThreadStore};
    use oxplow_domain::AgentStatusState;
    let thread_store = oxplow_db::SqliteThreadStore::new(state.db.clone());
    let threads = thread_store.list_for_stream(&id).await?;
    let statuses = state.thread_runtime.list_all().await?;
    let busy = threads.iter().any(|t| {
        statuses
            .iter()
            .any(|s| s.thread_id == t.id && s.state == AgentStatusState::Running)
    });
    if busy {
        return Err(IpcError::invalid(
            "cannot archive: an agent is still running in one of this stream's threads",
        ));
    }
    state.streams.archive_stream(&id, delete_worktree).await?;
    // Drop the stream's file rows from the search index.
    let _ = state.search_store.purge_stream(&id.to_string()).await;
    state.git.deregister(&id).await;
    state.snapshot_captures.unregister(&id);
    state.events.emit(OxplowEvent::StreamsChanged);
    Ok(())
}

/// Returns the primary stream — the project root. Useful for any UI
/// path that needs to know "what does the user think of as 'this'
/// project?" without enumerating the full list.
#[tauri::command]
#[specta::specta]
pub async fn get_primary_stream(
    state: tauri::State<'_, AppState>,
) -> Result<Option<Stream>, IpcError> {
    use oxplow_domain::stores::StreamStore;
    let stream_store = oxplow_db::SqliteStreamStore::new(state.db.clone());
    Ok(stream_store.primary().await?)
}

/// Currently-selected stream (None falls back to primary in the UI).
#[tauri::command]
#[specta::specta]
pub async fn get_current_stream(
    state: tauri::State<'_, AppState>,
) -> Result<Option<Stream>, IpcError> {
    Ok(state.streams.current().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn switch_stream(
    state: tauri::State<'_, AppState>,
    id: Option<StreamId>,
) -> Result<(), IpcError> {
    state.streams.set_current(id.as_ref()).await?;
    state
        .events
        .emit(OxplowEvent::CurrentStreamChanged { stream_id: id });
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RenameStreamRequest {
    pub id: StreamId,
    pub title: String,
}

#[tauri::command]
#[specta::specta]
pub async fn rename_stream(
    state: tauri::State<'_, AppState>,
    req: RenameStreamRequest,
) -> Result<Stream, IpcError> {
    let s = state.streams.rename(&req.id, req.title).await?;
    state.events.emit(OxplowEvent::StreamsChanged);
    Ok(s)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SetStreamPromptRequest {
    pub id: StreamId,
    pub prompt: Option<String>,
}

/// Per-stream custom prompt — appended to every agent system prompt
/// when this stream is active. `None` (or empty) clears it.
#[tauri::command]
#[specta::specta]
pub async fn set_stream_prompt(
    state: tauri::State<'_, AppState>,
    req: SetStreamPromptRequest,
) -> Result<Stream, IpcError> {
    use oxplow_domain::stores::StreamStore;
    let store = oxplow_db::SqliteStreamStore::new(state.db.clone());
    let mut s = store.get(&req.id).await?.ok_or_else(IpcError::not_found)?;
    s.custom_prompt = req.prompt.filter(|p| !p.is_empty());
    s.updated_at = oxplow_domain::Timestamp::now();
    store.upsert(&s).await?;
    state.events.emit(OxplowEvent::StreamsChanged);
    Ok(s)
}

/// Switch the worktree's HEAD branch. Updates the stream row and runs
/// `git checkout` inside the worktree.
#[tauri::command]
#[specta::specta]
pub async fn reorder_streams(
    state: tauri::State<'_, AppState>,
    order: Vec<StreamId>,
) -> Result<(), IpcError> {
    // Streams are ordered by created_at ASC after the partial-primary
    // ordering. We rewrite created_at to the supplied order's
    // monotonically-increasing offsets so the natural sort follows.
    use oxplow_domain::stores::StreamStore;
    let store = oxplow_db::SqliteStreamStore::new(state.db.clone());
    let now = oxplow_domain::Timestamp::now();
    for (idx, id) in order.iter().enumerate() {
        if let Some(mut s) = store.get(id).await? {
            // Preserve primary ordering: only worktrees get re-shuffled.
            if s.kind != oxplow_domain::StreamKind::Primary {
                s.created_at = oxplow_domain::Timestamp::from_unix_ms(now.unix_ms() + idx as i64);
                s.updated_at = now;
                store.upsert(&s).await?;
            }
        }
    }
    state.events.emit(OxplowEvent::StreamsChanged);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn checkout_stream_branch(
    state: tauri::State<'_, AppState>,
    id: StreamId,
    branch: String,
) -> Result<Stream, IpcError> {
    use oxplow_domain::stores::StreamStore;
    let store = oxplow_db::SqliteStreamStore::new(state.db.clone());
    let stream = store.get(&id).await?.ok_or_else(IpcError::not_found)?;
    let path = std::path::PathBuf::from(&stream.worktree_path);
    let branch_for_blocking = branch.clone();
    let result = tokio::task::spawn_blocking(move || {
        std::process::Command::new("git")
            .args(["checkout", &branch_for_blocking])
            .current_dir(&path)
            .output()
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(|e| IpcError::internal(e.to_string()))?;
    if !result.status.success() {
        return Err(IpcError::internal(
            String::from_utf8_lossy(&result.stderr).into_owned(),
        ));
    }
    let updated = state.streams.record_branch_checkout(&id, branch).await?;
    state.events.emit(OxplowEvent::StreamsChanged);
    Ok(updated)
}
