//! One host per `OXPLOW_HOME`: an exclusive lock on `host.lock`, and
//! `host.json` (`{ base_url, pid, version, for_app }`) once the host listens. A dev
//! build and an installed build have different homes, so they never meet.

use std::fs::File;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

/// Where a running host can be reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    pub base_url: String,
    pub pid: u32,
    pub version: String,
    /// Started by the Mac app (`serve --for-app`): the app stops it when it
    /// quits. Otherwise headless: it runs until it is stopped.
    #[serde(default)]
    pub for_app: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    #[error("{}", match .0 {
        Some(i) => format!(
            "{} is already running for this home: pid {} at {} (version {}){}",
            if i.for_app { "the Oxplow app's host" } else { "a headless host" },
            i.pid, i.base_url, i.version,
            if i.for_app { "" } else { "; stop it, or set another OXPLOW_HOME" },
        ),
        None => "a host is already running for this home (it has not published host.json yet)".to_string(),
    })]
    AlreadyRunning(Option<HostInfo>),
    #[error("cannot take the host lock: {0}")]
    Io(#[from] std::io::Error),
}

/// The host instance this process is. Dropping it releases the lock and
/// removes `host.json`.
pub struct HostInstance {
    _lock: File,
    info_path: PathBuf,
}

pub fn info_path(config_dir: &Path) -> PathBuf {
    config_dir.join("host.json")
}

/// The published info of the host running for `config_dir`, if any.
pub fn read_info(config_dir: &Path) -> Option<HostInfo> {
    serde_json::from_slice(&std::fs::read(info_path(config_dir)).ok()?).ok()
}

impl HostInstance {
    /// Take the lock, or say which host holds it.
    pub fn acquire(config_dir: &Path) -> Result<Self, InstanceError> {
        std::fs::create_dir_all(config_dir)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(config_dir.join("host.lock"))?;
        match lock.try_lock_exclusive() {
            Ok(()) => Ok(Self {
                _lock: lock,
                info_path: info_path(config_dir),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                Err(InstanceError::AlreadyRunning(read_info(config_dir)))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Take the lock, waiting up to `timeout` while the app's previous host
    /// ends: after a quit it stops its daemons first, and a new app start
    /// (`run:dev`) can come before it is done. A headless host is not
    /// waited for: it runs until someone stops it.
    pub fn acquire_waiting(
        config_dir: &Path,
        timeout: std::time::Duration,
    ) -> Result<Self, InstanceError> {
        let started = std::time::Instant::now();
        loop {
            match Self::acquire(config_dir) {
                Err(InstanceError::AlreadyRunning(info))
                    if info.as_ref().is_none_or(|i| i.for_app) && started.elapsed() < timeout =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                other => return other,
            }
        }
    }

    /// Publish where this host listens. A leftover file from a host that was
    /// killed is replaced.
    pub fn publish(&self, base_url: &str, for_app: bool) -> std::io::Result<()> {
        let info = HostInfo {
            base_url: base_url.to_string(),
            pid: std::process::id(),
            version: crate::VERSION.to_string(),
            for_app,
        };
        let bytes = serde_json::to_vec_pretty(&info).map_err(std::io::Error::other)?;
        oxplow_config::write_atomic(&self.info_path, &bytes)
    }
}

impl Drop for HostInstance {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.info_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_host_waits_for_the_apps_host_that_is_ending() {
        let tmp = tempfile::tempdir().unwrap();
        let first = HostInstance::acquire(tmp.path()).unwrap();
        first.publish("http://127.0.0.1:7400", false).unwrap();
        let asked = std::time::Instant::now();
        let message = HostInstance::acquire_waiting(tmp.path(), std::time::Duration::from_secs(5))
            .err()
            .unwrap()
            .to_string();
        assert!(message.contains("a headless host"), "{message}");
        assert!(message.contains("OXPLOW_HOME"), "{message}");
        assert!(
            asked.elapsed() < std::time::Duration::from_secs(1),
            "a headless host is not waited for"
        );
        // The app's previous host, ending after a quit, is waited for.
        first.publish("http://127.0.0.1:7400", true).unwrap();
        let ending = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(first);
        });
        HostInstance::acquire_waiting(tmp.path(), std::time::Duration::from_secs(5))
            .expect("the lock, once the first host is gone");
        ending.join().unwrap();
    }

    #[test]
    fn a_second_host_for_the_same_home_is_refused_and_names_the_first() {
        let tmp = tempfile::tempdir().unwrap();
        let first = HostInstance::acquire(tmp.path()).unwrap();
        first.publish("http://127.0.0.1:7400", true).unwrap();
        assert!(
            read_info(tmp.path()).unwrap().for_app,
            "the app's own host says so"
        );
        let Err(err) = HostInstance::acquire(tmp.path()) else {
            panic!("a second host got the lock");
        };
        let message = err.to_string();
        assert!(message.contains("already running"), "{message}");
        assert!(message.contains("http://127.0.0.1:7400"), "{message}");
        assert!(
            message.contains(&std::process::id().to_string()),
            "{message}"
        );
    }

    #[test]
    fn the_lock_and_host_json_go_when_the_host_does() {
        let tmp = tempfile::tempdir().unwrap();
        let first = HostInstance::acquire(tmp.path()).unwrap();
        first.publish("http://127.0.0.1:1", false).unwrap();
        assert_eq!(read_info(tmp.path()).unwrap().version, crate::VERSION);
        drop(first);
        assert!(read_info(tmp.path()).is_none());
        HostInstance::acquire(tmp.path()).expect("the lock is free again");
    }

    #[test]
    fn another_home_has_its_own_host() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let _first = HostInstance::acquire(a.path()).unwrap();
        HostInstance::acquire(b.path()).expect("a different home");
    }
}
