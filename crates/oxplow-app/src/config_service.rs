//! Reading `Services::config`. Changing it is `config.set` / `config.unset`
//! (`commands::config_commands`), the one writer of `.oxplow/project.yaml`.

use std::sync::{Arc, RwLock};

use oxplow_config::OxplowConfig;

/// Returns a clone of the current in-memory config.
pub fn read_config(config: &Arc<RwLock<OxplowConfig>>) -> OxplowConfig {
    // Recover from poisoning rather than cascading the panic: a thread
    // that panicked while holding this lock leaves the config readable,
    // and the config is a plain data snapshot with no broken invariant.
    config.read().unwrap_or_else(|e| e.into_inner()).clone()
}
