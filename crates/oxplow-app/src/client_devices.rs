//! The devices connected to this daemon now, as they name themselves on their
//! event socket (`.context/remote-daemon.md` "Who is asking"): a device's
//! name and kind, and its live connections. Held in memory: a project's
//! database stores only device ids, and once a host stands in front of the
//! daemon its registry holds the names.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use oxplow_domain::{ConnectionId, DeviceDescriptor, DeviceId};

#[derive(Default)]
pub struct ClientDevices {
    inner: Mutex<HashMap<DeviceId, Entry>>,
}

struct Entry {
    descriptor: DeviceDescriptor,
    connections: BTreeSet<ConnectionId>,
}

impl ClientDevices {
    /// `connection` of `descriptor`'s device opened its socket; its name and
    /// kind are the newest it gave.
    pub fn connected(&self, descriptor: DeviceDescriptor, connection: ConnectionId) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let entry = inner.entry(descriptor.id.clone()).or_insert_with(|| Entry {
            descriptor: descriptor.clone(),
            connections: BTreeSet::new(),
        });
        entry.descriptor = descriptor;
        entry.connections.insert(connection);
    }

    /// `connection` of `device` closed its socket. A device with no
    /// connection left is no longer connected.
    pub fn disconnected(&self, device: &DeviceId, connection: &ConnectionId) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = inner.get_mut(device) {
            entry.connections.remove(connection);
            if entry.connections.is_empty() {
                inner.remove(device);
            }
        }
    }

    /// A connected device, as it named itself.
    pub fn get(&self, device: &DeviceId) -> Option<DeviceDescriptor> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.get(device).map(|e| e.descriptor.clone())
    }

    /// The connected devices, by id.
    pub fn list(&self) -> Vec<DeviceDescriptor> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let mut out: Vec<_> = inner.values().map(|e| e.descriptor.clone()).collect();
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
        devices.connected(tab.clone(), first.clone());
        devices.connected(tab.clone(), second.clone());
        devices.disconnected(&tab.id, &first);
        assert_eq!(devices.get(&tab.id), Some(tab.clone()));
        devices.disconnected(&tab.id, &second);
        assert_eq!(devices.get(&tab.id), None);
        assert!(devices.list().is_empty());
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
