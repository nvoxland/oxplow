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
