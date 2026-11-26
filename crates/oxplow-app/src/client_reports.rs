//! What the person's windows show right now (`.context/pages-and-tabs.md`
//! "What agents see of the tabs"): each window (a client, by the id its
//! renderer keeps for its life) reports the live facts only — the thread it
//! shows, whether it is focused and visible, the detail its active page
//! publishes (a page's params) and its page runs. The tabs themselves are the
//! device's stored ones ([`stored_tabs`]). An agent reads both with
//! `oxplow_list_tabs` and `oxplow_get_open_page`.
//!
//! Held in memory: nothing here outlives the process, and a client that
//! stops reporting (a closed window) is forgotten after [`FORGET_AFTER`]. A
//! report older than [`STALE_AFTER`] is still listed, marked stale.

pub mod devices;
pub mod stored_tabs;

/// What a named event socket holds while it is open: its page runs, its
/// device's place among the connected ones, and its window's tab report.
/// Only the connection's current socket lets them go when it closes: a page
/// load keeps its connection across reconnects, so a replaced socket's late
/// close must leave the window that took its place alone. A new page load of
/// the device takes over (`devices::ClientDevices::connected`): the one it
/// replaces stops being shown to agents at once, and its socket is told to
/// close ([`SocketLease::replaced`]).
pub struct SocketLease {
    _runs: crate::pages::open_runs::PageClient,
    services: std::sync::Arc<crate::Services>,
    device: oxplow_domain::DeviceId,
    connection: oxplow_domain::ConnectionId,
    socket: devices::SocketHandle,
}

impl SocketLease {
    /// `connection` of `descriptor`'s device opened a socket; refused for a
    /// page load another one of its device replaced.
    pub fn open(
        services: &std::sync::Arc<crate::Services>,
        descriptor: oxplow_domain::DeviceDescriptor,
        connection: oxplow_domain::ConnectionId,
    ) -> Result<Self, devices::Replaced> {
        let device = descriptor.id.clone();
        let opened = services
            .client_devices
            .connected(descriptor, connection.clone())?;
        if let Some(old) = &opened.replaced {
            // It stops being shown to agents, and stops hosting their calls
            // (a waiting call is answered at once, not after its timeout).
            services.client_reports.forget(old);
            services.client_host.unregister(old.as_str());
        }
        Ok(Self {
            _runs: services.page_runs.connect_client(connection.as_str()),
            services: services.clone(),
            device,
            connection,
            socket: opened.socket,
        })
    }

    /// The person used this page load now.
    pub fn used(&self) {
        self.services.client_devices.used(&self.device);
    }

    /// Resolves once another page load of the device takes over: the socket
    /// then closes, saying it was replaced.
    pub async fn replaced(&self) {
        self.socket.replaced().await
    }

    /// Resolves once the daemon asks the socket to close (a window that
    /// missed a call's answer): a normal close, the page reconnects.
    pub async fn closing(&self) {
        self.socket.closing().await
    }
}

impl Drop for SocketLease {
    fn drop(&mut self) {
        if self
            .services
            .client_devices
            .closed(&self.device, &self.connection, self.socket.id())
        {
            self.services.client_reports.forget(&self.connection);
            // It hosts no call until it is back (it registers again).
            self.services
                .client_host
                .unregister(self.connection.as_str());
        }
    }
}

use std::collections::HashMap;
use std::sync::Mutex;

use oxplow_domain::{ThreadId, Timestamp};
use serde::{Deserialize, Serialize};

/// A client not heard from for this long is gone.
pub const FORGET_AFTER_MS: i64 = 5 * 60 * 1000;
/// A report older than this is listed as stale.
pub const STALE_AFTER_MS: i64 = 60 * 1000;

/// What a window's report is answered with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TabReportAnswer {
    /// The runs it reported that the daemon no longer holds for it.
    pub missing_runs: Vec<String>,
}

/// What one window shows, as its renderer reports it. Which window it is
/// comes from the request's caller (its page load and device), not from
/// the report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct TabReport {
    pub focused: bool,
    pub visible: bool,
    /// The thread the window shows.
    pub shown_thread: Option<ThreadId>,
    /// The detail the shown thread's active page publishes (JSON text: a
    /// page's `{pageId, params}`, an editor's `{path, revision, selection}`).
    pub detail_json: Option<String>,
    /// Every live page run in this window, including panels, inline
    /// markdown and hidden history bodies.
    #[serde(default)]
    pub runs: Vec<String>,
}

#[derive(Debug, Clone)]
struct Client {
    /// The page load reporting (`con-…`): what `client` says to agents.
    connection: String,
    /// Its device (`dev-…`).
    device: String,
    report: TabReport,
    reported_at: Timestamp,
    /// When it last reported being focused: which window the person is in.
    focused_at: Option<Timestamp>,
}

/// What a connected window shows now, for an agent.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveWindow {
    /// The window: its page load.
    pub connection: String,
    /// The device the window is on.
    pub device: String,
    pub visible: bool,
    /// The thread the window shows.
    pub shown_thread: Option<ThreadId>,
    /// The detail its active page publishes (JSON text).
    pub detail_json: Option<String>,
    pub reported_at: Timestamp,
    pub stale: bool,
}

impl LiveWindow {
    /// The tab selected in `thread` is on screen: the window shows the
    /// thread and is visible.
    pub fn shows(&self, thread: ThreadId) -> bool {
        self.visible && self.shown_thread == Some(thread)
    }
}

#[derive(Default)]
pub struct ClientReports {
    clients: Mutex<HashMap<String, Client>>,
    /// Which window the person used last: it leads.
    devices: std::sync::Arc<devices::ClientDevices>,
}

impl ClientReports {
    pub fn new(devices: std::sync::Arc<devices::ClientDevices>) -> Self {
        Self {
            clients: Mutex::default(),
            devices,
        }
    }

    /// Windows still reporting at `now`; expired reports are forgotten.
    pub fn live_ids(&self, now: Timestamp) -> Vec<String> {
        self.live(now).into_iter().map(|c| c.connection).collect()
    }

    /// Take `report` as what `caller`'s window shows at `now`.
    pub fn report(&self, caller: &oxplow_domain::Caller, report: TabReport, now: Timestamp) {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let connection = caller.connection.as_str().to_string();
        let focused_at = if report.focused {
            Some(now)
        } else {
            clients.get(&connection).and_then(|c| c.focused_at)
        };
        clients.insert(
            connection.clone(),
            Client {
                connection,
                device: caller.device.as_str().to_string(),
                report,
                reported_at: now,
                focused_at,
            },
        );
    }

    /// `connection`'s window is gone (its socket closed): what it showed
    /// is no longer shown.
    pub fn forget(&self, connection: &oxplow_domain::ConnectionId) {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        clients.remove(connection.as_str());
    }

    /// The live clients at `now`: the one the person used last first, then
    /// the one focused last; the gone ones are forgotten.
    fn live(&self, now: Timestamp) -> Vec<Client> {
        let used = self
            .devices
            .last_used()
            .map(|(_, c)| c.as_str().to_string());
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        clients.retain(|_, c| now.unix_ms() - c.reported_at.unix_ms() <= FORGET_AFTER_MS);
        let mut out: Vec<Client> = clients.values().cloned().collect();
        let is_used = |c: &Client| used.as_deref() == Some(c.connection.as_str());
        out.sort_by(|a, b| {
            is_used(b)
                .cmp(&is_used(a))
                .then(b.focused_at.cmp(&a.focused_at))
                .then(b.reported_at.cmp(&a.reported_at))
                .then(a.connection.cmp(&b.connection))
        });
        out
    }

    /// What the live windows show at `now`, the one the person used last
    /// first.
    pub fn windows(&self, now: Timestamp) -> Vec<LiveWindow> {
        self.live(now)
            .into_iter()
            .map(|c| LiveWindow {
                stale: now.unix_ms() - c.reported_at.unix_ms() > STALE_AFTER_MS,
                connection: c.connection,
                device: c.device,
                visible: c.report.visible,
                shown_thread: c.report.shown_thread,
                detail_json: c.report.detail_json,
                reported_at: c.reported_at,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_unix_ms(ms)
    }

    /// The page load a test names `a`, `b`, …, on one device.
    fn con(name: &str) -> oxplow_domain::ConnectionId {
        let n = name.bytes().next().unwrap();
        oxplow_domain::ConnectionId::parse(&format!(
            "con-00000000-0000-4000-8000-0000000000{n:02x}"
        ))
        .unwrap()
    }

    fn caller(name: &str) -> oxplow_domain::Caller {
        oxplow_domain::Caller {
            device: oxplow_domain::DeviceId::parse("dev-1b4e28ba-2fa1-11d2-883f-0016d3cca427")
                .unwrap(),
            connection: con(name),
        }
    }

    fn report(client: &str, focused: bool, shown: i64) -> TabReport {
        TabReport {
            runs: Vec::new(),
            focused,
            visible: true,
            shown_thread: Some(ThreadId::new(shown)),
            detail_json: Some(format!("{{\"from\":\"{client}\"}}")),
        }
    }

    /// The window used last leads, ahead of the one focused last: an agent
    /// reads the page the person is working on.
    #[test]
    fn the_window_used_last_leads() {
        use oxplow_domain::{DeviceDescriptor, DeviceId, DeviceKind};
        let devices = std::sync::Arc::new(devices::ClientDevices::default());
        let r = ClientReports::new(devices.clone());
        let window = |name: &str, focused: bool, at_ms: i64| {
            let device = DeviceDescriptor {
                id: DeviceId::generate(),
                name: name.into(),
                kind: DeviceKind::Desktop,
            };
            devices.connected(device.clone(), con(name)).unwrap();
            let caller = oxplow_domain::Caller {
                device: device.id.clone(),
                connection: con(name),
            };
            r.report(&caller, report(name, focused, 1), at(at_ms));
            device.id
        };
        window("a", true, 1_000);
        let tablet = window("b", false, 2_000);
        devices.used(&tablet);
        let windows = r.windows(at(2_500));
        assert_eq!(windows[0].connection, con("b").as_str());
    }

    /// Each window says what it shows; only a visible window showing the
    /// thread has it on screen; the one focused last leads, and a report
    /// older than the stale limit is marked.
    #[test]
    fn windows_say_what_they_show_and_how_fresh_it_is() {
        let r = ClientReports::new(std::sync::Arc::default());
        r.report(&caller("a"), report("a", true, 1), at(1_000));
        let b = oxplow_domain::Caller {
            connection: con("b"),
            ..caller("a")
        };
        r.report(&b, report("b", false, 2), at(2_000));
        let windows = r.windows(at(2_500));
        assert_eq!(
            windows
                .iter()
                .map(|w| w.connection.as_str())
                .collect::<Vec<_>>(),
            [con("a").as_str(), con("b").as_str()]
        );
        assert!(windows[0].shows(ThreadId::new(1)) && !windows[0].shows(ThreadId::new(2)));
        assert_eq!(windows[0].detail_json.as_deref(), Some("{\"from\":\"a\"}"));
        assert!(!windows[0].stale);
        assert!(r.windows(at(1_000 + STALE_AFTER_MS + 1))[0].stale);
    }

    #[test]
    fn a_window_that_stops_reporting_is_forgotten() {
        let r = ClientReports::new(std::sync::Arc::default());
        r.report(&caller("a"), report("a", true, 1), at(0));
        assert_eq!(r.windows(at(FORGET_AFTER_MS)).len(), 1);
        assert!(r.windows(at(FORGET_AFTER_MS + 1)).is_empty());
    }
}
