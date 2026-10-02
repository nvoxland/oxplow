//! Cores for the `threads` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::config_service::read_config;
use oxplow_app::Services;
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{StreamId, Thread, ThreadId};

use crate::error::IpcError;

pub async fn list_threads(svc: &Services, stream_id: StreamId) -> Result<Vec<Thread>, IpcError> {
    Ok(svc.thread_store.list_for_stream(&stream_id).await?)
}

/// The ACP agents this project can run, with approval and install state.
pub async fn list_acp_agents(
    svc: &Services,
) -> Result<Vec<oxplow_app::acp::agents::AcpAgentListing>, IpcError> {
    let config = read_config(&svc.config);
    Ok(oxplow_app::acp::agents::list(
        &svc.approvals,
        &svc.layout.project_dir,
        &config,
    ))
}

pub async fn list_closed_threads(
    svc: &Services,
    stream_id: StreamId,
) -> Result<Vec<Thread>, IpcError> {
    Ok(svc.threads.list_closed(&stream_id).await?)
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

pub async fn select_thread(svc: &Services, req: SelectThreadRequest) -> Result<(), IpcError> {
    svc.threads
        .select(&req.stream_id, req.thread_id.as_ref())
        .await?;
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
