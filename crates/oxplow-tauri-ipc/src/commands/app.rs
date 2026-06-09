use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::IpcError;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppVersion {
    pub version: &'static str,
}

#[tauri::command]
#[specta::specta]
pub async fn app_version() -> Result<AppVersion, IpcError> {
    Ok(AppVersion {
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Liveness check the UI uses to verify the daemon is reachable.
///
/// Takes `AppState` purely so the body can delegate to the shared
/// `oxplow_rpc` core (the `State` param is injected by Tauri and is
/// invisible to the generated TS binding — `ping()` stays arg-less).
#[tauri::command]
#[specta::specta]
pub async fn ping(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<&'static str, IpcError> {
    oxplow_rpc::commands::ping(&state).await
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
#[tauri::command]
#[specta::specta]
pub async fn log_ui(entry: UiLogEntry) -> Result<(), IpcError> {
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
