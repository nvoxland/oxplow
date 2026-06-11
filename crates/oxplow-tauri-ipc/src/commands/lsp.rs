pub use oxplow_rpc::commands::lsp::InstalledLspPackage;

use crate::error::IpcError;
use crate::state::AppState;

/// Spawn a new language-server child for `(stream_id, language_id)`.
/// Returns an opaque `client_id` the renderer uses to address
/// subsequent send/close commands. The cwd is resolved from the
/// stream's worktree path; if the stream isn't found we fall back to
/// the project dir.
#[tauri::command]
#[specta::specta]
pub async fn open_lsp_client(
    state: tauri::State<'_, AppState>,
    stream_id: String,
    language_id: String,
) -> Result<String, IpcError> {
    oxplow_rpc::commands::lsp::open_lsp_client(&state, stream_id, language_id).await
}

/// Forward a raw JSON-RPC frame body (no headers) from the renderer
/// to the language server addressed by `client_id`.
#[tauri::command]
#[specta::specta]
pub async fn send_lsp_message(
    state: tauri::State<'_, AppState>,
    client_id: String,
    payload: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::lsp::send_lsp_message(&state, client_id, payload).await
}

/// Tear down the language server backing `client_id`. Idempotent on
/// already-closed clients (returns `INVALID` rather than panicking).
#[tauri::command]
#[specta::specta]
pub async fn close_lsp_client(
    state: tauri::State<'_, AppState>,
    client_id: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::lsp::close_lsp_client(&state, client_id).await
}

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
    params: serde_json::Value,
) -> Result<serde_json::Value, IpcError> {
    oxplow_rpc::commands::lsp::lsp_request(&state, stream_id, language_id, method, params).await
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
    params: serde_json::Value,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::lsp::lsp_notify(&state, stream_id, language_id, method, params).await
}

/// All known language servers (oxplow.yaml + Mason-installed), with
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
