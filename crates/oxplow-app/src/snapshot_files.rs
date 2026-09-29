//! Reading and restoring captured files (P2.9, tsk433): one place for what
//! the RPC and MCP snapshot tools do, with honest ids —
//! a **`file_snapshot` id** names one captured file row, a **`snapshot`
//! id** a whole capture (read a path *at* it).
//!
//! A file is restored into **its stream's worktree** (the row's
//! `stream_id` → `streams.worktree_path`), not the primary checkout, so a
//! restore from a worktree stream lands where that stream works.

use std::path::PathBuf;

use oxplow_db::{FileSnapshot, SnapshotStorage};
use oxplow_domain::stores::StreamStore as _;
use oxplow_domain::StreamId;

use crate::snapshot_content::{read_snapshot_content, SnapshotReadError};
use crate::Services;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotFileError {
    #[error("no such file snapshot")]
    NotFound,
    /// Oversize or a deletion tombstone: there were never bytes.
    #[error("this file snapshot has no content (it was over the size cap, or records a deletion)")]
    NoContent,
    /// The record is permanent; the bytes age out of Local History.
    #[error(
        "this snapshot's content has expired from Local History — the record is permanent, \
         but file bytes are only kept for the retention window"
    )]
    Expired,
    #[error("{0}")]
    Other(String),
}

/// The bytes a `file_snapshot` row captured.
pub async fn read_file_snapshot(
    svc: &Services,
    file_snapshot_id: i64,
) -> Result<Vec<u8>, SnapshotFileError> {
    let row = svc
        .snapshot_store
        .get(file_snapshot_id)
        .await
        .map_err(|e| SnapshotFileError::Other(e.to_string()))?
        .ok_or(SnapshotFileError::NotFound)?;
    read_row(svc, &row).await
}

/// The bytes at `path` as of snapshot `snapshot_id` (the latest capture
/// of the path at or before it); `None` when the path didn't exist then.
pub async fn read_file_at_snapshot(
    svc: &Services,
    snapshot_id: i64,
    path: &str,
) -> Result<Option<Vec<u8>>, SnapshotFileError> {
    let Some(content) = svc
        .snapshot_store
        .content_ref_for_path(snapshot_id, path)
        .await
        .map_err(|e| SnapshotFileError::Other(e.to_string()))?
    else {
        return Ok(None);
    };
    read_bytes(svc, content.storage, content.hash)
        .await
        .map(Some)
}

/// Write a `file_snapshot` row's bytes back to its path in its stream's
/// worktree. Returns the file written.
pub async fn restore_file_snapshot(
    svc: &Services,
    file_snapshot_id: i64,
) -> Result<PathBuf, SnapshotFileError> {
    let row = svc
        .snapshot_store
        .get(file_snapshot_id)
        .await
        .map_err(|e| SnapshotFileError::Other(e.to_string()))?
        .ok_or(SnapshotFileError::NotFound)?;
    let bytes = read_row(svc, &row).await?;
    let target = worktree_of(svc, row.stream_id).await.join(&row.path);
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, &bytes).map(|()| target)
    })
    .await
    .map_err(|e| SnapshotFileError::Other(e.to_string()))?
    .map_err(|e| SnapshotFileError::Other(e.to_string()))
}

/// Where `stream` works: its worktree, or the project checkout for a
/// stream without one.
async fn worktree_of(svc: &Services, stream: StreamId) -> PathBuf {
    match svc.stream_store.get(&stream).await {
        Ok(Some(s)) if !s.worktree_path.is_empty() => PathBuf::from(s.worktree_path),
        _ => svc.layout.project_dir.clone(),
    }
}

async fn read_row(svc: &Services, row: &FileSnapshot) -> Result<Vec<u8>, SnapshotFileError> {
    let hash = row.blob_hash.clone().ok_or(SnapshotFileError::NoContent)?;
    read_bytes(svc, row.storage, hash).await
}

/// Blob-store or git-odb bytes, off the async runtime (it's file / git I/O).
async fn read_bytes(
    svc: &Services,
    storage: SnapshotStorage,
    hash: String,
) -> Result<Vec<u8>, SnapshotFileError> {
    let project = svc.layout.project_dir.clone();
    let blobs = svc.blobs.clone();
    tokio::task::spawn_blocking(move || read_snapshot_content(storage, &hash, &project, &blobs))
        .await
        .map_err(|e| SnapshotFileError::Other(e.to_string()))?
        .map_err(|e| match e {
            SnapshotReadError::Blob(_) => SnapshotFileError::Expired,
            SnapshotReadError::NoContent => SnapshotFileError::NoContent,
            other => SnapshotFileError::Other(other.to_string()),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::snapshot::SnapshotTrigger;
    use oxplow_domain::{Stream, StreamKind, Timestamp};
    use std::time::Duration;

    /// A worktree stream's file restores into that worktree — never the
    /// primary checkout.
    #[tokio::test]
    async fn a_worktree_streams_file_restores_into_its_worktree() {
        let f = crate::test_fixtures::services_with_effort().await;
        let worktree = tempfile::tempdir().unwrap();
        let now = Timestamp::from_unix_ms(1);
        let stream = f
            .svc
            .stream_store
            .upsert(&Stream {
                id: StreamId::new(2),
                kind: StreamKind::Worktree,
                title: "wt".into(),
                branch: "wt".into(),
                branch_ref: "refs/heads/wt".into(),
                branch_source: "main".into(),
                worktree_path: worktree.path().to_string_lossy().into(),
                working_pane: String::new(),
                talking_pane: String::new(),
                working_session_id: String::new(),
                talking_session_id: String::new(),
                custom_prompt: None,
                created_at: now,
                updated_at: now,
                archived_at: None,
            })
            .await
            .unwrap();
        let capture = crate::snapshot_capture::SnapshotCaptureService::new(
            f.svc.snapshot_store.clone(),
            f.svc.blobs.clone(),
            worktree.path().to_path_buf(),
            stream,
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(Duration::ZERO)
        .with_predrain_delay(Duration::ZERO);
        let file = worktree.path().join("notes.txt");
        std::fs::write(&file, "original").unwrap();
        capture.mark_dirty(file.clone(), oxplow_fs_watch::WatchEventKind::Other);
        let snapshot = capture
            .request_snapshot(crate::snapshot_capture::TakeRequest {
                trigger: SnapshotTrigger::Manual,
                thread_id: None,
                turn_id: None,
                effort_id: None,
                budget: None,
            })
            .await
            .unwrap()
            .unwrap();
        let row = f
            .svc
            .snapshot_store
            .list_files_for_snapshot(snapshot)
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.path == "notes.txt")
            .unwrap();

        std::fs::write(&file, "clobbered").unwrap();
        let restored = restore_file_snapshot(&f.svc, row.id).await.unwrap();
        assert_eq!(restored, file);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");
        assert!(!f.svc.layout.project_dir.join("notes.txt").exists());

        // And the reads agree.
        assert_eq!(
            read_file_snapshot(&f.svc, row.id).await.unwrap(),
            b"original"
        );
        assert_eq!(
            read_file_at_snapshot(&f.svc, snapshot, "notes.txt")
                .await
                .unwrap()
                .as_deref(),
            Some(&b"original"[..])
        );
        assert_eq!(
            read_file_at_snapshot(&f.svc, snapshot, "absent.txt")
                .await
                .unwrap(),
            None
        );
        assert!(matches!(
            read_file_snapshot(&f.svc, 999_999).await,
            Err(SnapshotFileError::NotFound)
        ));
    }
}
