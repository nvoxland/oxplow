//! The devices connected to this daemon now, as they name themselves on their
//! event socket (`.context/remote-daemon.md` "Who is asking"): a device's
//! name and kind, and its one live page load. Held in memory: a project's
//! database stores only device ids, and once a host stands in front of the
//! daemon its registry holds the names.
//!
//! A device keeps one page load per project ("One active tab", the design of
//! device identity): the first socket of a new page load takes over, the one
//! it replaces is told to close its sockets, and a replaced page load is
//! refused from then on, on its socket and on `/ipc` ([`Replaced`]).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use oxplow_domain::{ConnectionId, DeviceDescriptor, DeviceId};

#[derive(Default)]
pub struct ClientDevices {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    devices: HashMap<DeviceId, Entry>,
    /// Page loads another page load of their device took over: refused for
    /// the rest of the daemon's life.
    replaced: HashSet<ConnectionId>,
    /// Each device's latest page load, kept after its socket closes: a page
    /// load that dropped is still the device's until a newer one connects,
    /// so a newer one replaces it and it cannot take the device back.
    latest: HashMap<DeviceId, ConnectionId>,
    /// The last socket handed out; each socket gets the next.
    sockets: u64,
    /// When each device was last used in this project (the page says so,
    /// at most once a second, while the person works).
    used: HashMap<DeviceId, std::time::Instant>,
}

struct Entry {
    descriptor: DeviceDescriptor,
    /// The device's live page load and its current socket. A page load keeps
    /// its connection across reconnects, so its new socket can open before
    /// the old one is seen to close: the newest socket is the connection's.
    connection: ConnectionId,
    socket: SocketHandle,
}

/// One event socket of a connection, as [`ClientDevices::connected`] hands
/// it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Socket(u64);

/// A socket's handle: which one it is, and whether another page load of its
/// device took over (its socket then closes, saying so).
#[derive(Debug, Clone)]
pub struct SocketHandle {
    id: Socket,
    gone: Arc<Gone>,
}

#[derive(Debug, Default)]
struct Gone {
    replaced: AtomicBool,
    notify: tokio::sync::Notify,
}

impl SocketHandle {
    fn new(id: Socket) -> Self {
        Self {
            id,
            gone: Arc::default(),
        }
    }

    pub fn id(&self) -> Socket {
        self.id
    }

    /// Whether another page load of the device took over.
    pub fn is_replaced(&self) -> bool {
        self.gone.replaced.load(Ordering::SeqCst)
    }

    /// Resolves once another page load of the device takes over.
    pub async fn replaced(&self) {
        while !self.is_replaced() {
            self.gone.notify.notified().await;
        }
    }

    fn replace(&self) {
        self.gone.replaced.store(true, Ordering::SeqCst);
        self.gone.notify.notify_waiters();
        self.gone.notify.notify_one();
    }
}

/// The WebSocket close code that tells a page its page load was replaced:
/// it stops reconnecting and says oxplow is open in another tab
/// (`apps/desktop/src/tauri-bridge/transport.ts` mirrors it).
pub const REPLACED_CLOSE_CODE: u16 = 4001;

/// The page load was replaced by a newer one of its device.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("this window was replaced: oxplow is open in another tab of this device")]
pub struct Replaced;

/// A socket that opened: its handle, and the page load it replaced, if any.
#[derive(Debug)]
pub struct Opened {
    pub socket: SocketHandle,
    pub replaced: Option<ConnectionId>,
}

impl ClientDevices {
    /// `connection` of `descriptor`'s device opened a socket, which is now
    /// the connection's current one; the device's name and kind are the
    /// newest it gave. A new page load of the device replaces the older one;
    /// a replaced page load is refused.
    pub fn connected(
        &self,
        descriptor: DeviceDescriptor,
        connection: ConnectionId,
    ) -> Result<Opened, Replaced> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if inner.replaced.contains(&connection) {
            return Err(Replaced);
        }
        inner.sockets += 1;
        let socket = SocketHandle::new(Socket(inner.sockets));
        let device = descriptor.id.clone();
        let previous = inner.devices.insert(
            device.clone(),
            Entry {
                descriptor,
                connection: connection.clone(),
                socket: socket.clone(),
            },
        );
        // The live socket of the page load replaced is told to go.
        if let Some(old) = previous.filter(|old| old.connection != connection) {
            old.socket.replace();
        }
        let replaced = match inner.latest.insert(device, connection.clone()) {
            Some(old) if old != connection => {
                inner.replaced.insert(old.clone());
                Some(old)
            }
            _ => None,
        };
        Ok(Opened { socket, replaced })
    }

    /// The person used `device` now.
    pub fn used(&self, device: &DeviceId) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.used.insert(device.clone(), std::time::Instant::now());
    }

    /// The connected page load the person used last in this project (its
    /// device and connection): the device with the newest use, else, with
    /// no use told yet (just after a start), the newest connection.
    pub fn last_used(&self) -> Option<(DeviceId, ConnectionId)> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner
            .devices
            .iter()
            .max_by_key(|(id, e)| (inner.used.get(*id).copied(), e.socket.id.0))
            .map(|(id, e)| (id.clone(), e.connection.clone()))
    }

    /// Whether `connection` may still ask: a replaced page load may not.
    pub fn check(&self, connection: &ConnectionId) -> Result<(), Replaced> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if inner.replaced.contains(connection) {
            Err(Replaced)
        } else {
            Ok(())
        }
    }

    /// `socket` of `connection` closed. Only the connection's current socket
    /// ends it (true): a replaced socket's late close changes nothing
    /// (false). The device is then no longer connected.
    pub fn closed(&self, device: &DeviceId, connection: &ConnectionId, socket: Socket) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let current = inner
            .devices
            .get(device)
            .is_some_and(|e| &e.connection == connection && e.socket.id == socket);
        if current {
            inner.devices.remove(device);
        }
        current
    }

    /// A connected device, as it named itself.
    pub fn get(&self, device: &DeviceId) -> Option<DeviceDescriptor> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.devices.get(device).map(|e| e.descriptor.clone())
    }

    /// The connected devices, by id.
    pub fn list(&self) -> Vec<DeviceDescriptor> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let mut out: Vec<_> = inner
            .devices
            .values()
            .map(|e| e.descriptor.clone())
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::DeviceKind;

    fn device(name: &str) -> DeviceDescriptor {
        DeviceDescriptor {
            id: DeviceId::generate(),
            name: name.into(),
            kind: DeviceKind::Tablet,
        }
    }

    #[test]
    fn a_device_is_connected_while_its_page_load_is() {
        let devices = ClientDevices::default();
        let tab = device("Tab S9");
        let connection = ConnectionId::generate();
        let opened = devices.connected(tab.clone(), connection.clone()).unwrap();
        assert_eq!(devices.get(&tab.id), Some(tab.clone()));
        assert!(devices.closed(&tab.id, &connection, opened.socket.id()));
        assert_eq!(devices.get(&tab.id), None);
        assert!(devices.list().is_empty());
    }

    #[test]
    fn a_replaced_sockets_close_leaves_its_connection_connected() {
        let devices = ClientDevices::default();
        let tab = device("Tab S9");
        let connection = ConnectionId::generate();
        let old = devices.connected(tab.clone(), connection.clone()).unwrap();
        let new = devices.connected(tab.clone(), connection.clone()).unwrap();
        assert_eq!(new.replaced, None, "a reconnect is the same page load");
        assert!(!old.socket.is_replaced());
        assert!(
            !devices.closed(&tab.id, &connection, old.socket.id()),
            "a late close is not the connection's"
        );
        assert_eq!(devices.get(&tab.id), Some(tab.clone()));
        assert!(devices.closed(&tab.id, &connection, new.socket.id()));
        assert_eq!(devices.get(&tab.id), None);
    }

    /// A device keeps one page load per project: a new one takes over, the
    /// one it replaces is told (to close its sockets), and a replaced page
    /// load is refused from then on.
    #[test]
    fn a_new_page_load_of_a_device_replaces_the_older() {
        let devices = ClientDevices::default();
        let tab = device("Tab S9");
        let (old, new) = (ConnectionId::generate(), ConnectionId::generate());
        let first = devices.connected(tab.clone(), old.clone()).unwrap();
        assert!(devices.check(&old).is_ok());
        let second = devices.connected(tab.clone(), new.clone()).unwrap();
        assert_eq!(second.replaced, Some(old.clone()));
        assert!(
            first.socket.is_replaced(),
            "the older page load's socket is told to go"
        );
        assert!(
            devices.check(&old).is_err(),
            "a replaced page load is refused"
        );
        assert!(
            devices.connected(tab.clone(), old.clone()).is_err(),
            "and never comes back"
        );
        assert!(devices.check(&new).is_ok());
        // The replaced one's late close changes nothing.
        assert!(!devices.closed(&tab.id, &old, first.socket.id()));
        assert_eq!(devices.get(&tab.id), Some(tab));
    }

    /// A page load whose socket dropped is still the device's: a new page
    /// load that connects meanwhile replaces it, and it cannot come back.
    #[test]
    fn a_page_load_that_dropped_is_replaced_by_a_newer_one() {
        let devices = ClientDevices::default();
        let tab = device("Tab S9");
        let (old, new) = (ConnectionId::generate(), ConnectionId::generate());
        let first = devices.connected(tab.clone(), old.clone()).unwrap();
        assert!(devices.closed(&tab.id, &old, first.socket.id()));
        let second = devices.connected(tab.clone(), new.clone()).unwrap();
        assert_eq!(second.replaced, Some(old.clone()));
        assert!(devices.check(&old).is_err());
        assert!(
            devices.connected(tab.clone(), old).is_err(),
            "the older one can't take it back"
        );
        // The same page load reconnecting is still itself.
        assert!(devices.closed(&tab.id, &new, second.socket.id()));
        assert_eq!(devices.connected(tab, new).unwrap().replaced, None);
    }

    /// The page load used last is the device the person used last; before
    /// any use is told, the newest connection.
    #[test]
    fn the_last_used_page_load_is_the_one_the_person_used_last() {
        let devices = ClientDevices::default();
        let (mac, tab) = (device("Mac"), device("Tab S9"));
        let (on_mac, on_tab) = (ConnectionId::generate(), ConnectionId::generate());
        devices.connected(mac.clone(), on_mac.clone()).unwrap();
        devices.connected(tab.clone(), on_tab.clone()).unwrap();
        assert_eq!(
            devices.last_used(),
            Some((tab.id.clone(), on_tab.clone())),
            "the newest connection"
        );
        devices.used(&mac.id);
        assert_eq!(devices.last_used(), Some((mac.id.clone(), on_mac)));
        std::thread::sleep(std::time::Duration::from_millis(2));
        devices.used(&tab.id);
        assert_eq!(devices.last_used(), Some((tab.id.clone(), on_tab)));
    }

    #[test]
    fn a_device_is_known_by_the_newest_name_it_gave() {
        let devices = ClientDevices::default();
        let mut mac = device("Mac");
        let connection = ConnectionId::generate();
        devices.connected(mac.clone(), connection.clone()).unwrap();
        mac.name = "Studio".into();
        devices.connected(mac.clone(), connection).unwrap();
        assert_eq!(devices.get(&mac.id).unwrap().name, "Studio");
        assert_eq!(devices.list(), vec![mac]);
    }
}
