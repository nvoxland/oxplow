//! Cores for the `workspace` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::workspace_files::{WorkspaceEntry, WorkspaceFile, WorkspaceIndexedFile};
use oxplow_app::Services;
use oxplow_git::WorkspaceStatusSummary;

use crate::error::IpcError;

pub async fn list_workspace_entries(
    svc: &Services,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<Vec<WorkspaceEntry>, IpcError> {
    svc.workspace_files
        .list_entries(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn list_workspace_files(
    svc: &Services,
    stream_id: Option<String>,
) -> Result<Vec<WorkspaceIndexedFile>, IpcError> {
    // Same exclusion rule as fs-watch/snapshots: the `generated:`
    // config list, and nothing else (`.gitignore` is not consulted).
    // Keeps node_modules/dist junk out of quick-open results and bounds
    // the walk (a vendor tree is hundreds of thousands of entries the
    // index has no use for).
    let filter = {
        let cfg = svc.config.read();
        cfg.as_ref()
            .map(|c| {
                oxplow_fs_watch::WorkspaceFilter::for_project(
                    &svc.layout.project_dir,
                    &c.generated.exclude,
                    &c.generated.include,
                )
            })
            .unwrap_or_default()
    };
    svc.workspace_files
        .list_files(stream_id.as_deref(), filter)
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

pub async fn write_workspace_file(
    svc: &Services,
    stream_id: Option<String>,
    relative_path: String,
    content: String,
) -> Result<WorkspaceFile, IpcError> {
    svc.workspace_files
        .write(stream_id.as_deref(), relative_path, content)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn create_workspace_file(
    svc: &Services,
    stream_id: Option<String>,
    relative_path: String,
    content: String,
) -> Result<WorkspaceFile, IpcError> {
    svc.workspace_files
        .create_file(stream_id.as_deref(), relative_path, content)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn create_workspace_directory(
    svc: &Services,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<String, IpcError> {
    svc.workspace_files
        .create_directory(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn rename_workspace_path(
    svc: &Services,
    stream_id: Option<String>,
    from_path: String,
    to_path: String,
) -> Result<(String, String), IpcError> {
    svc.workspace_files
        .rename(stream_id.as_deref(), from_path, to_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn delete_workspace_path(
    svc: &Services,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<String, IpcError> {
    svc.workspace_files
        .delete(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn get_workspace_status_summary(
    svc: &Services,
    stream_id: Option<String>,
) -> Result<WorkspaceStatusSummary, IpcError> {
    Ok(svc.git.status_summary(stream_id.as_deref()).await)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn get_workspace_status_summary_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "get_workspace_status_summary",
            serde_json::json!({ "streamId": null }),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_object(), "expected a JSON object, got {out}");
    }

    #[tokio::test]
    async fn list_workspace_files_excludes_generated_dirs() {
        let (svc, dir) = crate::test_support::services();
        std::fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        std::fs::write(dir.path().join("node_modules/pkg/index.js"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        svc.services
            .config
            .write()
            .unwrap()
            .generated
            .exclude
            .push("node_modules".into());

        let out = crate::dispatch(
            "list_workspace_files",
            serde_json::json!({ "streamId": null }),
            &svc,
        )
        .await
        .unwrap();
        let paths: Vec<&str> = out
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert!(paths.contains(&"src/main.rs"), "got: {paths:?}");
        assert!(
            !paths.iter().any(|p| p.starts_with("node_modules")),
            "generated dirs must be pruned from the index, got: {paths:?}"
        );
    }

    #[tokio::test]
    async fn list_workspace_entries_dispatches_root_listing() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_workspace_entries",
            serde_json::json!({ "streamId": null, "relativePath": "" }),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array(), "expected a JSON array, got {out}");
    }
}
