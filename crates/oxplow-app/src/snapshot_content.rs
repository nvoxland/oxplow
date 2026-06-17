//! The single seam for reading a captured file's bytes back, routing
//! on its [`SnapshotStorage`] class.
//!
//! Snapshot rows store content in one of two places — oxplow's blob
//! store (`storage = oxplow`, `blob_hash` = xxh3-128) or the git object
//! db (`storage = git`, `blob_hash` = git blob OID). Every consumer that
//! wants the bytes (workspace file view, snapshot restore, search
//! indexer, MCP/diff readers) goes through [`read_snapshot_content`] so
//! none of them can forget the git fallback. `oversize` / `deleted` rows
//! have no readable bytes and return [`SnapshotReadError::NoContent`].

use std::path::Path;

use oxplow_db::{SnapshotContentRef, SnapshotStorage};

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
    /// A git-backed row whose OID no longer resolves in the object db —
    /// e.g. history was rewritten and the blob was GC'd. The bytes are
    /// genuinely gone (we deliberately never copied them).
    #[error("git object {0} unavailable (orphaned by a history rewrite?)")]
    GitUnavailable(String),
}

/// Read the bytes for `(storage, blob_hash)`, fetching from the blob
/// store or the git odb under `project_dir`. Blocking (does file / git
/// I/O) — call from `spawn_blocking` on the async path.
pub fn read_snapshot_content(
    storage: SnapshotStorage,
    blob_hash: &str,
    project_dir: &Path,
    blobs: &BlobStore,
) -> Result<Vec<u8>, SnapshotReadError> {
    match storage {
        SnapshotStorage::Oxplow => blobs
            .read(blob_hash)
            .map_err(|e| SnapshotReadError::Blob(e.to_string())),
        SnapshotStorage::Git => oxplow_git::read_blob(project_dir, blob_hash)
            .ok_or_else(|| SnapshotReadError::GitUnavailable(blob_hash.to_string())),
        SnapshotStorage::Oversize | SnapshotStorage::Deleted => Err(SnapshotReadError::NoContent),
    }
}

/// Convenience over [`read_snapshot_content`] for a
/// [`SnapshotContentRef`] (as returned by
/// `SnapshotStore::content_ref_for_path`).
pub fn read_content_ref(
    content_ref: &SnapshotContentRef,
    project_dir: &Path,
    blobs: &BlobStore,
) -> Result<Vec<u8>, SnapshotReadError> {
    read_snapshot_content(content_ref.storage, &content_ref.hash, project_dir, blobs)
}

/// Read content addressed by a `tree_at` **identity** string. That map
/// collapses storage class away — an entry is either an oxplow xxh3
/// (oxplow rows), a git blob OID (git rows), or an `"oversize:…"`
/// sentinel. We can't recover the class, but the two real address spaces
/// don't collide (xxh3 is 32 hex chars, a git OID 40), so try the blob
/// store first, then the git odb. `None` for oversize sentinels or a
/// genuine miss.
pub fn read_tree_identity(
    identity: &str,
    project_dir: &Path,
    blobs: &BlobStore,
) -> Option<Vec<u8>> {
    if identity.starts_with("oversize:") {
        return None;
    }
    if let Ok(bytes) = blobs.read(identity) {
        return Some(bytes);
    }
    oxplow_git::read_blob(project_dir, identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn oversize_and_deleted_have_no_content() {
        let dir = tempdir().unwrap();
        let blobs = BlobStore::new(dir.path().join("objects"));
        for storage in [SnapshotStorage::Oversize, SnapshotStorage::Deleted] {
            let err = read_snapshot_content(storage, "whatever", dir.path(), &blobs).unwrap_err();
            assert!(matches!(err, SnapshotReadError::NoContent));
        }
    }

    #[test]
    fn oxplow_reads_from_blob_store() {
        let dir = tempdir().unwrap();
        let blobs = BlobStore::new(dir.path().join("objects"));
        let hash = blobs.write(b"hello bytes").unwrap();
        let got =
            read_snapshot_content(SnapshotStorage::Oxplow, &hash, dir.path(), &blobs).unwrap();
        assert_eq!(got, b"hello bytes");
    }

    #[test]
    fn git_reads_committed_blob() {
        let dir = tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "t").unwrap();
        cfg.set_str("user.email", "t@e.com").unwrap();
        std::fs::write(dir.path().join("f.txt"), "git body").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("f.txt")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = repo.signature().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, &[])
            .unwrap();

        let oid = oxplow_git::clean_head_blob_oids(dir.path())
            .remove("f.txt")
            .unwrap();
        let blobs = BlobStore::new(dir.path().join(".oxplow/objects"));
        let got = read_snapshot_content(SnapshotStorage::Git, &oid, dir.path(), &blobs).unwrap();
        assert_eq!(got, b"git body");

        // A bogus OID surfaces GitUnavailable, not a panic.
        let err = read_snapshot_content(
            SnapshotStorage::Git,
            "0123456789abcdef0123456789abcdef01234567",
            dir.path(),
            &blobs,
        )
        .unwrap_err();
        assert!(matches!(err, SnapshotReadError::GitUnavailable(_)));
    }
}
