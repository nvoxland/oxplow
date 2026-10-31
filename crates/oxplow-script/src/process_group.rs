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
