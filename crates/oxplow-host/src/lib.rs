//! `oxplow-host`: one headless process per `OXPLOW_HOME`. It keeps a
//! registry of projects, the holds devices have on them, and one
//! `oxplow-daemon` per held project (started through `DaemonSupervisor`).
//! See `.context/host.md`.

pub mod api;
pub mod host;
pub mod instance;
pub mod logging;
pub mod registry;

/// The host's version, reported in `host.json` and `/host/status`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
