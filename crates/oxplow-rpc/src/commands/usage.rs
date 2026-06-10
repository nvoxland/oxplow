//! Cores for the `usage` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::{OxplowEvent, Services};
use oxplow_db::{UsageEvent, UsageRollup};

use crate::error::IpcError;

pub async fn record_usage(
    svc: &Services,
    kind: String,
    payload_json: String,
) -> Result<UsageEvent, IpcError> {
    let payload: serde_json::Value =
        serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null);
    let key = extract_key(&payload);
    let stream_id = extract_stream_id(&payload);
    let thread_id = extract_thread_id(&payload);
    let event = svc.usage_store.record(&kind, payload).await?;
    svc.events.emit(OxplowEvent::UsageRecorded {
        usage_kind: kind,
        key,
        stream_id,
        thread_id,
    });
    Ok(event)
}

/// Best-effort extraction of a stable identifier from the usage
/// payload. Renderer subscribers use this to dedupe / route events;
/// callers without an obvious key fall through to an empty string,
/// which is fine because the renderer's filter is keyed on `kind`.
fn extract_key(payload: &serde_json::Value) -> String {
    let obj = match payload.as_object() {
        Some(o) => o,
        None => return String::new(),
    };
    for field in [
        "key", "slug", "path", "id", "itemId", "item_id", "noteId", "note_id",
    ] {
        if let Some(v) = obj.get(field).and_then(|v| v.as_str()) {
            return v.to_string();
        }
    }
    String::new()
}

fn extract_stream_id(payload: &serde_json::Value) -> Option<oxplow_domain::StreamId> {
    payload
        .as_object()?
        .get("streamId")
        .or_else(|| payload.as_object()?.get("stream_id"))
        .and_then(|v| v.as_str())
        .and_then(oxplow_domain::StreamId::try_from_str)
}

fn extract_thread_id(payload: &serde_json::Value) -> Option<oxplow_domain::ThreadId> {
    payload
        .as_object()?
        .get("threadId")
        .or_else(|| payload.as_object()?.get("thread_id"))
        .and_then(|v| v.as_str())
        .and_then(oxplow_domain::ThreadId::try_from_str)
}

pub async fn list_recent_usage(svc: &Services, limit: u32) -> Result<Vec<UsageEvent>, IpcError> {
    Ok(svc.usage_store.list_recent(limit as usize).await?)
}

/// Per-key rollup of recent usage events of a single `kind`. Returns
/// the most-recently-touched keys (file paths, note slugs, task
/// ids, …) along with how many times each has been touched. Drives
/// "recent files" / "recent notes" affordances in the renderer.
pub async fn list_recent_usage_rollup(
    svc: &Services,
    kind: String,
    stream_id: Option<String>,
    limit: u32,
) -> Result<Vec<UsageRollup>, IpcError> {
    Ok(svc
        .usage_store
        .list_recent_rollup(&kind, stream_id.as_deref(), limit as usize)
        .await?)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn record_usage_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "record_usage",
            serde_json::json!({"kind": "file-open", "payloadJson": "{\"path\": \"src/main.rs\"}"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_object());
    }

    #[tokio::test]
    async fn list_recent_usage_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("list_recent_usage", serde_json::json!({"limit": 10}), &svc)
            .await
            .unwrap();
        assert!(out.is_array());
    }
}
