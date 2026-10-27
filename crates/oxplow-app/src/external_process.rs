//! Owned process groups for approved external programs. This is cleanup,
//! not confinement: a group bounds ordinary child cleanup, not the
//! authority of a process that escapes it. Platforms without groups just
//! run the program.

#[derive(Debug, Clone)]
pub struct ProcessGroup {
    pid: std::sync::Arc<std::sync::Mutex<Option<u32>>>,
}

impl ProcessGroup {
    pub fn prepare(command: &mut std::process::Command) {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(not(unix))]
        let _ = command;
    }

    pub fn new(pid: u32) -> Self {
        live().insert(pid);
        Self {
            pid: std::sync::Arc::new(std::sync::Mutex::new(Some(pid))),
        }
    }
    pub fn stop(&self) {
        if let Some(pid) = self.pid.lock().unwrap_or_else(|e| e.into_inner()).take() {
            // Only while still live: `stop_all` may have stopped it, and
            // its pid may since name another group.
            if live().remove(&pid) {
                Self::stop_pid(pid);
            }
        }
    }

    pub fn stop_pid(pid: u32) {
        #[cfg(unix)]
        if let Ok(pid) = i32::try_from(pid) {
            if pid > 0 {
                // SAFETY: this is the positive pid of the child whose
                // process group the host created, never the host's group.
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
        }
        #[cfg(not(unix))]
        let _ = pid;
    }
}

/// The groups this process started and hasn't stopped.
fn live() -> std::sync::MutexGuard<'static, std::collections::BTreeSet<u32>> {
    static LIVE: std::sync::Mutex<std::collections::BTreeSet<u32>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Stop every group this process started and hasn't stopped: the daemon's
/// last act before `process::exit`, which runs no destructors, so a group
/// a task still owns (a collector or build mid-run, cancelled but not yet
/// dropped) would outlive it.
pub fn stop_all() {
    let pids = std::mem::take(&mut *live());
    for pid in pids {
        ProcessGroup::stop_pid(pid);
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Dropping the connection also stops its live-consent monitor.
pub struct RevocationWatch(tokio::task::JoinHandle<()>);

impl RevocationWatch {
    pub fn start(group: ProcessGroup, gate: crate::run_context::LiveCheck) -> Self {
        Self(tokio::spawn(async move {
            loop {
                let check = gate.clone();
                // Version checks can hash files; keep that I/O off the
                // async runtime's workers. A failed check fails closed.
                if !matches!(
                    tokio::task::spawn_blocking(move || check()).await,
                    Ok(Ok(()))
                ) {
                    group.stop();
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }))
    }
}

impl Drop for RevocationWatch {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    /// A shell that starts a long-lived child of its own, notes its pid in
    /// `pidfile`, and waits for it.
    fn shell_with_child(pidfile: &std::path::Path) -> std::process::Child {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(format!("sleep 300 & echo $! > {}; wait", pidfile.display()))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        ProcessGroup::prepare(&mut command);
        command.spawn().expect("sh starts")
    }

    fn pid_of(pidfile: &std::path::Path) -> i32 {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(pid) = std::fs::read_to_string(pidfile)
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                return pid;
            }
            assert!(
                Instant::now() < deadline,
                "the shell never wrote its child's pid"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only probes whether the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn gone_soon(pid: i32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if !alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn stopping_the_group_reaps_the_program_and_its_children() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let mut shell = shell_with_child(&pidfile);
        let group = ProcessGroup::new(shell.id());
        let grandchild = pid_of(&pidfile);
        assert!(alive(grandchild));

        group.stop();
        let _ = shell.wait();
        assert!(
            gone_soon(grandchild),
            "the child of the program outlived its group"
        );
        // Stopping again is harmless.
        group.stop();
    }

    /// The daemon's exit runs no destructors, so a group still owned by a
    /// task (a collector mid-run) is stopped by `stop_all` instead.
    #[test]
    fn stop_all_reaps_every_live_group() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let mut shell = shell_with_child(&pidfile);
        let group = ProcessGroup::new(shell.id());
        let grandchild = pid_of(&pidfile);

        stop_all();
        let _ = shell.wait();
        assert!(gone_soon(grandchild), "a live group outlived stop_all");
        // Its owner stopping it later is harmless.
        drop(group);
    }

    #[test]
    fn dropping_the_group_and_a_failed_consent_check_also_reap_it() {
        let dir = tempfile::tempdir().unwrap();

        // Drop.
        let pidfile = dir.path().join("dropped");
        let mut shell = shell_with_child(&pidfile);
        let group = ProcessGroup::new(shell.id());
        let grandchild = pid_of(&pidfile);
        drop(group);
        let _ = shell.wait();
        assert!(gone_soon(grandchild));

        // A revoked approval, seen by the watch, stops the whole group.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let pidfile = dir.path().join("revoked");
        let mut shell = shell_with_child(&pidfile);
        let group = ProcessGroup::new(shell.id());
        let grandchild = pid_of(&pidfile);
        let approved = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let gate: crate::run_context::LiveCheck = {
            let approved = approved.clone();
            std::sync::Arc::new(move || {
                if approved.load(std::sync::atomic::Ordering::Acquire) {
                    Ok(())
                } else {
                    Err("revoked".into())
                }
            })
        };
        let watch = runtime.block_on(async { RevocationWatch::start(group, gate) });
        assert!(alive(grandchild), "approved: left running");
        approved.store(false, std::sync::atomic::Ordering::Release);
        assert!(
            gone_soon(grandchild),
            "revocation stops the group within the poll interval"
        );
        let _ = shell.wait();
        drop(watch);
    }
}
