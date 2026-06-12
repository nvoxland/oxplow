//! Cores for the `workspace` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::Services;
use oxplow_git::{WorkspaceEntry, WorkspaceFile, WorkspaceIndexedFile, WorkspaceStatusSummary};
use oxplow_tree_source::TreeVersion;

use crate::error::IpcError;

/// Versioned file read. Dispatches on `version`:
/// - `Disk` → `read_workspace_file` (working tree, possibly dirty).
/// - `Ref { ref }` → `read_file_at_ref` (committed blob).
/// - `Snapshot { id }` → `snapshot_store.blob_hash_for_path` + blob read.
///
/// Returns `Ok(None)` if the path doesn't exist at that version.
/// Callers MUST pass an explicit version — there is no implicit
/// "current working tree" default. This is the chokepoint that makes
/// it impossible to forget which version you're reading, the way the
/// duplication-scan bug did against `readWorkspaceFile`.
pub async fn read_file(
    svc: &Services,
    stream_id: Option<String>,
    relative_path: String,
    version: TreeVersion,
) -> Result<Option<String>, IpcError> {
    match version {
        TreeVersion::Disk => match svc
            .git
            .read_workspace_file(stream_id.as_deref(), relative_path)
            .await
        {
            Ok(file) => Ok(Some(file.content)),
            Err(e) => {
                // The git facade returns NotFound as an error; surface
                // that as Ok(None) so the IPC contract matches the
                // ref-reader's None semantics.
                if e.to_string().to_lowercase().contains("not found") {
                    Ok(None)
                } else {
                    Err(IpcError::internal(e.to_string()))
                }
            }
        },
        TreeVersion::Ref { r#ref } => Ok(svc.git.read_file_at_ref(r#ref, relative_path).await),
        TreeVersion::Snapshot { id } => {
            let snapshot_id: i64 = id
                .parse()
                .map_err(|_| IpcError::invalid(format!("invalid snapshot id: {id}")))?;
            let Some(hash) = svc
                .snapshot_store
                .blob_hash_for_path(snapshot_id, &relative_path)
                .await
                .map_err(|e| IpcError::internal(e.to_string()))?
            else {
                return Ok(None);
            };
            let blobs = svc.blobs.clone();
            let bytes = tokio::task::spawn_blocking(move || blobs.read(&hash))
                .await
                .map_err(|e| IpcError::internal(e.to_string()))?;
            match bytes {
                Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
                Err(_) => Ok(None),
            }
        }
    }
}

pub async fn list_workspace_entries(
    svc: &Services,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<Vec<WorkspaceEntry>, IpcError> {
    svc.git
        .list_workspace_entries(stream_id.as_deref(), relative_path)
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
            .map(|c| oxplow_fs_watch::WorkspaceFilter::with_user_entries(&c.generated))
            .unwrap_or_default()
    };
    svc.git
        .list_workspace_files(stream_id.as_deref(), filter)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn read_workspace_file(
    svc: &Services,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<WorkspaceFile, IpcError> {
    svc.git
        .read_workspace_file(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn write_workspace_file(
    svc: &Services,
    stream_id: Option<String>,
    relative_path: String,
    content: String,
) -> Result<WorkspaceFile, IpcError> {
    svc.git
        .write_workspace_file(stream_id.as_deref(), relative_path, content)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn create_workspace_file(
    svc: &Services,
    stream_id: Option<String>,
    relative_path: String,
    content: String,
) -> Result<WorkspaceFile, IpcError> {
    svc.git
        .create_workspace_file(stream_id.as_deref(), relative_path, content)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn create_workspace_directory(
    svc: &Services,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<String, IpcError> {
    svc.git
        .create_workspace_directory(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn rename_workspace_path(
    svc: &Services,
    stream_id: Option<String>,
    from_path: String,
    to_path: String,
) -> Result<(String, String), IpcError> {
    svc.git
        .rename_workspace_path(stream_id.as_deref(), from_path, to_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn delete_workspace_path(
    svc: &Services,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<String, IpcError> {
    svc.git
        .delete_workspace_path(stream_id.as_deref(), relative_path)
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
