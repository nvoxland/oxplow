//! The single seam for reading a captured file's bytes back, routing
//! on its [`SnapshotStorage`] class.
//!
//! Snapshot rows store content in one of two places — oxplow's blob
//! store (`storage = oxplow`, `blob_hash` = xxh3-128) or the VCS's object
//! store (`storage = git`, `blob_hash` = an object id; the persisted class
//! name predates the VCS capability, `.context/vcs.md`). Every consumer
//! that wants the bytes (workspace file view, snapshot restore, search
//! indexer, metrics, drift, diff readers) goes through
//! [`SnapshotContent`] so none of them can forget the VCS fallback.
//! `oversize` / `deleted` rows have no readable bytes and return
//! [`SnapshotReadError::NoContent`].

use std::sync::Arc;

use oxplow_db::{SnapshotContentRef, SnapshotStorage};
use oxplow_domain::vcs::{ObjectId, ObjectStore};

use crate::blob_store::BlobStore;

/// Why a snapshot's bytes couldn't be produced.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotReadError {
    /// The row class carries no bytes (oversize metadata-only or a
    /// deletion tombstone).
    #[error("snapshot row has no readable content (oversize or deleted)")]
    NoContent,
    /// An oxplow-blob-store row whose blob is missing/unreadable.
    #[error("blob store read failed: {0}")]
    Blob(String),
    /// A VCS-backed row whose object no longer resolves — e.g. history
    /// was rewritten and the object collected. The bytes are genuinely
    /// gone (we deliberately never copied them).
    #[error("object {0} unavailable (orphaned by a history rewrite?)")]
    ObjectUnavailable(String),
}

/// Reads captured bytes from either store. Every workspace of the
/// repository shares one object store, so one reader serves every
/// stream. Cheap to clone. Blocking — call from `spawn_blocking` on the
/// async path.
#[derive(Clone)]
pub struct SnapshotContent {
    blobs: BlobStore,
    objects: Arc<dyn ObjectStore>,
}

impl SnapshotContent {
    pub fn new(blobs: BlobStore, objects: Arc<dyn ObjectStore>) -> Self {
        Self { blobs, objects }
    }

    /// The bytes for `(storage, blob_hash)`.
    pub fn read(
        &self,
        storage: SnapshotStorage,
        blob_hash: &str,
    ) -> Result<Vec<u8>, SnapshotReadError> {
        match storage {
            SnapshotStorage::Oxplow => self
                .blobs
                .read(blob_hash)
                .map_err(|e| SnapshotReadError::Blob(e.to_string())),
            SnapshotStorage::Git => self
                .objects
                .read(&ObjectId(blob_hash.to_string()))
                .ok_or_else(|| SnapshotReadError::ObjectUnavailable(blob_hash.to_string())),
            SnapshotStorage::Oversize | SnapshotStorage::Deleted => {
                Err(SnapshotReadError::NoContent)
            }
        }
    }

    /// [`Self::read`] for a [`SnapshotContentRef`] (as returned by
    /// `SnapshotStore::content_ref_for_path`).
    pub fn read_ref(&self, content_ref: &SnapshotContentRef) -> Result<Vec<u8>, SnapshotReadError> {
        self.read(content_ref.storage, &content_ref.hash)
    }

    /// The content hash (xxh3, the blob store's key) of a VCS object — how
    /// VCS-backed rows join the one content-identity space
    /// (`SqliteSnapshotStore::with_content_hasher`).
    pub fn object_content_hash(&self, id: &str) -> Option<String> {
        self.objects
            .read(&ObjectId(id.to_string()))
            .map(|bytes| BlobStore::hash(&bytes))
    }

    pub fn objects(&self) -> &Arc<dyn ObjectStore> {
        &self.objects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::vcs::Vcs as _;
    use tempfile::tempdir;

    fn reader(dir: &std::path::Path) -> SnapshotContent {
        SnapshotContent::new(
            BlobStore::new(dir.join(".oxplow/objects")),
            crate::vcs::GitProvider.object_store(dir),
        )
    }

    #[test]
    fn oversize_and_deleted_have_no_content() {
        let dir = tempdir().unwrap();
        for storage in [SnapshotStorage::Oversize, SnapshotStorage::Deleted] {
            let err = reader(dir.path()).read(storage, "whatever").unwrap_err();
            assert!(matches!(err, SnapshotReadError::NoContent));
        }
    }

    #[test]
    fn oxplow_reads_from_blob_store() {
        let dir = tempdir().unwrap();
        let content = reader(dir.path());
        let hash = content.blobs.write(b"hello bytes").unwrap();
        let got = content.read(SnapshotStorage::Oxplow, &hash).unwrap();
        assert_eq!(got, b"hello bytes");
    }

    #[test]
    fn a_vcs_row_reads_its_committed_object() {
        let dir = tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        std::fs::write(dir.path().join("f.txt"), "git body").unwrap();
        crate::test_fixtures::commit_all(dir.path(), "c");
        let content = reader(dir.path());
        let oid = content.objects().id_of(b"git body").0;
        let got = content.read(SnapshotStorage::Git, &oid).unwrap();
        assert_eq!(got, b"git body");
        assert_eq!(
            content.object_content_hash(&oid),
            Some(BlobStore::hash(b"git body"))
        );

        // A bogus id surfaces ObjectUnavailable, not a panic.
        let err = content
            .read(
                SnapshotStorage::Git,
                "0123456789abcdef0123456789abcdef01234567",
            )
            .unwrap_err();
        assert!(matches!(err, SnapshotReadError::ObjectUnavailable(_)));
    }
}
