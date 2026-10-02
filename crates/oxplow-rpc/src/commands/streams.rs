//! Cores for the `streams` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::Services;
use oxplow_domain::{Stream, StreamId};

use crate::error::IpcError;

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

/// Currently-selected stream (None falls back to primary in the UI).
pub async fn get_current_stream(svc: &Services) -> Result<Option<Stream>, IpcError> {
    Ok(svc.streams.current().await?)
}

pub async fn switch_stream(svc: &Services, id: Option<StreamId>) -> Result<(), IpcError> {
    svc.streams.set_current(id.as_ref()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn switch_stream_accepts_optional_id() {
        let (svc, _dir) = crate::test_support::services();
        // Missing `id` key deserializes as None and clears the selection.
        let out = crate::dispatch("switch_stream", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert!(out.is_null());
    }
}
