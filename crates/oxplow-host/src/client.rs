//! The host as the Mac app uses it. The app's host is a child of the app,
//! as a daemon is a child of the host: the app starts its own
//! ([`start_own_host`], `serve --for-app`) with a token made for this run,
//! and the host's stdin is its lifeline, so it ends when the app quits,
//! crashes or is killed, every device's projects with it. The app never
//! finds, adopts or shares a host. A window holds its project while it is
//! open ([`HostClient::open`], [`HostClient::release`]) and drives the
//! project's daemon at `<host>/p/<id>` with the token. See `.context/host.md`
//! "The Mac app".

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use crate::host::{DaemonState, ProjectView};
use crate::registry::Project;

/// How long the app's host gets to say where it listens: past the wait for
/// the previous host's lock (`serve` waits up to 15 s for it after a quit).
pub const HOST_START_TIMEOUT: Duration = Duration::from_secs(30);
/// How long an open waits for the project's daemon: past the daemon's own
/// boot timeout, plus the boots queued before it (one at a time).
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(180);
const POLL: Duration = Duration::from_millis(100);
/// The host's last stderr lines kept for an error that says why it ended.
const STDERR_KEPT: usize = 5;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("the host did not start: {0}")]
    NoHost(String),
    #[error("{0}")]
    Refused(String),
    #[error("the backend for {name} did not start{}", .reason.as_deref().map(|r| format!(": {r}")).unwrap_or_default())]
    Crashed {
        name: String,
        reason: Option<String>,
    },
    #[error("the backend for {0} is still not running after {1:?}")]
    Slow(String, Duration),
    #[error("the host could not be reached: {0}")]
    Http(#[from] reqwest::Error),
}

/// A project the app opened: its id and the base its window drives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedProject {
    pub id: String,
    pub name: String,
    /// `<host>/p/<id>`: the daemon's routes, through the host.
    pub base: String,
}

/// One host, reached with the app's token.
#[derive(Clone)]
pub struct HostClient {
    base: String,
    token: String,
    http: reqwest::Client,
}

/// The app's own host: the child process and its lifeline. Dropping it
/// closes the lifeline, and the host ends (it does not wait for that).
pub struct OwnHost {
    pub client: HostClient,
    _lifeline: ChildStdin,
    child: Child,
}

impl OwnHost {
    /// Whether the host has ended (it crashed: every project with it).
    pub fn ended(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }
}

/// The `oxplow-host` beside the running executable: the app bundle's
/// sidecar, or `target/<profile>/oxplow-host` in a checkout.
pub fn binary_beside_exe() -> PathBuf {
    let name = if cfg!(windows) {
        "oxplow-host.exe"
    } else {
        "oxplow-host"
    };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Start `host` (a command for the `oxplow-host` binary, usually
/// [`binary_beside_exe`]) as the app's host (`serve --for-app`, loopback
/// only) with a token made for this run on stdin, and wait for its
/// `listening on` line. It inherits the environment, so it serves the app's
/// `OXPLOW_HOME`. A host that cannot start (another host has the home) ends,
/// and its last stderr lines are the error. **Blocking.**
/// `extra` are more `serve` flags ("Allow other devices",
/// `crate::settings::HostSettings::serve_args`).
pub fn start_own_host(mut host: Command, extra: &[String]) -> Result<OwnHost, ClientError> {
    let token = new_token();
    let program = PathBuf::from(host.get_program());
    let no_host = |e: std::io::Error| ClientError::NoHost(format!("{}: {e}", program.display()));
    let mut child = host
        .args([
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--token-stdin",
            "--for-app",
        ])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(no_host)?;
    let mut lifeline = child.stdin.take().expect("stdin was piped");
    writeln!(lifeline, "{token}").map_err(no_host)?;
    lifeline.flush().map_err(no_host)?;

    let (said, heard) = std::sync::mpsc::channel::<String>();
    let stdout = child.stdout.take().expect("stdout was piped");
    std::thread::Builder::new()
        .name("oxplow-host-stdout".into())
        .spawn(move || {
            // The first line says where it listens; the rest is drained so
            // the host never blocks on a full pipe.
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(url) = line.strip_prefix("oxplow-host listening on ") {
                    let _ = said.send(url.trim().to_string());
                }
            }
        })
        .map_err(no_host)?;
    let last_errors = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
    let kept = last_errors.clone();
    let stderr = child.stderr.take().expect("stderr was piped");
    std::thread::Builder::new()
        .name("oxplow-host-stderr".into())
        .spawn(move || {
            // Passed on as it comes (the host's log is also in its file).
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                eprintln!("{line}");
                let mut kept = kept.lock().unwrap_or_else(|e| e.into_inner());
                if kept.len() == STDERR_KEPT {
                    kept.pop_front();
                }
                kept.push_back(line);
            }
        })
        .map_err(no_host)?;

    let started = Instant::now();
    loop {
        if let Ok(base) = heard.recv_timeout(POLL) {
            return Ok(OwnHost {
                client: HostClient::new(&base, &token),
                _lifeline: lifeline,
                child,
            });
        }
        let ended = child.try_wait().ok().flatten();
        if ended.is_some() || started.elapsed() >= HOST_START_TIMEOUT {
            if ended.is_none() {
                let _ = child.kill();
            }
            // Let the stderr reader take the last lines.
            std::thread::sleep(POLL);
            let said: Vec<String> = last_errors
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .cloned()
                .collect();
            return Err(ClientError::NoHost(match ended {
                Some(status) if !said.is_empty() => {
                    format!("it ended ({status}): {}", said.join(" "))
                }
                Some(status) => format!("it ended ({status}); see logs/host.*.log"),
                None => format!("it said nothing in {HOST_START_TIMEOUT:?}; see logs/host.*.log"),
            }));
        }
    }
}

fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

impl HostClient {
    pub fn new(base: &str, token: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(10))
                .build()
                .expect("an HTTP client"),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// The token a window sends to the host (until pairing gives the app its
    /// own device token).
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Open `dir` for `device`: register it, hold it, and wait until its
    /// daemon runs. A daemon that does not start is let go of again.
    pub async fn open(&self, dir: &Path, device: &str) -> Result<OpenedProject, ClientError> {
        let project: Project = self
            .send(
                self.http
                    .post(self.url("/host/projects"))
                    .json(&serde_json::json!({ "path": dir })),
            )
            .await?
            .json()
            .await?;
        self.open_held(&project.id, device).await
    }

    /// Hold the registered project `id` for `device` (a paired device's own:
    /// the host makes it so) and wait until its daemon runs. One that does not
    /// start is let go of again.
    pub async fn open_held(&self, id: &str, device: &str) -> Result<OpenedProject, ClientError> {
        self.hold(device, id).await?;
        let started = Instant::now();
        loop {
            let view = self.project(id).await?;
            match view.state {
                DaemonState::Running => {
                    return Ok(OpenedProject {
                        base: format!("{}/p/{}", self.base, view.id),
                        id: view.id,
                        name: view.name,
                    })
                }
                DaemonState::Crashed { message, .. } => {
                    let _ = self.release(id, device).await;
                    return Err(ClientError::Crashed {
                        name: view.name,
                        reason: message,
                    });
                }
                _ if started.elapsed() >= OPEN_TIMEOUT => {
                    let _ = self.release(id, device).await;
                    return Err(ClientError::Slow(view.name, OPEN_TIMEOUT));
                }
                _ => tokio::time::sleep(POLL).await,
            }
        }
    }

    /// As a paired device: hold `id` (its daemon starts).
    pub async fn hold_project(&self, id: &str) -> Result<(), ClientError> {
        self.hold("", id).await
    }

    /// The host's projects, as this caller sees them (`held_by_you` for a
    /// device).
    pub async fn projects(&self) -> Result<Vec<serde_json::Value>, ClientError> {
        Ok(self
            .send(self.http.get(self.url("/host/projects")))
            .await?
            .json()
            .await?)
    }

    /// The host's version (`/host/status`).
    pub async fn version(&self) -> Result<String, ClientError> {
        let status: serde_json::Value = self
            .send(self.http.get(self.url("/host/status")))
            .await?
            .json()
            .await?;
        Ok(status["version"].as_str().unwrap_or_default().to_string())
    }

    pub(crate) async fn delete_me(&self) -> Result<(), ClientError> {
        self.send(self.http.delete(self.url("/host/me"))).await?;
        Ok(())
    }

    /// A pairing code for a device (`ABCD-EFGH`) and the seconds it is good.
    pub async fn pairing_code(&self) -> Result<(String, u64), ClientError> {
        let answer: serde_json::Value = self
            .send(self.http.post(self.url("/host/pairing")))
            .await?
            .json()
            .await?;
        Ok((
            answer["code"].as_str().unwrap_or_default().to_string(),
            answer["expires_in"].as_u64().unwrap_or_default(),
        ))
    }

    pub async fn devices(&self) -> Result<Vec<crate::devices::PairedDevice>, ClientError> {
        Ok(self
            .send(self.http.get(self.url("/host/devices")))
            .await?
            .json()
            .await?)
    }

    pub async fn rename_device(
        &self,
        id: &str,
        name: &str,
    ) -> Result<crate::devices::PairedDevice, ClientError> {
        Ok(self
            .send(
                self.http
                    .patch(self.url(&format!("/host/devices/{id}")))
                    .json(&serde_json::json!({ "name": name })),
            )
            .await?
            .json()
            .await?)
    }

    /// Forget a paired device: its token stops working, its holds go and its
    /// pages ask to pair again.
    pub async fn remove_device(&self, id: &str) -> Result<(), ClientError> {
        self.send(self.http.delete(self.url(&format!("/host/devices/{id}"))))
            .await?;
        Ok(())
    }

    /// The devices with an open window on this host now.
    pub async fn connected(&self) -> Result<Vec<crate::access::Connected>, ClientError> {
        Ok(self.status_parts().await?.0)
    }

    /// Who has a window open now, and where other devices reach this host
    /// (or why they cannot).
    pub async fn status_parts(
        &self,
    ) -> Result<(Vec<crate::access::Connected>, crate::access::PublicListener), ClientError> {
        let status: serde_json::Value = self
            .send(self.http.get(self.url("/host/status")))
            .await?
            .json()
            .await?;
        Ok((
            serde_json::from_value(status["connected"].clone()).unwrap_or_default(),
            serde_json::from_value(status["public"].clone()).unwrap_or_default(),
        ))
    }

    /// `device` lets go of `project`: the last release stops its daemon.
    pub async fn release(&self, project: &str, device: &str) -> Result<(), ClientError> {
        self.send(
            self.http
                .delete(self.url("/host/holds"))
                .json(&serde_json::json!({ "device": device, "project": project })),
        )
        .await?;
        Ok(())
    }

    async fn hold(&self, device: &str, project: &str) -> Result<(), ClientError> {
        self.send(
            self.http
                .post(self.url("/host/holds"))
                .json(&serde_json::json!({ "device": device, "project": project })),
        )
        .await?;
        Ok(())
    }

    async fn project(&self, id: &str) -> Result<ProjectView, ClientError> {
        let projects: Vec<ProjectView> = self
            .send(self.http.get(self.url("/host/projects")))
            .await?
            .json()
            .await?;
        projects
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| ClientError::Refused(format!("the host lost the project {id}")))
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Send with the token; an answer that is not a success is `Refused`
    /// with the host's own words.
    async fn send(&self, req: reqwest::RequestBuilder) -> Result<reqwest::Response, ClientError> {
        let res = req.bearer_auth(&self.token).send().await?;
        if res.status().is_success() {
            return Ok(res);
        }
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        Err(ClientError::Refused(if text.is_empty() {
            status.to_string()
        } else {
            text
        }))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::host::tests::{Fixture, RUNS};
    use crate::instance::HostInstance;
    use std::sync::Arc;

    const TOKEN: &str = "app-token";

    /// A host for `fx`'s home in this process, as `serve` runs one.
    pub(crate) async fn run_host(fx: &Fixture, token: &str) -> String {
        let instance = HostInstance::acquire(&fx.home()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        instance.publish(&url, true).unwrap();
        let stop = Arc::new(tokio::sync::Notify::new());
        let app = crate::web::app(crate::web::ServeConfig {
            host: fx.host.clone(),
            access: crate::access::Access::new(
                token.into(),
                crate::devices::Devices::open(&fx.home()).unwrap(),
                Duration::from_millis(10),
            ),
            listener: crate::access::Listener::Loopback,
            ui_dir: None,
            allow_hosts: vec![],
            boot_wait: Duration::from_secs(5),
            stop: stop.clone(),
            scheme: Default::default(),
            root_pem: Default::default(),
        });
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move { stop.notified().await })
                .await;
            drop(instance);
        });
        url
    }

    /// The Devices window's calls: a code, the paired devices, a rename, a
    /// removal, and who is connected.
    #[tokio::test]
    async fn the_owner_makes_a_code_and_keeps_the_devices() {
        let fx = Fixture::new(RUNS);
        let url = run_host(&fx, TOKEN).await;
        let client = HostClient::new(&url, TOKEN);
        let (code, expires_in) = client.pairing_code().await.unwrap();
        assert_eq!(code.len(), 9, "{code}");
        assert!(expires_in > 500, "{expires_in}");
        let paired: serde_json::Value = reqwest::Client::new()
            .post(format!("{url}/host/pair"))
            .json(&serde_json::json!({ "code": code, "name": "Tab", "kind": "tablet" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let id = paired["device"]["id"].as_str().unwrap();
        assert_eq!(client.devices().await.unwrap()[0].name, "Tab");
        assert_eq!(
            client.rename_device(id, "Kitchen").await.unwrap().name,
            "Kitchen"
        );
        assert!(
            client.connected().await.unwrap().is_empty(),
            "no window open"
        );
        client.remove_device(id).await.unwrap();
        assert!(client.devices().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn opening_a_folder_holds_it_until_its_daemon_runs_and_releasing_lets_go() {
        let fx = Fixture::new(RUNS);
        let url = run_host(&fx, TOKEN).await;
        let dir = fx.dir.path().join("alpha");
        std::fs::create_dir_all(dir.join(".oxplow")).unwrap();
        let client = HostClient::new(&url, TOKEN);

        let opened = client.open(&dir, "dev-mac").await.unwrap();
        assert_eq!(opened.name, "alpha");
        assert_eq!(opened.base, format!("{url}/p/{}", opened.id));
        assert_eq!(fx.state(&opened.id), DaemonState::Running);

        client.release(&opened.id, "dev-mac").await.unwrap();
        assert!(fx.host.holds().is_empty());
    }

    #[tokio::test]
    async fn a_daemon_that_does_not_start_is_said_and_let_go_of() {
        let fx = Fixture::new("echo 'cannot take the project lock' >&2; exit 3");
        let url = run_host(&fx, TOKEN).await;
        let dir = fx.dir.path().join("alpha");
        std::fs::create_dir_all(dir.join(".oxplow")).unwrap();
        let err = HostClient::new(&url, TOKEN)
            .open(&dir, "dev-mac")
            .await
            .err()
            .unwrap();
        assert!(matches!(err, ClientError::Crashed { .. }), "{err}");
        assert!(err.to_string().contains("alpha"), "{err}");
        assert!(fx.host.holds().is_empty(), "the hold went with it");
    }

    #[tokio::test]
    async fn a_folder_that_is_not_a_project_is_refused_with_the_hosts_words() {
        let fx = Fixture::new(RUNS);
        let url = run_host(&fx, TOKEN).await;
        let err = HostClient::new(&url, TOKEN)
            .open(fx.dir.path(), "dev-mac")
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("not an oxplow project"), "{err}");
    }
}
