//! Profile the snapshot startup sweep against a real worktree.
//!
//! Run from the repo root:
//!     cargo run --example profile_snapshot_sweep --release -- <project_dir>
//!
//! Reports time spent on (a) the directory walk + filter, (b) per-file
//! stat, and (c) parallel read+xxh3-128 for the files that fell through
//! the mtime+size short-circuit. Prints how many files reached each
//! phase + cumulative bytes hashed.

// Dev-only profiling tool — `unwrap()` is fine here, so relax the
// workspace `unwrap_used` guardrail for this example.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, UNIX_EPOCH};

use oxplow_app::blob_store::BlobStore;
use oxplow_db::{Database, SqliteSnapshotStore};
use oxplow_fs_watch::should_ignore_workspace_watch_path;
use rayon::prelude::*;
use xxhash_rust::xxh3::Xxh3;

fn mtime_to_unix_ms(m: &std::fs::Metadata) -> Option<i64> {
    m.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let project_dir: PathBuf = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("cwd"));
    let project_dir = project_dir.canonicalize().expect("canonical project_dir");

    eprintln!("profiling worktree: {}", project_dir.display());

    // Two passes:
    //   1. fresh in-memory DB (cold) — every file falls through to
    //      read+hash; measures the worst-case first-startup cost.
    //   2. re-use the same DB after pass 1 has populated stats —
    //      mtime+size short-circuit should fire on every file;
    //      measures the steady-state cost.
    let db = Database::in_memory();
    use oxplow_domain::stores::StreamStore;
    let stream_store = oxplow_db::SqliteStreamStore::new(db.clone());
    stream_store
        .upsert(&oxplow_domain::Stream {
            id: oxplow_domain::StreamId::new(1),
            kind: oxplow_domain::StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/r".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: oxplow_domain::Timestamp::from_unix_ms(0),
            updated_at: oxplow_domain::Timestamp::from_unix_ms(0),
            archived_at: None,
        })
        .await
        .unwrap();
    let store = Arc::new(SqliteSnapshotStore::new(db));
    let blobs = BlobStore::new(std::env::temp_dir().join("oxplow-profile-blobs"));

    eprintln!("\n=== pass 1 (cold): every file hashed ===");
    profile_pass(&project_dir, store.clone(), blobs.clone()).await;

    eprintln!("\n=== pass 2 (warm): mtime+size should short-circuit ===");
    profile_pass(&project_dir, store.clone(), blobs.clone()).await;
}

async fn profile_pass(project_dir: &Path, store: Arc<SqliteSnapshotStore>, blobs: BlobStore) {
    // Load the latest stat map (drives the short-circuit).
    let t_db = Instant::now();
    let latest = store.latest_stat_per_path().await.expect("latest_stat");
    let db_ms = t_db.elapsed().as_millis();
    eprintln!(
        "  load latest_stat_per_path: {db_ms} ms ({} rows)",
        latest.len()
    );

    let project_dir_owned = project_dir.to_path_buf();
    let project_dir = project_dir_owned.clone();
    let blobs_for_pass = blobs.clone();
    let report = tokio::task::spawn_blocking(move || {
        let mut entries_seen = 0u64;
        let mut files_seen = 0u64;
        let mut shortcircuit_hits = 0u64;
        let mut oversize_skipped = 0u64;
        let mut stat_ms = 0u128;
        let mut latest = latest;

        let walk_started = Instant::now();
        let mut entries: Vec<walkdir::DirEntry> = Vec::new();
        for entry in walkdir::WalkDir::new(&project_dir)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                let rel = e.path().strip_prefix(&project_dir).unwrap_or(e.path());
                !should_ignore_workspace_watch_path(rel)
            })
            .filter_map(Result::ok)
        {
            entries_seen += 1;
            entries.push(entry);
        }
        let walk_ms = walk_started.elapsed().as_millis();

        // Max-file-bytes mirrors the default the daemon uses.
        let max_bytes: u64 = 5 * 1024 * 1024;

        // Phase 1 (sequential): stat each file, run the
        // mtime+size short-circuit. Files that fall through queue
        // up for the parallel read+hash phase.
        let mut needs_hash: Vec<PathBuf> = Vec::new();
        for entry in entries {
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

            let t_stat = Instant::now();
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            stat_ms += t_stat.elapsed().as_micros();
            let size = metadata.len() as i64;
            let mtime_ms = mtime_to_unix_ms(&metadata);

            if let Some(p) = prior.as_ref() {
                if let (Some(prior_mtime), Some(cur_mtime)) = (p.mtime_ms, mtime_ms) {
                    if p.size_bytes == size && prior_mtime == cur_mtime {
                        shortcircuit_hits += 1;
                        continue;
                    }
                }
            }

            if size as u64 > max_bytes {
                oversize_skipped += 1;
                continue;
            }
            needs_hash.push(entry.path().to_path_buf());
        }

        // Phase 2 (parallel via rayon): read + xxh3-128 hash. Wall
        // time covers both read and hash; per-file split is no
        // longer meaningful (workers overlap).
        let hashed_bytes_atomic = AtomicU64::new(0);
        let hashed_count_atomic = AtomicU64::new(0);
        let parallel_started = Instant::now();
        needs_hash.par_iter().for_each(|path| {
            let Ok(bytes) = std::fs::read(path) else {
                return;
            };
            hashed_bytes_atomic.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            let mut h = Xxh3::new();
            h.update(&bytes);
            let _hash = format!("{:032x}", h.digest128());
            hashed_count_atomic.fetch_add(1, Ordering::Relaxed);
            let _ = blobs_for_pass.write(&bytes);
        });
        let parallel_ms = parallel_started.elapsed().as_millis();
        let hashed_count = hashed_count_atomic.load(Ordering::Relaxed);
        let hashed_bytes = hashed_bytes_atomic.load(Ordering::Relaxed);

        SweepReport {
            entries_seen,
            files_seen,
            shortcircuit_hits,
            oversize_skipped,
            hashed_count,
            hashed_bytes,
            walk_ms,
            stat_us: stat_ms,
            parallel_ms,
        }
    })
    .await
    .expect("blocking pass");

    eprintln!(
        "  walked    : {} entries in {} ms",
        report.entries_seen, report.walk_ms
    );
    eprintln!(
        "  files     : {} seen | {} short-circuited | {} oversize | {} hashed",
        report.files_seen, report.shortcircuit_hits, report.oversize_skipped, report.hashed_count,
    );
    eprintln!(
        "  hashed    : {} bytes ({:.1} MB)",
        report.hashed_bytes,
        report.hashed_bytes as f64 / 1_048_576.0
    );
    eprintln!(
        "  stat total : {:.1} ms  ({:.1} us / file)",
        report.stat_us as f64 / 1000.0,
        if report.files_seen == 0 {
            0.0
        } else {
            report.stat_us as f64 / report.files_seen as f64
        }
    );
    eprintln!(
        "  parallel read+hash: {} ms wall ({:.1} MB/s effective)",
        report.parallel_ms,
        if report.parallel_ms == 0 {
            0.0
        } else {
            (report.hashed_bytes as f64 / 1_048_576.0) / (report.parallel_ms as f64 / 1000.0)
        }
    );

    // After pass 1, populate the in-memory DB so pass 2 sees the
    // baseline. This block runs in async context after the blocking
    // sweep finished — re-walk + insert rows via the real store API.
    if report.hashed_count > 0 {
        // Detect: if pass 1 already ran (latest_stat had rows), skip.
        let probe = store.latest_stat_per_path().await.unwrap_or_default();
        if probe.is_empty() {
            seed_latest_stat(&project_dir_owned, store.as_ref()).await;
        }
    }
}

struct SweepReport {
    entries_seen: u64,
    files_seen: u64,
    shortcircuit_hits: u64,
    oversize_skipped: u64,
    hashed_count: u64,
    hashed_bytes: u64,
    walk_ms: u128,
    stat_us: u128,
    /// Wall-clock time spent on the rayon-parallel read+hash phase.
    parallel_ms: u128,
}

/// After pass 1's measurement loop, capture rows so the shared store
/// has a baseline for pass 2's short-circuit. Uses the real store API
/// so timing reflects production behavior on the warm pass.
async fn seed_latest_stat(project_dir: &Path, store: &SqliteSnapshotStore) {
    use oxplow_db::FileSnapshot;
    use oxplow_domain::Timestamp;
    let project_dir = project_dir.to_path_buf();
    let entries: Vec<(String, std::fs::Metadata, Option<String>)> =
        tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            for entry in walkdir::WalkDir::new(&project_dir)
                .into_iter()
                .filter_entry(|e| {
                    if e.depth() == 0 {
                        return true;
                    }
                    let rel = e.path().strip_prefix(&project_dir).unwrap_or(e.path());
                    !should_ignore_workspace_watch_path(rel)
                })
                .filter_map(Result::ok)
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                let rel = entry
                    .path()
                    .strip_prefix(&project_dir)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .into_owned();
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                let size = metadata.len();
                if size > 5 * 1024 * 1024 {
                    out.push((rel, metadata, None));
                    continue;
                }
                let Ok(bytes) = std::fs::read(entry.path()) else {
                    continue;
                };
                let mut h = Xxh3::new();
                h.update(&bytes);
                let hash = format!("{:032x}", h.digest128());
                out.push((rel, metadata, Some(hash)));
            }
            out
        })
        .await
        .expect("seed walk");
    for (rel, metadata, hash) in entries {
        let size = metadata.len() as i64;
        let mtime_ms = mtime_to_unix_ms(&metadata);
        let oversize = hash.is_none();
        let _ = store
            .capture(FileSnapshot {
                id: 0,
                stream_id: oxplow_domain::StreamId::new(1),
                path: rel,
                blob_hash: hash,
                size_bytes: size,
                captured_at: Timestamp::now(),
                oversize,
                snapshot_id: None,
                mtime_ms,
            })
            .await;
    }
}
