//! Cores for the `workspace` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::workspace_files::WorkspaceFile;
use oxplow_app::Services;

use crate::error::IpcError;

pub async fn read_workspace_file(
    svc: &Services,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<WorkspaceFile, IpcError> {
    svc.workspace_files
        .read(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}
