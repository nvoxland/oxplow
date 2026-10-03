//! Site-wide search: the file kind.
//!
//! The unified `search_store` (FTS5/BM25) holds every kind. Tasks,
//! comments, thread notes and wiki pages are search kinds whose rows come
//! from a model (`v_search_<kind>`), indexed as assets by `kind_search`
//! (tsk864). A file's text comes from snapshot blobs, outside the tables,
//! so it is ingestion: the `search.index` pump consumer indexes the files
//! a `snapshot.taken` captured — durable, redelivered after a crash.

use std::sync::Arc;

use oxplow_domain::StreamId;

use crate::Services;

pub const KIND_FILE: &str = "file";

/// The pump consumer's name.
pub const SEARCH_INDEX: &str = "search.index";

/// Register the `search.index` consumer on `svc`'s pump (boot, before it
/// spawns).
pub fn register(svc: &Arc<Services>) {
    svc.event_pump.register_async(Arc::new(SearchIndexConsumer {
        services: Arc::downgrade(svc),
    }));
}

/// Keeps snapshot files in the search index from the event log.
struct SearchIndexConsumer {
    services: std::sync::Weak<Services>,
}

#[async_trait::async_trait]
impl crate::event_pump::AsyncEventConsumer for SearchIndexConsumer {
    fn name(&self) -> &'static str {
        SEARCH_INDEX
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == "snapshot.taken"
    }

    async fn handle(
        &self,
        event: &oxplow_domain::StoredEvent,
    ) -> Result<(), oxplow_domain::DomainError> {
        let Some(svc) = self.services.upgrade() else {
            return Err(oxplow_domain::DomainError::Busy(
                "services are shutting down".into(),
            ));
        };
        let payload = &event.envelope.payload;
        let stream = payload["stream"]
            .as_str()
            .and_then(|r| r.strip_prefix("stream:"));
        let snapshot = payload["snapshot"]
            .as_str()
            .and_then(|r| r.strip_prefix("snapshot:"))
            .and_then(|n| n.parse::<i64>().ok());
        let files = payload["file_count"].as_u64().unwrap_or(0);
        if let (Some(stream), Some(snapshot), true) =
            (stream.and_then(StreamId::try_from_str), snapshot, files > 0)
        {
            Indexer::new(svc)
                .index_snapshot_files(&stream, snapshot)
                .await;
        }
        Ok(())
    }
}

/// Skip indexing file bodies larger than this (the FTS index stores its own
/// copy of the text; bound it so a few huge files can't bloat the DB).
const MAX_INDEX_FILE_BYTES: i64 = 512 * 1024;

/// Skip indexing a file whose longest line exceeds this — a single very
/// long line means minified/bundled output or a source map, which is
/// useless for search and bloats the index. Hand-written source rarely
/// exceeds a few hundred bytes per line.
const MAX_INDEX_LINE_BYTES: usize = 5_000;

/// Length (bytes) of the longest `\n`-delimited line in `bytes`.
fn longest_line_bytes(bytes: &[u8]) -> usize {
    bytes
        .split(|&b| b == b'\n')
        .map(|line| line.len())
        .max()
        .unwrap_or(0)
}

#[derive(Clone)]
pub struct Indexer {
    services: Arc<Services>,
}

impl Indexer {
    pub fn new(services: Arc<Services>) -> Self {
        Self { services }
    }

    /// Index the file contents captured under one snapshot for a stream.
    /// A row with no blob is a deletion → remove its index entry; an oversize
    /// row, a too-large body, or binary content is skipped; everything else is
    /// indexed as UTF-8 (lossy) text keyed by `(file, path, stream)`.
    pub async fn index_snapshot_files(&self, stream_id: &StreamId, snapshot_id: i64) {
        let Ok(files) = self
            .services
            .snapshot_store
            .list_files_for_snapshot(snapshot_id)
            .await
        else {
            return;
        };
        let stream_key = stream_id.to_string();
        for f in files {
            // Deletion capture (no blob) → drop the index row.
            let Some(hash) = f.blob_hash.as_deref() else {
                let _ = self
                    .services
                    .search_store
                    .remove(KIND_FILE, &f.path, Some(&stream_key))
                    .await;
                continue;
            };
            if f.storage.is_oversize() || f.size_bytes > MAX_INDEX_FILE_BYTES {
                continue;
            }
            // Route through the read seam so VCS-backed rows resolve via
            // the object store (their `blob_hash` is an object id).
            let Ok(bytes) = self.services.snapshot_content.read(f.storage, hash) else {
                continue;
            };
            // Skip binary: a NUL byte is the cheap, reliable heuristic.
            if bytes.contains(&0) {
                continue;
            }
            // Skip minified / bundled output + source maps — a single
            // very long line is the tell; useless for search, bloats FTS.
            if longest_line_bytes(&bytes) > MAX_INDEX_LINE_BYTES {
                continue;
            }
            let content = String::from_utf8_lossy(&bytes);
            let _ = self
                .services
                .search_store
                .upsert(KIND_FILE, &f.path, Some(&stream_key), &f.path, &content)
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn services() -> (Arc<Services>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let svc = Arc::new(Services::in_memory(dir.path()).expect("in-memory services"));
        (svc, dir)
    }

    #[test]
    fn longest_line_bytes_flags_minified() {
        assert_eq!(longest_line_bytes(b"abc\nde\nfghij"), 5);
        assert_eq!(longest_line_bytes(b""), 0);
        // A minified/bundled blob (one giant line) trips the threshold...
        let minified = vec![b'x'; MAX_INDEX_LINE_BYTES + 1000];
        assert!(longest_line_bytes(&minified) > MAX_INDEX_LINE_BYTES);
        // ...but ordinary multi-line source does not.
        let normal = b"fn main() { println!(\"hi\"); }\n".repeat(200);
        assert!(longest_line_bytes(&normal) <= MAX_INDEX_LINE_BYTES);
    }

    /// Capture a `file_snapshot` row for `path` with optional content, under a
    /// fresh snapshot id. Returns the snapshot id so the caller can index it.
    async fn capture_file(
        svc: &Services,
        stream: &StreamId,
        path: &str,
        content: Option<&[u8]>,
    ) -> i64 {
        use oxplow_domain::Timestamp;
        let snap_id = svc.snapshot_store.create_snapshot(*stream).await.unwrap();
        let (blob_hash, size) = match content {
            Some(bytes) => (Some(svc.blobs.write(bytes).unwrap()), bytes.len() as i64),
            None => (None, 0),
        };
        svc.snapshot_store
            .capture(oxplow_db::FileSnapshot {
                id: 0,
                stream_id: *stream,
                path: path.into(),
                blob_hash,
                size_bytes: size,
                captured_at: Timestamp::from_unix_ms(0),
                storage: oxplow_db::SnapshotStorage::Oxplow,
                snapshot_id: Some(snap_id),
                mtime_ms: Some(0),
                content_hash: None,
            })
            .await
            .unwrap();
        snap_id
    }

    #[tokio::test]
    async fn indexes_and_removes_file_contents() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let indexer = Indexer::new(svc.clone());

        let snap = capture_file(&svc, &stream.id, "src/frob.rs", Some(b"fn frobnicate() {}")).await;
        indexer.index_snapshot_files(&stream.id, snap).await;
        let hits = svc
            .search_store
            .search("frobnicate", Some(&stream.id.to_string()), &[], 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, KIND_FILE);
        assert_eq!(hits[0].ref_id, "src/frob.rs");

        // A later capture with no blob = deletion → index row removed.
        let snap2 = capture_file(&svc, &stream.id, "src/frob.rs", None).await;
        indexer.index_snapshot_files(&stream.id, snap2).await;
        assert!(svc
            .search_store
            .search("frobnicate", Some(&stream.id.to_string()), &[], 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn binary_content_is_skipped() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let indexer = Indexer::new(svc.clone());
        // NUL byte → treated as binary, not indexed.
        let snap = capture_file(&svc, &stream.id, "a.bin", Some(b"frobnicate\0\xffbinary")).await;
        indexer.index_snapshot_files(&stream.id, snap).await;
        assert!(svc
            .search_store
            .search("frobnicate", Some(&stream.id.to_string()), &[], 10)
            .await
            .unwrap()
            .is_empty());
    }
}
