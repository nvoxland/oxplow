use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::config_service::read_config;
use oxplow_app::OxplowEvent;
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{AgentKind, StreamId, Thread, ThreadId};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_threads(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<Vec<Thread>, IpcError> {
    Ok(state.thread_store.list_for_stream(&stream_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_thread(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<Option<Thread>, IpcError> {
    Ok(state.thread_store.get(&thread_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn upsert_thread(
    state: tauri::State<'_, AppState>,
    thread: Thread,
) -> Result<(), IpcError> {
    let stream_id = thread.stream_id.clone();
    state.thread_store.upsert(&thread).await?;
    state.events.emit(OxplowEvent::ThreadsChanged { stream_id });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_thread(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<(), IpcError> {
    // Capture stream_id before delete so the event can target it.
    let stream_id = state
        .thread_store
        .get(&thread_id)
        .await?
        .map(|t| t.stream_id);
    state.thread_store.delete(&thread_id).await?;
    if let Some(sid) = stream_id {
        state
            .events
            .emit(OxplowEvent::ThreadsChanged { stream_id: sid });
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CreateThreadRequest {
    #[serde(rename = "streamId")]
    pub stream_id: StreamId,
    pub title: String,
    #[serde(rename = "paneTarget")]
    pub pane_target: Option<String>,
    pub agent: Option<AgentKind>,
}

#[tauri::command]
#[specta::specta]
pub async fn create_thread(
    state: tauri::State<'_, AppState>,
    req: CreateThreadRequest,
) -> Result<Thread, IpcError> {
    let pane = req.pane_target.unwrap_or_else(|| "working".into());
    let config = read_config(&state.config);
    let agent = req
        .agent
        .unwrap_or_else(|| config.agents.first().copied().unwrap_or(AgentKind::Claude));
    if !config.agents.contains(&agent) {
        return Err(IpcError::invalid(format!(
            "agent {agent:?} is not enabled for this project"
        )));
    }
    let t = state
        .threads
        .create(&req.stream_id, req.title, pane, agent)
        .await?;
    state.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: req.stream_id,
    });
    Ok(t)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RenameThreadRequest {
    pub id: ThreadId,
    pub title: String,
}

#[tauri::command]
#[specta::specta]
pub async fn rename_thread(
    state: tauri::State<'_, AppState>,
    req: RenameThreadRequest,
) -> Result<Thread, IpcError> {
    let t = state.threads.rename(&req.id, req.title).await?;
    state.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: t.stream_id.clone(),
    });
    Ok(t)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SetThreadPromptRequest {
    pub id: ThreadId,
    pub prompt: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn set_thread_prompt(
    state: tauri::State<'_, AppState>,
    req: SetThreadPromptRequest,
) -> Result<Thread, IpcError> {
    let t = state.threads.set_prompt(&req.id, req.prompt).await?;
    state.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: t.stream_id.clone(),
    });
    Ok(t)
}

#[tauri::command]
#[specta::specta]
pub async fn promote_thread(
    state: tauri::State<'_, AppState>,
    id: ThreadId,
) -> Result<Thread, IpcError> {
    let t = state.threads.promote(&id).await?;
    state.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: t.stream_id.clone(),
    });
    Ok(t)
}

#[tauri::command]
#[specta::specta]
pub async fn close_thread(
    state: tauri::State<'_, AppState>,
    id: ThreadId,
) -> Result<Thread, IpcError> {
    let t = state.threads.close(&id).await?;
    state.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: t.stream_id.clone(),
    });
    Ok(t)
}

#[tauri::command]
#[specta::specta]
pub async fn reopen_thread(
    state: tauri::State<'_, AppState>,
    id: ThreadId,
) -> Result<Thread, IpcError> {
    let t = state.threads.reopen(&id).await?;
    state.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: t.stream_id.clone(),
    });
    Ok(t)
}

#[tauri::command]
#[specta::specta]
pub async fn list_closed_threads(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<Vec<Thread>, IpcError> {
    Ok(state.threads.list_closed(&stream_id).await?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReorderThreadQueueRequest {
    #[serde(rename = "streamId")]
    pub stream_id: StreamId,
    pub order: Vec<ThreadId>,
}

#[tauri::command]
#[specta::specta]
pub async fn reorder_thread_queue(
    state: tauri::State<'_, AppState>,
    req: ReorderThreadQueueRequest,
) -> Result<(), IpcError> {
    state
        .threads
        .reorder_queue(&req.stream_id, &req.order)
        .await?;
    state.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: req.stream_id,
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_selected_thread(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<Option<ThreadId>, IpcError> {
    Ok(state.threads.selected(&stream_id).await?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SelectThreadRequest {
    #[serde(rename = "streamId")]
    pub stream_id: StreamId,
    #[serde(rename = "threadId")]
    pub thread_id: Option<ThreadId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ThreadState {
    #[serde(rename = "selectedThreadId")]
    pub selected_thread_id: Option<ThreadId>,
    #[serde(rename = "activeThreadId")]
    pub active_thread_id: Option<ThreadId>,
    pub threads: Vec<Thread>,
}

/// Aggregate "what threads exist on this stream and what's selected/active".
#[tauri::command]
#[specta::specta]
pub async fn get_thread_state(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<ThreadState, IpcError> {
    let threads = state.thread_store.list_for_stream(&stream_id).await?;
    let active = threads
        .iter()
        .find(|t| t.status == oxplow_domain::ThreadStatus::Active)
        .map(|t| t.id.clone());
    let selected = state.threads.selected(&stream_id).await?;
    Ok(ThreadState {
        selected_thread_id: selected.or_else(|| active.clone()),
        active_thread_id: active,
        threads,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ThreadWorkState {
    #[serde(rename = "threadId")]
    pub thread_id: ThreadId,
    pub waiting: Vec<oxplow_domain::Task>,
    #[serde(rename = "inProgress")]
    pub in_progress: Vec<oxplow_domain::Task>,
    pub done: Vec<oxplow_domain::Task>,
    pub epics: Vec<oxplow_domain::Task>,
    pub items: Vec<oxplow_domain::Task>,
    pub followups: Vec<oxplow_app::Followup>,
}

/// Bucketed task view for the Work panel.
#[tauri::command]
#[specta::specta]
pub async fn get_thread_work_state(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<ThreadWorkState, IpcError> {
    use oxplow_domain::stores::TaskStore;
    use oxplow_domain::TaskStatus;
    let rows = state.task_store.list_for_thread(&thread_id).await?;
    // An "epic" is any task that has at least one child within this scope.
    let child_parents: std::collections::HashSet<oxplow_domain::TaskId> =
        rows.iter().filter_map(|r| r.parent_id).collect();
    let mut waiting = vec![];
    let mut in_progress = vec![];
    let mut done = vec![];
    let mut epics = vec![];
    let mut items = vec![];
    for r in rows {
        if child_parents.contains(&r.id) {
            epics.push(r);
            continue;
        }
        match r.status {
            TaskStatus::Blocked => waiting.push(r),
            TaskStatus::InProgress => in_progress.push(r),
            TaskStatus::Done | TaskStatus::Canceled | TaskStatus::Archived => done.push(r),
            TaskStatus::Ready => items.push(r),
        }
    }
    let followups = state.followups.list_for_thread(&thread_id);
    Ok(ThreadWorkState {
        thread_id,
        waiting,
        in_progress,
        done,
        epics,
        items,
        followups,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn select_thread(
    state: tauri::State<'_, AppState>,
    req: SelectThreadRequest,
) -> Result<(), IpcError> {
    state
        .threads
        .select(&req.stream_id, req.thread_id.as_ref())
        .await?;
    state.events.emit(OxplowEvent::SelectedThreadChanged {
        stream_id: req.stream_id,
        thread_id: req.thread_id,
    });
    Ok(())
}
