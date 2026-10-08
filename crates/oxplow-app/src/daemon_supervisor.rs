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
//! beside it in `.oxplow/daemon.json`, so a shell that didn't spawn it
//! can tell the project already has a backend ([`live_daemon`]).
//!
//! A daemon doesn't outlive its app: the app holds its stdin open, and the
//! daemon stops on end-of-file ([`stop_when_app_goes`]) however the app
//! went. So a live daemon in the file belongs to another app process, or
//! was started by hand, and a second launch defers to it rather than
//! killing it. Reattaching a window to a running daemon is a feature
//! someone may want later; it isn't built on a guess.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::RecvTimeoutError;
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

#[derive(Clone, Copy)]
enum Signal {
    Term,
    Kill,
}

/// Signal the process group a daemon this supervisor started leads
/// (`own_process_group`): the daemon and every process it spawned.
#[cfg(unix)]
fn signal_group(pgid: u32, sig: Signal) {
    let sig = match sig {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    unsafe {
        libc::kill(-(pgid as libc::pid_t), sig);
    }
}

#[cfg(not(unix))]
fn signal_group(_pgid: u32, _sig: Signal) {}

#[cfg(unix)]
fn terminate(pid: u32) {
    signal_tree(pid, libc::SIGTERM);
}

#[cfg(not(unix))]
fn terminate(_pid: u32) {}

/// The daemon's side of the lifeline (tsk1073): a supervising app keeps the
/// daemon's stdin open for as long as it lives, so end-of-file means the
/// app is gone — quit, crashed or killed — and the daemon stops with its
/// agents (its whole group, when it leads one; just itself when started
/// inside someone else's, as a test harness does). Watches on a thread of
/// its own; returns at once.
pub fn stop_when_app_goes(project_dir: PathBuf) {
    std::thread::spawn(move || {
        // `Stdin` is buffered: what follows the token line is read here.
        let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
        tracing::info!("the app that started this daemon is gone (its stdin closed); stopping");
        clear_daemon_info(&project_dir);
        terminate(std::process::id());
    });
}

/// The daemon running for `project_dir`, if its endpoint file names a live
/// one; a stale file (its process gone) is cleared. A live daemon is never
/// killed here: one whose app died has stopped on its own (the stdin
/// lifeline, [`stop_when_app_goes`]), so a live one belongs to another
/// app process or was started by hand. Killing it was how a second launch
/// took the first app's backend down (tsk1063).
pub fn live_daemon(project_dir: &Path) -> Option<DaemonInfo> {
    let info = read_daemon_info(project_dir)?;
    if process_alive(info.pid) {
        Some(info)
    } else {
        clear_daemon_info(project_dir);
        None
    }
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
        cmd.arg("--project")
            .arg(project_dir)
            // Port 0: the OS picks, so two projects can't collide.
            .arg("--bind")
            .arg("127.0.0.1:0")
            // The UI token comes on stdin (see `DaemonSupervisor::start`).
            .arg("--token-stdin");
        cmd
    }
}

/// How a window reaches its daemon: the loopback base URL and the UI
/// token every `/ipc` call and the `/events` socket must present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonEndpoint {
    pub base_url: String,
    pub token: String,
}

/// A fresh random UI token (256 bits from the OS generator).
fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// One running daemon.
struct DaemonHandle {
    base_url: String,
    token: String,
    child: Child,
    /// The daemon's stdin, held open for as long as this app lives: when
    /// the app goes, however it goes, the pipe closes and the daemon stops
    /// (`stop_when_app_goes`, tsk1073).
    _lifeline: Option<std::process::ChildStdin>,
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
    pub fn start(&self, project_dir: &Path) -> std::io::Result<DaemonEndpoint> {
        let key = canonical(project_dir);
        if let Some(existing) = self.lock().get(&key) {
            return Ok(DaemonEndpoint {
                base_url: existing.base_url.clone(),
                token: existing.token.clone(),
            });
        }

        let mut cmd = self.launcher.command(&key);
        // The daemon leads a process group of its own, so `stop` can
        // signal it and everything it spawned as one (`signal_group`).
        own_process_group(&mut cmd);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = cmd.spawn()?;
        // The UI token goes over stdin: argv and the environment are
        // readable by every process of this user, the daemon's agents
        // included. Stdin then stays open as the daemon's lifeline.
        let token = new_token();
        let mut lifeline = child.stdin.take();
        if let Some(stdin) = lifeline.as_mut() {
            use std::io::Write;
            let _ = writeln!(stdin, "{token}");
            let _ = stdin.flush();
        }
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
                        token: token.clone(),
                        child,
                        _lifeline: lifeline,
                        reader: Some(reader),
                    },
                );
                Ok(DaemonEndpoint { base_url, token })
            }
            Err(why) => {
                // `Disconnected` means stdout closed without a handshake:
                // the daemon exited ("already open in another process") or
                // shut its output. That's the signal to go by. A process
                // closes its pipes a moment before it can be reaped, so
                // asking `try_wait` here races (tsk326); give it a short
                // grace to learn which. `Timeout` is a daemon that is simply
                // too slow, which says something very different.
                let message = match why {
                    RecvTimeoutError::Disconnected => {
                        if wait_for_exit(&mut child, Duration::from_secs(2)) {
                            "oxplow-daemon exited before reporting an endpoint".to_string()
                        } else {
                            "oxplow-daemon closed its output before reporting an endpoint"
                                .to_string()
                        }
                    }
                    RecvTimeoutError::Timeout => format!(
                        "oxplow-daemon timed out after {:?} without reporting an endpoint",
                        self.startup_timeout
                    ),
                };
                signal_group(child.id(), Signal::Kill);
                let _ = child.wait();
                clear_daemon_info(&canonical(project_dir));
                Err(std::io::Error::other(message))
            }
        }
    }

    /// The running daemon's base URL for `project_dir`, if any.
    pub fn base_url(&self, project_dir: &Path) -> Option<String> {
        self.lock()
            .get(&canonical(project_dir))
            .map(|h| h.base_url.clone())
    }

    /// The running daemon's endpoint (base URL and UI token), if any.
    pub fn endpoint(&self, project_dir: &Path) -> Option<DaemonEndpoint> {
        self.lock()
            .get(&canonical(project_dir))
            .map(|h| DaemonEndpoint {
                base_url: h.base_url.clone(),
                token: h.token.clone(),
            })
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
            // `start` made the daemon lead its own group, so the group is
            // `pid` — known, not asked of the OS (`getpgid` fails once the
            // leader is a zombie, which would spare its children).
            let pid = handle.child.id();
            signal_group(pid, Signal::Term);
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
            signal_group(pid, Signal::Kill);
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
        let ep = sup.start(tmp.path()).expect("daemon starts");
        assert_eq!(ep.base_url, "http://127.0.0.1:12345");
        assert_eq!(ep.token.len(), 64);
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

    /// Closing stdout is what ends the handshake wait, and it happens a
    /// moment before the process is reaped. The error must come from that
    /// signal, not from racing `try_wait` (tsk326): this daemon closes its
    /// output and lingers, which used to report a 5s timeout instantly.
    #[test]
    fn a_daemon_that_closes_its_output_is_not_reported_as_a_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = supervisor("exec >&-; sleep 30");
        let started = std::time::Instant::now();
        let err = sup.start(tmp.path()).unwrap_err().to_string();
        assert!(!err.contains("timed out"), "got: {err}");
        assert!(err.contains("closed its output"), "got: {err}");
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "no waiting out the startup timeout"
        );
        assert_eq!(sup.running_count(), 0);
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

    /// The daemon dies on SIGTERM but a child of it ignores TERM (and
    /// holds its stdout): once the leader is gone — a zombie, whose group
    /// `getpgid` no longer reports on macOS — the SIGKILL must still reach
    /// the group, or the child lives on and `stop` waits on its output.
    #[test]
    fn stop_kills_a_child_that_outlives_the_daemon() {
        let tmp = tempfile::tempdir().unwrap();
        let pidfile = tmp.path().join("stubborn.pid");
        let script: &'static str = Box::leak(
            format!(
                "(trap '' TERM; exec sleep 30) & echo $! > {}; \
                 echo 'oxplow-daemon listening on http://127.0.0.1:12345'; \
                 wait",
                pidfile.display()
            )
            .into_boxed_str(),
        );
        let sup = DaemonSupervisor::with_launcher(Box::new(FakeDaemon(script)))
            .with_startup_timeout(Duration::from_secs(5))
            .with_shutdown_grace(Duration::from_millis(500));
        sup.start(tmp.path()).unwrap();
        let stubborn: u32 = std::fs::read_to_string(&pidfile)
            .expect("child pid file")
            .trim()
            .parse()
            .unwrap();
        let started = std::time::Instant::now();
        sup.stop(tmp.path());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "stop waited {:?} on a surviving child",
            started.elapsed()
        );
        for _ in 0..40 {
            if !process_alive(stubborn) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            !process_alive(stubborn),
            "the TERM-ignoring child must be killed"
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

    /// A live daemon in the endpoint file is someone's — another app
    /// process's, or one started by hand — so it is reported, never
    /// killed: a second launch used to take the first app's backend
    /// down this way.
    #[test]
    fn a_live_daemon_is_reported_and_left_running() {
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

        let live = live_daemon(tmp.path()).expect("a live daemon is reported");
        assert_eq!(live.pid, child.id());
        assert!(child.try_wait().unwrap().is_none(), "left running");
        assert!(read_daemon_info(tmp.path()).is_some(), "its file stays");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn a_stale_endpoint_file_is_cleared() {
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

        assert!(live_daemon(tmp.path()).is_none(), "nothing was running");
        assert!(
            read_daemon_info(tmp.path()).is_none(),
            "the stale file is cleaned up anyway"
        );
    }

    #[test]
    fn a_project_with_no_endpoint_file_has_no_live_daemon() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(live_daemon(tmp.path()).is_none());
    }

    /// The UI token reaches the daemon on stdin only: never argv, never
    /// the environment (both readable by other processes of the user).
    #[test]
    fn the_ui_token_is_handed_over_on_stdin() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = supervisor(
            "read t; env | grep -q \"$t\" && exit 3; echo \"oxplow-daemon listening on http://127.0.0.1:1/$t\"; sleep 30",
        );
        let ep = sup.start(tmp.path()).expect("daemon starts");
        assert_eq!(ep.base_url, format!("http://127.0.0.1:1/{}", ep.token));
        assert_eq!(sup.endpoint(tmp.path()), Some(ep));
        sup.stop_all();
    }
}
