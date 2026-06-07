//! Site-wide search indexer.
//!
//! Owns every write into the unified `search_store` (FTS5/BM25). It runs as a
//! single background task that first **backfills** the index from current
//! state, then **subscribes to the event bus** and keeps the index fresh as
//! content changes. One uniform mechanism for both DB-resident content
//! (tasks, comments, notes) and disk-derived content (wiki bodies, file
//! contents — file handling lives alongside in the snapshot-event handler).
//!
//! Coarse events drive *upserts* of the affected scope; deletes are handled
//! precisely where the signal allows (a wiki file gone from disk, a file
//! snapshot with no blob). Hard-deletes of DB entities converge on the next
//! boot backfill — the index is a derived cache, not a source of truth.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::broadcast;

use oxplow_domain::stores::{CommentStore, TaskNoteStore, TaskStore, ThreadStore};
use oxplow_domain::{CommentTarget, CommentThread, StreamId, Task, ThreadId};

use crate::events::OxplowEvent;
use crate::Services;

pub const KIND_TASK: &str = "task";
pub const KIND_COMMENT: &str = "comment";
pub const KIND_NOTE: &str = "note";
pub const KIND_WIKI: &str = "wiki";
pub const KIND_FILE: &str = "file";

/// Skip indexing file bodies larger than this (the FTS index stores its own
/// copy of the text; bound it so a few huge files can't bloat the DB).
const MAX_INDEX_FILE_BYTES: i64 = 512 * 1024;

#[derive(Clone)]
pub struct Indexer {
    services: Arc<Services>,
}

impl Indexer {
    pub fn new(services: Arc<Services>) -> Self {
        Self { services }
    }

    /// Backfill the DB + wiki portion of the index from current state, then
    /// process events forever. Spawned once at boot (see `main.rs`). File
    /// contents backfill for free via the snapshot startup sweep, which emits
    /// `FileSnapshotsBatchCreated`.
    pub async fn run(self, mut rx: broadcast::Receiver<OxplowEvent>) {
        self.backfill().await;
        loop {
            match rx.recv().await {
                Ok(ev) => self.handle(ev).await,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    /// Dispatch one event to the matching reindex. File-snapshot events are
    /// handled here too (see `index_snapshot_files`).
    pub async fn handle(&self, ev: OxplowEvent) {
        match ev {
            OxplowEvent::TasksChanged { thread_id } => {
                self.reindex_thread_tasks(thread_id.as_ref()).await
            }
            OxplowEvent::WorkNotesChanged {
                thread_id: Some(tid),
                ..
            } => self.reindex_thread_notes(&tid).await,
            OxplowEvent::CommentsChanged {
                target_kind,
                target_id,
                ..
            } => self.reindex_target_comments(&target_kind, &target_id).await,
            OxplowEvent::WikiPagesChanged { slug } => self.index_wiki(&slug).await,
            OxplowEvent::FileSnapshotCreated {
                stream_id: Some(stream_id),
                snapshot_id,
                ..
            }
            | OxplowEvent::FileSnapshotsBatchCreated {
                stream_id: Some(stream_id),
                snapshot_id,
                ..
            } => self.index_snapshot_files(&stream_id, snapshot_id).await,
            _ => {}
        }
    }

    // ---- backfill ----

    pub async fn backfill(&self) {
        let thread_stream = self.thread_stream_map().await;
        // Tasks (stream via thread; backlog tasks are global → None).
        if let Ok(tasks) = self.services.task_store.list_all_for_backfill().await {
            for t in tasks {
                let stream = t
                    .thread_id
                    .as_ref()
                    .and_then(|tid| thread_stream.get(tid))
                    .cloned();
                self.index_task(&t, stream.as_ref()).await;
            }
        }
        // Comments (per stream) + thread notes (per thread).
        if let Ok(streams) = self.services.streams.list_streams().await {
            for s in &streams {
                if let Ok(threads) = self.services.comment_store.list_for_stream(&s.id).await {
                    for ct in threads {
                        self.index_comment(&ct).await;
                    }
                }
            }
        }
        for thread_id in thread_stream.keys() {
            self.reindex_thread_notes(thread_id).await;
        }
        // Wiki pages (full body, project-global).
        if let Ok(pages) = self.services.wiki_page_store.list().await {
            for p in pages {
                self.index_wiki(&p.slug).await;
            }
        }
    }

    async fn thread_stream_map(&self) -> HashMap<ThreadId, StreamId> {
        let mut map = HashMap::new();
        if let Ok(streams) = self.services.streams.list_streams().await {
            for s in streams {
                if let Ok(threads) = self.services.thread_store.list_for_stream(&s.id).await {
                    for t in threads {
                        map.insert(t.id, s.id.clone());
                    }
                }
            }
        }
        map
    }

    // ---- tasks ----

    pub async fn reindex_thread_tasks(&self, thread_id: Option<&ThreadId>) {
        match thread_id {
            Some(tid) => {
                let stream = self.stream_for_thread(tid).await;
                if let Ok(tasks) = self.services.task_store.list_for_thread(tid).await {
                    for t in &tasks {
                        self.index_task(t, stream.as_ref()).await;
                    }
                }
            }
            None => {
                if let Ok(tasks) = self.services.task_store.list_backlog().await {
                    for t in &tasks {
                        self.index_task(t, None).await;
                    }
                }
            }
        }
    }

    async fn index_task(&self, t: &Task, stream: Option<&StreamId>) {
        let _ = self
            .services
            .search_store
            .upsert(
                KIND_TASK,
                &t.id.to_string(),
                stream.map(StreamId::as_str),
                &t.title,
                &t.description,
            )
            .await;
    }

    // ---- comments ----

    pub async fn reindex_target_comments(&self, target_kind: &str, target_id: &str) {
        let target = CommentTarget {
            kind: target_kind.to_string(),
            id: target_id.to_string(),
        };
        if let Ok(threads) = self.services.comment_store.list_for_target(&target).await {
            for ct in &threads {
                self.index_comment(ct).await;
            }
        }
    }

    async fn index_comment(&self, ct: &CommentThread) {
        // Title = the anchored quote; body = quote + every message, so a
        // search hits both the highlighted span and the discussion.
        let mut body = ct.comment.quote.clone();
        for m in &ct.messages {
            body.push('\n');
            body.push_str(&m.body);
        }
        let _ = self
            .services
            .search_store
            .upsert(
                KIND_COMMENT,
                &ct.comment.id.to_string(),
                Some(ct.comment.stream_id.as_str()),
                &ct.comment.quote,
                &body,
            )
            .await;
    }

    // ---- notes ----

    pub async fn reindex_thread_notes(&self, thread_id: &ThreadId) {
        let stream = self.stream_for_thread(thread_id).await;
        if let Ok(notes) = self
            .services
            .work_note_store
            .list_for_thread(thread_id)
            .await
        {
            for n in &notes {
                let _ = self
                    .services
                    .search_store
                    .upsert(
                        KIND_NOTE,
                        n.id.as_str(),
                        stream.as_ref().map(StreamId::as_str),
                        "",
                        &n.body,
                    )
                    .await;
            }
        }
    }

    // ---- wiki ----

    pub async fn index_wiki(&self, slug: &str) {
        let page = self.services.wiki_page_store.get(slug).await.ok().flatten();
        let Some(page) = page else {
            let _ = self
                .services
                .search_store
                .remove(KIND_WIKI, slug, None)
                .await;
            return;
        };
        let path = self.services.layout.project_dir.join(&page.body_path);
        match tokio::fs::read_to_string(&path).await {
            Ok(body) => {
                let _ = self
                    .services
                    .search_store
                    .upsert(KIND_WIKI, slug, None, &page.title, &body)
                    .await;
            }
            Err(_) => {
                let _ = self
                    .services
                    .search_store
                    .remove(KIND_WIKI, slug, None)
                    .await;
            }
        }
    }

    // ---- files (implemented in the file-content child) ----

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
        for f in files {
            // Deletion capture (no blob) → drop the index row.
            let Some(hash) = f.blob_hash.as_deref() else {
                let _ = self
                    .services
                    .search_store
                    .remove(KIND_FILE, &f.path, Some(stream_id.as_str()))
                    .await;
                continue;
            };
            if f.oversize || f.size_bytes > MAX_INDEX_FILE_BYTES {
                continue;
            }
            let Ok(bytes) = self.services.blobs.read(hash) else {
                continue;
            };
            // Skip binary: a NUL byte is the cheap, reliable heuristic.
            if bytes.contains(&0) {
                continue;
            }
            let content = String::from_utf8_lossy(&bytes);
            let _ = self
                .services
                .search_store
                .upsert(
                    KIND_FILE,
                    &f.path,
                    Some(stream_id.as_str()),
                    &f.path,
                    &content,
                )
                .await;
        }
    }

    // ---- helpers ----

    async fn stream_for_thread(&self, thread_id: &ThreadId) -> Option<StreamId> {
        self.services
            .thread_store
            .get(thread_id)
            .await
            .ok()
            .flatten()
            .map(|t| t.stream_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::CommentIntent;

    async fn services() -> (Arc<Services>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let svc = Arc::new(Services::in_memory(dir.path()).expect("in-memory services"));
        (svc, dir)
    }

    #[tokio::test]
    async fn indexes_tasks_comments_notes_via_backfill() {
        let (svc, _dir) = services().await;
        // Seed a stream + thread so task/note/comment scoping resolves.
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = svc
            .threads
            .create(&stream.id, "T", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();

        let task = svc
            .tasks
            .create(
                Some(thread.id.clone()),
                crate::CreateTaskInput {
                    title: "Indexable widget task".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        svc.work_note_store
            .add_for_thread(&thread.id, "a note about the gadget", "agent")
            .await
            .unwrap();
        svc.comment_store
            .create(
                &stream.id,
                Some(&thread.id),
                &CommentTarget {
                    kind: "task".into(),
                    id: task.id.to_string(),
                },
                "quoted sprocket text",
                "{}",
                &[],
                &[],
                CommentIntent::Note,
                "user",
                "comment body mentions doohickey",
            )
            .await
            .unwrap();

        Indexer::new(svc.clone()).backfill().await;

        let sid = stream.id.to_string();
        let hits = |q: &'static str| {
            let svc = svc.clone();
            let sid = sid.clone();
            async move {
                svc.search_store
                    .search(q, Some(&sid), &[], 10)
                    .await
                    .unwrap()
            }
        };
        assert!(hits("widget").await.iter().any(|h| h.kind == KIND_TASK));
        assert!(hits("gadget").await.iter().any(|h| h.kind == KIND_NOTE));
        assert!(hits("doohickey")
            .await
            .iter()
            .any(|h| h.kind == KIND_COMMENT));
        assert!(hits("sprocket")
            .await
            .iter()
            .any(|h| h.kind == KIND_COMMENT));
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
        let snap_id = svc
            .snapshot_store
            .create_snapshot(stream.clone())
            .await
            .unwrap();
        let (blob_hash, size) = match content {
            Some(bytes) => (Some(svc.blobs.write(bytes).unwrap()), bytes.len() as i64),
            None => (None, 0),
        };
        svc.snapshot_store
            .capture(oxplow_db::FileSnapshot {
                id: 0,
                stream_id: stream.clone(),
                path: path.into(),
                blob_hash,
                size_bytes: size,
                captured_at: Timestamp::from_unix_ms(0),
                oversize: false,
                snapshot_id: Some(snap_id),
                mtime_ms: Some(0),
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
            .search("frobnicate", Some(stream.id.as_str()), &[], 10)
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
            .search("frobnicate", Some(stream.id.as_str()), &[], 10)
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
            .search("frobnicate", Some(stream.id.as_str()), &[], 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn task_event_reindexes_incrementally() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = svc
            .threads
            .create(&stream.id, "T", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let indexer = Indexer::new(svc.clone());

        svc.tasks
            .create(
                Some(thread.id.clone()),
                crate::CreateTaskInput {
                    title: "fix the flux capacitor".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Simulate the bus event the create would emit.
        indexer.reindex_thread_tasks(Some(&thread.id)).await;

        let hits = svc
            .search_store
            .search("flux", Some(stream.id.as_str()), &[], 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, KIND_TASK);
    }
}
