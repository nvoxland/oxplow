//! This install's device (`oxplow_domain::DeviceId`): the Mac app is one
//! device for all its windows and projects, so the shell makes its id once
//! and keeps it in `device.json` in the global config dir, beside
//! `session.json`. Each window gets it in its injected context
//! (`window.__OXPLOW__.device`), and the renderer sends it on every request.
//!
//! A missing or damaged file makes a new device: the id names where a person
//! works, and a new one only means that place starts from its defaults.

use std::path::{Path, PathBuf};

use fs2::FileExt;
use oxplow_domain::DeviceId;
use serde::{Deserialize, Serialize};

/// The device this install is, as the shell tells its windows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThisDevice {
    pub id: DeviceId,
    pub name: String,
}

/// This install's device, made (with `name()`) and kept at `json_path` the
/// first time. Two processes starting at once agree: the read and the first
/// write happen under a lock on a `.lock` sidecar (see `atomic.rs`).
pub fn this_device(json_path: &Path, name: impl FnOnce() -> String) -> std::io::Result<ThisDevice> {
    if let Some(parent) = json_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path(json_path))?;
    lock.lock_exclusive()?;
    let kept = std::fs::read(json_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ThisDevice>(&bytes).ok());
    let device = match kept {
        Some(device) => device,
        None => {
            let device = ThisDevice {
                id: DeviceId::generate(),
                name: name(),
            };
            let bytes = serde_json::to_vec_pretty(&device).map_err(std::io::Error::other)?;
            crate::atomic::write_atomic(json_path, &bytes)?;
            device
        }
    };
    drop(lock);
    Ok(device)
}

fn lock_path(json_path: &Path) -> PathBuf {
    let mut name = json_path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    json_path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_is_made_once_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.json");
        let first = this_device(&path, || "Studio".into()).unwrap();
        assert_eq!(first.name, "Studio");
        assert!(first.id.as_str().starts_with("dev-"));
        let again = this_device(&path, || panic!("named again")).unwrap();
        assert_eq!(again, first);
    }

    #[test]
    fn a_damaged_file_makes_a_new_device() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.json");
        std::fs::write(&path, r#"{"id":"dev-not-an-id","name":"x"}"#).unwrap();
        let device = this_device(&path, || "Studio".into()).unwrap();
        assert_ne!(device.id.as_str(), "dev-not-an-id");
        assert_eq!(
            this_device(&path, || panic!("named again")).unwrap(),
            device
        );
    }
}
