//! Paired devices, and devices asking to pair.
//!
//! A device (a tablet's browser, a phone, another computer) pairs once: it
//! shows a short code, the owner types it, and it gets an id and a token
//! from the host.
//! The host keeps only a hash of each token, in `host.db`'s `device` table,
//! so a device stays paired across host restarts (each app start is one). A
//! token names its device: the host, not the page, says who a request is
//! from. A token is found by its hash: what a lookup's timing could tell is
//! about a SHA-256 of the guess, which says nothing of another token.
//!
//! A device asks to pair and shows a code; the owner types it, and only
//! then is the device paired ([`PairRequests`], in memory: 8 characters with
//! no look-alikes, 10 minutes).

use std::sync::Arc;
use std::time::{Duration, Instant};

use oxplow_domain::{DeviceId, DeviceKind, Timestamp};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::HostDb;

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

/// How long a device's ask to pair waits for the owner.
pub const ASK_LIFETIME: Duration = Duration::from_secs(10 * 60);
/// How many asks wait at once: a flood of them fills no memory.
pub const MAX_WAITING: usize = 8;

/// A device's ask to pair, as it answers the device: its secret (which
/// collects the key, once approved) and the code it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub request: String,
    /// `ABCD-EFGH`: what the owner types.
    pub code: String,
    pub expires: Instant,
}

/// A device waiting for the owner, as the owner sees it (never its code:
/// typing the code the device shows is the owner's proof it is that one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Waiting {
    pub name: String,
    pub kind: DeviceKind,
}

/// What a device collecting its ask finds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Collect {
    Waiting,
    /// Approved: its record and its token, handed over once.
    Paired(PairedDevice, String),
    /// Ended, collected already, or never asked.
    Gone,
}

#[derive(Debug, thiserror::Error)]
#[error("too many devices are waiting to pair; try again in a few minutes")]
pub struct TooMany;

struct Asked {
    request: String,
    /// Without its dash, upper case.
    code: String,
    name: String,
    kind: DeviceKind,
    expires: Instant,
    paired: Option<(PairedDevice, String)>,
}

/// The devices asking to pair, in memory. A device asks with its name and
/// shows the code it gets; the owner types that code (Devices, or `oxplow
/// host pair <code>`) and only then is it paired, so a code seen by
/// someone else lets nobody in, and nothing a device sends can be guessed
/// right. The device collects its key with its own secret ask.
#[derive(Default)]
pub struct PairRequests {
    asks: Vec<Asked>,
}

/// `given` as a code is kept: dashes and spaces gone, upper case.
fn bare_code(given: &str) -> String {
    given
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// A new code (8 characters, no look-alikes).
fn new_code() -> String {
    code_bytes()
        .iter()
        .map(|b| CODE_ALPHABET[*b as usize % CODE_ALPHABET.len()] as char)
        .collect()
}

impl PairRequests {
    /// `name` (a `kind`) asks to pair.
    pub fn ask(&mut self, name: &str, kind: DeviceKind, now: Instant) -> Result<Ask, TooMany> {
        self.asks.retain(|a| a.expires > now);
        if self.asks.iter().filter(|a| a.paired.is_none()).count() >= MAX_WAITING {
            return Err(TooMany);
        }
        let code = loop {
            let code = new_code();
            if !self.asks.iter().any(|a| a.code == code) {
                break code;
            }
        };
        let request = new_token();
        let expires = now + ASK_LIFETIME;
        self.asks.push(Asked {
            request: request.clone(),
            code: code.clone(),
            name: oxplow_domain::client::device_name(Some(name)),
            kind,
            expires,
            paired: None,
        });
        Ok(Ask {
            request,
            code: show_code(&code),
            expires,
        })
    }

    /// The devices waiting for the owner now.
    pub fn waiting(&self, now: Instant) -> Vec<Waiting> {
        self.asks
            .iter()
            .filter(|a| a.paired.is_none() && a.expires > now)
            .map(|a| Waiting {
                name: a.name.clone(),
                kind: a.kind,
            })
            .collect()
    }

    /// The owner typed `given`: the waiting ask with that code is paired by
    /// `pair` (as it asked to be named). None when no device waits with it.
    pub fn approve<E>(
        &mut self,
        given: &str,
        now: Instant,
        pair: impl FnOnce(&str, DeviceKind) -> Result<(PairedDevice, String), E>,
    ) -> Result<Option<PairedDevice>, E> {
        let code = bare_code(given);
        let Some(ask) = self
            .asks
            .iter_mut()
            .find(|a| a.paired.is_none() && a.expires > now && a.code == code)
        else {
            return Ok(None);
        };
        let (device, token) = pair(&ask.name, ask.kind)?;
        ask.paired = Some((device.clone(), token));
        Ok(Some(device))
    }

    /// The device collects its ask: still waiting, paired (once), or gone.
    pub fn collect(&mut self, request: &str, now: Instant) -> Collect {
        self.asks.retain(|a| a.expires > now);
        let Some(at) = self
            .asks
            .iter()
            .position(|a| crate::api::same_secret(&a.request, request))
        else {
            return Collect::Gone;
        };
        if self.asks[at].paired.is_none() {
            return Collect::Waiting;
        }
        match self.asks.remove(at).paired {
            Some((device, token)) => Collect::Paired(device, token),
            None => Collect::Gone,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn approve_as(
        devices: &Devices,
    ) -> impl FnOnce(&str, DeviceKind) -> Result<(PairedDevice, String), DevicesError> + '_ {
        move |name, kind| devices.pair(name, kind)
    }

    /// A device asks and shows its code; the owner types that code and the
    /// device is paired as it asked to be named. The device collects its key
    /// once, with its own secret ask, never by the code.
    #[test]
    fn a_device_asks_the_owner_approves_its_code_and_it_collects_its_key_once() {
        let home = tempfile::tempdir().unwrap();
        let devices = Devices::new(HostDb::open(home.path()).unwrap());
        let mut asks = PairRequests::default();
        let t0 = Instant::now();
        let ask = asks.ask("Garage tablet", DeviceKind::Tablet, t0).unwrap();
        assert_eq!(ask.code.len(), 9, "{}", ask.code);
        assert_ne!(ask.request, ask.code);
        assert_eq!(ask.expires, t0 + ASK_LIFETIME);
        assert_eq!(asks.collect(&ask.request, t0), Collect::Waiting);
        assert_eq!(
            asks.waiting(t0),
            [Waiting {
                name: "Garage tablet".into(),
                kind: DeviceKind::Tablet
            }]
        );

        assert!(asks
            .approve("WRON-GCOD", t0, approve_as(&devices))
            .unwrap()
            .is_none());
        // Any case, the dash or none.
        let given = ask.code.replace('-', "").to_lowercase();
        let paired = asks
            .approve(&given, t0, approve_as(&devices))
            .unwrap()
            .unwrap();
        assert_eq!(paired.name, "Garage tablet");
        assert!(asks.waiting(t0).is_empty(), "approved: no longer waiting");
        assert!(
            asks.approve(&given, t0, approve_as(&devices))
                .unwrap()
                .is_none(),
            "once"
        );

        let Collect::Paired(device, token) = asks.collect(&ask.request, t0) else {
            panic!("not paired");
        };
        assert_eq!(device, paired);
        assert_eq!(devices.who(&token), Some(paired));
        assert_eq!(
            asks.collect(&ask.request, t0),
            Collect::Gone,
            "the key is handed over once"
        );
        assert_eq!(
            asks.collect(&ask.code, t0),
            Collect::Gone,
            "a code collects nothing"
        );
    }

    /// An ask lasts ten minutes, and only a few wait at once.
    #[test]
    fn an_ask_ends_and_only_a_few_wait_at_once() {
        let home = tempfile::tempdir().unwrap();
        let devices = Devices::new(HostDb::open(home.path()).unwrap());
        let mut asks = PairRequests::default();
        let t0 = Instant::now();
        let late = asks.ask("Phone", DeviceKind::Phone, t0).unwrap();
        let after = t0 + ASK_LIFETIME + Duration::from_secs(1);
        assert!(asks
            .approve(&late.code, after, approve_as(&devices))
            .unwrap()
            .is_none());
        assert_eq!(asks.collect(&late.request, after), Collect::Gone);
        assert!(devices.list().is_empty());

        for i in 0..MAX_WAITING {
            asks.ask(&format!("Tab {i}"), DeviceKind::Tablet, t0)
                .unwrap();
        }
        assert!(asks.ask("One more", DeviceKind::Tablet, t0).is_err());
        assert!(
            asks.ask("Later", DeviceKind::Tablet, after).is_ok(),
            "the old ones ended"
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
    fn a_code_is_eight_characters_with_no_look_alikes() {
        let code = new_code();
        assert_eq!(code.len(), 8);
        assert!(code.bytes().all(|b| CODE_ALPHABET.contains(&b)), "{code}");
        assert_eq!(bare_code(&show_code(&code).to_lowercase()), code);
    }

    /// Every position draws from the whole alphabet (a UUID's version
    /// byte would have given one of them 16 values).
    #[test]
    fn every_character_of_a_code_ranges_over_the_alphabet() {
        let mut seen = vec![std::collections::HashSet::new(); CODE_LEN];
        for _ in 0..4000 {
            let code = new_code();
            for (at, c) in code.chars().enumerate() {
                seen[at].insert(c);
            }
        }
        for (at, chars) in seen.iter().enumerate() {
            assert_eq!(chars.len(), CODE_ALPHABET.len(), "position {at}");
        }
    }
}
