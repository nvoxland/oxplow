//! Paired devices and pairing codes.
//!
//! A device (a tablet's browser, a phone, another computer) pairs once with a
//! short code the owner shows it, and gets its own token. The host keeps only
//! a hash of each token, in `devices.json` in the home, so a device stays
//! paired across host restarts (each app start is one). A token names its
//! device: the host, not the page, says who a request is from.
//!
//! Codes live in memory: 8 characters with no look-alikes, one use, 10
//! minutes. Too many wrong codes close pairing for a while, so a code cannot
//! be guessed.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use fs2::FileExt;
use oxplow_domain::{DeviceId, DeviceKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How long a pairing code is good for.
pub const CODE_LIFETIME: Duration = Duration::from_secs(10 * 60);
/// Wrong codes allowed in [`GUESS_WINDOW`] before pairing closes.
pub const GUESSES_ALLOWED: usize = 10;
pub const GUESS_WINDOW: Duration = Duration::from_secs(10 * 60);
/// How long pairing stays closed after too many wrong codes.
pub const CLOSED_FOR: Duration = Duration::from_secs(10 * 60);
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
    #[error("the devices file failed: {0}")]
    Io(#[from] std::io::Error),
}

/// The paired devices of one home.
pub struct Devices {
    path: PathBuf,
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
    pub fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join("devices.json"),
        }
    }

    pub fn list(&self) -> Vec<PairedDevice> {
        self.read().into_iter().map(|e| e.device).collect()
    }

    /// Pair `id` (the device's own id, so what the daemon has seen of it
    /// stays its), and answer its token. A device that pairs again gets a
    /// new token; the old one no longer works.
    pub fn pair(&self, id: DeviceId, name: &str, kind: DeviceKind) -> Result<String, DevicesError> {
        let token = new_token();
        let entry = Entry {
            device: PairedDevice {
                id: id.clone(),
                name: oxplow_domain::client::device_name(Some(name)),
                kind,
                paired_at: now_secs(),
            },
            token_sha256: hash(&token),
        };
        self.change(|entries| {
            entries.retain(|e| e.device.id != id);
            entries.push(entry);
            Ok(())
        })?;
        Ok(token)
    }

    /// The device `token` belongs to.
    pub fn who(&self, token: &str) -> Option<PairedDevice> {
        let wanted = hash(token);
        self.read()
            .into_iter()
            .find(|e| crate::api::same_secret(&e.token_sha256, &wanted))
            .map(|e| e.device)
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

    fn read(&self) -> Vec<Entry> {
        std::fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<DevicesFile>(&bytes).ok())
            .map(|f| f.devices)
            .unwrap_or_default()
    }

    fn change<T>(
        &self,
        f: impl FnOnce(&mut Vec<Entry>) -> Result<T, DevicesError>,
    ) -> Result<T, DevicesError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(oxplow_config::lock_path(&self.path))?;
        lock.lock_exclusive()?;
        let mut devices = self.read();
        let out = f(&mut devices)?;
        let bytes =
            serde_json::to_vec_pretty(&DevicesFile { devices }).map_err(std::io::Error::other)?;
        oxplow_config::write_atomic(&self.path, &bytes)?;
        Ok(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PairError {
    #[error("that code is wrong or has expired; ask for a new one")]
    Wrong,
    #[error("too many wrong codes; pairing is closed for a few minutes")]
    Closed,
}

/// The codes the owner asked for, and the wrong guesses.
#[derive(Default)]
pub struct Pairing {
    codes: HashMap<String, Instant>,
    wrong: VecDeque<Instant>,
    closed_until: Option<Instant>,
}

/// `ABCDEFGH` as people read it: `ABCD-EFGH`.
pub fn show_code(code: &str) -> String {
    format!("{}-{}", &code[..CODE_LEN / 2], &code[CODE_LEN / 2..])
}

impl Pairing {
    /// A new code, good until the time answered.
    pub fn new_code(&mut self, now: Instant) -> (String, Instant) {
        self.codes.retain(|_, expires| *expires > now);
        let bytes = uuid::Uuid::new_v4();
        let code: String = bytes
            .as_bytes()
            .iter()
            .take(CODE_LEN)
            .map(|b| CODE_ALPHABET[*b as usize % CODE_ALPHABET.len()] as char)
            .collect();
        let expires = now + CODE_LIFETIME;
        self.codes.insert(code.clone(), expires);
        (code, expires)
    }

    /// Use `given` (any case, dashes and spaces ignored): it is good once.
    pub fn take(&mut self, given: &str, now: Instant) -> Result<(), PairError> {
        if self.closed_until.is_some_and(|until| until > now) {
            return Err(PairError::Closed);
        }
        self.closed_until = None;
        let code: String = given
            .chars()
            .filter(|c| !matches!(c, '-' | ' '))
            .map(|c| c.to_ascii_uppercase())
            .collect();
        match self.codes.remove(&code) {
            Some(expires) if expires > now => Ok(()),
            _ => {
                while self
                    .wrong
                    .front()
                    .is_some_and(|at| now.duration_since(*at) > GUESS_WINDOW)
                {
                    self.wrong.pop_front();
                }
                self.wrong.push_back(now);
                if self.wrong.len() >= GUESSES_ALLOWED {
                    tracing::warn!("too many wrong pairing codes; pairing closes for a while");
                    self.wrong.clear();
                    self.codes.clear();
                    self.closed_until = Some(now + CLOSED_FOR);
                    return Err(PairError::Closed);
                }
                Err(PairError::Wrong)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tablet() -> DeviceId {
        DeviceId::parse("dev-1b4e28ba-2fa1-11d2-883f-0016d3cca427").unwrap()
    }

    #[test]
    fn a_paired_device_is_known_by_its_token_until_it_is_removed() {
        let home = tempfile::tempdir().unwrap();
        let devices = Devices::new(home.path());
        let token = devices
            .pair(tablet(), "Tab S9", DeviceKind::Tablet)
            .unwrap();
        assert_eq!(token.len(), 64);
        let known = devices.who(&token).unwrap();
        assert_eq!(
            (known.id.clone(), known.name.as_str()),
            (tablet(), "Tab S9")
        );
        assert_eq!(devices.who("not-a-token"), None);
        let file = std::fs::read_to_string(home.path().join("devices.json")).unwrap();
        assert!(!file.contains(&token), "only a hash is kept");

        assert_eq!(
            devices.rename(&tablet(), "Kitchen tablet").unwrap().name,
            "Kitchen tablet"
        );
        assert_eq!(
            Devices::new(home.path()).list()[0].name,
            "Kitchen tablet",
            "kept"
        );
        devices.remove(&tablet()).unwrap();
        assert_eq!(devices.who(&token), None);
        assert!(matches!(
            devices.remove(&tablet()),
            Err(DevicesError::Unknown(_))
        ));
    }

    #[test]
    fn pairing_again_replaces_the_old_token() {
        let home = tempfile::tempdir().unwrap();
        let devices = Devices::new(home.path());
        let first = devices
            .pair(tablet(), "Tab S9", DeviceKind::Tablet)
            .unwrap();
        let second = devices
            .pair(tablet(), "Tab S9", DeviceKind::Tablet)
            .unwrap();
        assert_eq!(devices.who(&first), None);
        assert!(devices.who(&second).is_some());
        assert_eq!(devices.list().len(), 1);
    }

    #[test]
    fn a_code_is_good_once_in_any_case_and_until_it_expires() {
        let mut pairing = Pairing::default();
        let now = Instant::now();
        let (code, expires) = pairing.new_code(now);
        assert_eq!(code.len(), 8);
        assert!(code.bytes().all(|b| CODE_ALPHABET.contains(&b)), "{code}");
        assert_eq!(expires, now + CODE_LIFETIME);
        let typed = show_code(&code).to_lowercase();
        assert_eq!(pairing.take(&typed, now), Ok(()));
        assert_eq!(pairing.take(&code, now), Err(PairError::Wrong), "once");

        let (late, _) = pairing.new_code(now);
        assert_eq!(
            pairing.take(&late, now + CODE_LIFETIME + Duration::from_secs(1)),
            Err(PairError::Wrong)
        );
    }

    #[test]
    fn too_many_wrong_codes_close_pairing_for_a_while() {
        let mut pairing = Pairing::default();
        let now = Instant::now();
        let (good, _) = pairing.new_code(now);
        for _ in 0..GUESSES_ALLOWED - 1 {
            assert_eq!(pairing.take("WRONGCOD", now), Err(PairError::Wrong));
        }
        assert_eq!(pairing.take("WRONGCOD", now), Err(PairError::Closed));
        assert_eq!(
            pairing.take(&good, now),
            Err(PairError::Closed),
            "codes went too"
        );
        let later = now + CLOSED_FOR + Duration::from_secs(1);
        let (fresh, _) = pairing.new_code(later);
        assert_eq!(pairing.take(&fresh, later), Ok(()));
    }

    #[test]
    fn wrong_codes_long_ago_do_not_count() {
        let mut pairing = Pairing::default();
        let now = Instant::now();
        for _ in 0..GUESSES_ALLOWED - 1 {
            let _ = pairing.take("WRONGCOD", now);
        }
        let later = now + GUESS_WINDOW + Duration::from_secs(1);
        assert_eq!(pairing.take("WRONGCOD", later), Err(PairError::Wrong));
    }
}
