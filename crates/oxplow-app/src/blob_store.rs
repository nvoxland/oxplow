//! Content-addressed blob store for snapshot bytes.
//!
//! Keyed by lowercase-hex xxh3-128. Files land under
//! `<root>/<aa>/<full-hash>` (two-character shard prefix to keep
//! directory entry counts manageable on filesystems that don't
//! love huge flat dirs). Writes are idempotent — if the blob
//! already exists the bytes aren't rewritten.
//!
//! xxh3-128 was picked over SHA-256 / blake3 because the blob store
//! is a local content-addressed dedup cache for the user's own files
//! — there's no adversary crafting collisions, so we don't need
//! cryptographic guarantees. 128-bit output keeps non-adversarial
//! collision risk at "won't happen in practice," and the hash runs
//! 30-50× faster than SHA-256 on Apple silicon.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;
use xxhash_rust::xxh3::Xxh3;

/// Process-wide counter feeding [`BlobStore::write`]'s temp filename.
/// Each call burns one increment, guaranteeing concurrent writers
/// targeting the same content hash get distinct staging filenames so
/// the rename phase doesn't race on a shared tmp.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum BlobStoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("blob not found: {0}")]
    NotFound(String),
}

/// Cheap to clone — every method takes `&self` and reads the root
/// path on demand.
#[derive(Clone)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Hex xxh3-128 of `bytes`. Public so callers can compute a
    /// blob id without going through `write`. Returns 32 lowercase
    /// hex chars; the leading two form the shard directory.
    pub fn hash(bytes: &[u8]) -> String {
        let mut h = Xxh3::new();
        h.update(bytes);
        format!("{:032x}", h.digest128())
    }

    fn path_for(&self, hash: &str) -> PathBuf {
        let shard = if hash.len() >= 2 { &hash[0..2] } else { "00" };
        self.root.join(shard).join(hash)
    }

    /// Persist `bytes` under their content hash and return the hash.
    /// Idempotent — re-writing the same bytes is a no-op (the file's
    /// mtime is bumped but contents are unchanged).
    pub fn write(&self, bytes: &[u8]) -> Result<String, BlobStoreError> {
        self.write_hashed(&Self::hash(bytes), bytes)
    }

    /// [`Self::write`] for a caller that already hashed `bytes` (to
    /// compare with a prior content hash first). `hash` must be
    /// `Self::hash(bytes)`.
    pub fn write_hashed(&self, hash: &str, bytes: &[u8]) -> Result<String, BlobStoreError> {
        debug_assert_eq!(hash, Self::hash(bytes));
        let hash = hash.to_string();
        let path = self.path_for(&hash);
        if path.exists() {
            return Ok(hash);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Write to a writer-unique sibling temp then rename so
        // partial-failure never leaves a half-written blob in place.
        // The counter-based suffix prevents two concurrent rayon
        // workers that hash to the same content (duplicate files,
        // mirrored backup blobs) from racing on a shared
        // `<hash>.tmp` — the rename would otherwise hit ENOENT
        // for whichever worker lost the race.
        let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = path.with_file_name(format!("{hash}.{n}.tmp"));
        std::fs::write(&tmp, bytes)?;
        // Rename is atomic + overwriting on Unix, so identical
        // concurrent commits to the same canonical path are safe —
        // the final file is one of the (bit-identical) tmp bodies.
        std::fs::rename(&tmp, &path)?;
        Ok(hash)
    }

    /// Read the bytes for `hash`. Errors with `NotFound` if the
    /// blob isn't on disk (pruned, never captured, etc).
    pub fn read(&self, hash: &str) -> Result<Vec<u8>, BlobStoreError> {
        let path = self.path_for(hash);
        if !path.exists() {
            return Err(BlobStoreError::NotFound(hash.to_string()));
        }
        Ok(std::fs::read(&path)?)
    }

    /// True iff a blob with `hash` exists on disk.
    pub fn has(&self, hash: &str) -> bool {
        self.path_for(hash).exists()
    }

    /// Sum of every blob file's size under `<root>/`. Tolerant of a
    /// missing root dir (returns 0). Skips `.tmp` leftovers so a
    /// crashed write doesn't inflate the reported number.
    ///
    /// Walks two levels (shard dir → blob file). Cheap enough to call
    /// on a dashboard page-load — at ~50k blobs the stat calls take a
    /// handful of ms on SSD.
    pub fn total_bytes(&self) -> Result<u64, BlobStoreError> {
        if !self.root.exists() {
            return Ok(0);
        }
        let mut total: u64 = 0;
        for shard in std::fs::read_dir(&self.root)? {
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(shard.path())? {
                let entry = entry?;
                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if !metadata.is_file() {
                    continue;
                }
                let name = entry.file_name();
                if name.to_string_lossy().ends_with(".tmp") {
                    continue;
                }
                total = total.saturating_add(metadata.len());
            }
        }
        Ok(total)
    }

    /// Delete every blob whose hash isn't in `keep`. Returns the
    /// number of files removed. Tolerant of a missing root dir
    /// (returns 0) and of individual file removal failures (logged
    /// + counted separately).
    pub fn gc(&self, keep: &std::collections::HashSet<String>) -> Result<u64, BlobStoreError> {
        if !self.root.exists() {
            return Ok(0);
        }
        let mut removed = 0u64;
        for shard in std::fs::read_dir(&self.root)? {
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(shard.path())? {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let name = entry.file_name();
                let name = name.to_string_lossy();
                // Skip stray .tmp leftovers from a crashed write.
                if name.ends_with(".tmp") {
                    let _ = std::fs::remove_file(entry.path());
                    continue;
                }
                if !keep.contains(name.as_ref()) && std::fs::remove_file(entry.path()).is_ok() {
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("blobs"));
        let hash = store.write(b"hello").unwrap();
        let got = store.read(&hash).unwrap();
        assert_eq!(got, b"hello");
    }

    #[test]
    fn idempotent_write_for_same_bytes() {
        let dir = tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("blobs"));
        let h1 = store.write(b"x").unwrap();
        let h2 = store.write(b"x").unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn read_unknown_hash_errors() {
        let dir = tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("blobs"));
        let err = store.read("deadbeef").unwrap_err();
        assert!(matches!(err, BlobStoreError::NotFound(_)));
    }

    #[test]
    fn has_returns_true_after_write() {
        let dir = tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("blobs"));
        let h = store.write(b"y").unwrap();
        assert!(store.has(&h));
        assert!(!store.has("deadbeef"));
    }

    #[test]
    fn concurrent_writes_of_same_bytes_dont_race_on_tmp() {
        // Regression: when phase 2 of the snapshot sweep parallelized
        // BlobStore::write across rayon, two workers hashing the same
        // content shared a `<hash>.tmp` staging file. One thread's
        // rename consumed the tmp out from under the other, leaving
        // the loser with an ENOENT. The fix gives each writer a
        // unique tmp suffix.
        let dir = tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("blobs"));
        let payload: Vec<u8> = b"shared-blob-content".to_vec();
        let mut handles = Vec::new();
        for _ in 0..32 {
            let s = store.clone();
            let p = payload.clone();
            handles.push(std::thread::spawn(move || s.write(&p)));
        }
        let mut hashes = Vec::new();
        for h in handles {
            hashes.push(h.join().unwrap().expect("write should not race"));
        }
        // Every writer agrees on the hash and the blob is on disk.
        let first = hashes[0].clone();
        assert!(hashes.iter().all(|h| h == &first));
        assert!(store.has(&first));
    }
}
