//! Cores for the `usage` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::Services;
use oxplow_db::{UsageEvent, UsageRollup};

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
}
