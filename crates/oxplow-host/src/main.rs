//! `oxplow-host serve [--bind 127.0.0.1:0] [--token-stdin]`
//! `oxplow-host project <add <dir> | list | remove <id>>`
//!
//! The host has no stdin lifeline: it must outlive the app that started it.
//! Its daemons have one, held by the host.

use std::net::SocketAddr;
use std::sync::Arc;

use oxplow_app::daemon_supervisor::{BundledDaemon, DaemonSupervisor};
use oxplow_host::host::{Host, DEFAULT_RESTART_AFTER};
use oxplow_host::instance::HostInstance;
use oxplow_host::registry::Registry;

fn usage() -> ! {
    eprintln!(
        "usage: oxplow-host serve [--bind 127.0.0.1:0] [--token-stdin]\n\
         \x20      oxplow-host project add <dir>\n\
         \x20      oxplow-host project list\n\
         \x20      oxplow-host project remove <id>\n\
         \n\
         One host runs per OXPLOW_HOME. --token-stdin reads the host token from\n\
         the first line of stdin; otherwise a fresh one is generated and printed."
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
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "--bind" => bind = it.next().map(|s| s.to_string()).unwrap_or_else(|| usage()),
            "--token-stdin" => token_stdin = true,
            _ => usage(),
        }
    }
    let bind: SocketAddr = bind
        .parse()
        .unwrap_or_else(|e| fail(format!("invalid --bind address: {e}")));

    let instance = HostInstance::acquire(config_dir).unwrap_or_else(|e| fail(e));
    let (token, generated) = if token_stdin {
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() || line.trim().is_empty() {
            fail("--token-stdin but no token on stdin");
        }
        (line.trim().to_string(), false)
    } else {
        (generate_token(), true)
    };

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
            let supervisor = Arc::new(DaemonSupervisor::with_launcher(Box::new(BundledDaemon)));
            let host = Host::open(config_dir, supervisor, DEFAULT_RESTART_AFTER);
            let listener = tokio::net::TcpListener::bind(bind)
                .await
                .unwrap_or_else(|e| fail(format!("bind {bind} failed: {e}")));
            let addr = listener.local_addr().unwrap_or_else(|e| fail(e));
            let base_url = format!("http://{addr}");
            if let Err(e) = instance.publish(&base_url) {
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
            let app = oxplow_host::api::router(host.clone(), token);
            let reason = tokio::select! {
                served = axum::serve(listener, app) => format!("the server stopped: {served:?}"),
                signal = terminated() => format!("received {signal}"),
            };
            tracing::info!("oxplow-host stopping: {reason}");
            let stopping = host.clone();
            let _ = tokio::task::spawn_blocking(move || stopping.shutdown()).await;
            drop(instance);
            std::process::exit(0);
        });
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
