//! Paired devices and pairing codes.
//!
//! A device (a tablet's browser, a phone, another computer) pairs once with a
//! short code the owner shows it, and gets an id and a token from the host.
//! The host keeps only a hash of each token, in `devices.json` in the home, so
//! a device stays paired across host restarts (each app start is one). A token
//! names its device: the host, not the page, says who a request is from.
//!
//! The host is the only writer of the file and keeps it in memory; a file it
//! cannot read stops it at start, so a damaged file is never read as "no
//! devices" (every device would then have to pair again).
//!
//! Codes live in memory: 8 characters with no look-alikes, one use, 10
//! minutes. The pairing route makes a wrong code cost a wait, one attempt at
//! a time, so a code cannot be guessed in its lifetime.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use oxplow_domain::{DeviceId, DeviceKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How long a pairing code is good for.
pub const CODE_LIFETIME: Duration = Duration::from_secs(10 * 60);
/// The characters of a code: no 0/O, 1/I/L, so it reads aloud and types
/// on a tablet without doubt.
const CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const CODE_LEN: usize = 8;

/// A paired device as others see it (never its token or hash).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedDevice {
    pub id: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
    /// Seconds since the Unix epoch.
    pub paired_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    #[serde(flatten)]
    device: PairedDevice,
    token_sha256: String,
}

#[derive(Default, Serialize, Deserialize)]
struct DevicesFile {
    #[serde(default)]
    devices: Vec<Entry>,
}

#[derive(Debug, thiserror::Error)]
pub enum DevicesError {
    #[error("no paired device has the id {0}")]
    Unknown(String),
    #[error("cannot read {0}: {1}; fix or move it, then start the host again")]
    Unreadable(PathBuf, String),
    #[error("the devices file failed: {0}")]
    Io(#[from] std::io::Error),
}

/// The paired devices of one home.
pub struct Devices {
    path: PathBuf,
    entries: Mutex<Vec<Entry>>,
}

fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A token: 256 random bits as hex.
pub fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

impl Devices {
    /// Read the home's devices. A missing file is none; a file that cannot
    /// be read or parsed is an error.
    pub fn open(config_dir: &Path) -> Result<Self, DevicesError> {
        let path = config_dir.join("devices.json");
        let entries = match std::fs::read(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(DevicesError::Unreadable(path, e.to_string())),
            Ok(bytes) => {
                serde_json::from_slice::<DevicesFile>(&bytes)
                    .map_err(|e| DevicesError::Unreadable(path.clone(), e.to_string()))?
                    .devices
            }
        };
        Ok(Self {
            path,
            entries: Mutex::new(entries),
        })
    }

    pub fn list(&self) -> Vec<PairedDevice> {
        self.lock().iter().map(|e| e.device.clone()).collect()
    }

    /// Pair a new device; the host makes its id. Answers the device and its
    /// token, which the device keeps and the host never sees again.
    pub fn pair(
        &self,
        name: &str,
        kind: DeviceKind,
    ) -> Result<(PairedDevice, String), DevicesError> {
        let token = new_token();
        let device = PairedDevice {
            id: DeviceId::generate(),
            name: oxplow_domain::client::device_name(Some(name)),
            kind,
            paired_at: now_secs(),
        };
        let entry = Entry {
            device: device.clone(),
            token_sha256: hash(&token),
        };
        self.change(|entries| {
            entries.push(entry);
            Ok(())
        })?;
        Ok((device, token))
    }

    /// The device `token` belongs to.
    pub fn who(&self, token: &str) -> Option<PairedDevice> {
        let wanted = hash(token);
        self.lock()
            .iter()
            .find(|e| crate::api::same_secret(&e.token_sha256, &wanted))
            .map(|e| e.device.clone())
    }

    pub fn rename(&self, id: &DeviceId, name: &str) -> Result<PairedDevice, DevicesError> {
        self.change(|entries| {
            let entry = entries
                .iter_mut()
                .find(|e| &e.device.id == id)
                .ok_or_else(|| DevicesError::Unknown(id.to_string()))?;
            entry.device.name = oxplow_domain::client::device_name(Some(name));
            Ok(entry.device.clone())
        })
    }

    /// Forget `id`: its token no longer works.
    pub fn remove(&self, id: &DeviceId) -> Result<PairedDevice, DevicesError> {
        self.change(|entries| {
            let at = entries
                .iter()
                .position(|e| &e.device.id == id)
                .ok_or_else(|| DevicesError::Unknown(id.to_string()))?;
            Ok(entries.remove(at).device)
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Change the list and write it; a write that fails changes nothing.
    fn change<T>(
        &self,
        f: impl FnOnce(&mut Vec<Entry>) -> Result<T, DevicesError>,
    ) -> Result<T, DevicesError> {
        let mut entries = self.lock();
        let mut next = entries.clone();
        let out = f(&mut next)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&DevicesFile {
            devices: next.clone(),
        })
        .map_err(std::io::Error::other)?;
        oxplow_config::write_atomic(&self.path, &bytes)?;
        *entries = next;
        Ok(out)
    }
}

/// The codes the owner asked for.
#[derive(Default)]
pub struct Pairing {
    codes: HashMap<String, Instant>,
}

/// `ABCDEFGH` as people read it: `ABCD-EFGH`.
pub fn show_code(code: &str) -> String {
    format!("{}-{}", &code[..CODE_LEN / 2], &code[CODE_LEN / 2..])
}

/// Random bytes for a code: a v4 UUID's, less the two that carry its
/// version and variant bits (fixed, so they would narrow a character).
fn code_bytes() -> [u8; CODE_LEN] {
    let bytes = *uuid::Uuid::new_v4().as_bytes();
    let mut out = [0u8; CODE_LEN];
    for (slot, at) in out.iter_mut().zip([0, 1, 2, 3, 4, 5, 7, 9]) {
        *slot = bytes[at];
    }
    out
}

impl Pairing {
    /// A new code, good until the time answered.
    pub fn new_code(&mut self, now: Instant) -> (String, Instant) {
        self.codes.retain(|_, expires| *expires > now);
        let code: String = code_bytes()
            .iter()
            .map(|b| CODE_ALPHABET[*b as usize % CODE_ALPHABET.len()] as char)
            .collect();
        let expires = now + CODE_LIFETIME;
        self.codes.insert(code.clone(), expires);
        (code, expires)
    }

    /// Use `given` (any case, dashes and spaces ignored): it is good once,
    /// until it expires.
    pub fn take(&mut self, given: &str, now: Instant) -> bool {
        let code: String = given
            .chars()
            .filter(|c| !matches!(c, '-' | ' '))
            .map(|c| c.to_ascii_uppercase())
            .collect();
        matches!(self.codes.remove(&code), Some(expires) if expires > now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paired_device_is_known_by_its_token_until_it_is_removed() {
        let home = tempfile::tempdir().unwrap();
        let devices = Devices::open(home.path()).unwrap();
        let (tablet, token) = devices.pair("Tab S9", DeviceKind::Tablet).unwrap();
        assert_eq!(token.len(), 64);
        assert_eq!(devices.who(&token), Some(tablet.clone()));
        assert_eq!(devices.who("not-a-token"), None);
        let file = std::fs::read_to_string(home.path().join("devices.json")).unwrap();
        assert!(!file.contains(&token), "only a hash is kept");

        let (other, _) = devices.pair("Phone", DeviceKind::Phone).unwrap();
        assert_ne!(other.id, tablet.id, "the host makes each id");

        assert_eq!(
            devices.rename(&tablet.id, "Kitchen tablet").unwrap().name,
            "Kitchen tablet"
        );
        let reopened = Devices::open(home.path()).unwrap();
        assert_eq!(reopened.who(&token).unwrap().name, "Kitchen tablet", "kept");
        devices.remove(&tablet.id).unwrap();
        assert_eq!(devices.who(&token), None);
        assert!(matches!(
            devices.remove(&tablet.id),
            Err(DevicesError::Unknown(_))
        ));
    }

    /// A damaged file is never read as "no devices": every device would
    /// have to pair again, and the next write would lose them.
    #[test]
    fn a_file_that_cannot_be_read_stops_the_host() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("devices.json"), "{ not json").unwrap();
        let err = Devices::open(home.path()).err().unwrap().to_string();
        assert!(err.contains("devices.json"), "{err}");
    }

    #[test]
    fn a_code_is_good_once_in_any_case_and_until_it_expires() {
        let mut pairing = Pairing::default();
        let now = Instant::now();
        let (code, expires) = pairing.new_code(now);
        assert_eq!(code.len(), 8);
        assert!(code.bytes().all(|b| CODE_ALPHABET.contains(&b)), "{code}");
        assert_eq!(expires, now + CODE_LIFETIME);
        assert!(pairing.take(&show_code(&code).to_lowercase(), now));
        assert!(!pairing.take(&code, now), "once");

        let (late, _) = pairing.new_code(now);
        assert!(!pairing.take(&late, now + CODE_LIFETIME + Duration::from_secs(1)));
    }

    /// Every position draws from the whole alphabet (a UUID's version
    /// byte would have given one of them 16 values).
    #[test]
    fn every_character_of_a_code_ranges_over_the_alphabet() {
        let mut seen = vec![std::collections::HashSet::new(); CODE_LEN];
        let mut pairing = Pairing::default();
        for _ in 0..4000 {
            let (code, _) = pairing.new_code(Instant::now());
            for (at, c) in code.chars().enumerate() {
                seen[at].insert(c);
            }
        }
        for (at, chars) in seen.iter().enumerate() {
            assert_eq!(chars.len(), CODE_ALPHABET.len(), "position {at}");
        }
    }
}
