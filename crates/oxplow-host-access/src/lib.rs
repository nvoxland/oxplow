//! What the host and its daemons share (`oxplow-host` depends on
//! `oxplow-app`, so a daemon cannot use the host's crate): the host's
//! settings' types and checks (the host keeps them), this machine on its networks, and a call
//! to the running host as its owner. See `.context/host.md`.

use std::path::{Path, PathBuf};

pub mod network;
pub mod owner;
pub mod settings;

/// The home's own root certificate (PEM), which a device trusts once.
pub fn root_path(config_dir: &Path) -> PathBuf {
    config_dir.join("host-ca.pem")
}
