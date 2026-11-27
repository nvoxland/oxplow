//! The host's core: the project registry, the holds on projects, and the
//! daemon each held project runs.
//!
//! A hold is `(device, project)`. The first hold on a project starts its
//! daemon; the last one released stops it. A headless host keeps its holds in
//! `host.db`'s `hold` table, so a restarted host (a server's reboot) starts again
//! what was held. The app's own host lives as long as the app, so its holds
//! are the app's open windows, in memory ([`HoldsKept`]). Daemon states live
//! in memory only.
//!
//! Starting and stopping run on one worker thread, so daemons boot one after
//! another (a boot peaks at about 380 MB and a CPU for its first minutes). A
//! request changes the hold and the state, queues the project, and returns:
//! the state shows `starting` until the worker has the daemon's endpoint.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use oxplow_app::daemon_supervisor::{DaemonEndpoint, DaemonSupervisor};
use serde::{Deserialize, Serialize};

use crate::db::HostDb;
use crate::registry::{Project, Registry};

/// Where a host keeps its holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldsKept {
    /// In `host.db`: a headless host starts again what was held.
    InFile,
    /// In memory: the app's own host, whose holds are the app's windows.
    InMemory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
pub struct Hold {
    pub device: String,
    pub project: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DaemonState {
    Starting,
    Running,
    Stopping,
    Stopped,
    /// The daemon exited (`code`), or did not boot (`message` says why).
    Crashed {
        code: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}

/// A project with what the host knows of it now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectView {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    #[serde(flatten)]
    pub state: DaemonState,
    pub holds: usize,
}

/// What a request for a project's daemon finds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// Running: where it listens and the token only the host holds.
    Ready(DaemonEndpoint),
    /// Booting: worth waiting for.
    Starting,
    /// No daemon: not held, stopped, stopping or crashed.
    Down(DaemonState),
    /// The registry has no such project.
    Unknown,
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("no project has the id {0}")]
    UnknownProject(String),
    #[error("a device name is needed")]
    NoDevice,
    #[error("the holds store failed: {0}")]
    Store(#[from] rusqlite::Error),
}

struct Runtime {
    path: PathBuf,
    state: DaemonState,
}

#[derive(Default)]
struct Inner {
    holds: BTreeSet<Hold>,
    runtime: HashMap<String, Runtime>,
}

pub struct Host {
    registry: Registry,
    /// Where the holds are kept besides memory; `None` for the app's host.
    holds_db: Option<Arc<HostDb>>,
    supervisor: Arc<DaemonSupervisor>,
    inner: Mutex<Inner>,
    queue: Sender<String>,
    /// Moves at each change of a hold or a daemon's state (`watch`).
    changes: tokio::sync::watch::Sender<u64>,
}

enum Step {
    Start(PathBuf),
    Stop(PathBuf),
    Done,
}

impl Host {
    /// Open the host over its `host.db`: read the holds (a headless host's),
    /// and start what is held.
    pub fn open(
        db: Arc<HostDb>,
        supervisor: Arc<DaemonSupervisor>,
        kept: HoldsKept,
    ) -> Result<Arc<Self>, HostError> {
        let holds_db = (kept == HoldsKept::InFile).then(|| db.clone());
        let holds = match &holds_db {
            Some(db) => kept_holds(db)?,
            None => BTreeSet::new(),
        };
        let (queue, jobs) = channel::<String>();
        let host = Arc::new(Self {
            registry: Registry::new(db),
            holds_db,
            supervisor,
            inner: Mutex::new(Inner {
                holds,
                runtime: HashMap::new(),
            }),
            queue,
            changes: tokio::sync::watch::channel(0).0,
        });
        // The worker owns no strong reference: the host drops when its owner
        // does, which closes the queue and ends the thread.
        let weak: Weak<Host> = Arc::downgrade(&host);
        std::thread::Builder::new()
            .name("oxplow-host-boot".into())
            .spawn(move || {
                while let Ok(id) = jobs.recv() {
                    let Some(host) = weak.upgrade() else { break };
                    host.reconcile(&id);
                }
            })
            .expect("spawn the boot worker");
        host.resume();
        Ok(host)
    }

    /// The app's own host (its holds in memory), not a headless one.
    pub fn for_app(&self) -> bool {
        self.holds_db.is_none()
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Every registered project with its state.
    pub fn projects(&self) -> Vec<ProjectView> {
        let projects = self.registry.list();
        let inner = self.lock();
        projects.into_iter().map(|p| view(&inner, p)).collect()
    }

    /// Where `project`'s daemon is now, for a request to forward to it.
    pub fn reach(&self, project: &str) -> Reach {
        if self.registry.get(project).is_none() {
            return Reach::Unknown;
        }
        let inner = self.lock();
        let Some(rt) = inner.runtime.get(project) else {
            return Reach::Down(DaemonState::Stopped);
        };
        match &rt.state {
            DaemonState::Starting => Reach::Starting,
            DaemonState::Running => self
                .supervisor
                .endpoint(&rt.path)
                .map_or(Reach::Down(DaemonState::Stopped), Reach::Ready),
            other => Reach::Down(other.clone()),
        }
    }

    /// A count that moves at each change of a hold or of a daemon's state:
    /// what `/host/events` sends a new list on.
    pub fn watch(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn changed(&self) {
        self.changes.send_modify(|n| *n += 1);
    }

    pub fn holds(&self) -> Vec<Hold> {
        self.lock().holds.iter().cloned().collect()
    }

    /// `device` holds `project`. The first hold starts the daemon.
    pub fn add_hold(&self, device: &str, project: &str) -> Result<ProjectView, HostError> {
        if device.trim().is_empty() {
            return Err(HostError::NoDevice);
        }
        let known = self
            .registry
            .get(project)
            .ok_or_else(|| HostError::UnknownProject(project.to_string()))?;
        let hold = Hold {
            device: device.to_string(),
            project: project.to_string(),
        };
        let mut inner = self.lock();
        if let Some(db) = &self.holds_db {
            db.conn().execute(
                "INSERT OR IGNORE INTO hold (device, project) VALUES (?1, ?2)",
                [&hold.device, &hold.project],
            )?;
        }
        inner.holds.insert(hold);
        let rt = inner.runtime.entry(project.to_string()).or_insert(Runtime {
            path: known.path.clone(),
            state: DaemonState::Stopped,
        });
        // A crashed daemon starts again: a new hold is a person opening the
        // project again (after quitting the other Oxplow, say).
        let startable = matches!(rt.state, DaemonState::Stopped | DaemonState::Crashed { .. });
        if startable {
            rt.path = known.path.clone();
            rt.state = DaemonState::Starting;
        }
        let out = view(&inner, known);
        drop(inner);
        self.changed();
        self.enqueue(project);
        Ok(out)
    }

    /// `device` lets go of `project`. The last release stops the daemon.
    /// Releasing a hold that is not held changes nothing.
    pub fn release_hold(&self, device: &str, project: &str) -> Result<ProjectView, HostError> {
        let known = self.registry.get(project);
        let mut inner = self.lock();
        if known.is_none() && !inner.runtime.contains_key(project) {
            return Err(HostError::UnknownProject(project.to_string()));
        }
        if let Some(db) = &self.holds_db {
            db.conn().execute(
                "DELETE FROM hold WHERE device = ?1 AND project = ?2",
                [device, project],
            )?;
        }
        inner.holds.remove(&Hold {
            device: device.to_string(),
            project: project.to_string(),
        });
        let out = match (known, inner.runtime.get(project)) {
            (Some(p), _) => view(&inner, p),
            (None, Some(rt)) => view(
                &inner,
                Project {
                    id: project.to_string(),
                    path: rt.path.clone(),
                    name: project.to_string(),
                },
            ),
            (None, None) => unreachable!("checked above"),
        };
        drop(inner);
        self.changed();
        self.enqueue(project);
        Ok(out)
    }

    /// Record the daemons that have exited. The serve loop calls this twice a
    /// second; a crashed daemon stays `crashed` until a hold starts it again.
    pub fn poll_exits(&self) {
        for (path, status) in self.supervisor.poll_exits() {
            let mut inner = self.lock();
            let Some((id, rt)) = inner
                .runtime
                .iter_mut()
                .find(|(_, rt)| rt.path == path && rt.state == DaemonState::Running)
            else {
                continue;
            };
            tracing::warn!(project = %id, ?status, "a project's daemon exited");
            rt.state = DaemonState::Crashed {
                code: status.code(),
                message: None,
            };
            drop(inner);
            self.changed();
        }
    }

    /// Stop every daemon: the host is ending.
    pub fn shutdown(&self) {
        self.supervisor.stop_all();
        let mut inner = self.lock();
        for rt in inner.runtime.values_mut() {
            rt.state = DaemonState::Stopped;
        }
        drop(inner);
        self.changed();
    }

    /// Start what the kept holds say is held.
    fn resume(&self) {
        let held: BTreeSet<String> = self
            .lock()
            .holds
            .iter()
            .map(|h| h.project.clone())
            .collect();
        for id in held {
            let Some(project) = self.registry.get(&id) else {
                tracing::warn!(project = %id, "a hold names a project the registry no longer has");
                continue;
            };
            self.lock().runtime.insert(
                id.clone(),
                Runtime {
                    path: project.path,
                    state: DaemonState::Starting,
                },
            );
            self.enqueue(&id);
        }
    }

    fn enqueue(&self, project: &str) {
        // A closed queue means the host is going; nothing is left to start.
        let _ = self.queue.send(project.to_string());
    }

    /// Bring `id`'s daemon to what its holds say. Runs on the worker, so one
    /// daemon boots or stops at a time.
    fn reconcile(&self, id: &str) {
        loop {
            let step = {
                let mut inner = self.lock();
                let desired = inner.holds.iter().any(|h| h.project == id);
                let Some(rt) = inner.runtime.get_mut(id) else {
                    return;
                };
                match (&rt.state, desired) {
                    (DaemonState::Starting, true) => Step::Start(rt.path.clone()),
                    // Released before the worker reached it: nothing to boot.
                    (DaemonState::Starting, false) => {
                        rt.state = DaemonState::Stopped;
                        Step::Done
                    }
                    (DaemonState::Running, false) => {
                        rt.state = DaemonState::Stopping;
                        Step::Stop(rt.path.clone())
                    }
                    (DaemonState::Stopping, _) => Step::Stop(rt.path.clone()),
                    // Held again while it was stopping.
                    (DaemonState::Stopped, true) => {
                        rt.state = DaemonState::Starting;
                        Step::Start(rt.path.clone())
                    }
                    _ => Step::Done,
                }
            };
            // The step above may have moved the state (Starting, Stopping).
            self.changed();
            match step {
                Step::Done => return,
                Step::Start(path) => {
                    let started = match taken_elsewhere(&path) {
                        Some(why) => Err(std::io::Error::other(why)),
                        None => self.supervisor.start(&path),
                    };
                    let mut inner = self.lock();
                    if let Some(rt) = inner.runtime.get_mut(id) {
                        match started {
                            Ok(endpoint) => {
                                tracing::info!(project = %id, url = %endpoint.base_url, "daemon running");
                                rt.state = DaemonState::Running;
                            }
                            Err(e) => {
                                tracing::warn!(project = %id, error = %e, "daemon did not start");
                                rt.state = DaemonState::Crashed {
                                    code: None,
                                    message: Some(e.to_string()),
                                };
                            }
                        }
                    }
                    drop(inner);
                    self.changed();
                }
                Step::Stop(path) => {
                    self.supervisor.stop(&path);
                    tracing::info!(project = %id, "daemon stopped");
                    if let Some(rt) = self.lock().runtime.get_mut(id) {
                        rt.state = DaemonState::Stopped;
                    }
                    self.changed();
                }
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// How long a daemon of an app that just quit gets to let go of its project.
const OTHER_EXIT_WAIT: Duration = Duration::from_secs(5);

/// Why `path` cannot be started here: another oxplow's daemon (an older app,
/// one started by hand) has it. One on its way out is waited for.
fn taken_elsewhere(path: &Path) -> Option<String> {
    let running = oxplow_app::daemon_supervisor::live_daemon(path)?;
    if oxplow_app::wait_for_project_unlock(path, OTHER_EXIT_WAIT) {
        return None;
    }
    Some(format!(
        "{} is already open in another Oxplow (its backend is process {}). \
         Use that window, or quit it first.",
        path.display(),
        running.pid
    ))
}

/// The holds a headless host kept.
fn kept_holds(db: &HostDb) -> rusqlite::Result<BTreeSet<Hold>> {
    let conn = db.conn();
    let mut q = conn.prepare("SELECT device, project FROM hold")?;
    let holds = q
        .query_map([], |r| {
            Ok(Hold {
                device: r.get(0)?,
                project: r.get(1)?,
            })
        })?
        .collect();
    holds
}

fn view(inner: &Inner, project: Project) -> ProjectView {
    ProjectView {
        state: inner
            .runtime
            .get(&project.id)
            .map_or(DaemonState::Stopped, |rt| rt.state.clone()),
        holds: inner
            .holds
            .iter()
            .filter(|h| h.project == project.id)
            .count(),
        id: project.id,
        path: project.path,
        name: project.name,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use oxplow_app::daemon_supervisor::DaemonLauncher;
    use std::process::Command;

    /// Runs a shell script instead of a daemon. `$1` is the project dir; the
    /// script appends `begin <name>` and `ready <name>` to `$LOG`.
    pub(crate) struct Fake {
        pub script: String,
    }

    impl DaemonLauncher for Fake {
        fn command(&self, project_dir: &Path) -> Command {
            let mut cmd = Command::new("/bin/sh");
            cmd.arg("-c").arg(&self.script).arg("fake").arg(project_dir);
            cmd
        }
    }

    pub(crate) const RUNS: &str =
        "echo 'oxplow-daemon listening on http://127.0.0.1:12345'; sleep 30";

    pub(crate) struct Fixture {
        pub dir: tempfile::TempDir,
        /// The home's `host.db`, which this test process holds.
        pub db: Arc<HostDb>,
        pub host: Arc<Host>,
    }

    impl Fixture {
        pub(crate) fn new(script: &str) -> Self {
            Self::with(script, HoldsKept::InFile)
        }

        pub(crate) fn with(script: &str, kept: HoldsKept) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let supervisor = Arc::new(
                DaemonSupervisor::with_launcher(Box::new(Fake {
                    script: script.to_string(),
                }))
                .with_startup_timeout(Duration::from_secs(10))
                .with_shutdown_grace(Duration::from_millis(500)),
            );
            let db = HostDb::open(&dir.path().join("home")).unwrap();
            let host = Host::open(db.clone(), supervisor, kept).unwrap();
            Self { dir, db, host }
        }

        pub(crate) fn project(&self, name: &str) -> String {
            let dir = self.dir.path().join(name);
            std::fs::create_dir_all(dir.join(".oxplow")).unwrap();
            self.host.registry().add(&dir).unwrap().id
        }

        pub(crate) fn state(&self, id: &str) -> DaemonState {
            self.host
                .projects()
                .into_iter()
                .find(|p| p.id == id)
                .unwrap()
                .state
        }

        /// Wait until `id` is in a state `want` accepts.
        pub(crate) fn wait_for(&self, id: &str, want: impl Fn(&DaemonState) -> bool) {
            for _ in 0..400 {
                if want(&self.state(id)) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            panic!("timed out; the state is {:?}", self.state(id));
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.host.shutdown();
        }
    }

    fn running(s: &DaemonState) -> bool {
        *s == DaemonState::Running
    }

    /// A project another oxplow runs (an older app, a daemon started by
    /// hand) is not started a second time: it says which process has it.
    #[test]
    fn a_project_another_oxplow_runs_is_refused_with_its_process() {
        let fx = Fixture::new(RUNS);
        let id = fx.project("alpha");
        oxplow_app::daemon_supervisor::write_daemon_info(
            &fx.dir.path().join("alpha"),
            &oxplow_app::daemon_supervisor::DaemonInfo {
                base_url: "http://127.0.0.1:1".into(),
                pid: std::process::id(),
            },
        )
        .unwrap();
        let layout = oxplow_app::AppLayout::for_project(fx.dir.path().join("alpha"));
        let _held = oxplow_app::try_acquire_instance_lock(&layout).unwrap();
        fx.host.add_hold("dev-a", &id).unwrap();
        fx.wait_for(&id, |s| matches!(s, DaemonState::Crashed { .. }));
        let DaemonState::Crashed { message, .. } = fx.state(&id) else {
            unreachable!()
        };
        let message = message.unwrap();
        assert!(
            message.contains("already open in another Oxplow"),
            "{message}"
        );
        assert!(
            message.contains(&std::process::id().to_string()),
            "{message}"
        );
        assert_eq!(fx.host.supervisor.running_count(), 0);
    }

    /// Each change of a hold or a daemon's state moves the host's change
    /// count, which `/host/events` watches.
    #[test]
    fn a_hold_and_each_state_it_brings_are_changes() {
        let fx = Fixture::new(RUNS);
        let id = fx.project("alpha");
        let changes = fx.host.watch();
        let seen = *changes.borrow();
        fx.host.add_hold("dev-a", &id).unwrap();
        let held = *changes.borrow();
        assert!(held > seen, "the hold");
        fx.wait_for(&id, running);
        assert!(*changes.borrow() > held, "running");
        let before = *changes.borrow();
        fx.host.release_hold("dev-a", &id).unwrap();
        fx.wait_for(&id, |s| *s == DaemonState::Stopped);
        assert!(*changes.borrow() >= before + 2, "the release and the stop");
    }

    #[test]
    fn the_first_hold_starts_the_daemon_and_the_last_release_stops_it() {
        let fx = Fixture::new(RUNS);
        let id = fx.project("alpha");
        assert_eq!(fx.state(&id), DaemonState::Stopped);

        let view = fx.host.add_hold("dev-a", &id).unwrap();
        assert_eq!(view.state, DaemonState::Starting);
        fx.wait_for(&id, running);
        assert_eq!(fx.host.supervisor.running_count(), 1);

        fx.host.release_hold("dev-a", &id).unwrap();
        fx.wait_for(&id, |s| *s == DaemonState::Stopped);
        assert_eq!(fx.host.supervisor.running_count(), 0);
    }

    #[test]
    fn a_second_device_does_not_start_another_daemon_and_the_first_release_does_not_stop_it() {
        let fx = Fixture::new(RUNS);
        let id = fx.project("alpha");
        fx.host.add_hold("dev-a", &id).unwrap();
        fx.wait_for(&id, running);
        let second = fx.host.add_hold("dev-b", &id).unwrap();
        assert_eq!(second.state, DaemonState::Running);
        assert_eq!(second.holds, 2);
        assert_eq!(fx.host.supervisor.running_count(), 1);

        fx.host.release_hold("dev-a", &id).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(fx.state(&id), DaemonState::Running, "dev-b still holds it");

        // The same device holding twice is one hold.
        fx.host.add_hold("dev-b", &id).unwrap();
        assert_eq!(fx.host.holds().len(), 1);
        fx.host.release_hold("dev-b", &id).unwrap();
        fx.wait_for(&id, |s| *s == DaemonState::Stopped);
    }

    #[test]
    fn daemons_boot_one_at_a_time() {
        let log = tempfile::NamedTempFile::new().unwrap();
        let script = format!(
            "n=$(basename \"$1\"); echo \"begin $n\" >> {log}; sleep 0.3; echo \"ready $n\" >> {log}; \
             echo 'oxplow-daemon listening on http://127.0.0.1:12345'; sleep 30",
            log = log.path().display()
        );
        let fx = Fixture::new(&script);
        let (a, b, c) = (fx.project("a"), fx.project("b"), fx.project("c"));
        for id in [&a, &b, &c] {
            fx.host.add_hold("dev", id).unwrap();
        }
        for id in [&a, &b, &c] {
            fx.wait_for(id, running);
        }
        let lines: Vec<String> = std::fs::read_to_string(log.path())
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(lines.len(), 6, "{lines:?}");
        for pair in lines.chunks(2) {
            let name = pair[0].strip_prefix("begin ").expect("a begin first");
            assert_eq!(pair[1], format!("ready {name}"), "{lines:?}");
        }
    }

    #[test]
    fn a_daemon_that_exits_is_crashed_and_not_restarted() {
        let fx =
            Fixture::new("echo 'oxplow-daemon listening on http://127.0.0.1:1'; sleep 0.2; exit 3");
        let id = fx.project("alpha");
        fx.host.add_hold("dev", &id).unwrap();
        fx.wait_for(&id, running);
        for _ in 0..200 {
            fx.host.poll_exits();
            if matches!(fx.state(&id), DaemonState::Crashed { .. }) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(
            fx.state(&id),
            DaemonState::Crashed {
                code: Some(3),
                message: None
            }
        );
        std::thread::sleep(Duration::from_millis(300));
        fx.host.poll_exits();
        assert!(
            matches!(fx.state(&id), DaemonState::Crashed { .. }),
            "no restart loop"
        );
        assert_eq!(fx.host.supervisor.running_count(), 0);
    }

    #[test]
    /// Quit the other Oxplow, then open again: the open starts it at once.
    fn a_new_hold_starts_a_crashed_daemon_again() {
        let fx =
            Fixture::new("echo 'oxplow-daemon listening on http://127.0.0.1:1'; sleep 0.2; exit 3");
        let id = fx.project("alpha");
        fx.host.add_hold("dev-a", &id).unwrap();
        fx.wait_for(&id, running);
        for _ in 0..200 {
            fx.host.poll_exits();
            if matches!(fx.state(&id), DaemonState::Crashed { .. }) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let view = fx.host.add_hold("dev-b", &id).unwrap();
        assert_eq!(view.state, DaemonState::Starting);
        fx.wait_for(&id, running);
    }

    #[test]
    fn a_daemon_that_does_not_boot_is_crashed_with_the_reason() {
        let fx = Fixture::new("echo 'project already open' >&2; exit 1");
        let id = fx.project("alpha");
        fx.host.add_hold("dev", &id).unwrap();
        fx.wait_for(&id, |s| matches!(s, DaemonState::Crashed { .. }));
        let DaemonState::Crashed { code, message } = fx.state(&id) else {
            unreachable!()
        };
        assert_eq!(code, None);
        assert!(message.unwrap().contains("exited"));
    }

    #[test]
    fn a_hold_on_an_unknown_project_or_without_a_device_is_refused() {
        let fx = Fixture::new(RUNS);
        assert!(matches!(
            fx.host.add_hold("dev", "prj-00000000"),
            Err(HostError::UnknownProject(_))
        ));
        let id = fx.project("alpha");
        assert!(matches!(
            fx.host.add_hold(" ", &id),
            Err(HostError::NoDevice)
        ));
        assert!(fx.host.holds().is_empty());
        assert!(matches!(
            fx.host.release_hold("dev", "prj-00000000"),
            Err(HostError::UnknownProject(_))
        ));
    }

    #[test]
    fn holds_survive_a_restart_and_start_again() {
        let fx = Fixture::new(RUNS);
        let id = fx.project("alpha");
        fx.host.add_hold("dev", &id).unwrap();
        fx.wait_for(&id, running);
        fx.host.shutdown();

        let supervisor = Arc::new(
            DaemonSupervisor::with_launcher(Box::new(Fake {
                script: RUNS.into(),
            }))
            .with_startup_timeout(Duration::from_secs(10)),
        );
        let again = Host::open(fx.db.clone(), supervisor, HoldsKept::InFile).unwrap();
        assert_eq!(again.holds().len(), 1);
        let reopened = Fixture {
            dir: tempfile::tempdir().unwrap(),
            db: fx.db.clone(),
            host: again,
        };
        reopened.wait_for(&id, running);
    }

    /// The app's host lives as long as the app: its holds are the open
    /// windows, kept in memory. Holds a headless host kept are not read, and
    /// none are written.
    #[test]
    fn an_app_host_keeps_its_holds_in_memory() {
        let fx = Fixture::new(RUNS);
        let id = fx.project("alpha");
        fx.host.add_hold("dev-tablet", &id).unwrap();
        fx.wait_for(&id, running);
        fx.host.shutdown();
        let kept = || kept_holds(&fx.db).unwrap();
        let before = kept();
        assert_eq!(before.len(), 1);

        let app = Fixture {
            host: Host::open(
                fx.db.clone(),
                Arc::new(DaemonSupervisor::with_launcher(Box::new(Fake {
                    script: RUNS.into(),
                }))),
                HoldsKept::InMemory,
            )
            .unwrap(),
            db: fx.db.clone(),
            dir: tempfile::tempdir().unwrap(),
        };
        assert!(app.host.for_app() && !fx.host.for_app());
        assert!(app.host.holds().is_empty(), "the kept holds are not read");
        app.host.add_hold("dev-mac", &id).unwrap();
        app.host.release_hold("dev-mac", &id).unwrap();
        app.host.add_hold("dev-mac", &id).unwrap();
        assert_eq!(app.host.holds().len(), 1);
        assert_eq!(kept(), before, "nor written");
    }
}
