//! Lightweight self-diagnostics.
//!
//! Spawns a tokio task that, once a minute, samples three numbers and
//! emits them at `tracing::info`:
//!
//! - **RSS** (resident-set size, KB) — does the process leak memory
//!   over a long session?
//! - **open fds** — proxy for "are watchers / sockets / files
//!   piling up". A steady-state count means the process is releasing
//!   handles cleanly; monotonic growth is the signal we care about.
//! - **streams** — number of stream rows, which equals the number of
//!   per-stream `WorkspaceWatchRegistry` watcher pairs alive.
//!
//! Cheap by construction: one `ps` exec, one `read_dir("/dev/fd")`,
//! one `streams.list()` per minute. No new crate deps.
//!
//! Why this exists: a user reported a system-wide hang and wondered
//! whether oxplow's watchers were leaking handles. With this in place,
//! the next incident has data — grep `tracing` output for `diagnostics`
//! and look at the trend.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use oxplow_domain::stores::StreamStore;
use tracing::{info, warn};

/// How often to sample. Keep it long — these numbers move slowly and
/// we don't want diagnostics noise in the log.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(60);

/// Spawn the diagnostics loop on the current tokio runtime. Returns
/// immediately; the loop runs until the process exits.
pub fn spawn(streams: Arc<dyn StreamStore>) {
    tokio::spawn(async move {
        // Stagger the first sample so it doesn't race with boot.
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            sample_once(&streams).await;
            tokio::time::sleep(SAMPLE_INTERVAL).await;
        }
    });
}

async fn sample_once(streams: &Arc<dyn StreamStore>) {
    let rss_kb = read_rss_kb();
    let fd_count = read_fd_count();
    let stream_count = streams.list().await.map(|s| s.len()).unwrap_or(0);

    info!(
        target: "oxplow::diagnostics",
        rss_kb = rss_kb.map(|n| n as i64).unwrap_or(-1),
        open_fds = fd_count.map(|n| n as i64).unwrap_or(-1),
        streams = stream_count,
        "self-diagnostics sample"
    );
}

/// Resident-set size in KB. Shells out to `ps`; works on macOS and
/// Linux without a new crate dep.
fn read_rss_kb() -> Option<u64> {
    let pid = std::process::id().to_string();
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    std::str::from_utf8(&out.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

/// Open file-descriptor count for the current process.
///
/// `/dev/fd` is present on both macOS and Linux and lists the calling
/// process's fds. We subtract one for the directory handle `read_dir`
/// itself uses — close enough for trend-watching, which is all we
/// want.
fn read_fd_count() -> Option<usize> {
    let dir = Path::new("/dev/fd");
    match std::fs::read_dir(dir) {
        Ok(iter) => Some(iter.count().saturating_sub(1)),
        Err(e) => {
            warn!(target: "oxplow::diagnostics", error = %e, "could not read /dev/fd");
            None
        }
    }
}

/// How long the async runtime may make no progress before the watchdog
/// says it stalled.
const STALL_AFTER: Duration = Duration::from_secs(20);

/// How many stall samples (`stall-<unix secs>.txt`) to keep.
const KEEP_STALL_SAMPLES: usize = 5;

/// What one watchdog check found.
#[derive(Debug, PartialEq, Eq)]
enum Watch {
    Fine,
    /// Silent past [`STALL_AFTER`]: report it (once).
    Stalled,
    /// Beating again after a reported stall.
    Recovered,
}

/// The watchdog's decision, from how long the runtime has been silent and
/// whether a stall is already reported.
fn watch(silent: Duration, reported: bool) -> Watch {
    match (silent >= STALL_AFTER, reported) {
        (true, false) => Watch::Stalled,
        (false, true) => Watch::Recovered,
        _ => Watch::Fine,
    }
}

/// Watch the async runtime from outside it (tsk1070): a task bumps a
/// heartbeat every second, and a plain OS thread checks it. When the
/// runtime makes no progress for [`STALL_AFTER`] — every worker busy or
/// blocked, as when CPU-bound work ran on them — the thread logs it and,
/// on macOS, saves every thread's stack (`sample`) under `log_dir`, so a
/// hang names what it was doing; it logs the recovery too. Returns at once.
pub fn spawn_watchdog(log_dir: std::path::PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    };
    let beat = Arc::new(AtomicU64::new(now()));
    {
        let beat = beat.clone();
        tokio::spawn(async move {
            loop {
                beat.store(now(), Ordering::Relaxed);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    }
    let spawned = std::thread::Builder::new()
        .name("runtime-watchdog".into())
        .spawn(move || {
            let mut stalled_at: Option<u64> = None;
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let at = now();
                let silent = Duration::from_millis(at.saturating_sub(beat.load(Ordering::Relaxed)));
                match watch(silent, stalled_at.is_some()) {
                    Watch::Fine => {}
                    Watch::Stalled => {
                        stalled_at = Some(at - silent.as_millis() as u64);
                        warn!(
                            target: "oxplow::diagnostics",
                            silent_secs = silent.as_secs(),
                            "the async runtime has made no progress; every worker is busy or blocked"
                        );
                        save_stall_sample(&log_dir);
                    }
                    Watch::Recovered => {
                        let lasted = at.saturating_sub(stalled_at.take().unwrap_or(at)) / 1000;
                        info!(
                            target: "oxplow::diagnostics",
                            lasted_secs = lasted,
                            "the async runtime is making progress again"
                        );
                    }
                }
            }
        });
    if let Err(e) = spawned {
        warn!(target: "oxplow::diagnostics", error = %e, "could not start the runtime watchdog");
    }
}

/// Every thread's stack, sampled for a few seconds, into
/// `log_dir/stall-<unix secs>.txt` (macOS `sample`; elsewhere nothing).
fn save_stall_sample(log_dir: &Path) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let file = log_dir.join(format!("stall-{secs:010}.txt"));
    let _ = std::fs::create_dir_all(log_dir);
    let sampled = std::process::Command::new("/usr/bin/sample")
        .arg(std::process::id().to_string())
        .arg("3")
        .arg("-file")
        .arg(&file)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match sampled {
        Ok(s) if s.success() => {
            warn!(target: "oxplow::diagnostics", file = %file.display(), "saved the stalled threads' stacks");
            prune_stall_samples(log_dir);
        }
        _ => warn!(target: "oxplow::diagnostics", "could not sample the stalled threads"),
    }
}

/// Keep the newest [`KEEP_STALL_SAMPLES`] stall samples in `log_dir`.
fn prune_stall_samples(log_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };
    let mut samples: Vec<std::path::PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("stall-") && n.ends_with(".txt"))
        })
        .collect();
    // Zero-padded seconds: name order is time order.
    samples.sort();
    let excess = samples.len().saturating_sub(KEEP_STALL_SAMPLES);
    for old in &samples[..excess] {
        let _ = std::fs::remove_file(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The watchdog reports a stall once, when the runtime has been
    /// silent past the threshold, and its recovery once.
    #[test]
    fn a_stall_is_reported_once_and_so_is_its_recovery() {
        let past = STALL_AFTER + Duration::from_secs(1);
        assert_eq!(watch(Duration::from_secs(2), false), Watch::Fine);
        assert_eq!(watch(past, false), Watch::Stalled);
        assert_eq!(watch(past + Duration::from_secs(30), true), Watch::Fine);
        assert_eq!(watch(Duration::from_secs(1), true), Watch::Recovered);
    }

    /// Stall samples are kept to the newest few.
    #[test]
    fn old_stall_samples_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        for t in 1..=(KEEP_STALL_SAMPLES as u64 + 3) {
            std::fs::write(dir.path().join(format!("stall-{t:010}.txt")), "x").unwrap();
        }
        std::fs::write(dir.path().join("daemon.2026-10-07.log"), "x").unwrap();
        prune_stall_samples(dir.path());
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left.len(), KEEP_STALL_SAMPLES + 1, "{left:?}");
        assert!(left.contains(&"daemon.2026-10-07.log".to_string()));
        assert!(
            !left.contains(&"stall-0000000001.txt".to_string()),
            "{left:?}"
        );
    }
}
