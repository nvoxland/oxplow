//! Cores for the `app` command module.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::Services;

use crate::error::IpcError;

/// Liveness check the UI uses to verify the daemon is reachable.
pub async fn ping(_svc: &Services) -> Result<&'static str, IpcError> {
    Ok("pong")
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppVersion {
    pub version: &'static str,
}

pub async fn app_version(_svc: &Services) -> Result<AppVersion, IpcError> {
    Ok(AppVersion {
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UiLogEntry {
    #[serde(rename = "clientId")]
    pub client_id: Option<String>,
    pub level: String,
    pub message: String,
    /// JSON-encoded structured context (the renderer stringifies its
    /// own object so the boundary is plain `Option<String>`).
    pub context: Option<String>,
    pub timestamp: Option<String>,
}

/// Forward a UI-side log line into the daemon's tracing pipeline.
/// The renderer's logger.ts installs `console.log/warn/error`
/// proxies that call this; without it those logs never leave the
/// renderer's devtools.
pub async fn log_ui(_svc: &Services, entry: UiLogEntry) -> Result<(), IpcError> {
    let context = entry.context.clone().unwrap_or_default();
    let level = entry.level.to_lowercase();
    let client = entry.client_id.as_deref().unwrap_or("?");
    match level.as_str() {
        "error" => tracing::error!(target: "ui", client, %context, "{}", entry.message),
        "warn" => tracing::warn!(target: "ui", client, %context, "{}", entry.message),
        "debug" => tracing::debug!(target: "ui", client, %context, "{}", entry.message),
        _ => tracing::info!(target: "ui", client, %context, "{}", entry.message),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::test_support::services;

    #[tokio::test]
    async fn app_version_dispatches_with_no_args() {
        let (svc, _dir) = services();
        let out = crate::dispatch("app_version", json!(null), &svc)
            .await
            .unwrap();
        assert_eq!(
            out.get("version").and_then(|v| v.as_str()),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    #[tokio::test]
    async fn log_ui_accepts_entry_and_returns_null() {
        let (svc, _dir) = services();
        let out = crate::dispatch(
            "log_ui",
            json!({ "entry": {
                "clientId": "c1",
                "level": "info",
                "message": "hello",
                "context": null,
                "timestamp": null,
            }}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_null());
    }
}
