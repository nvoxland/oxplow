//! Supervising per-project `oxplow-daemon` processes (tsk256).
//!
//! In the daemon-backed shell one Tauri process owns every window, and
//! each project's backend runs as its own `oxplow-daemon` child. This
//! module is the shell's side of that: start a daemon for a project,
//! learn the loopback endpoint it bound, and make sure it dies when the
//! window does.
//!
//! **The endpoint comes from the daemon's own stdout.** It binds
//! `127.0.0.1:0` (an ephemeral port — no port-picking races between
//! projects) and prints `oxplow-daemon listening on http://ADDR`, which
//! [`parse_listening_line`] reads. A [`DaemonInfo`] file is written
//! beside it in `.oxplow/daemon.json` for the *other* discovery
//! problem: a daemon that outlived the shell that spawned it, which the
//! next boot sweeps with [`kill_orphan_daemon`]. Same shape as the
//! `instance.json` focus channel next door in `lib.rs`.
//!
//! Orphans are killed rather than adopted. A daemon outliving its window
//! (agents keep running while the UI is closed) is a feature someone may
//! want later; it is not what today's process-per-window model does, so
//! reattach isn't built on a guess.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How long to wait for a freshly spawned daemon to report its endpoint.
/// Generous on purpose: the daemon runs the full boot orchestration
/// (recovery, watchers, indexers) before it binds, which is seconds on a
/// debug build over a large project.
const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a daemon gets to exit on SIGTERM before it is killed.
const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// The endpoint a running daemon publishes for its project, so a shell
/// that didn't spawn it can still find it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonInfo {
    /// Loopback HTTP base, e.g. `http://127.0.0.1:60331`.
    pub base_url: String,
    /// OS process id, for liveness checks and the orphan sweep.
    pub pid: u32,
}

fn daemon_info_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".oxplow").join("daemon.json")
}

/// Publish a daemon's endpoint for `project_dir`. Called by the daemon
/// itself once it has bound.
pub fn write_daemon_info(project_dir: &Path, info: &DaemonInfo) -> std::io::Result<()> {
    let path = daemon_info_path(project_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(info)?)
}

/// Read the published endpoint for `project_dir`, if any. A present file
/// proves nothing about liveness — check the pid.
pub fn read_daemon_info(project_dir: &Path) -> Option<DaemonInfo> {
    let bytes = std::fs::read(daemon_info_path(project_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Remove the published endpoint (daemon stopped, or the file is stale).
pub fn clear_daemon_info(project_dir: &Path) {
    let _ = std::fs::remove_file(daemon_info_path(project_dir));
}

/// The endpoint out of the daemon's startup line, or `None` for any
/// other output. Deliberately anchored on the whole prefix so the
/// following `tunnel: ssh -L …` hint — which also contains an address —
/// can't be mistaken for it.
pub fn parse_listening_line(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("oxplow-daemon listening on ")?;
    let url = rest.trim();
    url.starts_with("http://").then(|| url.to_string())
}

/// Put `cmd`'s child in its own process group so the supervisor can
/// signal the daemon **and everything it spawned** (agent PTYs, LSP
/// servers, scan helpers) as a unit. Without this, killing the daemon
/// orphans its children — the exact failure this epic exists to remove.
#[cfg(unix)]
pub(crate) fn own_process_group(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

#[cfg(not(unix))]
pub(crate) fn own_process_group(_cmd: &mut Command) {}

/// Signal the whole process group led by `pid`, falling back to the
/// single process when `pid` doesn't lead its own group (a daemon
/// someone started by hand from a shell — signalling *that* group would
/// hit the user's terminal).
#[cfg(unix)]
fn signal_tree(pid: u32, sig: libc::c_int) {
    let leads_group = unsafe { libc::getpgid(pid as libc::pid_t) } == pid as libc::pid_t;
    unsafe {
        if leads_group {
            libc::kill(-(pid as libc::pid_t), sig);
        } else {
            libc::kill(pid as libc::pid_t, sig);
        }
    }
}

#[cfg(not(unix))]
fn signal_tree(_pid: u32, _sig: i32) {}

/// True when `pid` names a live process.
#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // Signal 0 performs the permission/existence checks without sending
    // anything — the standard liveness probe.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
fn terminate(pid: u32) {
    signal_tree(pid, libc::SIGTERM);
}

#[cfg(unix)]
fn hard_kill(pid: u32) {
    signal_tree(pid, libc::SIGKILL);
}

#[cfg(not(unix))]
fn terminate(_pid: u32) {}

#[cfg(not(unix))]
fn hard_kill(_pid: u32) {}

/// Kill a daemon left running for `project_dir` by a previous shell.
/// Returns whether one was actually running. The endpoint file is
/// cleared either way, so a stale file doesn't outlive its process.
pub fn kill_orphan_daemon(project_dir: &Path) -> bool {
    let Some(info) = read_daemon_info(project_dir) else {
        return false;
    };
    let alive = process_alive(info.pid);
    if alive {
        tracing::info!(pid = info.pid, project = %project_dir.display(), "killing orphaned daemon");
        terminate(info.pid);
    }
    clear_daemon_info(project_dir);
    alive
}

/// How a daemon process gets started. The real implementation resolves
/// the bundled binary; tests substitute a script.
pub trait DaemonLauncher: Send + Sync {
    /// A command that, when spawned, boots a daemon for `project_dir`
    /// and prints its listening line to stdout.
    fn command(&self, project_dir: &Path) -> Command;
}

/// Launches the `oxplow-daemon` shipped beside the running executable
/// (`bundle.externalBin` puts it there), falling back to a sibling of
/// the current binary in a dev target dir.
pub struct BundledDaemon;

impl BundledDaemon {
    /// Path to the daemon binary: next to the current executable, which
    /// covers both the packaged bundle and `target/debug`.
    pub fn binary_path() -> PathBuf {
        let name = if cfg!(windows) {
            "oxplow-daemon.exe"
        } else {
            "oxplow-daemon"
        };
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
            .unwrap_or_else(|| PathBuf::from(name))
    }
}

impl DaemonLauncher for BundledDaemon {
    fn command(&self, project_dir: &Path) -> Command {
        let mut cmd = Command::new(Self::binary_path());
        own_process_group(&mut cmd);
        cmd.arg("--project")
            .arg(project_dir)
            // Port 0: the OS picks, so two projects can't collide.
            .arg("--bind")
            .arg("127.0.0.1:0");
        cmd
    }
}

/// One running daemon.
struct DaemonHandle {
    base_url: String,
    child: Child,
    /// Drains the daemon's stdout for the life of the process. Joined on
    /// stop so no reader outlives the child that fed it.
    reader: Option<std::thread::JoinHandle<()>>,
}

/// The shell's registry of running daemons, one per project.
pub struct DaemonSupervisor {
    launcher: Box<dyn DaemonLauncher>,
    startup_timeout: Duration,
    shutdown_grace: Duration,
    running: Mutex<HashMap<PathBuf, DaemonHandle>>,
}

impl Default for DaemonSupervisor {
    fn default() -> Self {
        Self::with_launcher(Box::new(BundledDaemon))
    }
}

impl DaemonSupervisor {
    pub fn with_launcher(launcher: Box<dyn DaemonLauncher>) -> Self {
        Self {
            launcher,
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
            shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
            running: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    pub fn with_shutdown_grace(mut self, grace: Duration) -> Self {
        self.shutdown_grace = grace;
        self
    }

    /// Start (or reuse) the daemon for `project_dir` and return its
    /// loopback base URL.
    ///
    /// Blocks until the daemon reports its endpoint, so the caller can
    /// hand the URL straight to a window. A daemon that exits first, or
    /// never reports, is an error and leaves nothing registered.
    pub fn start(&self, project_dir: &Path) -> std::io::Result<String> {
        let key = canonical(project_dir);
        if let Some(existing) = self.lock().get(&key) {
            return Ok(existing.base_url.clone());
        }

        let mut cmd = self.launcher.command(&key);
        cmd.stdout(Stdio::piped()).stderr(Stdio::inherit());
        let mut child = cmd.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("daemon stdout was not captured".to_string()))?;

        // Read the handshake on a worker so the wait can time out, then
        // keep draining: an unread pipe eventually blocks the daemon's
        // own writes.
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let reader = std::thread::spawn(move || {
            let mut announced = false;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if !announced {
                    if let Some(base) = parse_listening_line(&line) {
                        announced = true;
                        let _ = tx.send(base);
                        continue;
                    }
                }
                tracing::debug!(target: "oxplow_daemon", "{line}");
            }
        });

        match rx.recv_timeout(self.startup_timeout) {
            Ok(base_url) => {
                self.lock().insert(
                    key,
                    DaemonHandle {
                        base_url: base_url.clone(),
                        child,
                        reader: Some(reader),
                    },
                );
                Ok(base_url)
            }
            Err(_) => {
                // Either the daemon exited (the sender dropped with the
                // stdout EOF) or it is simply too slow. Distinguish them
                // for the caller — "already open in another process" is
                // an exit, and says something very different from a hang.
                let exited = matches!(child.try_wait(), Ok(Some(_)));
                hard_kill(child.id());
                let _ = child.kill();
                let _ = child.wait();
                clear_daemon_info(&canonical(project_dir));
                Err(std::io::Error::other(if exited {
                    "oxplow-daemon exited before reporting an endpoint".to_string()
                } else {
                    format!(
                        "oxplow-daemon timed out after {:?} without reporting an endpoint",
                        self.startup_timeout
                    )
                }))
            }
        }
    }

    /// The running daemon's base URL for `project_dir`, if any.
    pub fn base_url(&self, project_dir: &Path) -> Option<String> {
        self.lock()
            .get(&canonical(project_dir))
            .map(|h| h.base_url.clone())
    }

    /// Number of daemons this supervisor is running.
    pub fn running_count(&self) -> usize {
        self.lock().len()
    }

    /// Stop the daemon for `project_dir` (window closed).
    ///
    /// SIGTERM first, then a grace period, then SIGKILL: the daemon can
    /// be mid-write to SQLite, and a hard kill there is how you get a
    /// hot journal. The fallback exists so a wedged child can never hold
    /// the app open.
    pub fn stop(&self, project_dir: &Path) {
        let key = canonical(project_dir);
        if let Some(mut handle) = self.lock().remove(&key) {
            let pid = handle.child.id();
            terminate(pid);
            if !wait_for_exit(&mut handle.child, self.shutdown_grace) {
                tracing::warn!(project = %key.display(), "daemon ignored SIGTERM; killing");
            }
            // SIGKILL any group member still standing — including when
            // the leader exited cleanly but a child ignored the TERM.
            //
            // This MUST happen before `wait()`: the leader's pid stays
            // reserved while it is a zombie, and once reaped the OS may
            // recycle it onto an unrelated process whose group we would
            // then be signalling.
            hard_kill(pid);
            let _ = handle.child.wait();
            if let Some(reader) = handle.reader.take() {
                let _ = reader.join();
            }
        }
        clear_daemon_info(&key);
    }

    /// Stop every daemon (shell exiting).
    pub fn stop_all(&self) {
        let keys: Vec<PathBuf> = self.lock().keys().cloned().collect();
        for key in keys {
            self.stop(&key);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, DaemonHandle>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for DaemonSupervisor {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// Poll `child` until it exits or `grace` elapses. `true` when it exited
/// on its own.
fn wait_for_exit(child: &mut Child, grace: Duration) -> bool {
    let deadline = std::time::Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(_) => return false,
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Canonicalize so the same project reached by different paths (symlinks,
/// `/tmp` vs `/private/tmp`) is one registry entry.
fn canonical(project_dir: &Path) -> PathBuf {
    std::fs::canonicalize(project_dir).unwrap_or_else(|_| project_dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A launcher that runs `/bin/sh -c <script>` instead of the real
    /// daemon, so the supervisor's spawn/handshake/reap mechanics are
    /// tested without booting a backend.
    struct FakeDaemon(&'static str);

    impl DaemonLauncher for FakeDaemon {
        fn command(&self, _project_dir: &std::path::Path) -> std::process::Command {
            let mut cmd = std::process::Command::new("/bin/sh");
            super::own_process_group(&mut cmd);
            cmd.arg("-c").arg(self.0);
            cmd
        }
    }

    fn supervisor(script: &'static str) -> DaemonSupervisor {
        DaemonSupervisor::with_launcher(Box::new(FakeDaemon(script)))
            .with_startup_timeout(Duration::from_secs(5))
    }

    #[test]
    fn parses_the_endpoint_the_daemon_prints() {
        assert_eq!(
            parse_listening_line("oxplow-daemon listening on http://127.0.0.1:60331"),
            Some("http://127.0.0.1:60331".to_string())
        );
        // The daemon's second line (the ssh tunnel hint) must not match.
        assert_eq!(
            parse_listening_line("  tunnel: ssh -L 60331:127.0.0.1:60331 <host>"),
            None
        );
        assert_eq!(parse_listening_line("some other log line"), None);
    }

    #[test]
    fn start_returns_the_endpoint_and_registers_the_daemon() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = supervisor("echo 'oxplow-daemon listening on http://127.0.0.1:12345'; sleep 30");
        let base = sup.start(tmp.path()).expect("daemon starts");
        assert_eq!(base, "http://127.0.0.1:12345");
        assert_eq!(
            sup.base_url(tmp.path()).as_deref(),
            Some("http://127.0.0.1:12345")
        );
        sup.stop_all();
    }

    /// Starting the same project twice reuses the running daemon rather
    /// than spawning a second one — the project instance lock would
    /// reject it anyway, and the caller wants the endpoint either way.
    #[test]
    fn start_is_idempotent_per_project() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = supervisor("echo 'oxplow-daemon listening on http://127.0.0.1:12345'; sleep 30");
        let first = sup.start(tmp.path()).unwrap();
        let second = sup.start(tmp.path()).unwrap();
        assert_eq!(first, second);
        assert_eq!(sup.running_count(), 1);
        sup.stop_all();
    }

    #[test]
    fn start_fails_when_the_daemon_exits_without_an_endpoint() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = supervisor("echo 'oxplow-daemon: project already open' >&2; exit 1");
        let err = sup.start(tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("exited"),
            "error should say the daemon exited, got: {err}"
        );
        assert_eq!(sup.running_count(), 0, "a failed start registers nothing");
    }

    #[test]
    fn start_times_out_when_the_daemon_never_reports() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = DaemonSupervisor::with_launcher(Box::new(FakeDaemon("sleep 30")))
            .with_startup_timeout(Duration::from_millis(300));
        let err = sup.start(tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("timed out"),
            "error should say it timed out, got: {err}"
        );
        // The child must not be left running after a timeout.
        assert_eq!(sup.running_count(), 0);
    }

    #[test]
    fn stop_kills_the_child_and_forgets_it() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = supervisor("echo 'oxplow-daemon listening on http://127.0.0.1:12345'; sleep 30");
        sup.start(tmp.path()).unwrap();
        sup.stop(tmp.path());
        assert_eq!(sup.running_count(), 0);
        assert!(sup.base_url(tmp.path()).is_none());
    }

    /// SIGTERM has to reach the daemon with time to act on it — a hard
    /// kill mid-write is how SQLite ends up with a hot journal. The fake
    /// traps the signal and leaves a marker file behind.
    #[test]
    fn stop_gives_the_daemon_a_chance_to_shut_down_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join("caught-sigterm");
        let script: &'static str = Box::leak(
            format!(
                "trap 'touch {}; exit 0' TERM; \
                 echo 'oxplow-daemon listening on http://127.0.0.1:12345'; \
                 while true; do sleep 0.05; done",
                marker.display()
            )
            .into_boxed_str(),
        );
        let sup = DaemonSupervisor::with_launcher(Box::new(FakeDaemon(script)))
            .with_startup_timeout(Duration::from_secs(5))
            .with_shutdown_grace(Duration::from_secs(3));
        sup.start(tmp.path()).unwrap();
        sup.stop(tmp.path());
        assert!(
            marker.exists(),
            "the daemon should have received SIGTERM and run its handler"
        );
        assert_eq!(sup.running_count(), 0);
    }

    /// Stopping a daemon must take its CHILDREN with it. The real daemon
    /// spawns agent PTYs, LSP servers and scan helpers; killing only the
    /// daemon pid would orphan every one of them — the exact leak this
    /// epic exists to remove. (nextest's "leaky" flag caught this.)
    #[test]
    fn stop_takes_the_daemons_children_with_it() {
        let tmp = tempfile::tempdir().unwrap();
        let pidfile = tmp.path().join("grandchild.pid");
        let script: &'static str = Box::leak(
            format!(
                "sleep 30 & echo $! > {}; \\
                 echo 'oxplow-daemon listening on http://127.0.0.1:12345'; \\
                 wait",
                pidfile.display()
            )
            .into_boxed_str(),
        );
        let sup = DaemonSupervisor::with_launcher(Box::new(FakeDaemon(script)))
            .with_startup_timeout(Duration::from_secs(5))
            .with_shutdown_grace(Duration::from_millis(500));
        sup.start(tmp.path()).unwrap();

        // The grandchild pid is written before the listening line, so it
        // is on disk by the time start() returns.
        let grandchild: u32 = std::fs::read_to_string(&pidfile)
            .expect("grandchild pid file")
            .trim()
            .parse()
            .unwrap();
        assert!(process_alive(grandchild), "grandchild should be running");

        sup.stop(tmp.path());
        // Give the signal a moment to land.
        for _ in 0..40 {
            if !process_alive(grandchild) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            !process_alive(grandchild),
            "stopping the daemon must kill its children too"
        );
    }

    /// The endpoint file is how a *later* shell finds a daemon this one
    /// left behind; stopping cleans it up so the next boot doesn't chase
    /// a dead pid.
    #[test]
    fn daemon_info_round_trips_and_clears() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".oxplow")).unwrap();
        let info = DaemonInfo {
            base_url: "http://127.0.0.1:7777".into(),
            pid: 4242,
        };
        write_daemon_info(tmp.path(), &info).unwrap();
        let read = read_daemon_info(tmp.path()).expect("info reads back");
        assert_eq!(read.base_url, info.base_url);
        assert_eq!(read.pid, info.pid);
        clear_daemon_info(tmp.path());
        assert!(read_daemon_info(tmp.path()).is_none());
    }

    #[test]
    fn orphan_sweep_kills_a_live_daemon_and_clears_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".oxplow")).unwrap();
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("sleep 30")
            .spawn()
            .unwrap();
        write_daemon_info(
            tmp.path(),
            &DaemonInfo {
                base_url: "http://127.0.0.1:7777".into(),
                pid: child.id(),
            },
        )
        .unwrap();

        assert!(kill_orphan_daemon(tmp.path()), "a live orphan is killed");
        // The child is gone (wait() returns rather than hanging).
        let status = child.wait().unwrap();
        assert!(!status.success(), "killed process should not exit cleanly");
        assert!(read_daemon_info(tmp.path()).is_none(), "file cleaned up");
    }

    #[test]
    fn orphan_sweep_is_a_no_op_for_a_stale_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".oxplow")).unwrap();
        // A pid that has certainly exited: spawn and reap one.
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .unwrap();
        let dead_pid = child.id();
        child.wait().unwrap();
        write_daemon_info(
            tmp.path(),
            &DaemonInfo {
                base_url: "http://127.0.0.1:7777".into(),
                pid: dead_pid,
            },
        )
        .unwrap();

        assert!(!kill_orphan_daemon(tmp.path()), "nothing was running");
        assert!(
            read_daemon_info(tmp.path()).is_none(),
            "the stale file is cleaned up anyway"
        );
    }

    #[test]
    fn orphan_sweep_tolerates_a_project_with_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!kill_orphan_daemon(tmp.path()));
    }
}
