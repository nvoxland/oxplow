//! Cores for the `threads` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::config_service::read_config;
use oxplow_app::{OxplowEvent, Services};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{AgentKind, StreamId, Thread, ThreadId};

use crate::error::IpcError;

pub async fn list_threads(svc: &Services, stream_id: StreamId) -> Result<Vec<Thread>, IpcError> {
    Ok(svc.thread_store.list_for_stream(&stream_id).await?)
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

pub async fn create_thread(svc: &Services, req: CreateThreadRequest) -> Result<Thread, IpcError> {
    let pane = req.pane_target.unwrap_or_else(|| "working".into());
    let config = read_config(&svc.config);
    let agent = req
        .agent
        .unwrap_or_else(|| config.agents.first().copied().unwrap_or(AgentKind::Claude));
    if !config.agents.contains(&agent) {
        return Err(IpcError::invalid(format!(
            "agent {agent:?} is not enabled for this project"
        )));
    }
    let t = svc
        .threads
        .create(&req.stream_id, req.title, pane, agent)
        .await?;
    svc.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: req.stream_id,
    });
    Ok(t)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RenameThreadRequest {
    pub id: ThreadId,
    pub title: String,
}

pub async fn rename_thread(svc: &Services, req: RenameThreadRequest) -> Result<Thread, IpcError> {
    let t = svc.threads.rename(&req.id, req.title).await?;
    svc.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: t.stream_id,
    });
    Ok(t)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SetThreadPromptRequest {
    pub id: ThreadId,
    pub prompt: Option<String>,
}

pub async fn set_thread_prompt(
    svc: &Services,
    req: SetThreadPromptRequest,
) -> Result<Thread, IpcError> {
    let t = svc.threads.set_prompt(&req.id, req.prompt).await?;
    svc.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: t.stream_id,
    });
    Ok(t)
}

pub async fn promote_thread(svc: &Services, id: ThreadId) -> Result<Thread, IpcError> {
    let t = svc.threads.promote(&id).await?;
    svc.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: t.stream_id,
    });
    Ok(t)
}

pub async fn close_thread(svc: &Services, id: ThreadId) -> Result<Thread, IpcError> {
    let t = svc.threads.close(&id).await?;
    svc.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: t.stream_id,
    });
    Ok(t)
}

pub async fn reopen_thread(svc: &Services, id: ThreadId) -> Result<Thread, IpcError> {
    let t = svc.threads.reopen(&id).await?;
    svc.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: t.stream_id,
    });
    Ok(t)
}

pub async fn list_closed_threads(
    svc: &Services,
    stream_id: StreamId,
) -> Result<Vec<Thread>, IpcError> {
    Ok(svc.threads.list_closed(&stream_id).await?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReorderThreadQueueRequest {
    #[serde(rename = "streamId")]
    pub stream_id: StreamId,
    pub order: Vec<ThreadId>,
}

pub async fn reorder_thread_queue(
    svc: &Services,
    req: ReorderThreadQueueRequest,
) -> Result<(), IpcError> {
    svc.threads
        .reorder_queue(&req.stream_id, &req.order)
        .await?;
    svc.events.emit(OxplowEvent::ThreadsChanged {
        stream_id: req.stream_id,
    });
    Ok(())
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
pub async fn get_thread_state(
    svc: &Services,
    stream_id: StreamId,
) -> Result<ThreadState, IpcError> {
    let threads = svc.thread_store.list_for_stream(&stream_id).await?;
    let active = threads
        .iter()
        .find(|t| t.status == oxplow_domain::ThreadStatus::Active)
        .map(|t| t.id);
    let selected = svc.threads.selected(&stream_id).await?;
    Ok(ThreadState {
        selected_thread_id: selected.or(active),
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
pub async fn get_thread_work_state(
    svc: &Services,
    thread_id: ThreadId,
) -> Result<ThreadWorkState, IpcError> {
    use oxplow_domain::stores::TaskStore;
    use oxplow_domain::TaskStatus;
    let rows = svc.task_store.list_for_thread(&thread_id).await?;
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
    let followups = svc.followups.list_for_thread(&thread_id);
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

pub async fn select_thread(svc: &Services, req: SelectThreadRequest) -> Result<(), IpcError> {
    svc.threads
        .select(&req.stream_id, req.thread_id.as_ref())
        .await?;
    svc.events.emit(OxplowEvent::SelectedThreadChanged {
        stream_id: req.stream_id,
        thread_id: req.thread_id,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_threads_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_threads",
            serde_json::json!({"streamId": "str999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn get_thread_state_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "get_thread_state",
            serde_json::json!({"streamId": "str999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_object());
        assert!(out.get("threads").is_some());
    }
}
