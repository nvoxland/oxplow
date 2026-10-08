//! Reading and restoring captured files (P2.9, tsk433): one place for what
//! the RPC and MCP snapshot tools do, with honest ids —
//! a **`file_snapshot` id** names one captured file row, a **`snapshot`
//! id** a whole capture (read a path *at* it).
//!
//! A file is restored into **its stream's worktree** (the row's
//! `stream_id` → `streams.worktree_path`), not the primary checkout, so a
//! restore from a worktree stream lands where that stream works.

use std::path::PathBuf;
use std::sync::Arc;

use oxplow_db::{FileSnapshot, SnapshotStorage, SqliteSnapshotStore, SqliteStreamStore};
use oxplow_domain::stores::StreamStore as _;
use oxplow_domain::StreamId;

use crate::snapshot_content::{SnapshotContent, SnapshotReadError};

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
    /// The stream it was captured in is archived or gone: restoring it
    /// would write into some other checkout.
    #[error(
        "the stream `{0}` this file was captured in is archived or gone; restoring it would \
         write into another checkout"
    )]
    StreamGone(StreamId),
    #[error("{0}")]
    Other(String),
}

/// What a restore wrote: the stream, the file's path in its worktree, and
/// the file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub stream: StreamId,
    pub path: String,
    pub file: PathBuf,
}

/// What reading and restoring a captured file needs: the snapshot rows,
/// the streams (where a row's stream works), the bytes, and the project
/// checkout (a stream without a worktree works there).
#[derive(Clone)]
pub struct SnapshotFiles {
    pub snapshots: Arc<SqliteSnapshotStore>,
    pub streams: Arc<SqliteStreamStore>,
    pub content: SnapshotContent,
    pub project_dir: PathBuf,
}

impl SnapshotFiles {
    /// The bytes a `file_snapshot` row captured.
    pub async fn read_file_snapshot(
        &self,
        file_snapshot_id: i64,
    ) -> Result<Vec<u8>, SnapshotFileError> {
        let row = self.row(file_snapshot_id).await?;
        self.read_row(&row).await
    }

    /// The bytes at `path` as of snapshot `snapshot_id` (the latest capture
    /// of the path at or before it); `None` when the path didn't exist then.
    pub async fn read_file_at_snapshot(
        &self,
        snapshot_id: i64,
        path: &str,
    ) -> Result<Option<Vec<u8>>, SnapshotFileError> {
        let Some(content) = self
            .snapshots
            .content_ref_for_path(snapshot_id, path)
            .await
            .map_err(|e| SnapshotFileError::Other(e.to_string()))?
        else {
            return Ok(None);
        };
        self.read_bytes(content.storage, content.hash)
            .await
            .map(Some)
    }

    /// Write a `file_snapshot` row's bytes back to its path in its stream's
    /// worktree. Returns the file written.
    pub async fn restore_file_snapshot(
        &self,
        file_snapshot_id: i64,
    ) -> Result<Restored, SnapshotFileError> {
        let row = self.row(file_snapshot_id).await?;
        let target = self.worktree_of(row.stream_id).await?.join(&row.path);
        let bytes = self.read_row(&row).await?;
        let file = tokio::task::spawn_blocking(move || {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, &bytes).map(|()| target)
        })
        .await
        .map_err(|e| SnapshotFileError::Other(e.to_string()))?
        .map_err(|e| SnapshotFileError::Other(e.to_string()))?;
        Ok(Restored {
            stream: row.stream_id,
            path: row.path,
            file,
        })
    }

    async fn row(&self, file_snapshot_id: i64) -> Result<FileSnapshot, SnapshotFileError> {
        self.snapshots
            .get(file_snapshot_id)
            .await
            .map_err(|e| SnapshotFileError::Other(e.to_string()))?
            .ok_or(SnapshotFileError::NotFound)
    }

    /// Where `stream` works: its worktree, or the project checkout for a
    /// stream without one (the primary). An archived or unknown stream has
    /// nowhere — never another stream's checkout.
    async fn worktree_of(&self, stream: StreamId) -> Result<PathBuf, SnapshotFileError> {
        match self.streams.get(&stream).await {
            Ok(Some(s)) if s.archived_at.is_some() => Err(SnapshotFileError::StreamGone(stream)),
            Ok(Some(s)) if s.worktree_path.is_empty() => Ok(self.project_dir.clone()),
            Ok(Some(s)) => Ok(PathBuf::from(s.worktree_path)),
            Ok(None) => Err(SnapshotFileError::StreamGone(stream)),
            Err(e) => Err(SnapshotFileError::Other(e.to_string())),
        }
    }

    async fn read_row(&self, row: &FileSnapshot) -> Result<Vec<u8>, SnapshotFileError> {
        let hash = row.blob_hash.clone().ok_or(SnapshotFileError::NoContent)?;
        self.read_bytes(row.storage, hash).await
    }

    /// Blob-store or VCS-object bytes, off the async runtime (it's file I/O).
    async fn read_bytes(
        &self,
        storage: SnapshotStorage,
        hash: String,
    ) -> Result<Vec<u8>, SnapshotFileError> {
        let content = self.content.clone();
        tokio::task::spawn_blocking(move || content.read(storage, &hash))
            .await
            .map_err(|e| SnapshotFileError::Other(e.to_string()))?
            .map_err(|e| match e {
                SnapshotReadError::Blob(_) => SnapshotFileError::Expired,
                SnapshotReadError::NoContent => SnapshotFileError::NoContent,
                other => SnapshotFileError::Other(other.to_string()),
            })
    }
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
                host: oxplow_domain::HostId::LOCAL,
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
            std::sync::Arc::new(crate::vcs::GitProvider),
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
        let files = f.svc.snapshot_files();
        let restored = files.restore_file_snapshot(row.id).await.unwrap();
        assert_eq!(restored.file, file);
        assert_eq!(restored.stream, stream);
        assert_eq!(restored.path, "notes.txt");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");
        assert!(!f.svc.layout.project_dir.join("notes.txt").exists());

        // Its stream archived, the file has nowhere of its own to go: the
        // restore is refused rather than written into the primary checkout.
        f.svc.stream_store.archive(&stream).await.unwrap();
        assert!(matches!(
            files.restore_file_snapshot(row.id).await,
            Err(SnapshotFileError::StreamGone(_))
        ));
        assert!(!f.svc.layout.project_dir.join("notes.txt").exists());

        // And the reads agree.
        assert_eq!(files.read_file_snapshot(row.id).await.unwrap(), b"original");
        assert_eq!(
            files
                .read_file_at_snapshot(snapshot, "notes.txt")
                .await
                .unwrap()
                .as_deref(),
            Some(&b"original"[..])
        );
        assert_eq!(
            files
                .read_file_at_snapshot(snapshot, "absent.txt")
                .await
                .unwrap(),
            None
        );
        assert!(matches!(
            files.read_file_snapshot(999_999).await,
            Err(SnapshotFileError::NotFound)
        ));
    }
}
