//! Measure file watching at large-repository scale.
//!
//! Run from the repo root, in release mode:
//!     cargo run --example profile_watch --release -- --root <dir>
//!     cargo run --example profile_watch --release -- --kept 100000 --ignored 1000000
//!
//! `--root <dir>` measures an existing tree and never writes in it (steps
//! that write are skipped). `--kept N --ignored M` builds a tree in a temp
//! folder first (N kept files, about N/5 directories, nested `.gitignore`s,
//! M ignored files in `node_modules`- and `target`-like folders at the top
//! level and nested; one `git` commit of the kept files) and removes it at
//! the end. One line per step, with milliseconds and counts.
//!
//! Macos file events stall while the machine is busy: read the numbers
//! with the load average (printed at the start) in mind.

// Dev-only measuring tool — `unwrap()` is fine here, so relax the
// workspace `unwrap_used` guardrail for this example.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use oxplow_app::blob_store::BlobStore;
use oxplow_app::trees::Trees;
use oxplow_app::vcs::GitProvider;
use oxplow_db::{Database, FileSnapshot, SnapshotStorage, SqliteSnapshotStore};
use oxplow_domain::vcs::Revision;
use oxplow_domain::{StreamId, Timestamp};
use oxplow_fs_watch::{FsWatcher, WorkspaceFilter};
use rayon::prelude::*;
use tokio::sync::broadcast::{self, error::RecvError};

const TOP_DIRS: usize = 20;
const FILES_PER_DIR: usize = 5;
const IGNORED_PER_DIR: usize = 1000;
const HISTORY_ROWS: usize = 10;

enum Source {
    Existing(PathBuf),
    Generate { kept: usize, ignored: usize },
}

fn parse_args() -> Source {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    if let Some(root) = value("--root") {
        return Source::Existing(PathBuf::from(root));
    }
    match (value("--kept"), value("--ignored")) {
        (Some(k), Some(m)) => Source::Generate {
            kept: k.parse().expect("--kept N"),
            ignored: m.parse().expect("--ignored M"),
        },
        _ => {
            eprintln!("usage: profile_watch --root <dir> | --kept N --ignored M");
            std::process::exit(2);
        }
    }
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn step(n: u32, name: &str, line: String) {
    println!("step {n} | {name:<34} | {line}");
}

fn skip(n: u32, name: &str, why: &str) {
    println!("step {n} | {name:<34} | skipped: {why}");
}

fn df(label: &str, path: &Path) {
    let out = Command::new("df").arg("-h").arg(path).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let last = text.lines().last().unwrap_or("");
    println!("df {label}: {last}");
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=perf", "-c", "user.email=perf@example.com"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

/// Where kept directory `k` lives: `src<k % 20>/g<k / 20>`.
fn kept_dir(root: &Path, k: usize) -> PathBuf {
    root.join(format!("src{:02}", k % TOP_DIRS))
        .join(format!("g{}", k / TOP_DIRS))
}

/// Build the tree. Returns the kept-file count actually written.
fn generate(root: &Path, kept: usize, ignored: usize) {
    let dirs = kept.div_ceil(FILES_PER_DIR);
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join(".gitignore"), "node_modules/\ntarget/\n*.log\n").unwrap();
    (0..dirs).into_par_iter().for_each(|k| {
        let dir = kept_dir(root, k);
        std::fs::create_dir_all(&dir).unwrap();
        // A few nested ignore files, each its own rule.
        if k < TOP_DIRS * 3 && k / TOP_DIRS < 3 {
            std::fs::write(dir.join(".gitignore"), "*.tmp\n").unwrap();
        }
        for f in 0..FILES_PER_DIR {
            if k * FILES_PER_DIR + f >= kept {
                break;
            }
            std::fs::write(dir.join(format!("file{f}.txt")), format!("file {k} {f}\n")).unwrap();
        }
    });
    // Ignored: two folders at the top level, two nested.
    let homes = [
        root.join("node_modules"),
        root.join("target"),
        kept_dir(root, 0).join("node_modules"),
        kept_dir(root, 1).join("target"),
    ];
    let per_home = ignored / homes.len();
    for home in &homes {
        write_files(home, per_home);
    }
    git(root, &["init", "-q"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "generated"]);
}

/// `count` one-byte files in `dir`, a thousand to a subfolder.
fn write_files(dir: &Path, count: usize) {
    let subs = count.div_ceil(IGNORED_PER_DIR);
    (0..subs).into_par_iter().for_each(|s| {
        let sub = dir.join(format!("pkg{s}"));
        std::fs::create_dir_all(&sub).unwrap();
        for f in 0..IGNORED_PER_DIR.min(count - s * IGNORED_PER_DIR) {
            std::fs::write(sub.join(format!("f{f}.js")), "x").unwrap();
        }
    });
}

/// Counts what one receiver gets.
#[derive(Default)]
struct Tally {
    events: AtomicU64,
    lagged_skips: AtomicU64,
    lag_notices: AtomicU64,
    /// Nanoseconds from `epoch` to the latest event.
    last_ns: AtomicU64,
}

struct Counter {
    tally: Arc<Tally>,
    epoch: Instant,
}

impl Counter {
    fn spawn(mut rx: broadcast::Receiver<oxplow_fs_watch::WatchEvent>, epoch: Instant) -> Self {
        let tally = Arc::new(Tally::default());
        let t = tally.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(_) => {
                        t.events.fetch_add(1, Ordering::Relaxed);
                        t.last_ns
                            .store(epoch.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    }
                    Err(RecvError::Lagged(n)) => {
                        t.lagged_skips.fetch_add(n, Ordering::Relaxed);
                        t.lag_notices.fetch_add(1, Ordering::Relaxed);
                        t.last_ns
                            .store(epoch.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        });
        Self { tally, epoch }
    }

    fn last_event(&self) -> Instant {
        self.epoch + Duration::from_nanos(self.tally.last_ns.load(Ordering::Relaxed))
    }
}

/// Wait until `counters` have all been silent for `quiet`. With
/// `expect_events`, silence only counts once the first event has arrived
/// (a busy machine delays the OS's events by seconds), up to `limit`.
async fn until_quiet(counters: &[&Counter], expect_events: bool, quiet: Duration, limit: Duration) {
    let started = Instant::now();
    loop {
        let newest = counters.iter().map(|c| c.last_event()).max().unwrap();
        let heard = counters
            .iter()
            .all(|c| c.tally.events.load(Ordering::Relaxed) > 0);
        if newest.elapsed() >= quiet && started.elapsed() >= quiet && (heard || !expect_events) {
            return;
        }
        if started.elapsed() > limit {
            println!("  (not quiet after {})", ms(limit));
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Time from `since` to the last event; zero if it came first.
fn lag_after(c: &Counter, since: Instant) -> Duration {
    c.last_event().saturating_duration_since(since)
}

fn walk_kept(root: &Path, filter: &WorkspaceFilter) -> (usize, usize, usize) {
    let (mut files, mut dirs, mut gitignores) = (0, 0, 0);
    for e in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0 || {
                let rel = e.path().strip_prefix(root).unwrap_or(e.path());
                !filter.ignore(rel, e.file_type().is_dir())
            }
        })
        .filter_map(Result::ok)
    {
        if e.file_type().is_dir() {
            dirs += 1;
        } else {
            files += 1;
            if e.depth() > 1 && e.file_name() == ".gitignore" {
                gitignores += 1;
            }
        }
    }
    (files, dirs, gitignores)
}

fn kept_files(root: &Path, filter: &WorkspaceFilter) -> Vec<String> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0 || {
                let rel = e.path().strip_prefix(root).unwrap_or(e.path());
                !filter.ignore(rel, e.file_type().is_dir())
            }
        })
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            e.path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn load_average() -> String {
    let out = Command::new("sysctl").args(["-n", "vm.loadavg"]).output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let source = parse_args();
    let (root, generated, temp) = match source {
        Source::Existing(p) => (p.canonicalize().expect("--root exists"), false, None),
        Source::Generate { kept, ignored } => {
            let temp = tempfile::Builder::new()
                .prefix("oxplow-profile-watch-")
                .tempdir()
                .unwrap();
            let root = temp.path().canonicalize().unwrap().join("tree");
            df("before generating", temp.path());
            let t = Instant::now();
            generate(&root, kept, ignored);
            println!(
                "generated {kept} kept + {ignored} ignored files in {}",
                ms(t.elapsed())
            );
            df("after generating", temp.path());
            (root, true, Some(temp))
        }
    };
    println!("root: {}", root.display());
    println!("load average at start: {}", load_average());
    let epoch = Instant::now();

    // 1. The filter, built as the daemon builds it.
    let t = Instant::now();
    let filter = WorkspaceFilter::for_project(&root, Vec::<String>::new(), Vec::<String>::new());
    let build = t.elapsed();

    // 2. What one ignore-file edit costs.
    let t = Instant::now();
    let _again = filter.refreshed();
    let refresh = t.elapsed();

    // 3. The startup sweep's walk.
    let t = Instant::now();
    let (files, dirs, nested) = walk_kept(&root, &filter);
    let walk = t.elapsed();
    step(
        1,
        "WorkspaceFilter::for_project",
        format!(
            "{} (nested .gitignores: {nested}, counted in step 3's walk)",
            ms(build)
        ),
    );
    step(2, "WorkspaceFilter::refreshed", ms(refresh));
    step(
        3,
        "WalkDir filter_entry(filter.ignore)",
        format!("{} ({files} files kept, {dirs} dirs kept)", ms(walk)),
    );

    // 4. Starting the watch.
    let t = Instant::now();
    let watcher = FsWatcher::watch_workspace(&root, filter.clone()).unwrap();
    step(4, "FsWatcher::watch_workspace", ms(t.elapsed()));
    // Let FSEvents settle; the numbers below are for a quiet tree.
    let raw0 = Counter::spawn(watcher.subscribe(), epoch);
    until_quiet(
        &[&raw0],
        false,
        Duration::from_secs(2),
        Duration::from_secs(30),
    )
    .await;

    if generated {
        // 5. Write storm under an ignored folder inside a kept directory.
        let raw = Counter::spawn(watcher.subscribe(), epoch);
        let storm = kept_dir(&root, 0).join("node_modules").join("storm");
        std::fs::create_dir_all(&storm).unwrap();
        let t = Instant::now();
        write_files(&storm, 20_000);
        let wrote = t.elapsed();
        let end = Instant::now();
        until_quiet(
            &[&raw],
            false,
            Duration::from_secs(5),
            Duration::from_secs(120),
        )
        .await;
        step(
            5,
            "write storm, 20000 ignored files",
            format!(
                "writes took {}; {} events reached subscribe() ({} lag notices, {} skipped); last event {} after the last write",
                ms(wrote),
                raw.tally.events.load(Ordering::Relaxed),
                raw.tally.lag_notices.load(Ordering::Relaxed),
                raw.tally.lagged_skips.load(Ordering::Relaxed),
                ms(lag_after(&raw, end)),
            ),
        );

        // 6. Burst of kept files, seen raw and debounced.
        let raw = Counter::spawn(watcher.subscribe(), epoch);
        let deb = Counter::spawn(
            watcher.subscribe_debounced(Duration::from_millis(250)),
            epoch,
        );
        let burst = kept_dir(&root, 1).join("burst");
        std::fs::create_dir_all(&burst).unwrap();
        let t = Instant::now();
        (0..5000).into_par_iter().for_each(|i| {
            std::fs::write(burst.join(format!("b{i}.txt")), format!("burst {i}\n")).unwrap();
        });
        let wrote = t.elapsed();
        let end = Instant::now();
        until_quiet(
            &[&raw, &deb],
            true,
            Duration::from_secs(3),
            Duration::from_secs(120),
        )
        .await;
        step(
            6,
            "burst, 5000 kept files",
            format!(
                "writes took {}; subscribe(): {} events, {} lag notices, {} skipped, last event {} after the last write; debounced 250ms: {} events, {} lag notices, {} skipped, last event {} after",
                ms(wrote),
                raw.tally.events.load(Ordering::Relaxed),
                raw.tally.lag_notices.load(Ordering::Relaxed),
                raw.tally.lagged_skips.load(Ordering::Relaxed),
                ms(lag_after(&raw, end)),
                deb.tally.events.load(Ordering::Relaxed),
                deb.tally.lag_notices.load(Ordering::Relaxed),
                deb.tally.lagged_skips.load(Ordering::Relaxed),
                ms(lag_after(&deb, end)),
            ),
        );

        // 7. Time from one write to its event, on a quiet tree.
        let mut rx = watcher.subscribe();
        let file = kept_dir(&root, 2).join("file0.txt");
        let mut delays = Vec::new();
        let mut missed = 0;
        for i in 0..20 {
            while rx.try_recv().is_ok() {}
            let t = Instant::now();
            std::fs::write(&file, format!("edit {i}\n")).unwrap();
            let seen = tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    match rx.recv().await {
                        Ok(ev) if ev.path == file => return true,
                        Ok(_) | Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => return false,
                    }
                }
            })
            .await
            .unwrap_or(false);
            if seen {
                delays.push(t.elapsed());
            } else {
                missed += 1;
            }
            tokio::time::sleep(Duration::from_millis(500).saturating_sub(t.elapsed())).await;
        }
        delays.sort();
        step(
            7,
            "event delay, one file x20 @500ms",
            if delays.is_empty() {
                format!("no events ({missed} missed)")
            } else {
                format!(
                    "min {} | median {} | max {} ({missed} missed)",
                    ms(delays[0]),
                    ms(delays[delays.len() / 2]),
                    ms(delays[delays.len() - 1]),
                )
            },
        );
    } else {
        skip(5, "write storm", "--root is read-only");
        skip(6, "burst", "--root is read-only");
        skip(7, "event delay", "--root is read-only");
    }
    drop(watcher);

    // 8. One snapshot's read of the last known stats, on disk.
    let paths = kept_files(&root, &filter);
    let work = tempfile::Builder::new()
        .prefix("oxplow-profile-watch-db-")
        .tempdir()
        .unwrap();
    let db = Database::open(work.path().join("profile.sqlite")).unwrap();
    {
        use oxplow_domain::stores::StreamStore;
        oxplow_db::SqliteStreamStore::new(db.clone())
            .upsert(&stream())
            .await
            .unwrap();
    }
    let store = Arc::new(SqliteSnapshotStore::new(db));
    let t = Instant::now();
    let mut batch = Vec::with_capacity(20_000);
    let mut rows = 0usize;
    for h in 0..HISTORY_ROWS {
        for p in &paths {
            batch.push(FileSnapshot {
                id: 0,
                stream_id: StreamId::new(1),
                path: p.clone(),
                blob_hash: Some(format!("{:032x}", rows)),
                size_bytes: 10 + h as i64,
                captured_at: Timestamp::from_unix_ms(1_000 + h as i64),
                storage: SnapshotStorage::Oxplow,
                snapshot_id: None,
                mtime_ms: Some(1_000 + h as i64),
                content_hash: None,
            });
            rows += 1;
            if batch.len() == 20_000 {
                store
                    .capture_batch(std::mem::take(&mut batch))
                    .await
                    .unwrap();
            }
        }
    }
    if !batch.is_empty() {
        store.capture_batch(batch).await.unwrap();
    }
    println!("  (seeded {rows} history rows in {})", ms(t.elapsed()));
    let t = Instant::now();
    let latest = store.latest_stat_per_path(StreamId::new(1)).await.unwrap();
    step(
        8,
        "SqliteSnapshotStore::latest_stat",
        format!(
            "{} ({} rows returned of {rows} stored, {HISTORY_ROWS} per path)",
            ms(t.elapsed()),
            latest.len()
        ),
    );
    drop(latest);

    // 9. What a working-files relist costs: Trees::diff(HEAD, Working).
    if root.join(".git").exists() {
        let trees = Trees::new(
            Arc::new(GitProvider),
            store.clone(),
            BlobStore::new(work.path().join("blobs")),
            // Defaults, read from an empty folder (nothing in `root` is read).
            Arc::new(RwLock::new(
                oxplow_config::load_project_config(work.path()).unwrap(),
            )),
        );
        if generated {
            std::fs::write(kept_dir(&root, 2).join("file0.txt"), "one more edit\n").unwrap();
        }
        let head = Revision::git("HEAD");
        let mut times = Vec::new();
        let mut changed = 0;
        for _ in 0..3 {
            let t = Instant::now();
            match trees.diff(&root, Some(&head), &Revision::Working).await {
                Ok(d) => changed = d.len(),
                Err(e) => {
                    println!("  (diff failed: {e})");
                    break;
                }
            }
            times.push(t.elapsed());
        }
        if !times.is_empty() {
            step(
                9,
                "Trees::diff(HEAD, Working)",
                format!(
                    "{} cold, then {} and {} ({changed} changed paths{})",
                    ms(times[0]),
                    times.get(1).map(|d| ms(*d)).unwrap_or_default(),
                    times.get(2).map(|d| ms(*d)).unwrap_or_default(),
                    if generated {
                        ", after one edit"
                    } else {
                        ", the tree as it is"
                    }
                ),
            );
        }
    } else {
        skip(9, "Trees::diff(HEAD, Working)", "not a git repository");
    }

    println!("load average at end: {}", load_average());
    drop(work);
    if let Some(temp) = temp {
        let t = Instant::now();
        drop(temp);
        println!("removed the generated tree in {}", ms(t.elapsed()));
        df("after removing", &std::env::temp_dir());
    }
}

fn stream() -> oxplow_domain::Stream {
    oxplow_domain::Stream {
        id: StreamId::new(1),
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
        host: oxplow_domain::HostId::LOCAL,
        custom_prompt: None,
        created_at: Timestamp::from_unix_ms(0),
        updated_at: Timestamp::from_unix_ms(0),
        archived_at: None,
    }
}
