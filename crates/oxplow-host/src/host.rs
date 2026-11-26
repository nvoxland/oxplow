//! The host's core: the project registry, the holds on projects, and the
//! daemon each held project runs.
//!
//! A hold is `(device, project)`. The first hold on a project starts its
//! daemon; the last one released stops it. Holds are kept in
//! `host-holds.json`, so a restarted host starts again what was held. Daemon
//! states live in memory only.
//!
//! Starting and stopping run on one worker thread, so daemons boot one after
//! another (a boot peaks at about 380 MB and a CPU for its first minutes). A
//! request changes the hold and the state, queues the project, and returns:
//! the state shows `starting` until the worker has the daemon's endpoint.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use oxplow_app::daemon_supervisor::DaemonSupervisor;
use serde::{Deserialize, Serialize};

use crate::registry::{Project, Registry};

/// A crashed daemon is started again by a new hold only after this long, so
/// a daemon that dies at boot is not a restart loop.
pub const DEFAULT_RESTART_AFTER: Duration = Duration::from_secs(60);

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

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("no project has the id {0}")]
    UnknownProject(String),
    #[error("a device name is needed")]
    NoDevice,
    #[error("the holds file failed: {0}")]
    Io(#[from] std::io::Error),
}

struct Runtime {
    path: PathBuf,
    state: DaemonState,
    crashed_at: Option<Instant>,
}

#[derive(Default)]
struct Inner {
    holds: BTreeSet<Hold>,
    runtime: HashMap<String, Runtime>,
}

pub struct Host {
    registry: Registry,
    holds_path: PathBuf,
    supervisor: Arc<DaemonSupervisor>,
    restart_after: Duration,
    inner: Mutex<Inner>,
    queue: Sender<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct HoldsFile {
    #[serde(default)]
    holds: BTreeSet<Hold>,
}

enum Step {
    Start(PathBuf),
    Stop(PathBuf),
    Done,
}

impl Host {
    /// Open the host for `config_dir`: read the holds, and start what is held.
    pub fn open(
        config_dir: &Path,
        supervisor: Arc<DaemonSupervisor>,
        restart_after: Duration,
    ) -> Arc<Self> {
        let holds_path = config_dir.join("host-holds.json");
        let holds = std::fs::read(&holds_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<HoldsFile>(&b).ok())
            .map(|f| f.holds)
            .unwrap_or_default();
        let (queue, jobs) = channel::<String>();
        let host = Arc::new(Self {
            registry: Registry::new(config_dir),
            holds_path,
            supervisor,
            restart_after,
            inner: Mutex::new(Inner {
                holds,
                runtime: HashMap::new(),
            }),
            queue,
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
        host
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
        let mut inner = self.lock();
        inner.holds.insert(Hold {
            device: device.to_string(),
            project: project.to_string(),
        });
        self.save_holds(&inner)?;
        let rt = inner.runtime.entry(project.to_string()).or_insert(Runtime {
            path: known.path.clone(),
            state: DaemonState::Stopped,
            crashed_at: None,
        });
        let startable = match &rt.state {
            DaemonState::Stopped => true,
            DaemonState::Crashed { .. } => rt
                .crashed_at
                .is_none_or(|at| at.elapsed() >= self.restart_after),
            _ => false,
        };
        if startable {
            rt.path = known.path.clone();
            rt.state = DaemonState::Starting;
        }
        let out = view(&inner, known);
        drop(inner);
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
        inner.holds.remove(&Hold {
            device: device.to_string(),
            project: project.to_string(),
        });
        self.save_holds(&inner)?;
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
            rt.crashed_at = Some(Instant::now());
        }
    }

    /// Stop every daemon: the host is ending.
    pub fn shutdown(&self) {
        self.supervisor.stop_all();
        let mut inner = self.lock();
        for rt in inner.runtime.values_mut() {
            rt.state = DaemonState::Stopped;
        }
    }

    /// Start what the holds file says is held.
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
                    crashed_at: None,
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
            match step {
                Step::Done => return,
                Step::Start(path) => {
                    let started = self.supervisor.start(&path);
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
                                rt.crashed_at = Some(Instant::now());
                            }
                        }
                    }
                }
                Step::Stop(path) => {
                    self.supervisor.stop(&path);
                    tracing::info!(project = %id, "daemon stopped");
                    if let Some(rt) = self.lock().runtime.get_mut(id) {
                        rt.state = DaemonState::Stopped;
                    }
                }
            }
        }
    }

    fn save_holds(&self, inner: &Inner) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(&HoldsFile {
            holds: inner.holds.clone(),
        })
        .map_err(std::io::Error::other)?;
        oxplow_config::write_atomic(&self.holds_path, &bytes)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
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
        pub host: Arc<Host>,
    }

    impl Fixture {
        pub(crate) fn new(script: &str) -> Self {
            Self::with(script, Duration::from_secs(60))
        }

        pub(crate) fn with(script: &str, restart_after: Duration) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let supervisor = Arc::new(
                DaemonSupervisor::with_launcher(Box::new(Fake {
                    script: script.to_string(),
                }))
                .with_startup_timeout(Duration::from_secs(10))
                .with_shutdown_grace(Duration::from_millis(500)),
            );
            let host = Host::open(&dir.path().join("home"), supervisor, restart_after);
            Self { dir, host }
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
    fn a_new_hold_starts_a_crashed_daemon_again_once_the_wait_is_over() {
        let fx = Fixture::with(
            "echo 'oxplow-daemon listening on http://127.0.0.1:1'; sleep 0.2; exit 3",
            Duration::ZERO,
        );
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
        let again = Host::open(
            &fx.dir.path().join("home"),
            supervisor,
            Duration::from_secs(60),
        );
        assert_eq!(again.holds().len(), 1);
        let reopened = Fixture {
            dir: tempfile::tempdir().unwrap(),
            host: again,
        };
        reopened.wait_for(&id, running);
    }
}
