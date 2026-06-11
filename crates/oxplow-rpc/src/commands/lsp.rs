//! Cores for the `lsp` command module.

use oxplow_app::lsp_installer::InstalledManifestEntry;
use oxplow_app::lsp_sessions::LspServerListing;
use oxplow_app::Services;
use oxplow_app::{BackgroundTaskKind, OxplowEvent, StartInput};
use serde::Serialize;
use serde_json::Value;
use specta::Type;

use crate::error::IpcError;

/// Resolve the working directory for a stream's language servers: the
/// stream's worktree, falling back to the project dir when the stream
/// isn't found.
async fn stream_cwd(svc: &Services, stream_id: &str) -> std::path::PathBuf {
    svc.streams
        .list_streams()
        .await
        .ok()
        .and_then(|streams| {
            streams
                .into_iter()
                .find(|s| s.id.to_string() == stream_id)
                .map(|s| std::path::PathBuf::from(&s.worktree_path))
        })
        .unwrap_or_else(|| svc.layout.project_dir.clone())
}

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
            svc.events.emit(OxplowEvent::LspServersChanged);
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

/// Issue a JSON-RPC request on the shared `(stream, language)` session
/// (spawned + initialized lazily) and return the raw LSP result.
pub async fn lsp_request(
    svc: &Services,
    stream_id: String,
    language_id: String,
    method: String,
    params: Value,
) -> Result<Value, IpcError> {
    let cwd = stream_cwd(svc, &stream_id).await;
    let out = svc
        .lsp_sessions
        .request_session(&stream_id, &language_id, cwd, &method, params)
        .await?;
    Ok(out)
}

/// Send a JSON-RPC notification on the shared `(stream, language)`
/// session. Document-sync notifications also update the backend's
/// document mirror (crash/restart replay).
pub async fn lsp_notify(
    svc: &Services,
    stream_id: String,
    language_id: String,
    method: String,
    params: Value,
) -> Result<(), IpcError> {
    let cwd = stream_cwd(svc, &stream_id).await;
    svc.lsp_sessions
        .notify_session(&stream_id, &language_id, cwd, &method, params)
        .await?;
    Ok(())
}

/// All known language servers (oxplow.yaml + Mason-installed), with
/// binary presence and live-session metadata for the settings UI.
pub async fn list_lsp_servers(svc: &Services) -> Result<Vec<LspServerListing>, IpcError> {
    Ok(svc.lsp_sessions.list_servers().await)
}

/// Tear down and respawn the `(stream, language)` session, replaying
/// every mirrored open document.
pub async fn restart_lsp_server(
    svc: &Services,
    stream_id: String,
    language_id: String,
) -> Result<(), IpcError> {
    let cwd = stream_cwd(svc, &stream_id).await;
    svc.lsp_sessions
        .restart(&stream_id, &language_id, cwd)
        .await?;
    Ok(())
}

/// Uninstall a Mason package: delete its files, manifest entry, and
/// language-server registrations.
pub async fn remove_lsp_package(svc: &Services, package_name: String) -> Result<(), IpcError> {
    svc.lsp_installer.remove(&package_name).await?;
    svc.events.emit(OxplowEvent::LspServersChanged);
    Ok(())
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
    async fn list_lsp_servers_dispatches_with_no_args() {
        let (svc, _dir) = services();
        let out = crate::dispatch("list_lsp_servers", json!(null), &svc)
            .await
            .unwrap();
        assert!(out.is_array(), "expected a JSON array, got {out}");
    }

    #[tokio::test]
    async fn lsp_request_surfaces_self_describing_no_config() {
        let (svc, _dir) = services();
        let err = crate::dispatch(
            "lsp_request",
            json!({
                "streamId": "s-1",
                "languageId": "rust",
                "method": "textDocument/hover",
                "params": {},
            }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
        assert!(
            err.message.contains("rust-analyzer"),
            "got: {}",
            err.message
        );
        assert!(
            err.message.contains("lsp_install_server"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn remove_lsp_package_is_idempotent() {
        let (svc, _dir) = services();
        crate::dispatch(
            "remove_lsp_package",
            json!({ "packageName": "not-installed" }),
            &svc,
        )
        .await
        .unwrap();
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
