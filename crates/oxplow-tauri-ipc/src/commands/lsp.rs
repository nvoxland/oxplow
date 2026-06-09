use oxplow_app::lsp_installer::InstalledManifestEntry;
use oxplow_app::{BackgroundTaskKind, StartInput};
use serde::Serialize;
use specta::Type;

use crate::error::IpcError;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Type)]
pub struct InstalledLspPackage {
    pub name: String,
    pub version: String,
    pub language_ids: Vec<String>,
    pub binary: String,
}

impl From<InstalledManifestEntry> for InstalledLspPackage {
    fn from(value: InstalledManifestEntry) -> Self {
        Self {
            name: value.name,
            version: value.version,
            language_ids: value.language_ids,
            binary: value.binary.to_string_lossy().to_string(),
        }
    }
}

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
    let cwd = state
        .streams
        .list_streams()
        .await
        .ok()
        .and_then(|streams| {
            streams
                .into_iter()
                .find(|s| s.id.to_string() == stream_id)
                .map(|s| std::path::PathBuf::from(&s.worktree_path))
        })
        .unwrap_or_else(|| state.layout.project_dir.clone());
    let id = state.lsp_clients.open(&language_id, cwd).await?;
    Ok(id)
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
    state.lsp_clients.send(&client_id, payload).await?;
    Ok(())
}

/// Tear down the language server backing `client_id`. Idempotent on
/// already-closed clients (returns `INVALID` rather than panicking).
#[tauri::command]
#[specta::specta]
pub async fn close_lsp_client(
    state: tauri::State<'_, AppState>,
    client_id: String,
) -> Result<(), IpcError> {
    state.lsp_clients.close(&client_id).await?;
    Ok(())
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
    let task = state.background_tasks.start(StartInput {
        kind: BackgroundTaskKind::Lsp,
        label: format!("Install language server: {package_name}"),
        detail: Some("downloading from mason-registry".into()),
        progress: None,
    });
    match state.lsp_installer.install(&package_name).await {
        Ok(entry) => {
            state.background_tasks.complete(&task.id, None);
            Ok(entry.into())
        }
        Err(e) => {
            let msg = e.to_string();
            state.background_tasks.fail(&task.id, msg.clone(), None);
            Err(e.into())
        }
    }
}

/// List all Mason packages currently installed for this project.
#[tauri::command]
#[specta::specta]
pub async fn list_installed_lsp_packages(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<InstalledLspPackage>, IpcError> {
    let entries = state.lsp_installer.list_installed().await?;
    Ok(entries.into_iter().map(Into::into).collect())
}
