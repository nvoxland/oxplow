pub use oxplow_rpc::commands::app::{AppVersion, UiLogEntry};

use crate::error::IpcError;

#[tauri::command]
#[specta::specta]
pub async fn app_version(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<AppVersion, IpcError> {
    oxplow_rpc::commands::app::app_version(&state).await
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
    oxplow_rpc::commands::app::ping(&state).await
}

/// Forward a UI-side log line into the daemon's tracing pipeline.
/// The renderer's logger.ts installs `console.log/warn/error`
/// proxies that call this; without it those logs never leave the
/// renderer's devtools.
#[tauri::command]
#[specta::specta]
pub async fn log_ui(
    state: tauri::State<'_, crate::state::AppState>,
    entry: UiLogEntry,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::app::log_ui(&state, entry).await
}
