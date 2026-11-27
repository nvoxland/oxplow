//! Paired devices and pairing codes.
//!
//! A device (a tablet's browser, a phone, another computer) pairs once with a
//! short code the owner shows it, and gets an id and a token from the host.
//! The host keeps only a hash of each token, in `host.db`'s `device` table,
//! so a device stays paired across host restarts (each app start is one). A
//! token names its device: the host, not the page, says who a request is
//! from. A token is found by its hash: what a lookup's timing could tell is
//! about a SHA-256 of the guess, which says nothing of another token.
//!
//! Codes live in memory: 8 characters with no look-alikes, one use, 10
//! minutes. The pairing route makes a wrong code cost a wait, one attempt at
//! a time, so a code cannot be guessed in its lifetime.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxplow_domain::{DeviceId, DeviceKind, Timestamp};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::HostDb;

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

#[derive(Debug, thiserror::Error)]
pub enum DevicesError {
    #[error("no paired device has the id {0}")]
    Unknown(String),
    #[error("the devices store failed: {0}")]
    Store(#[from] rusqlite::Error),
}

/// The paired devices of one home.
pub struct Devices {
    db: Arc<HostDb>,
}

fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// A token: 256 random bits as hex.
pub fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

const COLUMNS: &str = "id, name, kind, paired_at";

fn kind_text(kind: DeviceKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn device_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<PairedDevice> {
    let bad = |at: usize, e: String| {
        rusqlite::Error::FromSqlConversionFailure(at, rusqlite::types::Type::Text, e.into())
    };
    let id: String = row.get(0)?;
    let kind: String = row.get(2)?;
    let paired_at: String = row.get(3)?;
    Ok(PairedDevice {
        id: DeviceId::parse(&id).map_err(|e| bad(0, e.to_string()))?,
        name: row.get(1)?,
        kind: serde_json::from_value(serde_json::Value::String(kind))
            .map_err(|e| bad(2, e.to_string()))?,
        paired_at: Timestamp::parse(&paired_at)
            .map(|t| (t.unix_ms() / 1000).max(0) as u64)
            .map_err(|e| bad(3, e.to_string()))?,
    })
}

impl Devices {
    pub fn new(db: Arc<HostDb>) -> Self {
        Self { db }
    }

    pub fn list(&self) -> Vec<PairedDevice> {
        let conn = self.db.conn();
        let listed = conn
            .prepare(&format!("SELECT {COLUMNS} FROM device ORDER BY rowid"))
            .and_then(|mut q| q.query_map([], device_of)?.collect::<Result<Vec<_>, _>>());
        listed.unwrap_or_else(|e| {
            tracing::error!(error = %e, "cannot list the paired devices");
            Vec::new()
        })
    }

    /// Pair a new device; the host makes its id. Answers the device and its
    /// token, which the device keeps and the host never sees again.
    pub fn pair(
        &self,
        name: &str,
        kind: DeviceKind,
    ) -> Result<(PairedDevice, String), DevicesError> {
        let token = new_token();
        let now = Timestamp::now();
        let device = PairedDevice {
            id: DeviceId::generate(),
            name: oxplow_domain::client::device_name(Some(name)),
            kind,
            paired_at: (now.unix_ms() / 1000).max(0) as u64,
        };
        self.db.conn().execute(
            "INSERT INTO device (id, name, kind, paired_at, token_sha256) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![device.id.to_string(), device.name, kind_text(kind), now.to_text(), hash(&token)],
        )?;
        Ok((device, token))
    }

    /// The device `token` belongs to.
    pub fn who(&self, token: &str) -> Option<PairedDevice> {
        let conn = self.db.conn();
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM device WHERE token_sha256 = ?1"),
            [hash(token)],
            device_of,
        )
        .optional()
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "cannot look a device up");
            None
        })
    }

    pub fn rename(&self, id: &DeviceId, name: &str) -> Result<PairedDevice, DevicesError> {
        let conn = self.db.conn();
        let changed = conn.execute(
            "UPDATE device SET name = ?2 WHERE id = ?1",
            params![
                id.to_string(),
                oxplow_domain::client::device_name(Some(name))
            ],
        )?;
        if changed == 0 {
            return Err(DevicesError::Unknown(id.to_string()));
        }
        Ok(conn.query_row(
            &format!("SELECT {COLUMNS} FROM device WHERE id = ?1"),
            [id.to_string()],
            device_of,
        )?)
    }

    /// Forget `id`: its token no longer works.
    pub fn remove(&self, id: &DeviceId) -> Result<PairedDevice, DevicesError> {
        let conn = self.db.conn();
        let gone = conn
            .query_row(
                &format!("DELETE FROM device WHERE id = ?1 RETURNING {COLUMNS}"),
                [id.to_string()],
                device_of,
            )
            .optional()?;
        gone.ok_or_else(|| DevicesError::Unknown(id.to_string()))
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

    /// The code in force (the last to end), for the owner to show again;
    /// none when every code ended or was used.
    pub fn current(&self, now: Instant) -> Option<(String, Instant)> {
        self.codes
            .iter()
            .filter(|(_, ends)| **ends > now)
            .max_by_key(|(_, ends)| **ends)
            .map(|(code, ends)| (code.clone(), *ends))
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

    /// The code in force is the one that lasts longest; none once every
    /// code ended or was used.
    #[test]
    fn the_code_in_force_is_the_last_to_end() {
        let mut pairing = Pairing::default();
        let t0 = Instant::now();
        assert_eq!(pairing.current(t0), None);
        let (first, _) = pairing.new_code(t0);
        let (second, ends) = pairing.new_code(t0 + std::time::Duration::from_secs(5));
        assert_eq!(
            pairing.current(t0 + std::time::Duration::from_secs(6)),
            Some((second.clone(), ends))
        );
        assert!(pairing.take(&second, t0 + std::time::Duration::from_secs(7)));
        assert_eq!(
            pairing
                .current(t0 + std::time::Duration::from_secs(7))
                .map(|c| c.0),
            Some(first)
        );
        assert_eq!(
            pairing.current(t0 + CODE_LIFETIME + std::time::Duration::from_secs(10)),
            None
        );
    }

    #[test]
    fn a_paired_device_is_known_by_its_token_until_it_is_removed() {
        let home = tempfile::tempdir().unwrap();
        let db = HostDb::open(home.path()).unwrap();
        let devices = Devices::new(db.clone());
        let (tablet, token) = devices.pair("Tab S9", DeviceKind::Tablet).unwrap();
        assert_eq!(token.len(), 64);
        assert_eq!(devices.who(&token), Some(tablet.clone()));
        assert_eq!(devices.who("not-a-token"), None);
        let kept: String = db
            .conn()
            .query_row("SELECT token_sha256 FROM device", [], |r| r.get(0))
            .unwrap();
        assert_ne!(kept, token, "only a hash is kept");

        let (other, _) = devices.pair("Phone", DeviceKind::Phone).unwrap();
        assert_ne!(other.id, tablet.id, "the host makes each id");

        assert_eq!(
            devices.rename(&tablet.id, "Kitchen tablet").unwrap().name,
            "Kitchen tablet"
        );
        assert_eq!(devices.who(&token).unwrap().name, "Kitchen tablet");
        let paired = devices.list();
        assert_eq!(
            paired.iter().map(|d| d.id.clone()).collect::<Vec<_>>(),
            [tablet.id.clone(), other.id.clone()]
        );
        assert_eq!(paired[0].paired_at, tablet.paired_at, "read back as paired");
        drop(devices);
        drop(db);
        let reopened = Devices::new(HostDb::open(home.path()).unwrap());
        assert_eq!(reopened.who(&token).unwrap().name, "Kitchen tablet", "kept");
        let devices = reopened;
        devices.remove(&tablet.id).unwrap();
        assert_eq!(devices.who(&token), None);
        assert!(matches!(
            devices.remove(&tablet.id),
            Err(DevicesError::Unknown(_))
        ));
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
