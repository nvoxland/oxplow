//! One host per `OXPLOW_HOME`: the host that holds the home's `host.db`
//! (`db.rs`). A second one is refused and told who runs, which it asks the
//! running host over its owner socket. A dev build and an installed build
//! have different homes, so they never meet.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

use crate::db::{DbError, HostDb};

/// How long the app's host waits for the app's previous host, which stops
/// its daemons (a few seconds each) before it lets go.
pub const APP_HOST_WAIT: Duration = Duration::from_secs(15);

/// Who the running host is, as it says (`GET /host/status`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HostInfo {
    pub pid: u32,
    pub version: String,
    /// Started by the Mac app (`serve --for-app`): the app stops it when it
    /// quits. Otherwise headless: it runs until it is stopped.
    #[serde(default)]
    pub for_app: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    #[error("{}", match .0 {
        Some(i) => format!(
            "{} is already running for this home: pid {} (version {}){}",
            if i.for_app { "the Oxplow app's host" } else { "a headless host" },
            i.pid, i.version,
            if i.for_app { "" } else { "; stop it, or set another OXPLOW_HOME" },
        ),
        None => "a host is already running for this home (it did not say who it is)".to_string(),
    })]
    AlreadyRunning(Option<HostInfo>),
    #[error(transparent)]
    Db(DbError),
}

/// Become the home's host: hold its `host.db`. The app's host (`for_app`)
/// waits up to [`APP_HOST_WAIT`] while the app's previous host ends (after
/// a quit it stops its daemons first, and a new start can come before it is
/// done); a headless host is not waited for, since it runs until someone
/// stops it.
pub fn acquire(config_dir: &Path, for_app: bool) -> Result<Arc<HostDb>, InstanceError> {
    let opened = HostDb::open_waiting(config_dir, |waited| {
        for_app && should_wait(holder(config_dir).as_ref(), waited)
    });
    match opened {
        Ok(db) => Ok(db),
        Err(DbError::Held(_)) => Err(InstanceError::AlreadyRunning(holder(config_dir))),
        Err(e) => Err(InstanceError::Db(e)),
    }
}

/// Whether the app's host waits on, `waited` in, for the host that holds
/// the home: the app's previous one (or one that does not say), for a while.
/// Pure.
pub fn should_wait(holder: Option<&HostInfo>, waited: Duration) -> bool {
    holder.is_none_or(|h| h.for_app) && waited < APP_HOST_WAIT
}

/// The running host of `config_dir`, as it says over its owner socket.
fn holder(config_dir: &Path) -> Option<HostInfo> {
    let status =
        oxplow_host_access::owner::owner_call(config_dir, "GET", "/host/status", None).ok()?;
    serde_json::from_value(status).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(for_app: bool) -> HostInfo {
        HostInfo {
            pid: 42,
            version: "1.2.3".into(),
            for_app,
        }
    }

    #[test]
    fn the_apps_host_waits_only_for_the_apps_previous_host_and_only_a_while() {
        let soon = Duration::from_secs(1);
        assert!(should_wait(Some(&info(true)), soon));
        assert!(
            should_wait(None, soon),
            "one that does not say may be ending"
        );
        assert!(
            !should_wait(Some(&info(false)), soon),
            "a headless host runs on"
        );
        assert!(!should_wait(Some(&info(true)), APP_HOST_WAIT));
    }

    #[test]
    fn a_second_host_is_refused_and_told_who_runs() {
        let headless = InstanceError::AlreadyRunning(Some(info(false))).to_string();
        assert!(headless.contains("a headless host"), "{headless}");
        assert!(
            headless.contains("pid 42") && headless.contains("1.2.3"),
            "{headless}"
        );
        assert!(headless.contains("OXPLOW_HOME"), "{headless}");
        let app = InstanceError::AlreadyRunning(Some(info(true))).to_string();
        assert!(app.contains("the Oxplow app's host"), "{app}");

        let home = tempfile::tempdir().unwrap();
        let _first = acquire(home.path(), false).unwrap();
        // No owner socket here, so it cannot say who it is.
        assert!(matches!(
            acquire(home.path(), false),
            Err(InstanceError::AlreadyRunning(None))
        ));
    }

    #[test]
    fn the_apps_host_gets_the_home_once_the_previous_one_lets_go() {
        let home = tempfile::tempdir().unwrap();
        let first = acquire(home.path(), true).unwrap();
        let ending = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(first);
        });
        acquire(home.path(), true).expect("the home, once the first host is gone");
        ending.join().unwrap();
    }
}
