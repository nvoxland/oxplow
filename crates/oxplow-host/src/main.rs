//! `oxplow-host serve [--bind 127.0.0.1:0] [--token-stdin] [--ui <dir>]
//!   [--allow-host <name>]… [--daemon <path>] [--for-app]
//!   [--public-bind <ip> [--device-port <port>] [--tls-cert <pem> --tls-key <pem>]]`
//! `oxplow-host project <add <dir> | list | remove <id>>`
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
use oxplow_host::instance::HostInstance;
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
         \x20      oxplow-host pair | devices [remove <id>] | status | stop\n\
         \n\
         One host runs per OXPLOW_HOME. --token-stdin reads the host token from\n\
         the first line of stdin; otherwise a fresh one is generated and printed.\n\
         --ui is the built UI (default: beside the binary, or apps/desktop/dist in a\n\
         checkout). --allow-host names a host a request may reach this one by, besides\n\
         loopback. --daemon runs another daemon binary (the browser suite's).\n\
         --for-app makes it the Mac app's host: stdin after the token is its\n\
         lifeline, and it ends with the app. Without it the host is headless\n\
         and runs until it is stopped. --public-bind listens a second time, for\n\
         other devices (HTTPS, with the home's own certificate or --tls-cert and\n\
         --tls-key), on --device-port, else OXPLOW_DEVICE_PORT, else 7400; --bind\n\
         stays for this machine."
    );
    std::process::exit(2);
}

/// A call to the running host over its owner socket; fails with its words.
fn owner_call(config_dir: &std::path::Path, method: &str, path: &str) -> serde_json::Value {
    match oxplow_host::client::owner_call(config_dir, method, path) {
        Ok(value) => value,
        Err(e) => fail(e),
    }
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
            match Registry::new(&config_dir).add(std::path::Path::new(dir)) {
                Ok(p) => println!("{}  {}  {}", p.id, p.name, p.path.display()),
                Err(e) => fail(e),
            }
        }
        ["project", "list"] => {
            for p in Registry::new(&config_dir).list() {
                println!("{}  {}  {}", p.id, p.name, p.path.display());
            }
        }
        ["project", "remove", id] => match Registry::new(&config_dir).remove(id) {
            Ok(p) => println!("removed {}  {}", p.id, p.path.display()),
            Err(e) => fail(e),
        },
        ["pair"] => {
            let code = owner_call(&config_dir, "POST", "/host/pairing");
            println!(
                "pairing code: {}  (good for {} minutes, once)",
                code["code"].as_str().unwrap_or("?"),
                code["expires_in"].as_u64().unwrap_or(0) / 60
            );
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

    // Other devices' listener: an address of this machine's, on the port
    // `--device-port` names, else `OXPLOW_DEVICE_PORT`, else 7400.
    let public_bind: Option<SocketAddr> = public_bind.map(|b| {
        let ip: std::net::IpAddr = b.parse().unwrap_or_else(|e| {
            fail(format!(
                "invalid --public-bind address (an IP address; the port is --device-port): {e}"
            ))
        });
        let env = std::env::var(oxplow_host::settings::PORT_ENV).ok();
        let port = oxplow_host::settings::device_port(device_port_flag, env.as_deref(), None)
            .unwrap_or_else(|e| fail(e.to_string()));
        SocketAddr::new(ip, port.port)
    });
    if device_port_flag.is_some() && public_bind.is_none() {
        fail("--device-port is for --public-bind");
    }
    // Other devices come in over HTTPS only: with the certificate given
    // (signed by any CA), else one the home's own root issues for this
    // listener (`certs.rs`; a device trusts that root once). One that cannot
    // be had leaves the listener for other devices out, never a plain one in
    // its place; this machine's windows still work, and the Devices window
    // says why.
    let own_cert = match (tls_cert, tls_key) {
        (None, None) => None,
        (Some(_), Some(_)) if public_bind.is_none() => {
            fail("--tls-cert and --tls-key are for --public-bind")
        }
        (Some(cert), Some(key)) => Some((cert, key)),
        _ => fail("--tls-cert and --tls-key go together"),
    };
    let tls = public_bind.map(|addr| match &own_cert {
        Some((cert, key)) => oxplow_host::tls::acceptor(cert, key)
            .map(|a| (a, None))
            .map_err(|e| e.to_string()),
        None => issued_acceptor(config_dir, addr, &allow_hosts),
    });
    if for_app && !token_stdin {
        fail("--for-app needs --token-stdin: the app's stdin is the host's lifeline");
    }
    // The app's previous host may still be stopping its daemons after a quit.
    let instance = if for_app {
        HostInstance::acquire_waiting(config_dir, APP_HOST_LOCK_WAIT)
    } else {
        HostInstance::acquire(config_dir)
    }
    .unwrap_or_else(|e| fail(e));
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
            let host = Host::open(config_dir, supervisor, kept);
            let devices =
                oxplow_host::devices::Devices::open(config_dir).unwrap_or_else(|e| fail(e));
            let access = oxplow_host::access::Access::new(
                token.clone(),
                devices,
                oxplow_host::access::WRONG_CODE_WAIT,
            );
            use oxplow_host::access::Listener;
            let listener = tokio::net::TcpListener::bind(bind)
                .await
                .unwrap_or_else(|e| fail(format!("bind {bind} failed: {e}")));
            let addr = listener.local_addr().unwrap_or_else(|e| fail(e));
            let base_url = format!("http://{addr}");
            if let Err(e) = instance.publish(&base_url, for_app) {
                fail(format!("cannot write host.json: {e}"));
            }

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
            let config = |scheme, listener| oxplow_host::web::ServeConfig {
                root_pem: root_for_devices.clone(),
                host: host.clone(),
                access: access.clone(),
                listener,
                ui_dir: ui_dir.clone(),
                allow_hosts: allow_hosts.clone(),
                // A daemon that boots no faster than this is a crash.
                boot_wait: DEFAULT_STARTUP_TIMEOUT,
                stop: stop.clone(),
                scheme,
            };
            let app =
                oxplow_host::web::app(config(oxplow_host::web::Scheme::Http, Listener::Loopback));
            // The CLI's way in (`oxplow host pair`): the owner, by the file's
            // mode. A headless host's owner token is printed once and kept
            // nowhere, so this is how it makes a pairing code.
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
            // Other devices, on their own listener: HTTPS when a certificate
            // was given.
            // One that cannot bind is said and left out: this machine's
            // windows still work.
            let left_out = |error: String| {
                eprintln!("oxplow-host: other devices cannot reach this host: {error}");
                tracing::error!(%error, "the listener for other devices is left out");
                *access.public.lock().unwrap_or_else(|e| e.into_inner()) =
                    oxplow_host::access::PublicListener::Failed { error };
            };
            let (tls, root_pem) = match tls {
                Some(Err(e)) => {
                    left_out(e);
                    (None, None)
                }
                Some(Ok((acceptor, root))) => (Some(acceptor), root),
                None => (None, None),
            };
            *root_for_devices.lock().unwrap_or_else(|e| e.into_inner()) = root_pem;
            let public_tcp = match public_bind {
                None => None,
                Some(_) if tls.is_none() => None,
                Some(addr) => match tokio::net::TcpListener::bind(addr).await {
                    Ok(tcp) => Some(tcp),
                    Err(e) => {
                        left_out(format!("bind {addr} failed: {e}"));
                        None
                    }
                },
            };
            let public = match public_tcp {
                None => None,
                Some(tcp) => {
                    let at = tcp.local_addr().unwrap_or_else(|e| fail(e));
                    let scheme = oxplow_host::web::Scheme::Https;
                    println!(
                        "oxplow-host listening for other devices on {}://{at}",
                        scheme.as_str()
                    );
                    *access.public.lock().unwrap_or_else(|e| e.into_inner()) =
                        oxplow_host::access::PublicListener::Listening {
                            url: format!("{}://{at}", scheme.as_str()),
                        };
                    tracing::info!(%at, scheme = scheme.as_str(), "listening for other devices");
                    let public_app = oxplow_host::web::app(config(scheme, Listener::Public));
                    let acceptor = tls.clone().expect("a public listener has its TLS");
                    let listener = oxplow_host::tls::TlsListener::new(tcp, acceptor)
                        .unwrap_or_else(|e| fail(e));
                    Some(tokio::spawn(async move {
                        axum::serve(listener, public_app).await
                    }))
                }
            };
            let public_ended = async {
                match public {
                    Some(task) => format!("the public listener stopped: {:?}", task.await),
                    None => std::future::pending().await,
                }
            };
            let reason = tokio::select! {
                served = axum::serve(listener, app) => format!("the server stopped: {served:?}"),
                ended = public_ended => ended,
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
            drop(instance);
            std::process::exit(0);
        });
}

/// `host.sock` in the home, mode 0600, serving `app` as the owner. Removed
/// when the returned guard drops.
#[cfg(unix)]
fn owner_socket(config_dir: &std::path::Path, app: axum::Router) -> Option<OwnerSocket> {
    use std::os::unix::fs::PermissionsExt;
    let path = oxplow_host::client::owner_socket_path(config_dir);
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

/// TLS for the listener for other devices at `addr`, from the home's own
/// root: a certificate for that address (every private address of this
/// machine when it is unspecified) and the allowed names, issued now. The
/// root's PEM comes too, for devices to download (`/host/ca.pem`).
fn issued_acceptor(
    config_dir: &std::path::Path,
    addr: SocketAddr,
    names: &[String],
) -> Result<(tokio_rustls::TlsAcceptor, Option<String>), String> {
    use oxplow_host::certs;
    let root = certs::LocalRoot::load_or_make(config_dir).map_err(|e| e.to_string())?;
    let ips = if addr.ip().is_unspecified() {
        certs::private_addresses()
    } else {
        vec![addr.ip()]
    };
    let (cert, key) = root.issue(&ips, names).map_err(|e| {
        format!("{e}; give a certificate of your own (--tls-cert, --tls-key) for that")
    })?;
    let acceptor = oxplow_host::tls::acceptor_pem(
        cert.as_bytes(),
        key.as_bytes(),
        "the issued certificate",
        "its key",
    )
    .map_err(|e| e.to_string())?;
    let root_pem =
        std::fs::read_to_string(certs::root_path(config_dir)).map_err(|e| e.to_string())?;
    Ok((acceptor, Some(root_pem)))
}

/// How long the app's host waits for the lock of the app's previous host,
/// which stops its daemons (a few seconds each) before it lets go.
const APP_HOST_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

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
