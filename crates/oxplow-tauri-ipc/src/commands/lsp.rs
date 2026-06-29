pub use oxplow_rpc::commands::lsp::InstalledLspPackage;

use crate::error::IpcError;
use crate::state::AppState;

/// Download + install a Mason package by name, register the resulting
/// binary with `LspSessionManager`, and persist it to the manifest so
/// subsequent boots pick it up. Blocks for the duration of the
/// download — the renderer should surface a progress affordance.
#[tauri::command]
#[specta::specta]
pub async fn install_lsp_package(
    state: tauri::State<'_, AppState>,
    package_name: String,
) -> Result<InstalledLspPackage, IpcError> {
    oxplow_rpc::commands::lsp::install_lsp_package(&state, package_name).await
}

/// List all Mason packages currently installed for this project.
#[tauri::command]
#[specta::specta]
pub async fn list_installed_lsp_packages(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<InstalledLspPackage>, IpcError> {
    oxplow_rpc::commands::lsp::list_installed_lsp_packages(&state).await
}

/// Issue a JSON-RPC request on the shared `(stream, language)` session
/// (spawned + initialized lazily) and return the raw LSP result.
#[tauri::command]
#[specta::specta]
pub async fn lsp_request(
    state: tauri::State<'_, AppState>,
    stream_id: String,
    language_id: String,
    method: String,
    params_json: String,
) -> Result<String, IpcError> {
    oxplow_rpc::commands::lsp::lsp_request(&state, stream_id, language_id, method, params_json)
        .await
}

/// Send a JSON-RPC notification on the shared `(stream, language)`
/// session. Document-sync notifications also update the backend's
/// document mirror (crash/restart replay).
#[tauri::command]
#[specta::specta]
pub async fn lsp_notify(
    state: tauri::State<'_, AppState>,
    stream_id: String,
    language_id: String,
    method: String,
    params_json: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::lsp::lsp_notify(&state, stream_id, language_id, method, params_json).await
}

/// All known language servers (.oxplow/project.yaml + Mason-installed), with
/// binary presence and live-session metadata for the settings UI.
#[tauri::command]
#[specta::specta]
pub async fn list_lsp_servers(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<oxplow_app::lsp_sessions::LspServerListing>, IpcError> {
    oxplow_rpc::commands::lsp::list_lsp_servers(&state).await
}

/// Tear down and respawn the `(stream, language)` session, replaying
/// every mirrored open document.
#[tauri::command]
#[specta::specta]
pub async fn restart_lsp_server(
    state: tauri::State<'_, AppState>,
    stream_id: String,
    language_id: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::lsp::restart_lsp_server(&state, stream_id, language_id).await
}

/// Answer a server-initiated `workspace/applyEdit` forwarded to the
/// renderer as an `ApplyEditRequest` event.
#[tauri::command]
#[specta::specta]
pub async fn respond_lsp_apply_edit(
    state: tauri::State<'_, AppState>,
    token: u32,
    applied: bool,
    failure_reason: Option<String>,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::lsp::respond_lsp_apply_edit(&state, token, applied, failure_reason).await
}

/// Uninstall a Mason package: delete its files, manifest entry, and
/// language-server registrations.
#[tauri::command]
#[specta::specta]
pub async fn remove_lsp_package(
    state: tauri::State<'_, AppState>,
    package_name: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::lsp::remove_lsp_package(&state, package_name).await
}
