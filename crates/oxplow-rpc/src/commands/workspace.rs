//! Cores for the `workspace` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::workspace_files::WorkspaceFile;
use oxplow_app::Services;

use crate::error::IpcError;

/// The project's `generated:` lists; the stream's workspace adds its own
/// `.gitignore`s (`WorkspaceFiles::filter_for`) — the same exclusions as
/// fs-watch and snapshots. It keeps node_modules/dist junk out of
/// quick-open and text search, and bounds the walk.
fn generated(svc: &Services) -> oxplow_config::GeneratedConfig {
    svc.config
        .read()
        .map(|c| c.generated.clone())
        .unwrap_or_default()
}

/// Lines containing `query` (a fixed string) across the stream's files,
/// at most `limit` (default 200, at most 1000).
pub async fn search_workspace_text(
    svc: &Services,
    stream_id: Option<String>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<oxplow_app::workspace_files::TextSearchHit>, IpcError> {
    svc.workspace_files
        .search_text(
            stream_id.as_deref(),
            &generated(svc),
            query,
            limit.map_or(200, |n| n as usize).clamp(1, 1000),
        )
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

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
