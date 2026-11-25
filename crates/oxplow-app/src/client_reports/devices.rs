//! The devices connected to this daemon now, as they name themselves on their
//! event socket (`.context/remote-daemon.md` "Who is asking"): a device's
//! name and kind, and its live connections. Held in memory: a project's
//! database stores only device ids, and once a host stands in front of the
//! daemon its registry holds the names.

use std::collections::HashMap;
use std::sync::Mutex;

use oxplow_domain::{ConnectionId, DeviceDescriptor, DeviceId};

#[derive(Default)]
pub struct ClientDevices {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    devices: HashMap<DeviceId, Entry>,
    /// The last socket handed out; each socket gets the next.
    sockets: u64,
}

struct Entry {
    descriptor: DeviceDescriptor,
    /// Each live connection's current socket. A page load keeps its
    /// connection across reconnects, so its new socket can open before the
    /// old one is seen to close: the newest socket is the connection's.
    connections: HashMap<ConnectionId, Socket>,
}

/// One event socket of a connection, as [`ClientDevices::connected`] hands
/// it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Socket(u64);

impl ClientDevices {
    /// `connection` of `descriptor`'s device opened a socket, which is now
    /// the connection's current one; the device's name and kind are the
    /// newest it gave.
    pub fn connected(&self, descriptor: DeviceDescriptor, connection: ConnectionId) -> Socket {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.sockets += 1;
        let socket = Socket(inner.sockets);
        let entry = inner
            .devices
            .entry(descriptor.id.clone())
            .or_insert_with(|| Entry {
                descriptor: descriptor.clone(),
                connections: HashMap::new(),
            });
        entry.descriptor = descriptor;
        entry.connections.insert(connection, socket);
        socket
    }

    /// `socket` of `connection` closed. Only the connection's current socket
    /// ends it (true): a replaced socket's late close changes nothing
    /// (false). A device with no connection left is no longer connected.
    pub fn closed(&self, device: &DeviceId, connection: &ConnectionId, socket: Socket) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let Some(entry) = inner.devices.get_mut(device) else {
            return false;
        };
        if entry.connections.get(connection) != Some(&socket) {
            return false;
        }
        entry.connections.remove(connection);
        if entry.connections.is_empty() {
            inner.devices.remove(device);
        }
        true
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
    fn a_device_is_connected_while_any_of_its_connections_is() {
        let devices = ClientDevices::default();
        let tab = device("Tab S9");
        let (first, second) = (ConnectionId::generate(), ConnectionId::generate());
        let one = devices.connected(tab.clone(), first.clone());
        let two = devices.connected(tab.clone(), second.clone());
        assert!(devices.closed(&tab.id, &first, one));
        assert_eq!(devices.get(&tab.id), Some(tab.clone()));
        assert!(devices.closed(&tab.id, &second, two));
        assert_eq!(devices.get(&tab.id), None);
        assert!(devices.list().is_empty());
    }

    #[test]
    fn a_replaced_sockets_close_leaves_its_connection_connected() {
        let devices = ClientDevices::default();
        let tab = device("Tab S9");
        let connection = ConnectionId::generate();
        let old = devices.connected(tab.clone(), connection.clone());
        let new = devices.connected(tab.clone(), connection.clone());
        assert!(
            !devices.closed(&tab.id, &connection, old),
            "a late close is not the connection's"
        );
        assert_eq!(devices.get(&tab.id), Some(tab.clone()));
        assert!(devices.closed(&tab.id, &connection, new));
        assert_eq!(devices.get(&tab.id), None);
    }

    #[test]
    fn a_device_is_known_by_the_newest_name_it_gave() {
        let devices = ClientDevices::default();
        let mut mac = device("Mac");
        devices.connected(mac.clone(), ConnectionId::generate());
        mac.name = "Studio".into();
        devices.connected(mac.clone(), ConnectionId::generate());
        assert_eq!(devices.get(&mac.id).unwrap().name, "Studio");
        assert_eq!(devices.list(), vec![mac]);
    }
}
