//! One host per `OXPLOW_HOME`: an exclusive lock on `host.lock`, and
//! `host.json` (`{ base_url, pid, version }`) once the host listens. A dev
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
}

#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    #[error("{}", match .0 {
        Some(i) => format!("a host is already running for this home: pid {} at {} (version {})", i.pid, i.base_url, i.version),
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

    /// Publish where this host listens. A leftover file from a host that was
    /// killed is replaced.
    pub fn publish(&self, base_url: &str) -> std::io::Result<()> {
        let info = HostInfo {
            base_url: base_url.to_string(),
            pid: std::process::id(),
            version: crate::VERSION.to_string(),
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
    fn a_second_host_for_the_same_home_is_refused_and_names_the_first() {
        let tmp = tempfile::tempdir().unwrap();
        let first = HostInstance::acquire(tmp.path()).unwrap();
        first.publish("http://127.0.0.1:7400").unwrap();
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
        first.publish("http://127.0.0.1:1").unwrap();
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
