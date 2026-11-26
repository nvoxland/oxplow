//! `oxplow-host`: one headless process per `OXPLOW_HOME`. It keeps a
//! registry of projects, the holds devices have on them, and one
//! `oxplow-daemon` per held project (started through `DaemonSupervisor`),
//! forwards `/p/<project>/…` to it and serves the built UI.
//! See `.context/host.md`.

pub mod access;
pub mod api;
pub mod client;
pub mod devices;
pub mod host;
pub mod instance;
pub mod logging;
pub mod proxy;
pub mod registry;
pub mod tls;
pub mod ui;
pub mod web;

/// The host's version, reported in `host.json` and `/host/status`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
