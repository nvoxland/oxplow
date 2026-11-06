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
//! the end. One line per step, with milliseconds and counts, and the load
//! average and `fseventsd`'s CPU and memory around the watching.
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
use oxplow_domain::vcs::{Revision, Vcs};
use oxplow_domain::{StreamId, Timestamp};
use oxplow_fs_watch::{FsWatcher, Subscription, WorkspaceFilter};
use rayon::prelude::*;

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

fn step(n: impl std::fmt::Display, name: &str, line: String) {
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

/// Directories per middle folder: 200 directories of 5 files is a folder of
/// about 1,000 files.
const DIRS_PER_FOLDER: usize = 200;

/// Where kept directory `k` lives: `src<k % 20>/m<n>/g<j>`.
fn kept_dir(root: &Path, k: usize) -> PathBuf {
    let n = k / TOP_DIRS;
    root.join(format!("src{:02}", k % TOP_DIRS))
        .join(format!("m{}", n / DIRS_PER_FOLDER))
        .join(format!("g{}", n % DIRS_PER_FOLDER))
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
        if k / TOP_DIRS < 3 {
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

/// Counts what one subscription is told.
#[derive(Default)]
struct Tally {
    paths: AtomicU64,
    rescans: AtomicU64,
    batches: AtomicU64,
    /// Nanoseconds from `epoch` to the latest batch.
    last_ns: AtomicU64,
}

struct Counter {
    tally: Arc<Tally>,
    epoch: Instant,
}

impl Counter {
    /// Read `sub` as a consumer does: at once (`window` zero) or a burst as
    /// one batch.
    fn spawn(mut sub: Subscription, window: Duration, epoch: Instant) -> Self {
        let tally = Arc::new(Tally::default());
        let t = tally.clone();
        tokio::spawn(async move {
            while let Some(batch) = sub.next_batch_after(window).await {
                t.paths
                    .fetch_add(batch.paths.len() as u64, Ordering::Relaxed);
                t.rescans
                    .fetch_add(batch.rescans.len() as u64, Ordering::Relaxed);
                t.batches.fetch_add(1, Ordering::Relaxed);
                t.last_ns
                    .store(epoch.elapsed().as_nanos() as u64, Ordering::Relaxed);
            }
        });
        Self { tally, epoch }
    }

    fn heard(&self) -> bool {
        self.tally.batches.load(Ordering::Relaxed) > 0
    }

    fn last_event(&self) -> Instant {
        self.epoch + Duration::from_nanos(self.tally.last_ns.load(Ordering::Relaxed))
    }

    /// "N paths, M rescans in B batches".
    fn told(&self) -> String {
        format!(
            "{} paths, {} rescans in {} batches",
            self.tally.paths.load(Ordering::Relaxed),
            self.tally.rescans.load(Ordering::Relaxed),
            self.tally.batches.load(Ordering::Relaxed),
        )
    }
}

/// Wait until `counters` have all been silent for `quiet`. With
/// `expect_events`, silence only counts once the first batch has arrived
/// (a busy machine delays the OS's events by seconds), up to `limit`.
async fn until_quiet(counters: &[&Counter], expect_events: bool, quiet: Duration, limit: Duration) {
    let started = Instant::now();
    loop {
        let newest = counters.iter().map(|c| c.last_event()).max().unwrap();
        let heard = counters.iter().all(|c| c.heard());
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

/// The directory holding the number of kept files nearest `target` (a
/// folder with at least one), and how many it holds.
fn folder_near(paths: &[String], target: usize) -> (String, usize) {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for p in paths {
        let mut dir = Path::new(p).parent();
        while let Some(d) = dir {
            if d.as_os_str().is_empty() {
                break;
            }
            *counts.entry(d.to_string_lossy().into_owned()).or_default() += 1;
            dir = d.parent();
        }
    }
    counts
        .into_iter()
        .min_by_key(|(d, n)| (n.abs_diff(target), d.clone()))
        .unwrap_or_default()
}

fn load_average() -> String {
    // Linux has it in /proc, macOS in sysctl.
    if let Ok(text) = std::fs::read_to_string("/proc/loadavg") {
        return text
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" ");
    }
    let out = Command::new("sysctl").args(["-n", "vm.loadavg"]).output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// `fseventsd`'s CPU % and resident MB, now (what a watch costs every
/// process on the machine). Stops the run above 2 GB.
fn fseventsd(label: &str) {
    // Only macOS has one.
    if !cfg!(target_os = "macos") {
        return;
    }
    let out = Command::new("sh")
        .arg("-c")
        .arg("ps -o %cpu=,rss= -p $(pgrep -x fseventsd | head -1)")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let mut it = text.split_whitespace();
    let (cpu, rss_kb) = (it.next().unwrap_or("?"), it.next().unwrap_or("0"));
    let mb = rss_kb.parse::<u64>().unwrap_or(0) / 1024;
    println!("fseventsd {label}: {cpu}% cpu, {mb} MB resident");
    if mb > 2048 {
        println!("STOPPING: fseventsd is above 2 GB");
        std::process::exit(3);
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // The watch's own warnings (the OS's limit on watches) go to stderr.
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn")
        .with_writer(std::io::stderr)
        .try_init();
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
    fseventsd("before watching");
    let epoch = Instant::now();

    // 1. The filter, built as the daemon builds it (it reads no rules yet).
    let t = Instant::now();
    let filter = WorkspaceFilter::for_project(&root, Vec::<String>::new(), Vec::<String>::new());
    let build = t.elapsed();

    // 3. The startup sweep's walk (reads each folder's rules as it goes).
    let t = Instant::now();
    let (files, dirs, nested) = walk_kept(&root, &filter);
    let walk = t.elapsed();

    // 2. What one ignore-file edit costs: the file is noted, and the next
    // path judged reads the rules it needs again.
    let judged = kept_files(&root, &filter)
        .into_iter()
        .next()
        .unwrap_or_default();
    let t = Instant::now();
    filter.note_ignore_file(Path::new(".gitignore"));
    let noted = t.elapsed();
    let t = Instant::now();
    let _ = filter.ignore(Path::new(&judged), false);
    let judge = t.elapsed();

    step(
        1,
        "WorkspaceFilter::for_project",
        format!(
            "{} (reads no rules; nested .gitignores: {nested}, counted in step 3's walk)",
            ms(build)
        ),
    );
    step(
        2,
        "note_ignore_file + one judged path",
        format!("{} noted + {} judged ({judged})", ms(noted), ms(judge)),
    );
    step(
        3,
        "WalkDir filter_entry(filter.ignore)",
        format!("{} ({files} files kept, {dirs} dirs kept)", ms(walk)),
    );

    // 4. Starting the watch.
    let t = Instant::now();
    let watcher = FsWatcher::watch_workspace(&root, filter.clone()).unwrap();
    step(4, "FsWatcher::watch_workspace", ms(t.elapsed()));
    fseventsd("after the watch started");
    let uncovered = watcher.uncovered().list();
    step(
        "4b",
        "directories the OS left uncovered",
        format!(
            "{}{}",
            uncovered.len(),
            uncovered
                .first()
                .map(|d| format!(" (first: {})", d.display()))
                .unwrap_or_default()
        ),
    );
    // Let FSEvents settle; the numbers below are for a quiet tree.
    let raw0 = Counter::spawn(watcher.subscribe(), Duration::ZERO, epoch);
    until_quiet(
        &[&raw0],
        false,
        Duration::from_secs(2),
        Duration::from_secs(30),
    )
    .await;

    if generated {
        // 5. Write storm under an ignored folder inside a kept directory.
        let raw = Counter::spawn(watcher.subscribe(), Duration::ZERO, epoch);
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
                "writes took {}; subscribe() told {}; last batch {} after the last write",
                ms(wrote),
                raw.told(),
                ms(lag_after(&raw, end)),
            ),
        );

        // 6. Burst of kept files, seen raw and debounced.
        let raw = Counter::spawn(watcher.subscribe(), Duration::ZERO, epoch);
        let deb = Counter::spawn(watcher.subscribe(), Duration::from_millis(250), epoch);
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
                "writes took {}; at once: {}, last batch {} after the last write; next_batch_after(250ms): {}, last batch {} after",
                ms(wrote),
                raw.told(),
                ms(lag_after(&raw, end)),
                deb.told(),
                ms(lag_after(&deb, end)),
            ),
        );

        // 7. Time from one write to its event, on a quiet tree.
        let mut sub = watcher.subscribe();
        let file = kept_dir(&root, 2).join("file0.txt");
        let mut delays = Vec::new();
        let mut missed = 0;
        for i in 0..20 {
            while sub.try_batch().is_some() {}
            let t = Instant::now();
            std::fs::write(&file, format!("edit {i}\n")).unwrap();
            // Seen: the file named, or a rescan of a directory above it.
            let seen = tokio::time::timeout(Duration::from_secs(15), async {
                while let Some(batch) = sub.next_batch().await {
                    if batch.paths.iter().any(|(p, _)| *p == file)
                        || batch.rescans.iter().any(|d| file.starts_with(d))
                    {
                        return true;
                    }
                }
                false
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
        fseventsd("after the writes");
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

    // 8b. What a take of a few changed files costs now: ten paths, and one
    // folder of about 1,000 files.
    let ten: Vec<String> = (0..10)
        .map(|i| paths[i * paths.len() / 10].clone())
        .collect();
    let t = Instant::now();
    let got = store
        .latest_stat_for_paths(StreamId::new(1), &ten)
        .await
        .unwrap();
    step(
        "8b",
        "latest_stat_for_paths, 10 paths",
        format!("{} ({} rows)", ms(t.elapsed()), got.len()),
    );
    let (folder, in_folder) = folder_near(&paths, 1000);
    let t = Instant::now();
    let got = store
        .latest_stat_under(StreamId::new(1), &folder)
        .await
        .unwrap();
    step(
        "8c",
        "latest_stat_under, one folder",
        format!(
            "{} ({} rows; `{folder}` holds {in_folder} kept files, {HISTORY_ROWS} history rows each)",
            ms(t.elapsed()),
            got.len()
        ),
    );

    // 9. What a working-files relist costs: the whole-tree diff, and the
    // VCS's status with the diff of the paths it names.
    if root.join(".git").exists() {
        let vcs = Arc::new(GitProvider);
        let trees = Trees::new(
            vcs.clone(),
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
        // The head as its id: `diff_from_status` answers from the status only
        // when `from` is the head itself.
        let head = Revision::git(vcs.resolve(&root, "HEAD").await.unwrap());
        // The tree as it is now (the burst's 5,000 files are in a generated
        // one); then, in a generated tree, as one edit leaves it.
        let mut phases = vec![if generated {
            "after the burst and one edit"
        } else {
            "the tree as it is"
        }];
        if generated {
            phases.push("after one edit only");
        }
        for (i, phase) in phases.into_iter().enumerate() {
            if i == 1 {
                // The burst's folder removed and its edits undone, then one edit.
                std::fs::remove_dir_all(kept_dir(&root, 1).join("burst")).unwrap();
                git(&root, &["checkout", "-q", "--", "."]);
                std::fs::write(kept_dir(&root, 2).join("file0.txt"), "a single edit\n").unwrap();
            }
            let mut full = Vec::new();
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
                full.push(t.elapsed());
            }
            let mut status_times = Vec::new();
            let mut from_status = Vec::new();
            let (mut entries, mut changed_by_status) = (0, 0);
            for _ in 0..3 {
                let t = Instant::now();
                let status = match vcs.status(&root).await {
                    Ok(s) => s,
                    Err(e) => {
                        println!("  (status failed: {e})");
                        break;
                    }
                };
                status_times.push(t.elapsed());
                entries = status.entries.len();
                let t = Instant::now();
                match trees.diff_from_status(&root, Some(&head), &status).await {
                    Ok(d) => changed_by_status = d.len(),
                    Err(e) => {
                        println!("  (diff_from_status failed: {e})");
                        break;
                    }
                }
                from_status.push(t.elapsed());
            }
            let three = |t: &[Duration]| {
                format!(
                    "{} cold, then {} and {}",
                    t.first().map(|d| ms(*d)).unwrap_or_default(),
                    t.get(1).map(|d| ms(*d)).unwrap_or_default(),
                    t.get(2).map(|d| ms(*d)).unwrap_or_default(),
                )
            };
            let tag = if i == 0 { "9" } else { "9'" };
            step(
                format!("{tag}a"),
                "Trees::diff(HEAD, Working)",
                format!("{} ({changed} changed paths; {phase})", three(&full)),
            );
            step(
                format!("{tag}b"),
                "vcs.status alone",
                format!("{} ({entries} entries; {phase})", three(&status_times)),
            );
            step(
                format!("{tag}c"),
                "Trees::diff_from_status",
                format!(
                    "{} ({changed_by_status} changed paths; {phase})",
                    three(&from_status)
                ),
            );
        }
    } else {
        skip(9, "Trees::diff(HEAD, Working)", "not a git repository");
    }

    println!("load average at end: {}", load_average());
    fseventsd("at the end");
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
