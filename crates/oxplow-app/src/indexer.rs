//! Site-wide search indexer.
//!
//! Owns every write into the unified `search_store` (FTS5/BM25). It first
//! **backfills** the index from current state, then keeps it fresh two
//! ways: tasks, wiki pages and snapshot files from the **event log** (the
//! `search.index` pump consumer, P3.10/P5.C4 — durable, redelivered after a
//! crash, on `work_item.created/edited/transitioned/deleted`,
//! `knowledge.page.written/deleted` and `snapshot.taken`), and notes and
//! comments from the **in-memory bus** until those capabilities log events. One uniform mechanism for both DB-resident content
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

/// The pump consumer's name.
pub const SEARCH_INDEX: &str = "search.index";

/// Register the `search.index` consumer on `svc`'s pump (boot, before it
/// spawns).
pub fn register(svc: &Arc<Services>) {
    svc.event_pump.register_async(Arc::new(SearchIndexConsumer {
        services: Arc::downgrade(svc),
    }));
}

/// Keeps tasks and snapshot files in the search index from the event log.
struct SearchIndexConsumer {
    services: std::sync::Weak<Services>,
}

#[async_trait::async_trait]
impl crate::event_pump::AsyncEventConsumer for SearchIndexConsumer {
    fn name(&self) -> &'static str {
        SEARCH_INDEX
    }

    fn handles(&self, event_type: &str) -> bool {
        matches!(
            event_type,
            "work_item.created"
                | "work_item.edited"
                | "work_item.transitioned"
                | "work_item.deleted"
                | "knowledge.page.written"
                | "knowledge.page.deleted"
                | "snapshot.taken"
        )
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
        let indexer = Indexer::new(svc.clone());
        let payload = &event.envelope.payload;
        if let Some(slug) = event
            .envelope
            .event_type
            .starts_with("knowledge.page.")
            .then(|| {
                payload["page"]
                    .as_str()
                    .and_then(|p| p.strip_prefix("wiki:"))
            })
            .flatten()
        {
            // Restated from the page as it stands: a deleted one drops out.
            indexer.index_wiki(slug).await;
            return Ok(());
        }
        if event.envelope.event_type == "snapshot.taken" {
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
                indexer.index_snapshot_files(&stream, snapshot).await;
            }
            return Ok(());
        }
        // Only oxplow tasks have rows to index.
        let Some(task) = payload["work_item"]
            .as_str()
            .and_then(oxplow_domain::refs::build::task_of_work_item_ref)
        else {
            return Ok(());
        };
        // Out of every stream first: a task that moved leaves no row where
        // it was, and a deleted one is simply gone.
        svc.search_store
            .remove_everywhere(KIND_TASK, &task.to_string())
            .await?;
        if let Some(t) = svc.task_store.get(task).await? {
            if t.deleted_at.is_none() {
                let stream = match t.thread_id.as_ref() {
                    Some(tid) => indexer.stream_for_thread(tid).await,
                    None => None,
                };
                indexer.index_task(&t, stream.as_ref()).await;
            }
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

    /// Backfill the DB + wiki portion of the index from current state, then
    /// process events forever. Spawned once at boot (see `main.rs`). File
    /// contents backfill for free via the snapshot startup sweep, which emits
    /// `SnapshotTaken`.
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
            OxplowEvent::WorkNotesChanged {
                thread_id: Some(tid),
                ..
            } => self.reindex_thread_notes(&tid).await,
            OxplowEvent::CommentsChanged {
                target_kind,
                target_id,
                ..
            } => self.reindex_target_comments(&target_kind, &target_id).await,
            // Tasks, wiki pages and snapshot files come off the event log
            // (`register`).
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
                        map.insert(t.id, s.id);
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
        let stream = stream.map(|s| s.to_string());
        // Fold the id (e.g. "tsk30") into the searchable body so typing a
        // task id in the launcher surfaces that task directly — the id is
        // otherwise only a non-FTS routing key. Body (not title) so the
        // displayed hit title stays clean; the launcher floats an exact
        // id match to the top regardless of BM25 weight.
        let body = format!("{}\n{}", t.id, t.description);
        let _ = self
            .services
            .search_store
            .upsert(
                KIND_TASK,
                &t.id.to_string(),
                stream.as_deref(),
                &t.title,
                &body,
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
                Some(&ct.comment.stream_id.to_string()),
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
            let stream = stream.as_ref().map(|s| s.to_string());
            for n in &notes {
                let _ = self
                    .services
                    .search_store
                    .upsert(KIND_NOTE, &n.id.to_string(), stream.as_deref(), "", &n.body)
                    .await;
            }
        }
    }

    // ---- wiki ----

    pub async fn index_wiki(&self, slug: &str) {
        // The body is the row's (P6.E2), written with it in the command's or
        // the watcher's transaction — already committed when the pump
        // hands this the event.
        match self.services.wiki_page_store.body(slug).await {
            Ok(Some((title, body))) => {
                let _ = self
                    .services
                    .search_store
                    .upsert(KIND_WIKI, slug, None, &title, &body)
                    .await;
            }
            _ => {
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

    /// P3.10 (tsk480): a task is indexed from its `work_item.*` events on
    /// the pump — filed, edited, and removed when deleted — with no
    /// in-memory bus in the loop.
    #[tokio::test]
    async fn tasks_are_indexed_from_their_events() {
        let (svc, _dir) = services().await;
        register(&svc);
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = svc
            .threads
            .create(&stream.id, "T", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let sid = stream.id.to_string();
        let found = |q: &'static str| {
            let svc = svc.clone();
            let sid = sid.clone();
            async move {
                svc.search_store
                    .search(q, Some(&sid), &[], 10)
                    .await
                    .unwrap()
                    .iter()
                    .any(|h| h.kind == KIND_TASK)
            }
        };
        let task = svc
            .tasks
            .create(
                Some(thread.id),
                crate::CreateTaskInput {
                    title: "Quux the sprocket".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert!(found("sprocket").await, "indexed on work_item.created");

        let mut edited = task.clone();
        edited.title = "Quux the flange".into();
        svc.task_store.update(&edited).await.unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert!(found("flange").await, "re-indexed on work_item.edited");

        svc.task_store.soft_delete(task.id).await.unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert!(!found("flange").await, "removed on work_item.deleted");
    }

    /// P5.C4: a wiki page is indexed from its `knowledge.page.*` events —
    /// written by command or by hand, and gone when deleted.
    #[tokio::test]
    async fn wiki_pages_are_indexed_from_their_events() {
        let (svc, dir) = services().await;
        register(&svc);
        svc.streams.ensure_primary().await.unwrap();
        let found = |q: &'static str| {
            let svc = svc.clone();
            async move {
                svc.search_store
                    .search(q, None, &[], 10)
                    .await
                    .unwrap()
                    .iter()
                    .any(|h| h.kind == KIND_WIKI)
            }
        };
        svc.commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::knowledge::WRITE_PAGE,
                serde_json::json!({ "slug": "gears", "body": "# Gears\n\nThe sprocket turns.\n" }),
                true,
            )
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert!(found("sprocket").await, "indexed on knowledge.page.written");

        // A hand edit converges and re-indexes.
        std::fs::write(
            crate::knowledge::page_path(dir.path(), "gears"),
            "# Gears\n\nThe flange holds.\n",
        )
        .unwrap();
        crate::wiki_pages::sync_page(&svc.db, &svc.event_schemas, dir.path(), "gears")
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert!(found("flange").await, "re-indexed after a hand edit");

        svc.commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::knowledge::DELETE_PAGE,
                serde_json::json!({ "slug": "gears" }),
                true,
            )
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert!(!found("flange").await, "removed on knowledge.page.deleted");
    }

    /// A task moved to another thread is indexed where it lives now and
    /// nowhere else (tsk508).
    #[tokio::test]
    async fn a_moved_task_is_indexed_only_where_it_now_lives() {
        let (svc, _dir) = services().await;
        register(&svc);
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = svc
            .threads
            .create(&stream.id, "T", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let task = svc
            .tasks
            .create(
                Some(thread.id),
                crate::CreateTaskInput {
                    title: "Quux the sprocket".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        let rows = |svc: Arc<Services>, id: String| async move {
            svc.db
                .read(move |c| {
                    let mut s = c
                        .prepare(
                            "SELECT stream_id FROM search_entry WHERE kind = 'task' AND ref_id = ?1",
                        )
                        .map_err(oxplow_db::map_sql_err)?;
                    let r = s
                        .query_map([id], |r| r.get::<_, Option<String>>(0))
                        .map_err(oxplow_db::map_sql_err)?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(oxplow_db::map_sql_err)?;
                    Ok(r)
                })
                .await
                .unwrap()
        };
        assert_eq!(
            rows(svc.clone(), task.id.to_string()).await,
            vec![Some(stream.id.to_string())]
        );
        svc.task_store.move_task(task.id, None).await.unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(rows(svc.clone(), task.id.to_string()).await, vec![None]);
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
                Some(thread.id),
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
                    kind: "work_item".into(),
                    id: format!("oxplow:{}", task.id),
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

    #[tokio::test]
    async fn finds_a_task_by_its_id_string() {
        // Typing a task id (e.g. "tsk3") in the launcher must surface that
        // task directly — the id is folded into the task's searchable body
        // even when neither the title nor the description mentions it.
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = svc
            .threads
            .create(&stream.id, "T", "working", oxplow_domain::AgentKind::Claude)
            .await
            .unwrap();
        let task = svc
            .tasks
            .create(
                Some(thread.id),
                crate::CreateTaskInput {
                    title: "Totally unrelated title".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        Indexer::new(svc.clone()).backfill().await;

        let id = task.id.to_string(); // e.g. "tsk3"
        let sid = stream.id.to_string();
        let hits = svc
            .search_store
            .search(&id, Some(&sid), &[], 10)
            .await
            .unwrap();
        assert!(
            hits.iter().any(|h| h.kind == KIND_TASK && h.ref_id == id),
            "searching the task id {id:?} should surface the task; got {hits:?}"
        );
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
                Some(thread.id),
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
            .search("flux", Some(&stream.id.to_string()), &[], 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, KIND_TASK);
    }
}
