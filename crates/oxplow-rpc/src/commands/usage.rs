//! Cores for the `usage` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::Services;
use oxplow_db::UsageEvent;

use crate::error::IpcError;

pub async fn record_usage(
    svc: &Services,
    kind: String,
    payload_json: String,
) -> Result<UsageEvent, IpcError> {
    let payload: serde_json::Value =
        serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null);
    // Views re-read `v_usage_event`.
    #[expect(clippy::disallowed_methods, reason = "off the bus: usage recording")]
    let event = svc.usage_store.record(&kind, payload).await?;
    Ok(event)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn record_usage_dispatches() {
        let (svc, _dir) = crate::test_support::services().await;
        let out = crate::dispatch(
            "record_usage",
            serde_json::json!({"kind": "file-open", "payloadJson": "{\"path\": \"src/main.rs\"}"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_object());
    }
}
