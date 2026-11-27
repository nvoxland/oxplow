//! Site-wide search: the file kind.
//!
//! The unified `search_store` holds every kind. Tasks, knowledge pages,
//! pages, commands and the rest are search kinds whose rows come from a
//! dataset (`v_search_<kind>`), restated as assets by `kind_search`. A
//! file's text comes from snapshot blobs, outside the tables, so it is
//! ingestion: on each `snapshot.taken` (the `search.index` pump consumer —
//! durable, redelivered after a crash) and once per stream at boot, the
//! stream's files are **restated to its snapshot tree** — every file the
//! tree has, named by its file name, titled by its path, with its contents
//! as the body when they can be read. Each entry is versioned by its blob,
//! so only the files that changed are read and written; a file the tree no
//! longer has leaves the index.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxplow_db::search_store::{self, EntryKey, EntryText, Restated};
use oxplow_domain::uri::build::file_uri;
use oxplow_domain::uri::resources::FILE;
use oxplow_domain::StreamId;

use crate::Services;

/// The pump consumer's name.
pub const SEARCH_INDEX: &str = "search.index";

/// Register the `search.index` consumer on `svc`'s pump (boot, before it
/// spawns).
pub fn register(svc: &Arc<Services>) {
    svc.event_pump.register_async(Arc::new(SearchIndexConsumer {
        services: Arc::downgrade(svc),
    }));
}

/// Restate every stream's files to its latest snapshot (boot): the index
/// heals whatever a missed delivery or an older index left.
pub async fn reconcile_all(svc: &Arc<Services>) {
    let Ok(streams) = svc.streams.list_streams().await else {
        return;
    };
    let indexer = Indexer::new(svc.clone());
    for stream in streams.iter().filter(|s| s.archived_at.is_none()) {
        if let Ok(Some(snapshot)) = svc
            .snapshot_store
            .latest_snapshot_id_for_stream(stream.id)
            .await
        {
            indexer.reconcile_stream_files(&stream.id, snapshot).await;
        }
    }
}

/// Keeps each stream's files in the search index from the event log.
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
            .and_then(|s| s.parse::<StreamId>().ok());
        let snapshot = payload["snapshot"].as_i64();
        let files = payload["file_count"].as_u64().unwrap_or(0);
        // A snapshot that changed no file has the tree already indexed.
        if let (Some(stream), Some(snapshot), true) = (stream, snapshot, files > 0) {
            Indexer::new(svc)
                .reconcile_stream_files(&stream, snapshot)
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

/// How many files one write transaction restates: a first index of a
/// large tree doesn't hold the write lock for all of it.
const WRITE_CHUNK: usize = 500;

/// Length (bytes) of the longest `\n`-delimited line in `bytes`.
fn longest_line_bytes(bytes: &[u8]) -> usize {
    bytes
        .split(|&b| b == b'\n')
        .map(|line| line.len())
        .max()
        .unwrap_or(0)
}

/// One restate of a stream's files at a time: the boot pass and the
/// consumer may both reach the same stream.
static RECONCILING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone)]
pub struct Indexer {
    services: Arc<Services>,
}

impl Indexer {
    pub fn new(services: Arc<Services>) -> Self {
        Self { services }
    }

    /// Restate `stream_id`'s files to its tree at `snapshot_id` (see the
    /// module). Failures are logged: the next snapshot restates again.
    pub async fn reconcile_stream_files(&self, stream_id: &StreamId, snapshot_id: i64) -> Restated {
        let _one = RECONCILING.lock().await;
        match self.reconcile(stream_id, snapshot_id).await {
            Ok(restated) => restated,
            Err(error) => {
                tracing::warn!(%error, stream = %stream_id, snapshot_id, "indexing the stream's files failed");
                Restated::default()
            }
        }
    }

    async fn reconcile(
        &self,
        stream_id: &StreamId,
        snapshot_id: i64,
    ) -> Result<Restated, oxplow_domain::DomainError> {
        let svc = &self.services;
        if !self.live(stream_id).await {
            return Ok(Restated::default());
        }
        // Every file the stream has: its working tree as the stream's
        // filter keeps it (what `v_file` lists). A worktree stream's
        // snapshots hold only the files it touched, so they can't say.
        let generated = svc
            .config
            .read()
            .map(|c| c.generated.clone())
            .unwrap_or_default();
        let stream = stream_id.to_string();
        let paths = svc
            .workspace_files
            .list_paths(Some(&stream), &generated, None)
            .await
            .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))?;
        // The contents, where the snapshot tree has them. "Track changes
        // only" keeps none: then every file is found by name alone.
        let bodies = svc.snapshot_captures.no_contents_reason().is_none();
        let mut captured: BTreeMap<String, oxplow_db::FileSnapshot> = BTreeMap::new();
        if bodies {
            for f in svc.snapshot_store.list_tree_files_at(snapshot_id).await? {
                if f.blob_hash.is_some() && !f.storage.is_oversize() {
                    captured.insert(f.path.clone(), f);
                }
            }
        }
        let mut versions: BTreeMap<EntryKey, String> = BTreeMap::new();
        let mut files: BTreeMap<EntryKey, (String, Option<oxplow_db::FileSnapshot>)> =
            BTreeMap::new();
        for p in paths.into_iter().filter(|p| !p.is_dir) {
            let key = (file_uri(&p.path), Some(stream.clone()));
            let snap = captured.remove(&p.path);
            let version = match &snap {
                Some(f) => format!(
                    "{}:{}",
                    f.storage.as_db_str(),
                    f.blob_hash.as_deref().unwrap_or_default()
                ),
                None => "name".to_string(),
            };
            versions.insert(key.clone(), version);
            files.insert(key, (p.path, snap));
        }
        let plan = {
            let (stream, versions) = (stream.clone(), versions.clone());
            svc.db
                .read(move |tx| search_store::plan_restate_tx(tx, FILE, Some(&stream), &versions))
                .await?
        };
        let keys: Vec<EntryKey> = plan.write.iter().map(|(key, ..)| key.clone()).collect();
        let chunks: Vec<&[EntryKey]> = if keys.is_empty() {
            if plan.remove.is_empty() {
                return Ok(Restated::default());
            }
            vec![&[]]
        } else {
            keys.chunks(WRITE_CHUNK).collect()
        };
        let versions = std::sync::Arc::new(versions);
        let mut out = Restated::default();
        let last = chunks.len() - 1;
        for (at, chunk) in chunks.into_iter().enumerate() {
            // A stream archived meanwhile keeps none of its files.
            if !self.live(stream_id).await {
                return Ok(out);
            }
            let wanted: Vec<(EntryKey, String, Option<oxplow_db::FileSnapshot>)> = chunk
                .iter()
                .filter_map(|key| {
                    let (path, snap) = files.get(key)?;
                    Some((key.clone(), path.clone(), snap.clone()))
                })
                .collect();
            let content = svc.snapshot_content.clone();
            let texts: BTreeMap<EntryKey, EntryText> = tokio::task::spawn_blocking(move || {
                wanted
                    .into_iter()
                    .map(|(key, path, snap)| {
                        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                        let body = snap.map(|f| body(&content, &f)).unwrap_or_default();
                        (key, EntryText::named(name, path, body))
                    })
                    .collect()
            })
            .await
            .map_err(|e| oxplow_domain::DomainError::Storage(format!("reading files: {e}")))?;
            // Planned again in the write: what is stored may have moved
            // since the read. This chunk writes the entries it read the
            // text of; the last one also removes the files gone.
            let (stream, versions) = (stream.clone(), versions.clone());
            let wrote = svc
                .db
                .transaction(move |tx| {
                    let mut plan =
                        search_store::plan_restate_tx(tx, FILE, Some(&stream), &versions)?;
                    plan.write.retain(|(key, ..)| texts.contains_key(key));
                    if at != last {
                        plan.remove.clear();
                    }
                    search_store::apply_restate_tx(tx, FILE, &plan, &texts)
                })
                .await?;
            out.added += wrote.added;
            out.changed += wrote.changed;
            out.removed += wrote.removed;
        }
        Ok(out)
    }

    /// Whether the stream is still there and not archived.
    async fn live(&self, stream_id: &StreamId) -> bool {
        matches!(
            self.services.streams.list_streams().await,
            Ok(streams) if streams.iter().any(|s| s.id == *stream_id && s.archived_at.is_none())
        )
    }
}

/// A file's contents as text, or nothing: a too-large body, binary content
/// or minified output has none worth indexing.
fn body(content: &crate::snapshot_content::SnapshotContent, f: &oxplow_db::FileSnapshot) -> String {
    let Some(hash) = f.blob_hash.as_deref() else {
        return String::new();
    };
    if f.size_bytes > MAX_INDEX_FILE_BYTES {
        return String::new();
    }
    // Route through the read seam so VCS-backed rows resolve via the
    // object store (their `blob_hash` is an object id).
    let Ok(bytes) = content.read(f.storage, hash) else {
        return String::new();
    };
    // Binary: a NUL byte is the cheap, reliable heuristic.
    if bytes.contains(&0) || longest_line_bytes(&bytes) > MAX_INDEX_LINE_BYTES {
        return String::new();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn services() -> (Arc<Services>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let svc = Services::in_memory(dir.path())
            .await
            .expect("in-memory services");
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
        // The file in the working tree too, as a capture finds it.
        let on_disk = svc.layout.project_dir.join(path);
        match content {
            Some(bytes) => {
                std::fs::create_dir_all(on_disk.parent().unwrap()).unwrap();
                std::fs::write(&on_disk, bytes).unwrap();
            }
            None => {
                let _ = std::fs::remove_file(&on_disk);
            }
        }
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

    async fn search(svc: &Services, stream: &StreamId, q: &str) -> Vec<String> {
        svc.search_store
            .search(q, Some(&stream.to_string()), &[], 10)
            .await
            .unwrap()
            .into_iter()
            .map(|h| h.uri)
            .collect()
    }

    #[tokio::test]
    async fn indexes_and_removes_file_contents() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let indexer = Indexer::new(svc.clone());

        let snap = capture_file(&svc, &stream.id, "src/frob.rs", Some(b"fn frobnicate() {}")).await;
        indexer.reconcile_stream_files(&stream.id, snap).await;
        let hits = svc
            .search_store
            .search("frobnicate", Some(&stream.id.to_string()), &[], 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].resource, oxplow_domain::uri::resources::FILE);
        assert_eq!(hits[0].uri, "oxplow:oxplow/file?path=src/frob.rs");

        // A later capture with no blob = deletion → index row removed.
        let snap2 = capture_file(&svc, &stream.id, "src/frob.rs", None).await;
        indexer.reconcile_stream_files(&stream.id, snap2).await;
        assert!(search(&svc, &stream.id, "frobnicate").await.is_empty());
        assert!(search(&svc, &stream.id, "frob").await.is_empty());
    }

    /// The whole tree is indexed, not the snapshot's delta alone: a file
    /// captured earlier and unchanged since is found by its name.
    #[tokio::test]
    async fn every_file_of_the_tree_is_found_by_its_name() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        capture_file(&svc, &stream.id, "src/MarkdownView.tsx", Some(b"export {}")).await;
        let snap = capture_file(&svc, &stream.id, "DEV.md", Some(b"run from source")).await;
        Indexer::new(svc.clone())
            .reconcile_stream_files(&stream.id, snap)
            .await;
        assert_eq!(
            search(&svc, &stream.id, "view").await,
            vec!["oxplow:oxplow/file?path=src/MarkdownView.tsx"]
        );
        assert_eq!(
            search(&svc, &stream.id, "dev").await,
            vec!["oxplow:oxplow/file?path=DEV.md"]
        );
    }

    /// A file of the working tree no snapshot holds (a worktree stream's
    /// snapshots hold only what it touched) is found by its name.
    #[tokio::test]
    async fn a_file_no_snapshot_holds_is_found_by_its_name() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let root = svc.layout.project_dir.clone();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/untouched.md"), "never captured").unwrap();
        let snap = capture_file(&svc, &stream.id, "a.rs", Some(b"fn a() {}")).await;
        Indexer::new(svc.clone())
            .reconcile_stream_files(&stream.id, snap)
            .await;
        assert_eq!(
            search(&svc, &stream.id, "untouched").await,
            vec!["oxplow:oxplow/file?path=docs/untouched.md"]
        );
    }

    /// "Track changes only" keeps no file bytes: files are found by name
    /// alone.
    #[tokio::test]
    async fn without_contents_files_are_found_by_name_only() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        svc.snapshot_captures
            .set_content_policy(crate::snapshot_capture::ContentPolicy::HashesOnly);
        let snap = capture_file(&svc, &stream.id, "src/frob.rs", Some(b"fn widget() {}")).await;
        Indexer::new(svc.clone())
            .reconcile_stream_files(&stream.id, snap)
            .await;
        assert!(search(&svc, &stream.id, "widget").await.is_empty());
        assert_eq!(
            search(&svc, &stream.id, "frob").await,
            vec!["oxplow:oxplow/file?path=src/frob.rs"]
        );
    }

    #[tokio::test]
    async fn binary_content_is_found_by_name_only() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        // NUL byte → treated as binary: its body isn't indexed.
        let snap = capture_file(&svc, &stream.id, "a.bin", Some(b"frobnicate\0\xffbinary")).await;
        Indexer::new(svc.clone())
            .reconcile_stream_files(&stream.id, snap)
            .await;
        assert!(search(&svc, &stream.id, "frobnicate").await.is_empty());
        assert_eq!(
            search(&svc, &stream.id, "a.bin").await,
            vec!["oxplow:oxplow/file?path=a.bin"]
        );
    }

    /// Restating the same tree writes nothing.
    #[tokio::test]
    async fn an_unchanged_tree_writes_nothing() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let snap = capture_file(&svc, &stream.id, "a.rs", Some(b"fn a() {}")).await;
        let indexer = Indexer::new(svc.clone());
        assert!(indexer
            .reconcile_stream_files(&stream.id, snap)
            .await
            .wrote());
        assert!(!indexer
            .reconcile_stream_files(&stream.id, snap)
            .await
            .wrote());
    }
}
