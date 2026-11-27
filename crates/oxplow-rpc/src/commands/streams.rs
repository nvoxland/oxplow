//! Cores for the `streams` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::Services;
use oxplow_domain::{Stream, StreamId};

use crate::error::IpcError;
use crate::RpcRequest;

pub async fn list_streams(svc: &Services) -> Result<Vec<Stream>, IpcError> {
    Ok(svc.streams.list_streams().await?)
}

/// Returns the primary stream — the project root. Useful for any UI
/// path that needs to know "what does the user think of as 'this'
/// project?" without enumerating the full list.
pub async fn get_primary_stream(svc: &Services) -> Result<Option<Stream>, IpcError> {
    use oxplow_domain::stores::StreamStore;
    let stream_store = oxplow_db::SqliteStreamStore::new(svc.db.clone());
    Ok(stream_store.primary().await?)
}

/// The stream the caller's device shows (None, and for a request with no
/// device: the UI falls back to the primary stream).
pub async fn get_current_stream(req: &RpcRequest<'_>) -> Result<Option<Stream>, IpcError> {
    let device = req.caller.map(|c| &c.device);
    Ok(req.streams.current(device).await?)
}

/// The caller's device shows stream `id` (`None` clears its choice). A
/// request with no device may not choose.
pub async fn switch_stream(req: &RpcRequest<'_>, id: Option<StreamId>) -> Result<(), IpcError> {
    let device = &req.caller()?.device;
    req.streams.set_current(device, id.as_ref()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn switch_stream_accepts_optional_id() {
        let (svc, _dir) = crate::test_support::services().await;
        // Missing `id` key deserializes as None and clears the selection.
        let caller = crate::test_support::caller();
        let out = crate::dispatch_as("switch_stream", serde_json::json!({}), &svc, Some(&caller))
            .await
            .unwrap();
        assert!(out.is_null());
    }
}
