//! `oxplow-host serve [--bind 127.0.0.1:0] [--token-stdin] [--ui <dir>]
//!   [--allow-host <name>]… [--daemon <path>] [--for-app]
//!   [--public-bind <addr> [--tls-cert <pem> --tls-key <pem>]]`
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
         \x20                        [--public-bind <addr> [--tls-cert <pem> --tls-key <pem>]]\n\
         \x20      oxplow-host project add <dir>\n\
         \x20      oxplow-host project list\n\
         \x20      oxplow-host project remove <id>\n\
         \n\
         One host runs per OXPLOW_HOME. --token-stdin reads the host token from\n\
         the first line of stdin; otherwise a fresh one is generated and printed.\n\
         --ui is the built UI (default: beside the binary, or apps/desktop/dist in a\n\
         checkout). --allow-host names a host a request may reach this one by, besides\n\
         loopback. --daemon runs another daemon binary (the browser suite's).\n\
         --for-app makes it the Mac app's host: stdin after the token is its\n\
         lifeline, and it ends with the app. Without it the host is headless\n\
         and runs until it is stopped. --public-bind listens a second time, for\n\
         other devices (HTTPS with --tls-cert and --tls-key); --bind stays for\n\
         this machine."
    );
    std::process::exit(2);
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

    let public_bind: Option<SocketAddr> = public_bind.map(|b| {
        b.parse()
            .unwrap_or_else(|e| fail(format!("invalid --public-bind address: {e}")))
    });
    let tls = match (tls_cert, tls_key) {
        (None, None) => None,
        (Some(cert), Some(key)) if public_bind.is_some() => {
            Some(oxplow_host::tls::acceptor(&cert, &key).unwrap_or_else(|e| fail(e)))
        }
        (Some(_), Some(_)) => fail("--tls-cert and --tls-key are for --public-bind"),
        _ => fail("--tls-cert and --tls-key go together"),
    };
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
            let listener = tokio::net::TcpListener::bind(bind)
                .await
                .unwrap_or_else(|e| fail(format!("bind {bind} failed: {e}")));
            let addr = listener.local_addr().unwrap_or_else(|e| fail(e));
            let base_url = format!("http://{addr}");
            if let Err(e) = instance.publish(&base_url, for_app) {
                fail(format!("cannot write host.json: {e}"));
            }
            println!("oxplow-host listening on {base_url}");
            if generated {
                println!("  host token: {token}");
            }
            tracing::info!(%base_url, "oxplow-host listening");

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
            let config = |scheme| oxplow_host::web::ServeConfig {
                host: host.clone(),
                token: token.clone(),
                ui_dir: ui_dir.clone(),
                allow_hosts: allow_hosts.clone(),
                // A daemon that boots no faster than this is a crash.
                boot_wait: DEFAULT_STARTUP_TIMEOUT,
                stop: stop.clone(),
                scheme,
            };
            let app = oxplow_host::web::app(config(oxplow_host::web::Scheme::Http));
            // Other devices, on their own listener: HTTPS when a certificate
            // was given.
            let public = match public_bind {
                None => None,
                Some(addr) => {
                    let tcp = tokio::net::TcpListener::bind(addr)
                        .await
                        .unwrap_or_else(|e| fail(format!("bind {addr} failed: {e}")));
                    let at = tcp.local_addr().unwrap_or_else(|e| fail(e));
                    let scheme = if tls.is_some() {
                        oxplow_host::web::Scheme::Https
                    } else {
                        oxplow_host::web::Scheme::Http
                    };
                    println!(
                        "oxplow-host listening for other devices on {}://{at}",
                        scheme.as_str()
                    );
                    tracing::info!(%at, scheme = scheme.as_str(), "listening for other devices");
                    let public_app = oxplow_host::web::app(config(scheme));
                    Some(match tls.clone() {
                        Some(acceptor) => {
                            let listener = oxplow_host::tls::TlsListener::new(tcp, acceptor)
                                .unwrap_or_else(|e| fail(e));
                            tokio::spawn(async move { axum::serve(listener, public_app).await })
                        }
                        None => tokio::spawn(async move { axum::serve(tcp, public_app).await }),
                    })
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
            let stopping = host.clone();
            let _ = tokio::task::spawn_blocking(move || stopping.shutdown()).await;
            drop(instance);
            std::process::exit(0);
        });
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
