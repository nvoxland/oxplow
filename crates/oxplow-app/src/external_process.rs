//! Owned process groups for approved broader execution. Restricted
//! confinement is unavailable; a group bounds ordinary child cleanup,
//! not the authority of a process explicitly allowed to escape it.

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
        Self {
            pid: std::sync::Arc::new(std::sync::Mutex::new(Some(pid))),
        }
    }
    pub fn stop(&self) {
        if let Some(pid) = self.pid.lock().unwrap_or_else(|e| e.into_inner()).take() {
            Self::stop_pid(pid);
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

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Dropping the connection also stops its live-consent monitor.
pub struct RevocationWatch(tokio::task::JoinHandle<()>);

impl RevocationWatch {
    pub fn start(group: ProcessGroup, gate: crate::exec_consent::Gate) -> Self {
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
