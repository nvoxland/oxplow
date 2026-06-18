//! Request-driven file-snapshot capture.
//!
//! A singleton manager subscribes to `oxplow_fs_watch` events for
//! the project worktree and accumulates a *dirty set* of paths that
//! changed since the last capture. **Nothing is written to the
//! `file_snapshot` table until someone calls `request_snapshot()`.**
//! That call drains the dirty set, captures each path once, and
//! returns the new snapshot ids.
//!
//! Bytes are persisted to a content-addressed blob store under
//! `<project>/.oxplow/snapshots/<aa>/<aaaa...>`, keyed by the
//! SHA-256 hash. The `local_blame` overlay and
//! `restore_file_from_snapshot` both read through `BlobStore::read`
//! to recover past file content.
//!
//! Cheap to clone — the underlying state is held in an `Arc`. Spawn
//! the watcher loop once at boot via `spawn_watcher()`; everything
//! else is method calls on the cloned handle.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Cloneable result of an in-flight snapshot capture, published to
/// concurrent waiters via `tokio::sync::watch`. The error is collapsed
/// to an `Arc<str>` because `Box<dyn Error + Send + Sync>` isn't
/// `Clone`; concurrent waiters reconstruct an error from the message.
type SharedSnapshotResult = Result<Option<i64>, Arc<str>>;

use tracing::{debug, info, warn};

use std::time::UNIX_EPOCH;

use oxplow_db::{FileSnapshot, SnapshotStorage, SqliteSnapshotStore};

/// How long a path must persist on disk after we first hear about it
/// before we'll write a content row. Editor atomic-write temp files
/// (e.g. `foo.tsx.tmp.NNNNN.HASH`) live for a few milliseconds; a 1 s
/// settle window is comfortably above that floor and below human
/// perception. Tests override this via [`SnapshotCaptureService::with_settle_duration`].
pub const DEFAULT_SETTLE_DURATION: Duration = Duration::from_millis(1000);

/// How long [`SnapshotCaptureService::request_snapshot`] waits before
/// draining the dirty set, giving the fs-watch debouncer (250 ms in
/// `workspace_watch`) time to deliver any in-flight events through
/// the broadcast channel and into the dirty set. Without this, an
/// edit followed almost immediately by `complete_task` produces an
/// empty bracket — the edit hasn't propagated yet, so
/// `end_snapshot_id == start_snapshot_id` and the file-review diff
/// reports the claim as unchanged.
///
/// 300 ms = 250 ms debouncer + a 50 ms cushion for the broadcast
/// hop + run_watcher loop. Tests override via
/// [`SnapshotCaptureService::with_predrain_delay`].
pub const DEFAULT_PREDRAIN_DELAY: Duration = Duration::from_millis(300);

/// A capped rayon pool for the startup sweep's read+hash+blob fan-out,
/// so hashing a large worktree doesn't peg every core — the sweep is
/// one-time background work and the UI/agents need headroom. Leaves a
/// couple of cores free (never fewer than 2 threads). `None` ⇒ caller
/// falls back to the global rayon pool.
fn sweep_thread_pool() -> Option<rayon::ThreadPool> {
    let cores = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);
    let threads = cores.saturating_sub(2).max(2);
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .ok()
}

/// Extract `mtime` from a `Metadata` and convert to unix
/// milliseconds. Returns `None` when the platform / filesystem
/// doesn't expose mtime (rare) — callers fall back to hashing.
fn mtime_to_unix_ms(m: &std::fs::Metadata) -> Option<i64> {
    m.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}
use oxplow_domain::{StreamId, Timestamp};
use oxplow_fs_watch::{FsWatcher, WatchEventKind, WorkspaceFilter};

use crate::blob_store::BlobStore;
use crate::events::{EventBus, OxplowEvent, SnapshotSourceKind};

/// Pre-computed metadata supplied to `mark_dirty_with_staging` by
/// callers that already read + hashed the file (and wrote the blob).
/// When attached to a dirty-set entry, the capture loop skips re-stat
/// / re-read / re-hash / re-write and builds the DB row directly.
///
/// `blob_hash` is an xxh3-128 (`storage = Oxplow`), a git blob OID
/// (`storage = Git`), or `None` for `Oversize` / `Deleted` rows — the
/// `storage` field is the discriminator.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureStaging {
    pub size_bytes: i64,
    pub mtime_ms: Option<i64>,
    pub blob_hash: Option<String>,
    pub storage: SnapshotStorage,
}

/// State per path tracked in the dirty set between snapshot drains.
/// `first_seen` is the earliest moment fs-watch told us *something*
/// happened to this path; it's the input to the settle gate that
/// drops short-lived temp files. `staging`, when populated by the
/// startup sweep, lets the capture loop skip stat/read/hash/write.
#[derive(Debug, Clone)]
struct DirtyEntry {
    staging: Option<CaptureStaging>,
    first_seen: Instant,
    last_kind: WatchEventKind,
}

#[derive(Clone)]
pub struct SnapshotCaptureService {
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<SqliteSnapshotStore>,
    blobs: BlobStore,
    project_dir: PathBuf,
    stream_id: StreamId,
    /// Files larger than this skip blob hashing and are flagged
    /// `oversize`. Pulled from `OxplowConfig::snapshot_max_file_bytes`.
    max_file_bytes: u64,
    /// Workspace-relative path filter — drives BOTH the fs-watch
    /// event handler's "should I react to this change" check and the
    /// startup-sweep WalkDir's filter_entry. Built from the project's
    /// `generated` config at bootstrap; the default segments
    /// (`.git`, `.oxplow`, `target`, `node_modules`, …) are always
    /// in effect even with an empty user list.
    ///
    /// Behind a lock so the UI's generated-paths toggle (`set_generated`
    /// IPC) can swap it at runtime via [`SnapshotCaptureService::set_workspace_filter`]
    /// without restarting the app.
    workspace_filter: RwLock<WorkspaceFilter>,
    /// Optional event bus. When set, each captured snapshot fires a
    /// `FileSnapshotCreated` event so the renderer can refresh the
    /// Snapshots panel without polling.
    events: RwLock<Option<EventBus>>,
    /// Paths that have changed since the last `request_snapshot()`.
    /// The watcher loop pushes into this map; `request_snapshot`
    /// drains it. Keyed by path so repeated edits between requests
    /// collapse into a single capture.
    ///
    /// The value carries optional pre-staged metadata: callers that
    /// already read + hashed the file (currently just the startup
    /// sweep, after writing the blob inline) supply
    /// `Some(CaptureStaging)`, letting `request_snapshot` skip the
    /// stat / read / hash / blob.write entirely and just build the DB
    /// row. fs-watch and explicit `mark_dirty` callers store `None`;
    /// those paths go through the full parallel-process pipeline in
    /// `request_snapshot`.
    dirty: Mutex<HashMap<PathBuf, DirtyEntry>>,
    /// How long a newly-observed path must persist on disk before we
    /// accept it as real. Entries whose `first_seen` is younger than
    /// `now - settle_duration` defer to the next snapshot drain. See
    /// [`DEFAULT_SETTLE_DURATION`]. Tests set this to `Duration::ZERO`
    /// to bypass the gate.
    settle_duration: Duration,
    /// How long `request_snapshot` waits before draining the dirty
    /// set. Lets the fs-watch debouncer flush in-flight events. See
    /// [`DEFAULT_PREDRAIN_DELAY`]. Tests set this to `Duration::ZERO`
    /// to capture immediately.
    predrain_delay: Duration,
    /// Single-flight slot for `request_snapshot`. When a capture is
    /// running, this holds a `watch` receiver that publishes the
    /// eventual result. Concurrent callers clone the receiver and
    /// await the same result — they neither drain the dirty set nor
    /// start a second capture. The slot is cleared back to `None`
    /// after the running capture publishes its result, so the next
    /// call starts fresh.
    in_flight: Mutex<Option<tokio::sync::watch::Receiver<Option<SharedSnapshotResult>>>>,
    /// Signalled by [`SnapshotCaptureService::shutdown`] to tear down
    /// the `spawn_watcher` task. `run_watcher` selects on this
    /// alongside `rx.recv()`, so a registry `unregister` ends the
    /// otherwise-immortal watcher (its `FsWatcher` is a task-local, and
    /// the task holds its own clone of the service — dropping the
    /// registry's `Arc` alone never wakes it). `notify_one` stores a
    /// permit when no waiter is parked yet, so a shutdown that races
    /// ahead of the watcher's first poll is not lost.
    shutdown: tokio::sync::Notify,
    /// `true` once this stream's initial startup sweep has completed (or
    /// there was nothing to sweep). Starts `true` so a service that
    /// never runs a startup sweep (non-primary streams) never gates;
    /// boot flips it `false` for the duration of the primary's sweep via
    /// [`SnapshotCaptureService::begin_initial_sweep`]. Effort-start
    /// awaits this ([`SnapshotCaptureService::await_initial_ready`]) so
    /// an effort's baseline reflects the whole pre-edit tree, not a
    /// half-swept one.
    initial_ready: tokio::sync::watch::Sender<bool>,
}

impl SnapshotCaptureService {
    pub fn new(
        store: Arc<SqliteSnapshotStore>,
        blobs: BlobStore,
        project_dir: PathBuf,
        stream_id: StreamId,
        max_file_bytes: u64,
        workspace_filter: WorkspaceFilter,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                blobs,
                project_dir,
                stream_id,
                max_file_bytes,
                workspace_filter: RwLock::new(workspace_filter),
                events: RwLock::new(None),
                dirty: Mutex::new(HashMap::new()),
                settle_duration: DEFAULT_SETTLE_DURATION,
                predrain_delay: DEFAULT_PREDRAIN_DELAY,
                in_flight: Mutex::new(None),
                shutdown: tokio::sync::Notify::new(),
                initial_ready: tokio::sync::watch::channel(true).0,
            }),
        }
    }

    /// Mark that a startup sweep is about to run for this stream —
    /// [`await_initial_ready`] will block until [`mark_initial_complete`].
    pub fn begin_initial_sweep(&self) {
        let _ = self.inner.initial_ready.send_replace(false);
    }

    /// Signal that the initial sweep finished (or had nothing to do),
    /// releasing any effort-start waiters.
    pub fn mark_initial_complete(&self) {
        let _ = self.inner.initial_ready.send_replace(true);
    }

    /// Wait until this stream's initial startup snapshot is complete —
    /// returns immediately when it already is (or when this stream never
    /// sweeps). Effort-start awaits this so the baseline is whole.
    pub async fn await_initial_ready(&self) {
        let mut rx = self.inner.initial_ready.subscribe();
        // Err only if the sender dropped (it lives in `inner`'s Arc, so
        // that won't happen while we hold a handle) — treat as ready.
        let _ = rx.wait_for(|&ready| ready).await;
    }

    /// Swap the workspace path filter at runtime. Called when the
    /// project's `generated` config changes (via the `set_generated`
    /// IPC) so include/exclude edits take effect on the next fs-watch
    /// event and the next startup sweep without an app restart.
    pub fn set_workspace_filter(&self, filter: WorkspaceFilter) {
        *self
            .inner
            .workspace_filter
            .write()
            .unwrap_or_else(|e| e.into_inner()) = filter;
    }

    /// Override the settle window (default [`DEFAULT_SETTLE_DURATION`]).
    /// Setting this to `Duration::ZERO` disables the gate and captures
    /// every newly-observed path on the next snapshot — used by tests
    /// to keep the existing capture semantics.
    pub fn with_settle_duration(mut self, settle: Duration) -> Self {
        // We're the sole reference until the service is shared via
        // Arc::clone — Arc::get_mut is safe here in the builder.
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.settle_duration = settle;
        } else {
            warn!("with_settle_duration called after the service was shared; ignoring");
        }
        self
    }

    /// Override the predrain delay (default [`DEFAULT_PREDRAIN_DELAY`]).
    /// Setting this to `Duration::ZERO` makes `request_snapshot` drain
    /// the dirty set immediately — used by tests that drive
    /// `mark_dirty` directly and don't need to wait for fs-watch.
    pub fn with_predrain_delay(mut self, delay: Duration) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.predrain_delay = delay;
        } else {
            warn!("with_predrain_delay called after the service was shared; ignoring");
        }
        self
    }

    /// Attach an `EventBus` so capture emits `FileSnapshotCreated`
    /// after each successful insert.
    pub fn with_events(self, events: EventBus) -> Self {
        *self.inner.events.write().unwrap_or_else(|e| e.into_inner()) = Some(events);
        self
    }

    pub fn project_dir(&self) -> &Path {
        &self.inner.project_dir
    }

    pub fn stream_id(&self) -> &StreamId {
        &self.inner.stream_id
    }

    pub fn blobs(&self) -> &BlobStore {
        &self.inner.blobs
    }

    pub fn store(&self) -> &Arc<SqliteSnapshotStore> {
        &self.inner.store
    }

    /// Spawn the fs-watch listener. The listener only updates the
    /// in-memory dirty set; it never writes to the database. Returns
    /// the `JoinHandle` so callers can await teardown if needed.
    pub fn spawn_watcher(&self) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move { this.run_watcher().await })
    }

    /// Signal the `spawn_watcher` task (if any) to exit. Idempotent and
    /// safe to call even when no watcher was spawned — the notification
    /// is just dropped. Called by `SnapshotCaptureRegistry::unregister`
    /// when a stream is archived/removed so its watcher doesn't linger
    /// until process exit.
    pub fn shutdown(&self) {
        self.inner.shutdown.notify_one();
    }

    /// Spawn a listener that turns `OxplowEvent::GitRefsChanged` into
    /// a snapshot request for this stream. The event fires whenever
    /// HEAD or any ref moves (commit, branch switch, fetch, pull,
    /// rebase, …), so a fresh commit shows up in Local History as a
    /// snapshot row tagged with the new HEAD even when the worktree
    /// itself didn't change between snapshots.
    ///
    /// Requires `with_events` to have been called. No-op (returns a
    /// finished task) when no bus is attached.
    pub fn spawn_git_refs_listener(&self) -> tokio::task::JoinHandle<()> {
        let bus = self
            .inner
            .events
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(bus) = bus else {
            return tokio::spawn(async {});
        };
        let mut rx = bus.subscribe();
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(OxplowEvent::GitRefsChanged { stream_id })
                        if stream_id == this.inner.stream_id =>
                    {
                        if let Err(e) = this.request_snapshot_for_git_refs().await {
                            debug!(error = %e, "snapshot: git-refs trigger failed");
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "snapshot capture: git-refs bus lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    }

    /// Drain any pending dirty files (via `request_snapshot`), then
    /// — if the worktree is clean and HEAD has moved past whatever
    /// the latest snapshot recorded — re-stamp the latest snapshot's
    /// `git_commit` to point at the new HEAD. No new row is created:
    /// the worktree state didn't change, so the existing snapshot is
    /// already the right representation of disk; it just now also
    /// corresponds to a new commit. Called from
    /// `spawn_git_refs_listener`.
    pub async fn request_snapshot_for_git_refs(
        &self,
    ) -> Result<Option<i64>, Box<dyn std::error::Error + Send + Sync>> {
        let after_drain = self.request_snapshot(SnapshotSourceKind::GitRefs).await?;

        // Deliberately bypass GitService's status / HEAD caches here:
        // GitService subscribes to the same `GitRefsChanged` event we
        // do, and its cache-invalidation hop runs concurrently with
        // this task. If we read `git.statuses(...)` we can see the
        // pre-event cache and incorrectly conclude the worktree is
        // dirty (skipping the re-stamp) or stamp the old HEAD.
        let project_dir = self.inner.project_dir.clone();
        let statuses = tokio::task::spawn_blocking({
            let p = project_dir.clone();
            move || oxplow_git::list_git_statuses(&p)
        })
        .await
        .unwrap_or_default();
        if !statuses.is_empty() {
            // Worktree dirty — the next normal capture will record
            // the commit when things settle.
            return Ok(after_drain);
        }
        let Some(head_sha) = tokio::task::spawn_blocking({
            let p = project_dir.clone();
            move || oxplow_git::head_commit_sha(&p)
        })
        .await
        .ok()
        .flatten() else {
            return Ok(after_drain);
        };
        let Some(latest_id) = self
            .inner
            .store
            .latest_snapshot_id_for_stream(self.inner.stream_id)
            .await?
        else {
            // No snapshot yet — the regular capture path will create
            // the first row the next time something is dirty.
            return Ok(None);
        };
        let latest_commit = self.inner.store.get_snapshot_git_commit(latest_id).await?;
        if latest_commit.as_deref() == Some(head_sha.as_str()) {
            return Ok(Some(latest_id));
        }
        self.inner
            .store
            .set_snapshot_git_commit(latest_id, head_sha)
            .await?;
        // Emit a 0-file batch event so renderer surfaces subscribed
        // to snapshot events (Local History dashboard, ChangeAnalysis)
        // refetch and pick up the new `git_commit` value.
        self.emit_batch_event(latest_id, 0, SnapshotSourceKind::GitRefs);
        info!(
            snapshot_id = latest_id,
            "snapshot: re-stamped latest snapshot with new HEAD (no file changes)",
        );
        Ok(Some(latest_id))
    }

    async fn run_watcher(self) {
        let watcher = match FsWatcher::watch(self.inner.project_dir.clone()) {
            Ok(w) => w,
            Err(e) => {
                warn!(error = %e, "snapshot capture: failed to start fs-watch");
                return;
            }
        };
        // Subscribe to the raw, un-debounced stream: the dirty set must
        // be current the instant a snapshot is requested, so we mark
        // paths dirty as soon as the OS reports them rather than after
        // a debounce window. The general-purpose `WorkspaceChanged`
        // feed (workspace_watch) is the one that debounces.
        let mut rx = watcher.subscribe();
        // Park on the shutdown signal once, before the loop, so a
        // `notify_one` that fires between iterations isn't missed. The
        // pinned future is re-polled by each `select!`; `notify_one`'s
        // stored permit also covers a shutdown that races ahead of the
        // first poll.
        let shutdown = self.inner.shutdown.notified();
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    debug!("snapshot capture: watcher shutting down");
                    break;
                }
                ev = rx.recv() => match ev {
                    Ok(event) => {
                        let path = event.path;
                        let rel = path.strip_prefix(&self.inner.project_dir).unwrap_or(&path);
                        if self
                            .inner
                            .workspace_filter
                            .read()
                            .unwrap_or_else(|e| e.into_inner())
                            .ignore(rel, path.is_dir())
                        {
                            continue;
                        }
                        self.mark_dirty(path, event.kind);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "snapshot capture: fs-watch lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    }

    /// Add a path to the dirty set. The next `request_snapshot` will
    /// stat + read + hash + blob.write + INSERT for it. fs-watch and
    /// most call sites use this; the startup sweep prefers
    /// [`mark_dirty_with_staging`] to skip the redundant re-read of
    /// bytes it already had in memory.
    ///
    /// `kind` records the most recent fs-watch verdict for this path
    /// — used by the settle gate to distinguish transient creates
    /// from real ones. Callers without an event source (tests, manual
    /// triggers) pass `WatchEventKind::Other`.
    pub fn mark_dirty(&self, path: PathBuf, kind: WatchEventKind) {
        let mut set = self.inner.dirty.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        set.entry(path)
            .and_modify(|e| {
                // Preserve the earliest `first_seen` so a path that's
                // been bouncing in the dirty set still measures its
                // age from when we first heard about it. Don't
                // downgrade an already-staged entry — keep its
                // pre-computed metadata so capture stays fast.
                e.last_kind = kind.clone();
            })
            .or_insert_with(|| DirtyEntry {
                staging: None,
                first_seen: now,
                last_kind: kind,
            });
    }

    /// Add a path to the dirty set along with pre-computed staging
    /// metadata. `request_snapshot` will build the DB row from the
    /// staging fields and skip the file-read / blob-write — the
    /// caller must have already written the blob (when applicable).
    /// If the path already has a staging entry from a prior call,
    /// the newer staging wins.
    ///
    /// Staged entries set `first_seen` far enough in the past that
    /// the settle gate always lets them through — the startup sweep
    /// already proved the file existed when it staged the row, so
    /// there's no transient-file concern.
    pub fn mark_dirty_with_staging(&self, path: PathBuf, staging: CaptureStaging) {
        let mut set = self.inner.dirty.lock().unwrap_or_else(|e| e.into_inner());
        // Anchor `first_seen` to a point comfortably before any
        // realistic settle window so staged entries are never gated.
        let bypass = Instant::now()
            .checked_sub(Duration::from_secs(3600))
            .unwrap_or_else(Instant::now);
        set.insert(
            path,
            DirtyEntry {
                staging: Some(staging),
                first_seen: bypass,
                last_kind: WatchEventKind::Other,
            },
        );
    }

    /// Prune snapshot rows older than `retention_days` (keeping the
    /// most-recent row per path) and GC any on-disk blobs no longer
    /// referenced. Returns `(rows_pruned, blobs_removed)`.
    pub async fn run_cleanup(
        &self,
        retention_days: u32,
    ) -> Result<(u64, u64), Box<dyn std::error::Error + Send + Sync>> {
        let cutoff = Timestamp::from_unix_ms(
            Timestamp::now().unix_ms() - (retention_days as i64) * 86_400_000,
        );
        let pruned = self.inner.store.prune_older_than(cutoff).await?;
        let referenced = self.inner.store.referenced_blob_hashes().await?;
        let blobs = self.inner.blobs.clone();
        let removed = tokio::task::spawn_blocking(move || blobs.gc(&referenced)).await??;
        Ok((pruned, removed))
    }

    /// Spawn a long-running cleanup loop: runs once shortly after
    /// boot, then every 24h. When `bts` is provided, each iteration
    /// surfaces as a row in the BackgroundTask HUD.
    pub fn spawn_cleanup_loop(
        &self,
        retention_days: u32,
        bts: Option<crate::background_task::BackgroundTaskStore>,
    ) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            // Brief delay so we don't pile cleanup on top of the
            // startup sweep's hashing work.
            tokio::time::sleep(Duration::from_secs(60)).await;
            loop {
                let task = bts.as_ref().map(|s| {
                    s.start(crate::background_task::StartInput {
                        kind: crate::background_task::BackgroundTaskKind::Snapshot,
                        label: "Pruning snapshot history".into(),
                        detail: None,
                        progress: None,
                    })
                });
                let cleanup_started = Instant::now();
                match this.run_cleanup(retention_days).await {
                    Ok((rows, blobs)) => {
                        tracing::info!(
                            rows_pruned = rows,
                            blobs_removed = blobs,
                            retention_days,
                            elapsed_ms = cleanup_started.elapsed().as_millis() as u64,
                            "snapshot cleanup pass",
                        );
                        if let (Some(s), Some(t)) = (bts.as_ref(), task.as_ref()) {
                            s.complete(
                                &t.id,
                                Some(serde_json::json!({
                                    "rowsPruned": rows,
                                    "blobsRemoved": blobs,
                                })),
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "snapshot cleanup failed");
                        if let (Some(s), Some(t)) = (bts.as_ref(), task.as_ref()) {
                            s.fail(&t.id, e.to_string(), None);
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
            }
        })
    }

    /// Walk the worktree and mark every file whose current content
    /// differs from the most recent snapshot. Also marks paths that
    /// had a non-deleted latest snapshot but are no longer on disk —
    /// those get a deletion row when the dirty set is captured.
    ///
    /// Honors the service's `WorkspaceFilter`, so build dirs
    /// and `.oxplow/` internals are skipped (wiki pages pass through
    /// because the filter explicitly allows `.oxplow/wiki/`).
    ///
    /// Doesn't write anything itself — call `request_snapshot` after
    /// to flush the dirty set.
    pub async fn enqueue_startup_diff(
        &self,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let sweep_started = Instant::now();
        let db_started = Instant::now();
        let mut latest = self.inner.store.latest_stat_per_path().await?;
        let prior_rows = latest.len();
        let db_load_ms = db_started.elapsed().as_millis() as u64;
        info!(
            prior_rows,
            db_load_ms, "snapshot startup sweep: loaded latest_stat_per_path",
        );
        let project_dir = self.inner.project_dir.clone();
        let max_bytes = self.inner.max_file_bytes;
        let blobs = self.inner.blobs.clone();
        let filter = self
            .inner
            .workspace_filter
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        // Walk + stat off the async runtime — it's all blocking I/O.
        // The walk + per-file stat-shortcircuit stays single-threaded
        // (cheap; ~50 ms / ~17k files). The expensive read+hash+
        // blob-write for paths that fall through is fanned out across
        // the rayon thread pool — embarrassingly parallel.
        //
        // Phase 2 also writes the blob inline (we already have the
        // bytes in memory). The resulting `CaptureStaging` is shipped
        // back across the spawn_blocking boundary and queued via
        // `mark_dirty_with_staging`, so `request_snapshot` can build
        // each `file_snapshot` row without touching disk or hashing
        // again.
        let queued = tokio::task::spawn_blocking(move || -> Vec<(PathBuf, CaptureStaging)> {
            use rayon::prelude::*;

            // Git baseline: HEAD tree OIDs + the git index's cached stat,
            // so phase 1 can decide "is this committed file still
            // byte-clean?" from the stat it already takes — no extra
            // status/stat pass. Clean files are backed by the git odb
            // (`storage = 'git'`) instead of being read + hashed + copied,
            // so a clean checkout of a large repo writes almost no blobs.
            // Built once (in-memory tree walk + one index read); empty
            // when not a git repo.
            let baseline_started = Instant::now();
            let git_baseline = oxplow_git::GitCleanBaseline::build(&project_dir);
            info!(
                clean_candidates = git_baseline.candidate_count(),
                baseline_ms = baseline_started.elapsed().as_millis() as u64,
                "snapshot startup sweep: git baseline ready",
            );

            // Phase 1 (sequential): walk, stat each file, decide
            // which paths fall through to read+hash. Outputs:
            //   - `staged`: oversize-new (already known) +
            //     reverse-deletions (path missing on disk). No
            //     read needed — staging built from stat only.
            //   - `needs_hash`: paths whose (size, mtime) didn't
            //     match the stored stat — fall through to phase 2.
            // One file handed to phase 2, which decides per item (in
            // parallel) between a git-back (clean vs HEAD) and a
            // read+hash+blob. `mtime` is full-precision for the git
            // stat-shortcut; `mtime_ms` feeds the stored row.
            struct Pending {
                path: PathBuf,
                size: i64,
                mtime_ms: Option<i64>,
                mtime: Option<(i64, u32)>,
                prior_hash: Option<String>,
            }
            let mut staged: Vec<(PathBuf, CaptureStaging)> = Vec::new();
            let mut needs_hash: Vec<Pending> = Vec::new();
            let mut files_seen: u64 = 0;
            let mut shortcircuit_hits: u64 = 0;
            let mut oversize_new: u64 = 0;
            let phase1_started = Instant::now();
            for entry in walkdir::WalkDir::new(&project_dir)
                .into_iter()
                .filter_entry(|e| {
                    if e.depth() == 0 {
                        return true;
                    }
                    let rel = e.path().strip_prefix(&project_dir).unwrap_or(e.path());
                    !filter.ignore(rel, e.file_type().is_dir())
                })
                .filter_map(Result::ok)
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                files_seen += 1;
                let rel = entry
                    .path()
                    .strip_prefix(&project_dir)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .into_owned();
                let prior = latest.remove(&rel);
                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let size = metadata.len() as i64;
                let mtime_ms = mtime_to_unix_ms(&metadata);
                // Full-precision mtime for the git stat-shortcut (the ms
                // form above loses the nanoseconds git compares against).
                let mtime_secnsec = metadata.modified().ok().and_then(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|d| (d.as_secs() as i64, d.subsec_nanos()))
                });
                // Fast equality check: when both size and mtime
                // match (and we have an mtime to compare against —
                // pre-V15 rows have None), the file hasn't been
                // touched since the last capture. Skip read+hash.
                if let Some(p) = prior.as_ref() {
                    if let (Some(prior_mtime), Some(cur_mtime)) = (p.mtime_ms, mtime_ms) {
                        if p.size_bytes == size && prior_mtime == cur_mtime {
                            shortcircuit_hits += 1;
                            continue;
                        }
                    }
                }
                // Oversize files are tracked by metadata-only rows,
                // so we can't hash-compare. Capture only when there's
                // no prior row at all — otherwise the row would be
                // identical to the existing one.
                if size as u64 > max_bytes {
                    if prior.is_none() {
                        oversize_new += 1;
                        staged.push((
                            entry.path().to_path_buf(),
                            CaptureStaging {
                                size_bytes: size,
                                mtime_ms,
                                blob_hash: None,
                                storage: SnapshotStorage::Oversize,
                            },
                        ));
                    }
                    continue;
                }
                // Defer the git-back-vs-read decision to phase 2 so the
                // 19k-file fan-out stays parallel (doing it here, in the
                // sequential walk, regressed phase 1 badly).
                needs_hash.push(Pending {
                    path: entry.path().to_path_buf(),
                    size,
                    mtime_ms,
                    mtime: mtime_secnsec,
                    prior_hash: prior.and_then(|s| s.blob_hash),
                });
            }
            let phase1_ms = phase1_started.elapsed().as_millis() as u64;
            let needs_hash_count = needs_hash.len() as u64;
            info!(
                files_seen,
                shortcircuit_hits,
                oversize_new,
                needs_hash = needs_hash_count,
                phase1_ms,
                "snapshot startup sweep: phase 1 (walk + stat) done",
            );

            // Phase 2 (parallel): read + hash + write-blob the
            // fall-through set across the rayon pool. Each worker
            // is independent — we only emit a staging entry when
            // the new hash differs from the stored one (or there
            // was no stored hash). `BlobStore::write` short-circuits
            // when the content-addressed blob is already on disk,
            // so re-runs are cheap.
            let bytes_read = AtomicU64::new(0);
            let blobs_written = AtomicU64::new(0);
            let git_backed = AtomicU64::new(0);
            let phase2_started = Instant::now();
            let run_phase2 = || -> Vec<(PathBuf, CaptureStaging)> {
                needs_hash
                    .into_par_iter()
                    .filter_map(|p| {
                        let Pending {
                            path,
                            size,
                            mtime_ms,
                            mtime,
                            prior_hash,
                        } = p;
                        // Git-sourced baseline: a committed file still
                        // byte-clean vs HEAD (judged from the walk's stat —
                        // no read) is recorded by its HEAD blob OID, which
                        // lives in the git odb. Decided here so the 19k-file
                        // fan-out stays parallel. The `rel` borrow ends
                        // inside the closure so `path` is free to move.
                        let clean_oid = mtime.and_then(|mt| {
                            let rel = path
                                .strip_prefix(&project_dir)
                                .unwrap_or(&path)
                                .to_string_lossy();
                            git_baseline
                                .clean_head_oid(&rel, size as u64, mt)
                                .map(str::to_string)
                        });
                        if let Some(oid) = clean_oid {
                            // Prior row already at this OID → unchanged.
                            if prior_hash.as_deref() == Some(oid.as_str()) {
                                return None;
                            }
                            git_backed.fetch_add(1, Ordering::Relaxed);
                            return Some((
                                path,
                                CaptureStaging {
                                    size_bytes: size,
                                    mtime_ms,
                                    blob_hash: Some(oid),
                                    storage: SnapshotStorage::Git,
                                },
                            ));
                        }
                        // Genuinely dirty/new → read + hash + blob.
                        let bytes = std::fs::read(&path).ok()?;
                        bytes_read.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                        let hash = BlobStore::hash(&bytes);
                        if let Some(prior) = prior_hash.as_ref() {
                            if *prior == hash {
                                return None;
                            }
                        }
                        // Persist the blob now — we already have the
                        // bytes in memory. The serial capture path
                        // would otherwise re-read the same bytes off
                        // disk a moment later.
                        match blobs.write(&bytes) {
                            Ok(_) => {
                                blobs_written.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(e) => {
                                warn!(?path, error = %e, "snapshot sweep: blob write failed");
                                return None;
                            }
                        }
                        Some((
                            path,
                            CaptureStaging {
                                size_bytes: size,
                                mtime_ms,
                                blob_hash: Some(hash),
                                storage: SnapshotStorage::Oxplow,
                            },
                        ))
                    })
                    .collect()
            };
            // Run the read+hash+blob fan-out on a capped pool so a large
            // worktree sweep leaves cores for the UI / agents instead of
            // pegging the machine (it's one-time background work).
            let hashed: Vec<(PathBuf, CaptureStaging)> = match sweep_thread_pool() {
                Some(pool) => pool.install(run_phase2),
                None => run_phase2(),
            };
            let phase2_ms = phase2_started.elapsed().as_millis() as u64;
            let phase2_bytes = bytes_read.load(Ordering::Relaxed);
            info!(
                rows = hashed.len() as u64,
                git_backed = git_backed.load(Ordering::Relaxed),
                blobs_written = blobs_written.load(Ordering::Relaxed),
                bytes_read = phase2_bytes,
                mb_read = phase2_bytes as f64 / 1_048_576.0,
                phase2_ms,
                throughput_mb_per_s = if phase2_ms == 0 {
                    0.0
                } else {
                    (phase2_bytes as f64 / 1_048_576.0) / (phase2_ms as f64 / 1000.0)
                },
                "snapshot startup sweep: phase 2 (parallel read+hash+blob) done",
            );
            staged.extend(hashed);

            // Any paths still in `latest` had a snapshot but no
            // file on disk now. Re-record deletions only for
            // those whose latest row wasn't already a deletion.
            for (path, stat) in latest {
                if stat.blob_hash.is_some() {
                    staged.push((
                        project_dir.join(path),
                        CaptureStaging {
                            size_bytes: 0,
                            mtime_ms: None,
                            blob_hash: None,
                            storage: SnapshotStorage::Deleted,
                        },
                    ));
                }
            }
            staged
        })
        .await?;

        let count = queued.len();
        for (path, staging) in queued {
            self.mark_dirty_with_staging(path, staging);
        }
        info!(
            queued = count,
            elapsed_ms = sweep_started.elapsed().as_millis() as u64,
            "snapshot startup sweep: done",
        );
        Ok(count)
    }

    /// Capture every path currently in the dirty set. Drains the
    /// set first so concurrent fs-events landing during the capture
    /// loop accumulate for the next request rather than being lost
    /// or double-captured.
    ///
    /// Returns the **`snapshot.id`** that groups every
    /// `file_snapshot` row written by this call. When the dirty set
    /// is empty, no new snapshot row is inserted; the most recent
    /// existing snapshot id for this stream is returned instead (or
    /// `None` if no snapshot has ever been taken for the stream).
    ///
    /// Captures are serialized: if a call arrives while another is
    /// already in flight, it awaits the in-flight capture and returns
    /// the same snapshot id. The dirty set is not drained twice; new
    /// paths that land during the wait get picked up by a subsequent
    /// explicit call.
    pub async fn request_snapshot(
        &self,
        source: SnapshotSourceKind,
    ) -> Result<Option<i64>, Box<dyn std::error::Error + Send + Sync>> {
        enum SlotAction {
            Wait(tokio::sync::watch::Receiver<Option<SharedSnapshotResult>>),
            Run(tokio::sync::watch::Sender<Option<SharedSnapshotResult>>),
        }

        let action = {
            let mut slot = self
                .inner
                .in_flight
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(rx) = slot.as_ref() {
                SlotAction::Wait(rx.clone())
            } else {
                let (tx, rx) = tokio::sync::watch::channel(None);
                *slot = Some(rx);
                SlotAction::Run(tx)
            }
        };

        match action {
            SlotAction::Wait(mut rx) => loop {
                if let Some(shared) = rx.borrow().clone() {
                    return shared.map_err(|msg| -> Box<dyn std::error::Error + Send + Sync> {
                        msg.to_string().into()
                    });
                }
                if rx.changed().await.is_err() {
                    return Err(
                        "in-flight snapshot capture was dropped without publishing a result".into(),
                    );
                }
            },
            SlotAction::Run(tx) => {
                let result = self.capture_inner(source).await;
                let shared: SharedSnapshotResult = match &result {
                    Ok(v) => Ok(*v),
                    Err(e) => Err(Arc::from(e.to_string())),
                };
                let _ = tx.send(Some(shared));
                {
                    let mut slot = self
                        .inner
                        .in_flight
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    *slot = None;
                }
                result
            }
        }
    }

    /// Body of `request_snapshot` — runs the actual drain → blob.write
    /// → DB-insert → git-commit-record pipeline. Callers are expected
    /// to have already taken the single-flight slot in
    /// `request_snapshot`.
    async fn capture_inner(
        &self,
        source: SnapshotSourceKind,
    ) -> Result<Option<i64>, Box<dyn std::error::Error + Send + Sync>> {
        // Yield long enough for the fs-watch debouncer + broadcast hop
        // + `run_watcher` to drain any in-flight events into the dirty
        // set. Without this, an edit that landed on disk less than
        // 250 ms before this call hasn't propagated yet and we'd
        // capture an empty bracket — see `DEFAULT_PREDRAIN_DELAY`.
        let predrain = self.inner.predrain_delay;
        if !predrain.is_zero() {
            tokio::time::sleep(predrain).await;
        }
        let drained: Vec<(PathBuf, DirtyEntry)> = {
            let mut set = self.inner.dirty.lock().unwrap_or_else(|e| e.into_inner());
            set.drain().collect()
        };
        if drained.is_empty() {
            return Ok(self
                .inner
                .store
                .latest_snapshot_id_for_stream(self.inner.stream_id)
                .await?);
        }
        let capture_started = Instant::now();
        let drained_count = drained.len();
        // Settle classification + "have we seen this path before?"
        // depend on the current contents of `file_snapshot`. One
        // query yields both lookups; staged sweep already uses the
        // same call so the query plan is hot.
        let known_paths: std::collections::HashSet<String> = self
            .inner
            .store
            .latest_stat_per_path()
            .await?
            .into_keys()
            .collect();

        // Split into:
        //   - staged   — short-circuit straight to a row.
        //   - unstaged — parallel stat / read / hash / blob.write.
        let mut staged_paths: Vec<(PathBuf, CaptureStaging)> = Vec::new();
        let mut unstaged_entries: Vec<(PathBuf, DirtyEntry)> = Vec::new();
        for (path, mut entry) in drained {
            // `take()` moves the staging out when present (leaving the
            // entry valid for the unstaged branch) — no unwrap needed.
            match entry.staging.take() {
                Some(staging) => staged_paths.push((path, staging)),
                None => unstaged_entries.push((path, entry)),
            }
        }
        let staged_count = staged_paths.len() as u64;
        let unstaged_count = unstaged_entries.len() as u64;

        let project_dir = self.inner.project_dir.clone();
        let stream_id = self.inner.stream_id;
        let max_bytes = self.inner.max_file_bytes;
        let blobs = self.inner.blobs.clone();
        let settle = self.inner.settle_duration;
        let classify_now = Instant::now();

        // The rayon worker yields one of three outcomes per entry: a
        // row to insert, a deferral (re-queue for the next snapshot),
        // or a drop (silently ignore).
        enum Outcome {
            Row(FileSnapshot),
            Defer(PathBuf, DirtyEntry),
        }

        let (rows, deferred): (Vec<FileSnapshot>, Vec<(PathBuf, DirtyEntry)>) =
            tokio::task::spawn_blocking(move || {
                use rayon::prelude::*;

                fn rel_of(project_dir: &Path, path: &Path) -> String {
                    path.strip_prefix(project_dir)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .into_owned()
                }

                let now = Timestamp::now();
                let mut rows: Vec<FileSnapshot> =
                    Vec::with_capacity(staged_paths.len() + unstaged_entries.len());

                // Staged: trivial — staging already carries everything
                // the row needs. The settle gate doesn't apply because
                // mark_dirty_with_staging anchors first_seen in the past.
                for (path, s) in staged_paths {
                    rows.push(FileSnapshot {
                        id: 0,
                        stream_id,
                        path: rel_of(&project_dir, &path),
                        blob_hash: s.blob_hash,
                        size_bytes: s.size_bytes,
                        captured_at: now,
                        storage: s.storage,
                        snapshot_id: None,
                        mtime_ms: s.mtime_ms,
                    });
                }

                // Unstaged: classify per the truth table —
                //   exists, has prior        → capture content
                //   exists, no prior, fresh  → defer (settle gate)
                //   exists, no prior, aged   → capture content
                //   missing, has prior       → deletion row
                //   missing, no prior        → drop (transient temp)
                let outcomes: Vec<Outcome> = unstaged_entries
                    .into_par_iter()
                    .filter_map(|(path, entry)| {
                        let rel = rel_of(&project_dir, &path);
                        let has_prior = known_paths.contains(&rel);
                        let metadata = match std::fs::metadata(&path) {
                            Ok(m) => Some(m),
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                            Err(e) => {
                                debug!(?path, error = %e, "snapshot capture: stat failed");
                                return None;
                            }
                        };
                        match (metadata, has_prior) {
                            (None, false) => None,
                            (None, true) => Some(Outcome::Row(FileSnapshot {
                                id: 0,
                                stream_id,
                                path: rel,
                                blob_hash: None,
                                size_bytes: 0,
                                captured_at: now,
                                storage: SnapshotStorage::Deleted,
                                snapshot_id: None,
                                mtime_ms: None,
                            })),
                            (Some(metadata), has_prior) => {
                                if !metadata.is_file() {
                                    return None;
                                }
                                // Settle gate: only applies when this
                                // path has never been captured before.
                                // Established paths skip straight to
                                // capture.
                                let age = classify_now.saturating_duration_since(entry.first_seen);
                                if !has_prior && age < settle {
                                    return Some(Outcome::Defer(path, entry));
                                }
                                let size = metadata.len();
                                let mtime_ms = mtime_to_unix_ms(&metadata);
                                let oversize = size > max_bytes;
                                let blob_hash = if oversize {
                                    None
                                } else {
                                    match std::fs::read(&path) {
                                        Ok(bytes) => match blobs.write(&bytes) {
                                            Ok(h) => Some(h),
                                            Err(e) => {
                                                debug!(?path, error = %e, "snapshot capture: blob write failed");
                                                return None;
                                            }
                                        },
                                        Err(e) => {
                                            debug!(?path, error = %e, "snapshot capture: read failed");
                                            return None;
                                        }
                                    }
                                };
                                Some(Outcome::Row(FileSnapshot {
                                    id: 0,
                                    stream_id,
                                    path: rel,
                                    blob_hash,
                                    size_bytes: size as i64,
                                    captured_at: now,
                                    storage: if oversize {
                                        SnapshotStorage::Oversize
                                    } else {
                                        SnapshotStorage::Oxplow
                                    },
                                    snapshot_id: None,
                                    mtime_ms,
                                }))
                            }
                        }
                    })
                    .collect();
                let mut deferred: Vec<(PathBuf, DirtyEntry)> = Vec::new();
                for outcome in outcomes {
                    match outcome {
                        Outcome::Row(r) => rows.push(r),
                        Outcome::Defer(p, e) => deferred.push((p, e)),
                    }
                }
                (rows, deferred)
            })
            .await?;

        // Re-queue deferred entries so they're reconsidered on the
        // next drain. We deliberately don't spawn an auto-followup
        // `request_snapshot` here: doing so would create a recursive
        // Send-bound cycle on the anonymous Future types
        // (request_snapshot → capture_inner → tokio::spawn(...) →
        // request_snapshot). In practice, the next external
        // `request_snapshot` (task lifecycle transition, periodic
        // sweep) will pick the deferred entries up. Fresh-file
        // captures may land in a later snapshot than the one that
        // first observed them — a small latency cost for completely
        // suppressing transient-file rows.
        let deferred_count = deferred.len() as u64;
        if !deferred.is_empty() {
            let mut set = self.inner.dirty.lock().unwrap_or_else(|e| e.into_inner());
            for (path, entry) in deferred {
                set.entry(path).or_insert(entry);
            }
        }

        // The rayon worker left `snapshot_id = None` because the
        // snapshot_id wasn't known yet. We only create the snapshot
        // row once we know there's actually something to insert, so a
        // drain that resolves entirely to "defer" or "drop" doesn't
        // leak an empty snapshot.
        if rows.is_empty() {
            info!(
                drained = drained_count as u64,
                staged = staged_count,
                unstaged = unstaged_count,
                deferred = deferred_count,
                source = ?source,
                "snapshot request: nothing to capture (all deferred or dropped)",
            );
            return Ok(self
                .inner
                .store
                .latest_snapshot_id_for_stream(self.inner.stream_id)
                .await?);
        }

        let snapshot_id = self
            .inner
            .store
            .create_snapshot(self.inner.stream_id)
            .await?;
        // Fill in the real snapshot_id now that the row exists.
        let mut rows = rows;
        for row in &mut rows {
            row.snapshot_id = Some(snapshot_id);
        }

        let assembled = rows.len() as u64;
        let insert_started = Instant::now();
        let ids = self.inner.store.capture_batch(rows).await?;
        let insert_ms = insert_started.elapsed().as_millis() as u64;
        self.emit_batch_event(snapshot_id, ids.len() as u32, source);
        let capture_ms = capture_started.elapsed().as_millis() as u64;
        info!(
            snapshot_id,
            drained = drained_count as u64,
            staged = staged_count,
            unstaged = unstaged_count,
            inserted = ids.len() as u64,
            assembled,
            insert_ms,
            capture_ms,
            source = ?source,
            "snapshot request: captured drained set",
        );
        // After capture, record the current git commit if (and only
        // if) the worktree is clean — gitignored files don't count.
        // The check happens AFTER capture so any in-flight edits
        // were already drained into this snapshot's file rows.
        //
        // Bypass GitService caches here — we may be running on the
        // same `GitRefsChanged` event GitService is busy invalidating
        // on. A live `git status` + HEAD read is cheap and avoids
        // recording the pre-event commit by mistake. Skipped when
        // the project dir isn't a git repo at all.
        let project_dir = self.inner.project_dir.clone();
        let commit_record_started = Instant::now();
        let statuses = tokio::task::spawn_blocking({
            let p = project_dir.clone();
            move || oxplow_git::list_git_statuses(&p)
        })
        .await
        .unwrap_or_default();
        let clean = statuses.is_empty();
        if clean {
            let sha = tokio::task::spawn_blocking({
                let p = project_dir.clone();
                move || oxplow_git::head_commit_sha(&p)
            })
            .await
            .ok()
            .flatten();
            if let Some(sha) = sha {
                if let Err(e) = self
                    .inner
                    .store
                    .set_snapshot_git_commit(snapshot_id, sha)
                    .await
                {
                    debug!(error = %e, "snapshot: failed to record git commit");
                }
            }
        }
        info!(
            snapshot_id,
            clean,
            git_commit_record_ms = commit_record_started.elapsed().as_millis() as u64,
            "snapshot request: git commit record step",
        );
        Ok(Some(snapshot_id))
    }

    fn emit_batch_event(&self, snapshot_id: i64, file_count: u32, source: SnapshotSourceKind) {
        let guard = self.inner.events.read().unwrap_or_else(|e| e.into_inner());
        if let Some(bus) = guard.as_ref() {
            bus.emit(OxplowEvent::FileSnapshotsBatchCreated {
                stream_id: Some(self.inner.stream_id),
                snapshot_id,
                file_count,
                source,
                effort_id: None,
                thread_id: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;
    use tempfile::tempdir;

    const TEST_STREAM: StreamId = StreamId::new(1);

    async fn seed_stream(db: &Database) {
        use oxplow_domain::stores::StreamStore;
        let streams = oxplow_db::SqliteStreamStore::new(db.clone());
        streams
            .upsert(&oxplow_domain::Stream {
                id: TEST_STREAM,
                kind: oxplow_domain::StreamKind::Primary,
                title: "t".into(),
                branch: "main".into(),
                branch_ref: "refs/heads/main".into(),
                branch_source: "main".into(),
                worktree_path: "/r".into(),
                working_pane: String::new(),
                talking_pane: String::new(),
                working_session_id: String::new(),
                talking_session_id: String::new(),
                custom_prompt: None,
                created_at: Timestamp::from_unix_ms(0),
                updated_at: Timestamp::from_unix_ms(0),
                archived_at: None,
            })
            .await
            .unwrap();
    }

    async fn svc_for(
        project: &std::path::Path,
    ) -> (SnapshotCaptureService, Arc<SqliteSnapshotStore>) {
        let db = Database::in_memory();
        seed_stream(&db).await;
        let store = Arc::new(SqliteSnapshotStore::new(db));
        let blobs = BlobStore::new(project.join(".oxplow/snapshots"));
        // Tests bypass the settle gate so they observe immediate
        // captures; the gate is independently tested elsewhere.
        let svc = SnapshotCaptureService::new(
            store.clone(),
            blobs,
            project.to_path_buf(),
            TEST_STREAM,
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(Duration::ZERO)
        .with_predrain_delay(Duration::ZERO);
        (svc, store)
    }

    /// Init a git repo at `dir` and commit `files` so they're clean
    /// tracked content (eligible for git-backed snapshot storage).
    fn init_git_repo_with(dir: &std::path::Path, files: &[(&str, &str)]) {
        let repo = git2::Repository::init(dir).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "t").unwrap();
        cfg.set_str("user.email", "t@e.com").unwrap();
        let mut idx = repo.index().unwrap();
        for (p, c) in files {
            let full = dir.join(p);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, c).unwrap();
            idx.add_path(std::path::Path::new(p)).unwrap();
        }
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = repo.signature().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
    }

    #[tokio::test]
    async fn startup_sweep_git_backs_clean_tracked_files() {
        let project = tempdir().unwrap();
        init_git_repo_with(project.path(), &[("tracked.txt", "hello world\n")]);
        // An untracked, uncommitted file — must take the oxplow path.
        std::fs::write(project.path().join("dirty.txt"), "scratch").unwrap();

        let (svc, store) = svc_for(project.path()).await;
        let queued = svc.enqueue_startup_diff().await.unwrap();
        assert!(queued >= 2, "both files should queue, got {queued}");
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();

        // Clean tracked file → git-backed: blob_hash is the HEAD OID and
        // NO blob was copied into the oxplow store.
        let tracked = store.list_for_path("tracked.txt").await.unwrap();
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].storage, SnapshotStorage::Git);
        let oid = tracked[0].blob_hash.clone().unwrap();
        let expect = oxplow_git::clean_head_blob_oids(project.path())
            .remove("tracked.txt")
            .unwrap();
        assert_eq!(oid, expect, "blob_hash must be the git blob OID");
        assert!(
            !svc.inner.blobs.has(&oid),
            "git-backed file must not write a blob",
        );
        // And it reads back through the seam from the git odb.
        let bytes = crate::snapshot_content::read_snapshot_content(
            SnapshotStorage::Git,
            &oid,
            project.path(),
            &svc.inner.blobs,
        )
        .unwrap();
        assert_eq!(bytes, b"hello world\n");

        // Untracked file → oxplow-backed with a real blob on disk.
        let dirty = store.list_for_path("dirty.txt").await.unwrap();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].storage, SnapshotStorage::Oxplow);
        assert!(svc.inner.blobs.has(dirty[0].blob_hash.as_ref().unwrap()));
    }

    #[tokio::test]
    async fn shutdown_ends_the_spawned_watcher_task() {
        let project = tempdir().unwrap();
        let (svc, _store) = svc_for(project.path()).await;
        let handle = svc.spawn_watcher();
        // Give the watcher a moment to start its FsWatcher and park on
        // `rx.recv()`. Without the shutdown signal it would stay parked
        // forever (no fs events arrive), so the handle would never join.
        tokio::time::sleep(Duration::from_millis(50)).await;
        svc.shutdown();
        let joined = tokio::time::timeout(Duration::from_secs(5), handle).await;
        assert!(
            joined.is_ok(),
            "watcher task did not exit within 5s of shutdown()",
        );
    }

    #[tokio::test]
    async fn set_workspace_filter_swaps_the_effective_filter_at_runtime() {
        let project = tempdir().unwrap();
        let (svc, _store) = svc_for(project.path()).await;
        let rel = std::path::Path::new("build/out.js");

        // Default filter doesn't ignore a "build" dir.
        assert!(
            !svc.inner.workspace_filter.read().unwrap().ignore(rel, true),
            "precondition: default filter should not ignore build/",
        );

        // Simulate the `set_generated` toggle adding "build" to the
        // generated list — the new filter must take effect immediately.
        svc.set_workspace_filter(oxplow_fs_watch::WorkspaceFilter::with_user_entries([
            "build",
        ]));
        assert!(
            svc.inner.workspace_filter.read().unwrap().ignore(rel, true),
            "set_workspace_filter must swap the live filter without a restart",
        );
    }

    #[tokio::test]
    async fn request_snapshot_captures_dirty_files_and_drains_set() {
        let project = tempdir().unwrap();
        let a = project.path().join("a.txt");
        let b = project.path().join("b.txt");
        std::fs::write(&a, "hello").unwrap();
        std::fs::write(&b, "world").unwrap();
        let (svc, store) = svc_for(project.path()).await;
        svc.mark_dirty(a.clone(), WatchEventKind::Other);
        svc.mark_dirty(b.clone(), WatchEventKind::Other);

        let parent = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .expect("parent id");
        // Both file rows point at the same parent.
        let files = store.list_files_for_snapshot(parent).await.unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|f| f.snapshot_id == Some(parent)));

        // Second request: dirty set was drained, nothing to capture —
        // returns the same parent id (no new row inserted).
        let again = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert_eq!(again, Some(parent));

        assert_eq!(store.list_for_path("a.txt").await.unwrap().len(), 1);
        assert_eq!(store.list_for_path("b.txt").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn request_snapshot_concurrent_callers_share_result() {
        let project = tempdir().unwrap();
        // Seed enough files that the capture takes long enough for a
        // racing caller to land on the in-flight slot.
        for i in 0..50 {
            let p = project.path().join(format!("f{i}.txt"));
            std::fs::write(&p, format!("contents-{i}")).unwrap();
        }
        let (svc, store) = svc_for(project.path()).await;
        for i in 0..50 {
            svc.mark_dirty(
                project.path().join(format!("f{i}.txt")),
                WatchEventKind::Other,
            );
        }

        let svc_a = svc.clone();
        let svc_b = svc.clone();
        let (a, b) = tokio::join!(
            tokio::spawn(async move { svc_a.request_snapshot(SnapshotSourceKind::Startup).await }),
            tokio::spawn(async move { svc_b.request_snapshot(SnapshotSourceKind::Startup).await }),
        );
        let snapshot_a = a.unwrap().unwrap().expect("snapshot id a");
        let snapshot_b = b.unwrap().unwrap().expect("snapshot id b");
        // Both callers see the same snapshot id.
        assert_eq!(snapshot_a, snapshot_b);
        // Only one snapshot row was created — the second caller did
        // not start a fresh capture.
        let all = store
            .list_snapshots_for_stream(TEST_STREAM, 100)
            .await
            .unwrap();
        assert_eq!(
            all.len(),
            1,
            "expected exactly one snapshot row, got {}",
            all.len()
        );
        // Dirty set was drained exactly once.
        assert_eq!(svc.inner.dirty.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn request_snapshot_collapses_repeated_dirty_marks() {
        let project = tempdir().unwrap();
        let file = project.path().join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let (svc, store) = svc_for(project.path()).await;
        for _ in 0..10 {
            svc.mark_dirty(file.clone(), WatchEventKind::Other);
        }
        let parent = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .expect("parent id");
        assert_eq!(
            store.list_files_for_snapshot(parent).await.unwrap().len(),
            1
        );
        assert_eq!(store.list_for_path("a.txt").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn clean_worktree_pins_snapshot_to_head_commit() {
        let project = tempdir().unwrap();
        let repo = git2::Repository::init(project.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "t").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        // Real projects gitignore `.oxplow/` so the snapshot
        // manager's own writes don't dirty the worktree. Mirror that
        // here.
        std::fs::write(project.path().join(".gitignore"), ".oxplow\n").unwrap();
        let tracked = project.path().join("tracked.txt");
        std::fs::write(&tracked, "v1").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("tracked.txt")).unwrap();
        idx.add_path(std::path::Path::new(".gitignore")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = repo.signature().unwrap();
        let head_oid = repo
            .commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        let head_sha = head_oid.to_string();

        let (svc, store) = svc_for(project.path()).await;

        // Clean tree → snapshot records HEAD.
        svc.mark_dirty(tracked.clone(), WatchEventKind::Other);
        let clean_id = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store.get_snapshot_git_commit(clean_id).await.unwrap(),
            Some(head_sha.clone())
        );

        // Mutate the tracked file → worktree now dirty. The next
        // snapshot must NOT carry a git_commit.
        std::fs::write(&tracked, "v2").unwrap();
        svc.mark_dirty(tracked.clone(), WatchEventKind::Other);
        let dirty_id = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .unwrap();
        assert!(store
            .get_snapshot_git_commit(dirty_id)
            .await
            .unwrap()
            .is_none());

        // Gitignored files don't affect cleanliness. Reset the
        // tracked file, then extend .gitignore to also cover junk.log
        // and commit that change so the tree is clean.
        std::fs::write(&tracked, "v1").unwrap();
        std::fs::write(project.path().join(".gitignore"), ".oxplow\njunk.log\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new(".gitignore")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.find_commit(head_oid).unwrap();
        let head_oid2 = repo
            .commit(Some("HEAD"), &sig, &sig, "ignore", &tree, &[&parent])
            .unwrap();
        // Create an ignored file — should not break cleanliness.
        std::fs::write(project.path().join("junk.log"), "noise").unwrap();
        svc.mark_dirty(tracked.clone(), WatchEventKind::Other);
        let with_ignored = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store.get_snapshot_git_commit(with_ignored).await.unwrap(),
            Some(head_oid2.to_string())
        );
    }

    #[tokio::test]
    async fn git_refs_trigger_restamps_latest_snapshot_when_worktree_unchanged() {
        let project = tempdir().unwrap();
        let repo = git2::Repository::init(project.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "t").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        std::fs::write(project.path().join(".gitignore"), ".oxplow\n").unwrap();
        let tracked = project.path().join("tracked.txt");
        std::fs::write(&tracked, "v1").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("tracked.txt")).unwrap();
        idx.add_path(std::path::Path::new(".gitignore")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = repo.signature().unwrap();
        let head1_oid = repo
            .commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        let (svc, store) = svc_for(project.path()).await;

        // Initial snapshot — captures the tracked file and records the
        // first commit.
        svc.mark_dirty(tracked.clone(), WatchEventKind::Other);
        let first_id = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store.get_snapshot_git_commit(first_id).await.unwrap(),
            Some(head1_oid.to_string())
        );

        // User commits a new revision externally — worktree stays
        // byte-identical to its previous state from oxplow's POV.
        let parent = repo.find_commit(head1_oid).unwrap();
        let head2_oid = repo
            .commit(Some("HEAD"), &sig, &sig, "empty 2", &tree, &[&parent])
            .unwrap();

        // No fs-watch event would fire (no file changed), but the
        // git-refs listener triggers this path. The existing snapshot
        // is re-stamped — no new row is created.
        let second_id = svc.request_snapshot_for_git_refs().await.unwrap().unwrap();
        assert_eq!(
            second_id, first_id,
            "no file changes → must re-use the existing row, not create a new one"
        );
        assert_eq!(
            store.get_snapshot_git_commit(second_id).await.unwrap(),
            Some(head2_oid.to_string())
        );

        // Calling again at the same HEAD is a no-op.
        let third_id = svc.request_snapshot_for_git_refs().await.unwrap().unwrap();
        assert_eq!(third_id, second_id);
        assert_eq!(
            store.get_snapshot_git_commit(third_id).await.unwrap(),
            Some(head2_oid.to_string())
        );
    }

    #[tokio::test]
    async fn empty_request_returns_latest_snapshot_id() {
        let project = tempdir().unwrap();
        let (svc, store) = svc_for(project.path()).await;

        // No snapshots yet — request with empty dirty set returns None.
        let first = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert!(first.is_none());

        // Take a real snapshot.
        let file = project.path().join("a.txt");
        std::fs::write(&file, "hi").unwrap();
        svc.mark_dirty(file, WatchEventKind::Other);
        let parent = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .expect("parent id");

        // Subsequent empty requests reuse the same snapshot — no new
        // snapshot row is inserted.
        for _ in 0..3 {
            let again = svc
                .request_snapshot(SnapshotSourceKind::Startup)
                .await
                .unwrap();
            assert_eq!(again, Some(parent));
        }
        // Only one snapshot row exists.
        let latest = store
            .latest_snapshot_id_for_stream(TEST_STREAM)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest, parent);
    }

    #[tokio::test]
    async fn deleted_file_with_prior_row_records_a_deletion() {
        // A path that has a prior content row, then disappears,
        // gets a real deletion row on the next snapshot.
        let project = tempdir().unwrap();
        let file = project.path().join("real.txt");
        let (svc, store) = svc_for(project.path()).await;

        // Prime: real capture so a content row exists.
        std::fs::write(&file, "hello").unwrap();
        svc.mark_dirty(file.clone(), WatchEventKind::Other);
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert_eq!(store.list_for_path("real.txt").await.unwrap().len(), 1);

        // Delete, mark dirty (as fs-watch would), capture again.
        std::fs::remove_file(&file).unwrap();
        svc.mark_dirty(file, WatchEventKind::Removed);
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();

        let rows = store.list_for_path("real.txt").await.unwrap();
        assert_eq!(rows.len(), 2);
        // Newest row is the deletion (no blob).
        assert!(rows[0].blob_hash.is_none());
        assert_eq!(rows[0].size_bytes, 0);
    }

    #[tokio::test]
    async fn dirty_unknown_path_thats_missing_writes_no_row() {
        // A path that fs-watch told us about but that isn't on disk
        // and has no prior content row — the classic "tmp file came
        // and went between snapshot drains" case. Should produce no
        // file_snapshot row at all.
        let project = tempdir().unwrap();
        let file = project.path().join("ghost.txt");
        let (svc, store) = svc_for(project.path()).await;
        svc.mark_dirty(file, WatchEventKind::Removed);
        let snap = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        // No rows written; no snapshot created either.
        assert!(snap.is_none(), "no snapshot should be created");
        let rows = store.list_for_path("ghost.txt").await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn settle_window_defers_a_fresh_new_path() {
        // A path that has no prior content row and was first observed
        // within the settle window must defer to a later snapshot.
        let project = tempdir().unwrap();
        let file = project.path().join("fresh.txt");
        std::fs::write(&file, "x").unwrap();
        let db = Database::in_memory();
        seed_stream(&db).await;
        let store = Arc::new(SqliteSnapshotStore::new(db));
        let blobs = BlobStore::new(project.path().join(".oxplow/snapshots"));
        let svc = SnapshotCaptureService::new(
            store.clone(),
            blobs,
            project.path().to_path_buf(),
            TEST_STREAM,
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(Duration::from_millis(100))
        .with_predrain_delay(Duration::ZERO);

        svc.mark_dirty(file.clone(), WatchEventKind::Created);
        let snap = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert!(snap.is_none(), "first drain should defer the fresh path");
        let rows = store.list_for_path("fresh.txt").await.unwrap();
        assert!(rows.is_empty(), "no row written yet");
        // The deferred entry stays queued. Wait past the settle, then
        // re-request — the entry now ages past the gate and captures.
        tokio::time::sleep(Duration::from_millis(150)).await;
        let parent = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .expect("parent id");
        let rows = store.list_for_path("fresh.txt").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].snapshot_id, Some(parent));
    }

    #[tokio::test]
    async fn settle_window_drops_a_transient_create_then_delete() {
        // Mark a fresh path dirty, then immediately delete it; before
        // the settle window elapses, request a snapshot. No row should
        // be written — the path never had a prior content row and is
        // missing on disk, so it's a pure transient.
        let project = tempdir().unwrap();
        let file = project.path().join("transient.txt");
        std::fs::write(&file, "tmp").unwrap();
        let db = Database::in_memory();
        seed_stream(&db).await;
        let store = Arc::new(SqliteSnapshotStore::new(db));
        let blobs = BlobStore::new(project.path().join(".oxplow/snapshots"));
        let svc = SnapshotCaptureService::new(
            store.clone(),
            blobs,
            project.path().to_path_buf(),
            TEST_STREAM,
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(Duration::from_secs(60))
        .with_predrain_delay(Duration::ZERO);

        svc.mark_dirty(file.clone(), WatchEventKind::Created);
        std::fs::remove_file(&file).unwrap();
        svc.mark_dirty(file, WatchEventKind::Removed);
        let snap = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert!(snap.is_none(), "transient should not create a parent");
        let rows = store.list_for_path("transient.txt").await.unwrap();
        assert!(rows.is_empty(), "no row for a path that came and went");
    }

    async fn bare_service(project: &std::path::Path) -> SnapshotCaptureService {
        let db = Database::in_memory();
        seed_stream(&db).await;
        let store = Arc::new(SqliteSnapshotStore::new(db));
        let blobs = BlobStore::new(project.join(".oxplow/snapshots"));
        SnapshotCaptureService::new(
            store,
            blobs,
            project.to_path_buf(),
            TEST_STREAM,
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
    }

    #[tokio::test]
    async fn initial_ready_gate_blocks_until_complete() {
        let project = tempdir().unwrap();
        let svc = bare_service(project.path()).await;
        svc.begin_initial_sweep();
        let s2 = svc.clone();
        let waiter = tokio::spawn(async move { s2.await_initial_ready().await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            !waiter.is_finished(),
            "effort-start gate must block while the initial sweep is in flight",
        );
        svc.mark_initial_complete();
        tokio::time::timeout(Duration::from_millis(500), waiter)
            .await
            .expect("gate should release after mark_initial_complete")
            .expect("waiter task panicked");
    }

    #[tokio::test]
    async fn initial_ready_default_does_not_gate() {
        let project = tempdir().unwrap();
        let svc = bare_service(project.path()).await;
        // No begin_initial_sweep → ready by default (a stream that never
        // sweeps must not block effort-start forever).
        tokio::time::timeout(Duration::from_millis(200), svc.await_initial_ready())
            .await
            .expect("an un-swept service must not gate");
    }

    #[tokio::test]
    async fn startup_sweep_short_circuits_when_size_and_mtime_match() {
        let project = tempdir().unwrap();
        let file = project.path().join("a.txt");
        std::fs::write(&file, "v1").unwrap();
        let (svc, store) = svc_for(project.path()).await;

        // Prime: capture once so a baseline row exists with mtime.
        svc.mark_dirty(file.clone(), WatchEventKind::Other);
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        let rows = store.list_for_path("a.txt").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].mtime_ms.is_some(), "mtime should be recorded");

        // No changes at all → sweep queues nothing.
        let queued = svc.enqueue_startup_diff().await.unwrap();
        assert_eq!(queued, 0);

        // Real change: write longer content. Size mismatches → falls
        // through to the read+hash path and queues the file.
        std::fs::write(&file, "v3-much-longer").unwrap();
        let queued = svc.enqueue_startup_diff().await.unwrap();
        assert_eq!(queued, 1);
    }

    #[tokio::test]
    async fn startup_sweep_captures_only_changed_files() {
        let project = tempdir().unwrap();
        let a = project.path().join("a.txt");
        let b = project.path().join("b.txt");
        let c = project.path().join("c.txt");
        std::fs::write(&a, "one").unwrap();
        std::fs::write(&b, "two").unwrap();
        std::fs::write(&c, "three").unwrap();
        let (svc, store) = svc_for(project.path()).await;

        // Prime: capture all three so they have a baseline row.
        svc.mark_dirty(a.clone(), WatchEventKind::Other);
        svc.mark_dirty(b.clone(), WatchEventKind::Other);
        svc.mark_dirty(c.clone(), WatchEventKind::Other);
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert_eq!(store.list_for_path("a.txt").await.unwrap().len(), 1);

        // Mutate `a`, leave `b` alone, delete `c`.
        std::fs::write(&a, "one!").unwrap();
        std::fs::remove_file(&c).unwrap();

        let queued = svc.enqueue_startup_diff().await.unwrap();
        assert_eq!(queued, 2);
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();

        // `a` got a new row, `c` got a deletion row, `b` is unchanged.
        assert_eq!(store.list_for_path("a.txt").await.unwrap().len(), 2);
        assert_eq!(store.list_for_path("b.txt").await.unwrap().len(), 1);
        let c_rows = store.list_for_path("c.txt").await.unwrap();
        assert_eq!(c_rows.len(), 2);
        assert!(c_rows[0].blob_hash.is_none());
    }

    #[tokio::test]
    async fn cleanup_prunes_old_rows_and_gcs_orphan_blobs() {
        let project = tempdir().unwrap();
        let file = project.path().join("a.txt");
        let (svc, store) = svc_for(project.path()).await;

        // First capture — content "v1".
        std::fs::write(&file, "v1").unwrap();
        svc.mark_dirty(file.clone(), WatchEventKind::Other);
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();

        // Mutate and capture again — content "v2".
        std::fs::write(&file, "v2").unwrap();
        svc.mark_dirty(file.clone(), WatchEventKind::Other);
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert_eq!(store.list_for_path("a.txt").await.unwrap().len(), 2);

        // Backdate the older row so it falls outside any positive
        // retention window. Then run cleanup with 1 day retention —
        // the older row should be pruned but the newest kept.
        oxplow_db::SqliteSnapshotStore::backdate_for_test(
            store.clone(),
            "a.txt",
            Timestamp::from_unix_ms(0),
        )
        .await;
        let (rows, blobs) = svc.run_cleanup(1).await.unwrap();
        assert_eq!(rows, 1, "old row should be pruned");
        // The pruned row's blob is no longer referenced → GC removes
        // it. The kept row's blob stays.
        assert_eq!(blobs, 1, "orphan blob should be removed");
        let remaining = store.list_for_path("a.txt").await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(svc
            .inner
            .blobs
            .has(remaining[0].blob_hash.as_ref().unwrap()));
    }

    #[tokio::test]
    async fn request_snapshot_uses_staged_metadata_without_reading_disk() {
        // Pre-stage a row for a path whose file doesn't exist on disk.
        // If the capture loop ignored the staging it would either skip
        // the row (stat fails) or record a deletion row. Instead it
        // must emit a row carrying the staged hash + size.
        let project = tempdir().unwrap();
        let (svc, store) = svc_for(project.path()).await;
        let path = project.path().join("phantom.txt");
        // File deliberately not created.
        svc.mark_dirty_with_staging(
            path.clone(),
            CaptureStaging {
                size_bytes: 42,
                mtime_ms: Some(1_700_000_000_000),
                blob_hash: Some("deadbeef".repeat(4)),
                storage: SnapshotStorage::Oxplow,
            },
        );
        let _parent = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .expect("parent id");
        // list_for_path is the only read API that surfaces mtime_ms;
        // list_files_for_snapshot drops it. Both are real but only
        // the per-path one verifies staging carried mtime through.
        let rows = store.list_for_path("phantom.txt").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].size_bytes, 42);
        assert_eq!(rows[0].mtime_ms, Some(1_700_000_000_000));
        assert_eq!(
            rows[0].blob_hash.as_deref(),
            Some("deadbeefdeadbeefdeadbeefdeadbeef"),
        );
        assert!(!rows[0].storage.is_oversize());
    }

    #[tokio::test]
    async fn request_snapshot_handles_mixed_staged_and_unstaged() {
        // Half the dirty set is pre-staged; the other half is raw
        // paths that still need stat+read+hash+blob.write. Both must
        // land in the same snapshot and produce real rows.
        let project = tempdir().unwrap();
        let (svc, store) = svc_for(project.path()).await;

        let staged = project.path().join("staged.txt");
        std::fs::write(&staged, "staged-body").unwrap();
        let staged_bytes = std::fs::read(&staged).unwrap();
        let staged_hash = svc.inner.blobs.write(&staged_bytes).unwrap();
        svc.mark_dirty_with_staging(
            staged.clone(),
            CaptureStaging {
                size_bytes: staged_bytes.len() as i64,
                mtime_ms: Some(42),
                blob_hash: Some(staged_hash.clone()),
                storage: SnapshotStorage::Oxplow,
            },
        );

        let unstaged = project.path().join("unstaged.txt");
        std::fs::write(&unstaged, "unstaged-body-which-is-longer").unwrap();
        svc.mark_dirty(unstaged.clone(), WatchEventKind::Other);

        let parent = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .expect("parent id");
        let files = store.list_files_for_snapshot(parent).await.unwrap();
        assert_eq!(files.len(), 2);
        let staged_row = files.iter().find(|f| f.path == "staged.txt").unwrap();
        let unstaged_row = files.iter().find(|f| f.path == "unstaged.txt").unwrap();
        assert_eq!(staged_row.blob_hash.as_deref(), Some(staged_hash.as_str()));
        // Unstaged side actually read+hashed the file and got a real
        // hash from the BlobStore.
        assert!(unstaged_row.blob_hash.is_some());
        assert!(svc
            .inner
            .blobs
            .has(unstaged_row.blob_hash.as_ref().unwrap()));
    }

    #[tokio::test]
    async fn capture_batch_inserts_all_in_one_transaction() {
        // Drive the new store API directly: 100 rows in one call,
        // each gets a distinct id and shows up in latest_stat_per_path.
        let db = Database::in_memory();
        seed_stream(&db).await;
        let store = SqliteSnapshotStore::new(db);
        let parent = store.create_snapshot(TEST_STREAM).await.unwrap();
        let snaps: Vec<oxplow_db::FileSnapshot> = (0..100)
            .map(|i| oxplow_db::FileSnapshot {
                id: 0,
                stream_id: TEST_STREAM,
                path: format!("file_{i:03}.txt"),
                blob_hash: Some(format!("{:032x}", i)),
                size_bytes: i as i64,
                captured_at: Timestamp::now(),
                storage: SnapshotStorage::Oxplow,
                snapshot_id: Some(parent),
                mtime_ms: Some(1000 + i as i64),
            })
            .collect();
        let ids = store.capture_batch(snaps).await.unwrap();
        assert_eq!(ids.len(), 100);
        assert_eq!(
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            100
        );
        let latest = store.latest_stat_per_path().await.unwrap();
        assert_eq!(latest.len(), 100);
    }

    #[tokio::test]
    async fn predrain_delay_lets_late_dirty_marks_join_the_capture() {
        // Models the bug fix: a path marked dirty *after*
        // `request_snapshot` was already called must still land in the
        // same capture, provided it arrives within the predrain delay.
        // Without the delay (predrain=ZERO control), the late mark is
        // missed and the snapshot returns None.
        let project = tempdir().unwrap();
        let file = project.path().join("late.txt");
        std::fs::write(&file, "hello").unwrap();
        let db = Database::in_memory();
        seed_stream(&db).await;
        let store = Arc::new(SqliteSnapshotStore::new(db));
        let blobs = BlobStore::new(project.path().join(".oxplow/snapshots"));
        let svc = Arc::new(
            SnapshotCaptureService::new(
                store.clone(),
                blobs,
                project.path().to_path_buf(),
                TEST_STREAM,
                1_000_000,
                oxplow_fs_watch::WorkspaceFilter::default(),
            )
            .with_settle_duration(Duration::ZERO)
            // 200 ms gives the spawned task plenty of room to land
            // its mark_dirty before the drain starts.
            .with_predrain_delay(Duration::from_millis(200)),
        );
        // Kick off the snapshot first — drain is gated by the
        // predrain delay so this races against the spawned mark.
        let svc_for_mark = svc.clone();
        let mark = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            svc_for_mark.mark_dirty(file, WatchEventKind::Other);
        });
        let snap = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        mark.await.unwrap();
        assert!(
            snap.is_some(),
            "predrain delay should have let the late mark join the capture",
        );
        let rows = store.list_for_path("late.txt").await.unwrap();
        assert_eq!(rows.len(), 1, "late file should have been captured");
    }

    #[tokio::test]
    async fn oversize_file_skips_hash_and_blob() {
        let project = tempdir().unwrap();
        let file = project.path().join("big.bin");
        std::fs::write(&file, vec![0u8; 1024]).unwrap();
        let db = Database::in_memory();
        seed_stream(&db).await;
        let store = Arc::new(SqliteSnapshotStore::new(db));
        let blobs = BlobStore::new(project.path().join(".oxplow/snapshots"));
        let svc = SnapshotCaptureService::new(
            store.clone(),
            blobs,
            project.path().to_path_buf(),
            TEST_STREAM,
            512, // 512 byte cap → 1KB is oversize
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(Duration::ZERO)
        .with_predrain_delay(Duration::ZERO);
        svc.mark_dirty(file, WatchEventKind::Other);
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        let rows = store.list_for_path("big.bin").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].storage.is_oversize());
        assert!(rows[0].blob_hash.is_none());
    }
}
