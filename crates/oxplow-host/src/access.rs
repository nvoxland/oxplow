//! Who a request is from, and which event sockets are open for whom.
//!
//! Two roles. The **owner** holds the token the host was started with (the
//! app's per-run token on stdin, or a headless host's), and is good only on
//! the loopback listener and on the owner socket (`host.sock`, mode 0600, for
//! the CLI). A **device** holds the token it got at pairing, good on every
//! listener. They are roles, not a wall: a paired device can open a terminal
//! on the host's machine.
//!
//! The host, not the page, says which device a request is from: a device's
//! requests carry its record's id, whatever the page sent.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use oxplow_domain::{DeviceId, DeviceKind};
use serde::Serialize;

use crate::devices::{Devices, PairedDevice, Pairing};

/// The close code a page's socket gets when the host ends (the app quit).
pub const HOST_ENDING_CLOSE_CODE: u16 = 4002;
/// The close code a page's socket gets when its device was removed.
pub const DEVICE_REMOVED_CLOSE_CODE: u16 = 4003;

/// The listener a request came in on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listener {
    /// `--bind`: this machine (the Mac app's windows).
    Loopback,
    /// `--public-bind`: other devices.
    Public,
    /// `host.sock`: the CLI, the owner by file permission.
    OwnerSocket,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    Owner,
    Device(PairedDevice),
}

/// The host's secrets and who holds them, shared by every listener.
pub struct Access {
    owner_token: String,
    pub devices: Devices,
    /// One pairing attempt at a time: a wrong code costs a wait.
    pub pairing: tokio::sync::Mutex<Pairing>,
    pub tunnels: Tunnels,
    /// What a wrong code costs, one attempt at a time: a code's 10 minutes
    /// then allow 600 guesses of 31^8.
    pub wrong_code_wait: std::time::Duration,
}

/// What a wrong pairing code costs.
pub const WRONG_CODE_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

impl Access {
    /// `wrong_code_wait` is [`WRONG_CODE_WAIT`] but in a test.
    pub fn new(
        owner_token: String,
        devices: Devices,
        wrong_code_wait: std::time::Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            owner_token,
            devices,
            pairing: tokio::sync::Mutex::new(Pairing::default()),
            tunnels: Tunnels::default(),
            wrong_code_wait,
        })
    }

    /// Who `token`, on `listener`, is.
    pub fn caller(&self, listener: Listener, token: Option<&str>) -> Option<Caller> {
        if listener == Listener::OwnerSocket {
            return Some(Caller::Owner);
        }
        let token = token?;
        if listener == Listener::Loopback && crate::api::same_secret(token, &self.owner_token) {
            return Some(Caller::Owner);
        }
        self.devices.who(token).map(Caller::Device)
    }
}

/// A device with an open event socket now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Connected {
    pub device: String,
    pub name: String,
    pub kind: Option<DeviceKind>,
}

struct Open {
    connected: Connected,
    close: tokio::sync::watch::Sender<Option<u16>>,
}

/// The open event tunnels, by device.
#[derive(Default)]
pub struct Tunnels {
    open: Mutex<HashMap<u64, Open>>,
    next: AtomicU64,
}

/// One open tunnel; dropping it forgets it. `closed` says when the host
/// closes it, and with which code.
pub struct TunnelLease {
    id: u64,
    access: Arc<Access>,
    pub closed: tokio::sync::watch::Receiver<Option<u16>>,
}

impl Drop for TunnelLease {
    fn drop(&mut self) {
        self.access.tunnels.lock().remove(&self.id);
    }
}

impl Tunnels {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Open>> {
        self.open.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every device with an open socket, once each.
    pub fn connected(&self) -> Vec<Connected> {
        let mut seen: Vec<Connected> = Vec::new();
        for open in self.lock().values() {
            if !seen.iter().any(|c| c.device == open.connected.device) {
                seen.push(open.connected.clone());
            }
        }
        seen.sort_by(|a, b| a.name.cmp(&b.name));
        seen
    }

    /// Close `device`'s sockets with `code`.
    pub fn close_device(&self, device: &DeviceId, code: u16) {
        for open in self.lock().values() {
            if open.connected.device == device.as_str() {
                let _ = open.close.send(Some(code));
            }
        }
    }

    /// Close every socket with `code` (the host is ending).
    pub fn close_all(&self, code: u16) {
        for open in self.lock().values() {
            let _ = open.close.send(Some(code));
        }
    }
}

impl Access {
    /// A socket opens for `connected`.
    pub fn open_tunnel(self: &Arc<Self>, connected: Connected) -> TunnelLease {
        let (close, closed) = tokio::sync::watch::channel(None);
        let id = self.tunnels.next.fetch_add(1, Ordering::Relaxed);
        self.tunnels.lock().insert(id, Open { connected, close });
        TunnelLease {
            id,
            access: self.clone(),
            closed,
        }
    }

    /// Forget `device`: its token goes, its holds are released and its
    /// sockets close (the page forgets its token and asks to pair again).
    pub fn remove_device(
        &self,
        host: &crate::host::Host,
        device: &DeviceId,
    ) -> Result<PairedDevice, crate::devices::DevicesError> {
        let removed = self.devices.remove(device)?;
        for hold in host.holds() {
            if hold.device == device.as_str() {
                if let Err(e) = host.release_hold(&hold.device, &hold.project) {
                    tracing::warn!(device = %device, project = %hold.project, error = %e, "could not release a removed device's hold");
                }
            }
        }
        self.tunnels.close_device(device, DEVICE_REMOVED_CLOSE_CODE);
        tracing::info!(device = %device, name = %removed.name, "a device was removed");
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access() -> (tempfile::TempDir, Arc<Access>) {
        let home = tempfile::tempdir().unwrap();
        let devices = Devices::open(home.path()).unwrap();
        (
            home,
            Access::new("owner-secret".into(), devices, WRONG_CODE_WAIT),
        )
    }

    #[test]
    fn the_owner_token_is_good_only_on_loopback_and_a_device_token_everywhere() {
        let (_home, access) = access();
        let (tablet, token) = access.devices.pair("Tab S9", DeviceKind::Tablet).unwrap();
        assert_eq!(
            access.caller(Listener::Loopback, Some("owner-secret")),
            Some(Caller::Owner)
        );
        assert_eq!(access.caller(Listener::Public, Some("owner-secret")), None);
        for listener in [Listener::Loopback, Listener::Public] {
            assert_eq!(
                access.caller(listener, Some(&token)),
                Some(Caller::Device(tablet.clone()))
            );
            assert_eq!(access.caller(listener, None), None);
            assert_eq!(access.caller(listener, Some("nope")), None);
        }
        assert_eq!(
            access.caller(Listener::OwnerSocket, None),
            Some(Caller::Owner),
            "the socket's file mode is the check"
        );
    }

    #[test]
    fn open_sockets_say_who_is_connected_and_close_by_device() {
        let (_home, access) = access();
        let (tablet, _) = access.devices.pair("Tab S9", DeviceKind::Tablet).unwrap();
        let tab = |name: &str, id: &str| Connected {
            device: id.into(),
            name: name.into(),
            kind: Some(DeviceKind::Tablet),
        };
        let first = access.open_tunnel(tab("Tab S9", tablet.id.as_str()));
        let second = access.open_tunnel(tab("Tab S9", tablet.id.as_str()));
        let mac = access.open_tunnel(tab("Mac", "dev-mac"));
        assert_eq!(access.tunnels.connected().len(), 2, "one entry per device");

        access
            .tunnels
            .close_device(&tablet.id, DEVICE_REMOVED_CLOSE_CODE);
        assert_eq!(*first.closed.borrow(), Some(DEVICE_REMOVED_CLOSE_CODE));
        assert_eq!(*second.closed.borrow(), Some(DEVICE_REMOVED_CLOSE_CODE));
        assert_eq!(*mac.closed.borrow(), None);
        drop((first, second));
        assert_eq!(access.tunnels.connected(), vec![tab("Mac", "dev-mac")]);
        access.tunnels.close_all(HOST_ENDING_CLOSE_CODE);
        assert_eq!(*mac.closed.borrow(), Some(HOST_ENDING_CLOSE_CODE));
    }
}
