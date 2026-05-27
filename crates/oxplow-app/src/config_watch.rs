//! Watches the project's `oxplow.yaml` and hot-reloads the in-memory
//! config when it changes on disk.
//!
//! Without this, a config edit made out-of-band — most importantly the
//! agent running `/oxplow:configure`, which Writes a `collection:` block
//! to `oxplow.yaml` — wouldn't take effect until the app process
//! restarted, because config is otherwise read once at `Services` boot.
//! The IPC setters (`set_generated`, `set_agent_prompt_append`) mutate
//! the in-memory config directly; this watcher covers every *other* way
//! the file changes.
//!
//! Spawned once at boot from the host binary; the returned handle holds
//! the `FsWatcher` for the process lifetime (drop cancels the watch).

use std::sync::Arc;
use std::time::Duration;

use oxplow_config::OXPLOW_CONFIG_FILE;
use oxplow_fs_watch::{FsWatcher, RecursiveMode, WatchEvent};
use tracing::{debug, warn};

use crate::Services;

/// Holds the config-file watcher alive. Drop to stop watching.
pub struct ConfigWatcher {
    _fs: FsWatcher,
}

impl ConfigWatcher {
    /// Start watching `<project>/oxplow.yaml`. Returns `None` if the
    /// watcher can't be created (logged, non-fatal — the app just won't
    /// hot-reload config).
    pub fn spawn(services: Arc<Services>) -> Option<Self> {
        let project_dir = services.layout.project_dir.clone();
        // Watch the project root non-recursively rather than the file
        // directly: editors (and atomic writers) replace the file via
        // rename, which breaks a single-file watch but is still visible
        // as a directory event.
        let watcher = match FsWatcher::watch_paths(vec![(
            project_dir.clone(),
            RecursiveMode::NonRecursive,
        )]) {
            Ok(w) => w,
            Err(e) => {
                warn!(error = %e, ?project_dir, "config watcher failed to start");
                return None;
            }
        };
        // Debounced: a save can fire create+modify; we only need to
        // settle on the final content.
        let mut rx = watcher.subscribe_debounced(Duration::from_millis(300));
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(WatchEvent { path, .. }) => {
                        let is_config = path
                            .file_name()
                            .map(|n| n == OXPLOW_CONFIG_FILE)
                            .unwrap_or(false);
                        if !is_config {
                            continue;
                        }
                        match services.reload_config_from_disk() {
                            Ok(()) => debug!("reloaded oxplow.yaml after on-disk change"),
                            Err(e) => warn!(error = %e, "failed to reload oxplow.yaml"),
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Some(Self { _fs: watcher })
    }
}
