//! Cores for the `lsp` command module.

use oxplow_app::lsp_installer::InstalledManifestEntry;
use oxplow_app::Services;
use oxplow_app::{BackgroundTaskKind, StartInput};
use serde::Serialize;
use specta::Type;

use crate::error::IpcError;

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
pub async fn open_lsp_client(
    svc: &Services,
    stream_id: String,
    language_id: String,
) -> Result<String, IpcError> {
    let cwd = svc
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
        .unwrap_or_else(|| svc.layout.project_dir.clone());
    let id = svc.lsp_clients.open(&language_id, cwd).await?;
    Ok(id)
}

/// Forward a raw JSON-RPC frame body (no headers) from the renderer
/// to the language server addressed by `client_id`.
pub async fn send_lsp_message(
    svc: &Services,
    client_id: String,
    payload: String,
) -> Result<(), IpcError> {
    svc.lsp_clients.send(&client_id, payload).await?;
    Ok(())
}

/// Tear down the language server backing `client_id`. Idempotent on
/// already-closed clients (returns `INVALID` rather than panicking).
pub async fn close_lsp_client(svc: &Services, client_id: String) -> Result<(), IpcError> {
    svc.lsp_clients.close(&client_id).await?;
    Ok(())
}

/// Download + install a Mason package by name, register the resulting
/// binary with `LspSessionManager`, and persist it to the manifest so
/// subsequent boots pick it up. Blocks for the duration of the
/// download — the renderer should surface a progress affordance.
pub async fn install_lsp_package(
    svc: &Services,
    package_name: String,
) -> Result<InstalledLspPackage, IpcError> {
    let task = svc.background_tasks.start(StartInput {
        kind: BackgroundTaskKind::Lsp,
        label: format!("Install language server: {package_name}"),
        detail: Some("downloading from mason-registry".into()),
        progress: None,
    });
    match svc.lsp_installer.install(&package_name).await {
        Ok(entry) => {
            svc.background_tasks.complete(&task.id, None);
            Ok(entry.into())
        }
        Err(e) => {
            let msg = e.to_string();
            svc.background_tasks.fail(&task.id, msg.clone(), None);
            Err(e.into())
        }
    }
}

/// List all Mason packages currently installed for this project.
pub async fn list_installed_lsp_packages(
    svc: &Services,
) -> Result<Vec<InstalledLspPackage>, IpcError> {
    let entries = svc.lsp_installer.list_installed().await?;
    Ok(entries.into_iter().map(Into::into).collect())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::test_support::services;

    #[tokio::test]
    async fn list_installed_lsp_packages_dispatches_with_no_args() {
        let (svc, _dir) = services();
        let out = crate::dispatch("list_installed_lsp_packages", json!(null), &svc)
            .await
            .unwrap();
        assert!(out.is_array(), "expected a JSON array, got {out}");
    }

    #[tokio::test]
    async fn send_lsp_message_rejects_unknown_client() {
        let (svc, _dir) = services();
        let err = crate::dispatch(
            "send_lsp_message",
            json!({ "clientId": "nope", "payload": "{}" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert!(!err.code.is_empty());
    }
}
