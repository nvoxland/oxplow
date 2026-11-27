//! `oxplow-host serve [--bind 127.0.0.1:0] [--token-stdin] [--ui <dir>]
//!   [--allow-host <name>]… [--daemon <path>] [--for-app]
//!   [--public-bind <ip> [--device-port <port>] [--tls-cert <pem> --tls-key <pem>]]`
//! `oxplow-host project <add <dir> | list | remove <id>>`
//! `oxplow-host settings [set key=value…]`
//!
//! A headless host (no `--for-app`) runs until it is stopped: a signal or
//! `POST /host/shutdown`. The app's own host (`--for-app`) is a child of the
//! app: stdin, after the token line, is its lifeline, and it ends when the
//! app goes. No host ends because nothing is held. Its daemons each have a
//! lifeline held by the host.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use oxplow_app::daemon_supervisor::{
    BinaryDaemon, BundledDaemon, DaemonLauncher, DaemonSupervisor, DEFAULT_STARTUP_TIMEOUT,
};
use oxplow_host::host::{HoldsKept, Host};
use oxplow_host::registry::Registry;

fn usage() -> ! {
    eprintln!(
        "usage: oxplow-host serve [--bind 127.0.0.1:0] [--token-stdin] [--ui <dir>]\n\
         \x20                        [--allow-host <name>]... [--daemon <path>]\n\
         \x20                        [--for-app]\n\
         \x20                        [--public-bind <ip> [--device-port <port>]\n\
         \x20                         [--tls-cert <pem> --tls-key <pem>]]\n\
         \x20      oxplow-host project add <dir>\n\
         \x20      oxplow-host project list\n\
         \x20      oxplow-host project remove <id>\n\
         \x20      oxplow-host settings [set allow=on|off address=<ip> port=<port>\n\
         \x20                        names=<a,b> tls-cert=<pem> tls-key=<pem>]\n\
         \x20      oxplow-host pair [<code>] | devices [remove <id>] | status | stop\n\
         \n\
         One host runs per OXPLOW_HOME. --token-stdin reads the host token from\n\
         the first line of stdin; otherwise a fresh one is generated and printed.\n\
         --ui is the built UI (default: beside the binary, or apps/desktop/dist in a\n\
         checkout). --allow-host names a host a request may reach this one by, besides\n\
         loopback. --daemon runs another daemon binary (the browser suite's).\n\
         --for-app makes it the Mac app's host: stdin after the token is its\n\
         lifeline, and it ends with the app. Without it the host is headless\n\
         and runs until it is stopped. Other devices come in on a second listener\n\
         (HTTPS, with the home's own certificate or one of yours) as `settings`\n\
         says; a change applies at once. --public-bind, --device-port (else\n\
         OXPLOW_DEVICE_PORT), --allow-host and --tls-cert/--tls-key replace what\n\
         `settings` keeps for this run only, and are never kept. --bind stays for\n\
         this machine."
    );
    std::process::exit(2);
}

/// A call to the running host over its owner socket; fails with its words.
fn owner_call(config_dir: &std::path::Path, method: &str, path: &str) -> serde_json::Value {
    match oxplow_host_access::owner::owner_call(config_dir, method, path, None) {
        Ok(value) => value,
        Err(e) => fail(e),
    }
}

/// A project call: to the running host when one answers (it holds
/// `host.db`), else `alone` over `host.db` opened here.
fn projects_call<E: std::fmt::Display>(
    config_dir: &std::path::Path,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
    alone: impl FnOnce(&Registry) -> Result<serde_json::Value, E>,
) -> serde_json::Value {
    use oxplow_host_access::owner::{owner_call, OwnerCallError};
    match owner_call(config_dir, method, path, body.as_ref()) {
        Ok(value) => value,
        Err(OwnerCallError::NoHost(_)) => {
            let db = oxplow_host::db::HostDb::open(config_dir).unwrap_or_else(|e| fail(e));
            alone(&Registry::new(db)).unwrap_or_else(|e| fail(e))
        }
        Err(e) => fail(e),
    }
}

/// "Allow other devices" as the running host has them, else as `host.db`
/// keeps them (no run, so no flags over them).
fn settings_view(config_dir: &std::path::Path) -> serde_json::Value {
    use oxplow_host_access::owner::{owner_call, OwnerCallError};
    match owner_call(config_dir, "GET", "/host/settings", None) {
        Ok(view) => view,
        Err(OwnerCallError::NoHost(_)) => {
            let db = oxplow_host::db::HostDb::open(config_dir).unwrap_or_else(|e| fail(e));
            let other = oxplow_host::other_devices::OtherDevices::new(db, Default::default());
            serde_json::to_value(other.view()).unwrap_or_default()
        }
        Err(e) => fail(e),
    }
}

/// `settings set key=value…` over what is saved: `allow=on|off`,
/// `address=<ip>`, `port=<port>` (empty: 7400), `names=<a,b>`,
/// `tls-cert=<pem>`, `tls-key=<pem>` (empty: the host's own). Through the
/// running host, which applies them at once, else into `host.db`.
fn set_settings(config_dir: &std::path::Path, changes: &[&str]) {
    use oxplow_host_access::owner::{owner_call, OwnerCallError};
    use oxplow_host_access::settings::HostSettings;
    let view = settings_view(config_dir);
    let mut wanted: HostSettings =
        serde_json::from_value(view["saved"].clone()).unwrap_or_default();
    let path = |v: &str| (!v.trim().is_empty()).then(|| PathBuf::from(v.trim()));
    for change in changes {
        let Some((key, value)) = change.split_once('=') else {
            fail(format!("\"{change}\" is not key=value"));
        };
        match key {
            "allow" => {
                wanted.allow_other_devices = match value {
                    "on" | "yes" | "true" => true,
                    "off" | "no" | "false" => false,
                    _ => fail("allow is on or off"),
                }
            }
            "address" => wanted.address = value.trim().to_string(),
            "port" => {
                wanted.port = match value.trim() {
                    "" => None,
                    text => Some(
                        oxplow_host_access::settings::parse_port(text).unwrap_or_else(|e| fail(e)),
                    ),
                }
            }
            "names" => wanted.names = value.split(',').map(|n| n.trim().to_string()).collect(),
            "tls-cert" => wanted.tls_cert = path(value),
            "tls-key" => wanted.tls_key = path(value),
            _ => fail(format!(
                "no setting {key}: allow, address, port, names, tls-cert, tls-key"
            )),
        }
    }
    let wanted = wanted
        .checked()
        .unwrap_or_else(|e| fail(format!("{}: {e}", e.field())));
    let body = serde_json::to_value(&wanted).unwrap_or_default();
    match owner_call(config_dir, "PUT", "/host/settings", Some(&body)) {
        Ok(_) => println!("saved; in force now"),
        Err(OwnerCallError::NoHost(_)) => {
            let db = oxplow_host::db::HostDb::open(config_dir).unwrap_or_else(|e| fail(e));
            oxplow_host::other_devices::OtherDevices::new(db, Default::default())
                .save(wanted)
                .unwrap_or_else(|e| fail(e));
            println!("saved; in force when the host starts");
        }
        Err(e) => fail(e),
    }
}

/// One project as the CLI prints it: id, name and folder.
fn project_line(p: &serde_json::Value) -> String {
    format!(
        "{}  {}  {}",
        p["id"].as_str().unwrap_or("?"),
        p["name"].as_str().unwrap_or("?"),
        p["path"].as_str().unwrap_or("?")
    )
}

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("oxplow-host: {message}");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(config_dir) = oxplow_config::global_config_dir() else {
        fail("no config directory for this user; set OXPLOW_HOME");
    };
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["serve", ref rest @ ..] => serve(&config_dir, rest),
        ["project", "add", dir] => {
            let dir = std::path::absolute(dir).unwrap_or_else(|e| fail(e));
            let body = serde_json::json!({ "path": dir });
            let p = projects_call(&config_dir, "POST", "/host/projects", Some(body), |reg| {
                reg.add(&dir)
                    .map(|p| serde_json::to_value(p).unwrap_or_default())
            });
            println!("{}", project_line(&p));
        }
        ["project", "list"] => {
            let list = projects_call(&config_dir, "GET", "/host/projects", None, |reg| {
                Ok::<_, std::convert::Infallible>(
                    serde_json::to_value(reg.list()).unwrap_or_default(),
                )
            });
            for p in list.as_array().into_iter().flatten() {
                println!("{}", project_line(p));
            }
        }
        ["project", "remove", id] => {
            let path = format!("/host/projects/{id}");
            let p = projects_call(&config_dir, "DELETE", &path, None, |reg| {
                reg.remove(id)
                    .map(|p| serde_json::to_value(p).unwrap_or_default())
            });
            println!("removed {}", project_line(&p));
        }
        ["settings"] => {
            let view = settings_view(&config_dir);
            println!(
                "{}",
                serde_json::to_string_pretty(&view).unwrap_or_default()
            );
        }
        ["settings", "set", ref changes @ ..] if !changes.is_empty() => {
            set_settings(&config_dir, changes);
        }
        ["pair"] => {
            let waiting = owner_call(&config_dir, "GET", "/host/pair/waiting");
            let waiting = waiting.as_array().cloned().unwrap_or_default();
            if waiting.is_empty() {
                println!("no device is waiting to pair: open this host's address on it");
            }
            for w in &waiting {
                println!(
                    "waiting: {}  ({})",
                    w["name"].as_str().unwrap_or("?"),
                    w["kind"].as_str().unwrap_or("?")
                );
            }
            if !waiting.is_empty() {
                println!("pair one with: oxplow host pair <the code it shows>");
            }
        }
        ["pair", code] => {
            let body = serde_json::json!({ "code": code });
            match oxplow_host_access::owner::owner_call(
                &config_dir,
                "POST",
                "/host/pair/approve",
                Some(&body),
            ) {
                Ok(d) => println!(
                    "paired {}  {}  {}",
                    d["id"].as_str().unwrap_or("?"),
                    d["kind"].as_str().unwrap_or("?"),
                    d["name"].as_str().unwrap_or("?")
                ),
                Err(e) => fail(e),
            }
        }
        ["devices"] => {
            let devices = owner_call(&config_dir, "GET", "/host/devices");
            for d in devices.as_array().into_iter().flatten() {
                println!(
                    "{}  {}  {}",
                    d["id"].as_str().unwrap_or("?"),
                    d["kind"].as_str().unwrap_or("?"),
                    d["name"].as_str().unwrap_or("?")
                );
            }
        }
        ["devices", "remove", id] => {
            owner_call(&config_dir, "DELETE", &format!("/host/devices/{id}"));
            println!("removed {id}");
        }
        ["status"] => {
            let status = owner_call(&config_dir, "GET", "/host/status");
            println!(
                "{}",
                serde_json::to_string_pretty(&status).unwrap_or_default()
            );
        }
        ["stop"] => {
            owner_call(&config_dir, "POST", "/host/shutdown");
            println!("the host is stopping");
        }
        _ => usage(),
    }
}

fn serve(config_dir: &std::path::Path, rest: &[&str]) {
    let mut bind = "127.0.0.1:0".to_string();
    let mut token_stdin = false;
    let mut ui: Option<PathBuf> = None;
    let mut allow_hosts: Vec<String> = Vec::new();
    let mut daemon: Option<PathBuf> = None;
    let mut for_app = false;
    let mut public_bind: Option<String> = None;
    let mut device_port_flag: Option<u16> = None;
    let mut tls_cert: Option<PathBuf> = None;
    let mut tls_key: Option<PathBuf> = None;
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "--bind" => bind = it.next().map(|s| s.to_string()).unwrap_or_else(|| usage()),
            "--token-stdin" => token_stdin = true,
            "--for-app" => for_app = true,
            "--public-bind" => {
                public_bind = Some(it.next().map(|s| s.to_string()).unwrap_or_else(|| usage()))
            }
            "--device-port" => {
                // 0 is a free port, as with --bind (a test's).
                let text = it.next().unwrap_or_else(|| usage());
                device_port_flag =
                    Some(text.parse::<u16>().unwrap_or_else(|_| {
                        fail(format!("--device-port: \"{text}\" is not a port"))
                    }));
            }
            "--tls-cert" => {
                tls_cert = Some(
                    it.next()
                        .map(|s| PathBuf::from(*s))
                        .unwrap_or_else(|| usage()),
                )
            }
            "--tls-key" => {
                tls_key = Some(
                    it.next()
                        .map(|s| PathBuf::from(*s))
                        .unwrap_or_else(|| usage()),
                )
            }
            "--ui" => {
                ui = Some(
                    it.next()
                        .map(|s| PathBuf::from(*s))
                        .unwrap_or_else(|| usage()),
                )
            }
            "--allow-host" => {
                allow_hosts.push(it.next().map(|s| s.to_string()).unwrap_or_else(|| usage()))
            }
            "--daemon" => {
                daemon = Some(
                    it.next()
                        .map(|s| PathBuf::from(*s))
                        .unwrap_or_else(|| usage()),
                )
            }
            _ => usage(),
        }
    }
    let bind: SocketAddr = bind
        .parse()
        .unwrap_or_else(|e| fail(format!("invalid --bind address: {e}")));
    // The owner's token is good on this listener: it is for this machine.
    if !bind.ip().is_loopback() {
        fail("--bind is for this machine (a loopback address); other devices come in on --public-bind");
    }

    // What this run was told about other devices, over what is saved
    // (`host.db`): never kept.
    let overrides = oxplow_host_access::settings::Overrides {
        address: public_bind.map(|b| {
            b.parse().unwrap_or_else(|e| {
                fail(format!(
                    "invalid --public-bind address (an IP address; the port is --device-port): {e}"
                ))
            })
        }),
        port: device_port_flag,
        port_env: std::env::var(oxplow_host_access::settings::PORT_ENV).ok(),
        names: (!allow_hosts.is_empty()).then_some(allow_hosts),
        tls: match (tls_cert, tls_key) {
            (None, None) => None,
            (Some(cert), Some(key)) => Some((cert, key)),
            _ => fail("--tls-cert and --tls-key go together"),
        },
    };
    if for_app && !token_stdin {
        fail("--for-app needs --token-stdin: the app's stdin is the host's lifeline");
    }
    // The home's `host.db`, held for this host's life: one host per home.
    // The app's previous host may still be stopping its daemons after a quit.
    let db = oxplow_host::instance::acquire(config_dir, for_app).unwrap_or_else(|e| fail(e));
    let (token, generated) = if token_stdin {
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() || line.trim().is_empty() {
            fail("--token-stdin but no token on stdin");
        }
        (line.trim().to_string(), false)
    } else {
        (generate_token(), true)
    };

    let stop = Arc::new(tokio::sync::Notify::new());
    if for_app {
        let lifeline = stop.clone();
        std::thread::Builder::new()
            .name("oxplow-host-lifeline".into())
            .spawn(move || {
                // Whatever the app writes is ignored; the end of it is the
                // app's end (quit, crash or kill).
                let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
                tracing::info!("the app is gone");
                lifeline.notify_one();
            })
            .unwrap_or_else(|e| fail(format!("cannot watch the app: {e}")));
    }

    oxplow_host::logging::init(config_dir);
    tracing::info!(
        pid = std::process::id(),
        version = oxplow_host::VERSION,
        "oxplow-host starting"
    );

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(async move {
            let launcher: Box<dyn DaemonLauncher> = match daemon {
                Some(path) => Box::new(BinaryDaemon(path)),
                None => Box::new(BundledDaemon),
            };
            let supervisor = Arc::new(DaemonSupervisor::with_launcher(launcher));
            let kept = if for_app {
                HoldsKept::InMemory
            } else {
                HoldsKept::InFile
            };
            let host = Host::open(db.clone(), supervisor, kept).unwrap_or_else(|e| fail(e));
            let access = oxplow_host::access::Access::new(token.clone(), db.clone(), overrides);
            use oxplow_host::access::Listener;
            let listener = tokio::net::TcpListener::bind(bind)
                .await
                .unwrap_or_else(|e| fail(format!("bind {bind} failed: {e}")));
            let addr = listener.local_addr().unwrap_or_else(|e| fail(e));
            let base_url = format!("http://{addr}");

            let watcher = host.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_millis(500));
                loop {
                    tick.tick().await;
                    let host = watcher.clone();
                    let _ = tokio::task::spawn_blocking(move || host.poll_exits()).await;
                }
            });
            let ui_dir = ui.or_else(default_ui_dir);
            match &ui_dir {
                Some(dir) => tracing::info!(ui = %dir.display(), "serving the UI"),
                None => tracing::warn!("no UI folder found; pass --ui <dir>"),
            }
            // The home's own root, once the listener for other devices has a
            // certificate from it: a device downloads it to trust the host.
            let root_for_devices: Arc<std::sync::Mutex<Option<String>>> = Default::default();
            // The names every listener answers to; the listener for other
            // devices sets them as the settings change.
            let allowed = oxplow_host::web::AllowedHosts::default();
            let config = {
                let (root_pem, host, access) =
                    (root_for_devices.clone(), host.clone(), access.clone());
                let (ui_dir, allowed, stop) = (ui_dir.clone(), allowed.clone(), stop.clone());
                move |scheme, listener| oxplow_host::web::ServeConfig {
                    root_pem: root_pem.clone(),
                    host: host.clone(),
                    access: access.clone(),
                    listener,
                    ui_dir: ui_dir.clone(),
                    allow_hosts: allowed.clone(),
                    // A daemon that boots no faster than this is a crash.
                    boot_wait: DEFAULT_STARTUP_TIMEOUT,
                    stop: stop.clone(),
                    scheme,
                }
            };
            let app =
                oxplow_host::web::app(config(oxplow_host::web::Scheme::Http, Listener::Loopback));
            // The CLI's way in (`oxplow host pair <code>`): the owner, by the file's
            // mode. A headless host's owner token is printed once and kept
            // nowhere, so this is how it pairs a device.
            #[cfg(unix)]
            let owner_socket = owner_socket(
                config_dir,
                oxplow_host::web::app(config(
                    oxplow_host::web::Scheme::Http,
                    Listener::OwnerSocket,
                )),
            );
            // Said once everything this machine reaches is up: the app (and
            // the CLI's socket) may use it from this line on.
            println!("oxplow-host listening on {base_url}");
            if generated {
                println!("  host token: {token}");
            }
            tracing::info!(%base_url, "oxplow-host listening");
            // Other devices, on their own listener (HTTPS only), as the
            // settings in force say; a saved change applies at once. One
            // that cannot be had is said and left out: this machine's
            // windows still work, and the Devices page says why.
            let public = {
                let config = config.clone();
                oxplow_host::public::PublicServer::new(
                    config_dir,
                    access.clone(),
                    root_for_devices.clone(),
                    allowed.clone(),
                    Box::new(move || {
                        oxplow_host::web::app(config(
                            oxplow_host::web::Scheme::Https,
                            Listener::Public,
                        ))
                    }),
                )
            };
            let _ = access.public_server.set(public.clone());
            let started = match access.other_devices.effective() {
                Ok(in_force) => public.apply(&in_force).await,
                Err(e) => Err(e.to_string()),
            };
            match started {
                Err(error) => {
                    eprintln!("oxplow-host: other devices cannot reach this host: {error}");
                    tracing::error!(%error, "the listener for other devices is left out");
                    *access.public.lock().unwrap_or_else(|e| e.into_inner()) =
                        oxplow_host::access::PublicListener::Failed { error };
                }
                Ok(()) => {
                    if let oxplow_host::access::PublicListener::Listening { url, .. } =
                        &*access.public.lock().unwrap_or_else(|e| e.into_inner())
                    {
                        println!("oxplow-host listening for other devices on {url}");
                    }
                }
            }
            let reason = tokio::select! {
                served = axum::serve(listener, app) => format!("the server stopped: {served:?}"),
                signal = terminated() => format!("received {signal}"),
                _ = stop.notified() => "asked to stop".to_string(),
            };
            tracing::info!("oxplow-host stopping: {reason}");
            // Each page says why its socket went ("Oxplow was quit").
            access
                .tunnels
                .close_all(oxplow_host::access::HOST_ENDING_CLOSE_CODE);
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            #[cfg(unix)]
            drop(owner_socket);
            let stopping = host.clone();
            let _ = tokio::task::spawn_blocking(move || stopping.shutdown()).await;
            std::process::exit(0);
        });
}

/// `host.sock` in the home, mode 0600, serving `app` as the owner. Removed
/// when the returned guard drops.
#[cfg(unix)]
fn owner_socket(config_dir: &std::path::Path, app: axum::Router) -> Option<OwnerSocket> {
    use std::os::unix::fs::PermissionsExt;
    let path = oxplow_host_access::owner::owner_socket_path(config_dir);
    // The lock is ours, so a socket file here is a killed host's.
    let _ = std::fs::remove_file(&path);
    let listener = match tokio::net::UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "no owner socket; the CLI cannot reach this host");
            return None;
        }
    };
    if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!(error = %e, "could not limit the owner socket to this user; leaving it out");
        let _ = std::fs::remove_file(&path);
        return None;
    }
    tokio::spawn(async move { axum::serve(listener, app).await });
    Some(OwnerSocket(path))
}

#[cfg(unix)]
struct OwnerSocket(PathBuf);

#[cfg(unix)]
impl Drop for OwnerSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The built UI when `--ui` is not given: a `ui` folder beside the binary
/// (packaged), else `apps/desktop/dist` of the checkout the binary was built
/// in (`target/<profile>/oxplow-host`).
fn default_ui_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let beside = exe.parent()?;
    [
        beside.join("ui"),
        beside.join("../Resources/ui"),
        beside.join("../../apps/desktop/dist"),
    ]
    .into_iter()
    .find(|dir| dir.join("index.html").is_file())
}

fn generate_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

#[cfg(unix)]
async fn terminated() -> &'static str {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut term), Ok(mut int), Ok(mut hup)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
        signal(SignalKind::hangup()),
    ) else {
        return std::future::pending().await;
    };
    tokio::select! {
        _ = term.recv() => "SIGTERM",
        _ = int.recv() => "SIGINT",
        _ = hup.recv() => "SIGHUP",
    }
}

#[cfg(not(unix))]
async fn terminated() -> &'static str {
    let _ = tokio::signal::ctrl_c().await;
    "Ctrl-C"
}
