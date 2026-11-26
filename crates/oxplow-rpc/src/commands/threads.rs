//! Cores for the `threads` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::config_service::read_config;
use oxplow_app::Services;
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{StreamId, Thread, ThreadId};

use crate::error::IpcError;
use crate::RpcRequest;

pub async fn list_threads(svc: &Services, stream_id: StreamId) -> Result<Vec<Thread>, IpcError> {
    Ok(svc.thread_store.list_for_stream(&stream_id).await?)
}

/// The registered agent harnesses, in declaration order, with which the
/// project enables and which run in a terminal or as a Codex observer.
pub async fn list_agent_harnesses(
    svc: &Services,
) -> Result<Vec<oxplow_app::harnesses::HarnessListing>, IpcError> {
    let config = read_config(&svc.config);
    Ok(oxplow_app::harnesses::listing(
        &svc.harnesses,
        &svc.capabilities,
        &config.agent_enabled,
    ))
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
    pub threads: Vec<Thread>,
}

/// What threads are open on this stream, and which the caller's device is
/// on (the first open thread for a device that chose none, or a request
/// with no device).
pub async fn get_thread_state(
    req: &RpcRequest<'_>,
    stream_id: StreamId,
) -> Result<ThreadState, IpcError> {
    let svc: &Services = req;
    // The stream's open threads: a closed one is history, listed on the
    // Closed Threads page, not in the nav.
    let threads: Vec<Thread> = svc
        .thread_store
        .list_for_stream(&stream_id)
        .await?
        .into_iter()
        .filter(Thread::is_open)
        .collect();
    // A selection that was closed falls back to the first open thread.
    let selected = svc
        .threads
        .selected(req.caller.map(|c| &c.device), &stream_id)
        .await?
        .filter(|id| threads.iter().any(|t| t.id == *id))
        .or_else(|| threads.first().map(|t| t.id));
    Ok(ThreadState {
        selected_thread_id: selected,
        threads,
    })
}

/// The caller's device shows `req.thread_id` in `req.stream_id`. A request
/// with no device may not choose.
pub async fn select_thread(
    request: &RpcRequest<'_>,
    req: SelectThreadRequest,
) -> Result<(), IpcError> {
    let device = &request.caller()?.device;
    request
        .threads
        .select(device, &req.stream_id, req.thread_id.as_ref())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_threads_dispatches() {
        let (svc, _dir) = crate::test_support::services().await;
        let out = crate::dispatch(
            "list_threads",
            serde_json::json!({"streamId": "str999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }

    /// A closed thread leaves the stream's thread state (the nav lists open
    /// threads; closed ones are on the Closed Threads page), and a stream
    /// whose selected thread was closed falls back to its first open one.
    #[tokio::test]
    async fn a_closed_thread_leaves_the_thread_state() {
        let (svc, _dir) = crate::test_support::services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let run = |name: &'static str, input: serde_json::Value| {
            let svc = svc.clone();
            async move {
                svc.commands
                    .run(&oxplow_domain::Actor::Human, name, input, false)
                    .await
                    .unwrap()
                    .result
            }
        };
        let stream_ref = stream.id.to_string();
        run(
            "oxplow.thread.create",
            serde_json::json!({ "stream": stream_ref, "title": "First" }),
        )
        .await;
        let other = run(
            "oxplow.thread.create",
            serde_json::json!({ "stream": stream_ref, "title": "Other" }),
        )
        .await;
        let other_id = other["id"].as_str().unwrap().to_string();
        let caller = crate::test_support::caller();
        crate::dispatch_as(
            "select_thread",
            serde_json::json!({ "req": { "streamId": stream.id.to_string(), "threadId": other_id } }),
            &svc,
            Some(&caller),
        )
        .await
        .unwrap();
        run(
            "oxplow.thread.close",
            serde_json::json!({ "thread": other_id.to_string() }),
        )
        .await;

        let state = crate::dispatch_as(
            "get_thread_state",
            serde_json::json!({ "streamId": stream.id.to_string() }),
            &svc,
            Some(&caller),
        )
        .await
        .unwrap();
        let ids: Vec<&str> = state["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap())
            .collect();
        assert!(!ids.contains(&other_id.as_str()), "{ids:?}");
        assert_eq!(state["selectedThreadId"], ids[0]);
        assert!(state.get("activeThreadId").is_none());
    }

    /// Each device shows its own thread in a stream and its own stream; a
    /// reader with no device gets the defaults (the first open thread, no
    /// stream chosen), and may not choose.
    #[tokio::test]
    async fn each_device_shows_its_own_stream_and_thread() {
        let (svc, _dir) = crate::test_support::services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let device = |n: u8| oxplow_domain::Caller {
            device: oxplow_domain::DeviceId::generate(),
            connection: oxplow_domain::ConnectionId::parse(&format!(
                "con-00000000-0000-4000-8000-00000000000{n}"
            ))
            .unwrap(),
        };
        let (mac, tablet) = (device(1), device(2));
        let made = svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.thread.create",
                serde_json::json!({ "stream": stream.id.to_string(), "title": "Other" }),
                false,
            )
            .await
            .unwrap();
        let other = made.result["id"].as_str().unwrap().to_string();
        let ask =
            |name: &'static str, args: serde_json::Value, caller: Option<oxplow_domain::Caller>| {
                let svc = svc.clone();
                async move { crate::dispatch_as(name, args, &svc, caller.as_ref()).await }
            };
        let select =
            serde_json::json!({ "req": { "streamId": stream.id.to_string(), "threadId": other } });
        ask("select_thread", select.clone(), Some(mac.clone()))
            .await
            .unwrap();
        let state = |caller| {
            ask(
                "get_thread_state",
                serde_json::json!({ "streamId": stream.id.to_string() }),
                caller,
            )
        };
        let first = state(None).await.unwrap()["threads"][0]["id"].clone();
        assert_ne!(first, serde_json::json!(other));
        assert_eq!(
            state(Some(mac.clone())).await.unwrap()["selectedThreadId"],
            serde_json::json!(other)
        );
        assert_eq!(
            state(Some(tablet.clone())).await.unwrap()["selectedThreadId"],
            first
        );
        assert_eq!(state(None).await.unwrap()["selectedThreadId"], first);
        let refused = ask("select_thread", select, None).await.unwrap_err();
        assert_eq!(refused.code, "INVALID", "{refused:?}");

        let switch = serde_json::json!({ "id": stream.id.to_string() });
        ask("switch_stream", switch.clone(), Some(mac.clone()))
            .await
            .unwrap();
        assert_eq!(
            ask("get_current_stream", serde_json::json!({}), Some(mac))
                .await
                .unwrap()["id"],
            serde_json::json!(stream.id.to_string())
        );
        assert_eq!(
            ask("get_current_stream", serde_json::json!({}), Some(tablet))
                .await
                .unwrap(),
            serde_json::Value::Null
        );
        assert_eq!(
            ask("switch_stream", switch, None).await.unwrap_err().code,
            "INVALID"
        );
    }

    /// A device's stream that was archived since (on another device, say) is
    /// no longer its stream: it shows the primary again.
    #[tokio::test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test seeds the stream it archives"
    )]
    async fn an_archived_stream_is_no_devices_current_stream() {
        use oxplow_domain::stores::StreamStore as _;
        let (svc, _dir) = crate::test_support::services().await;
        let primary = svc.streams.ensure_primary().await.unwrap();
        let mut other = primary.clone();
        other.id = oxplow_domain::StreamId::new(primary.id.value() + 1);
        other.kind = oxplow_domain::StreamKind::Worktree;
        other.title = "Other".into();
        svc.stream_store.upsert(&other).await.unwrap();
        let caller = crate::test_support::caller();
        crate::dispatch_as(
            "switch_stream",
            serde_json::json!({ "id": other.id.to_string() }),
            &svc,
            Some(&caller),
        )
        .await
        .unwrap();
        svc.stream_store.archive(&other.id).await.unwrap();
        let current = crate::dispatch_as(
            "get_current_stream",
            serde_json::json!({}),
            &svc,
            Some(&caller),
        )
        .await
        .unwrap();
        assert_eq!(current, serde_json::Value::Null, "{current}");
    }

    #[tokio::test]
    async fn get_thread_state_dispatches() {
        let (svc, _dir) = crate::test_support::services().await;
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
