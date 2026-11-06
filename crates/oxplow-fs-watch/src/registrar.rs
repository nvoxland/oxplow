//! The one place the OS's watch registration is reached from the follower, so
//! the follower's decisions can be tested with no OS.

use std::path::Path;

use notify::{RecursiveMode, Watcher};

/// Adds and removes OS watches.
pub(crate) trait Registrar: Send {
    fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()>;
    fn unwatch(&mut self, path: &Path) -> notify::Result<()>;
}

impl Registrar for notify::RecommendedWatcher {
    fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
        Watcher::watch(self, path, mode)
    }

    fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        Watcher::unwatch(self, path)
    }
}
