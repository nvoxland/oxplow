//! `oxplow-daemon --project <dir> [--bind 127.0.0.1:7420]`
//!
//! Headless backend entrypoint. See lib.rs for the HTTP surface and
//! the crate docs for the SSH-tunnel deployment model.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use oxplow_app::{AppLayout, Services};
use oxplow_daemon::{run_server, DaemonState};

/// Default port; chosen to be memorable and outside common dev-server
/// ranges. Override with `--bind`.
const DEFAULT_BIND: &str = "127.0.0.1:7420";

fn usage() -> ! {
    eprintln!(
        "usage: oxplow-daemon --project <dir> [--bind 127.0.0.1:7420]\n\
         \n\
         The project dir may also come from OXPLOW_PROJECT_DIR. The\n\
         daemon binds loopback only — reach it from another machine\n\
         via: ssh -L <localPort>:127.0.0.1:<port> <host>"
    );
    std::process::exit(2);
}

struct Args {
    project_dir: PathBuf,
    bind: SocketAddr,
}

/// Hand-rolled arg parsing (two flags) — not worth a clap dependency.
fn parse_args() -> Args {
    let mut project: Option<PathBuf> = None;
    let mut bind: Option<String> = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--project" => project = it.next().map(PathBuf::from),
            "--bind" => bind = it.next(),
            "--help" | "-h" => usage(),
            other => {
                eprintln!("unknown argument: {other}");
                usage();
            }
        }
    }
    let project_dir = project
        .or_else(|| std::env::var_os("OXPLOW_PROJECT_DIR").map(PathBuf::from))
        .unwrap_or_else(|| usage());
    let bind = bind
        .unwrap_or_else(|| DEFAULT_BIND.to_string())
        .parse()
        .unwrap_or_else(|e| {
            eprintln!("invalid --bind address: {e}");
            usage();
        });
    Args { project_dir, bind }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = parse_args();
    let project_dir = args.project_dir.canonicalize().unwrap_or_else(|e| {
        eprintln!(
            "oxplow-daemon: project dir {} not accessible: {e}",
            args.project_dir.display()
        );
        std::process::exit(1);
    });
    if !project_dir.join(".oxplow").is_dir() {
        eprintln!(
            "oxplow-daemon: {} is not an oxplow project (no .oxplow/). \
             Open it once in the desktop app to set it up.",
            project_dir.display()
        );
        std::process::exit(1);
    }

    let layout = AppLayout::for_project(&project_dir);

    // Same per-project single-instance guard as the desktop shell —
    // two processes on one `.oxplow/state.sqlite` would double the
    // watchers and contend on SQLite's writer lock.
    match oxplow_app::try_acquire_instance_lock(&layout) {
        Ok(Some(lock)) => {
            Box::leak(Box::new(lock));
        }
        Ok(None) => {
            eprintln!(
                "oxplow-daemon: project already open in another oxplow process: {}",
                layout.project_dir.display()
            );
            std::process::exit(1);
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to acquire instance lock; continuing without guard");
        }
    }

    let services = Services::boot(layout).unwrap_or_else(|e| {
        eprintln!("oxplow-daemon: services boot failed: {e}");
        std::process::exit(1);
    });
    let state = Arc::new(services);

    // Recovery + primary stream + the standard background fleet —
    // identical to the desktop shell's boot.
    oxplow_app::boot::run_boot_orchestration(&state).await;

    // Hook + MCP control plane (agents spawned in tmux on this box
    // talk to it over its own loopback listener).
    let control_plane = oxplow_control_plane::spawn(state.clone())
        .await
        .unwrap_or_else(|e| {
            eprintln!("oxplow-daemon: control plane boot failed: {e}");
            std::process::exit(1);
        });

    let daemon_state = DaemonState {
        services: state,
        hook_base_url: control_plane.hook_base_url(),
        mcp_endpoint_url: control_plane.mcp_endpoint_url(),
        hook_token: control_plane.hook_token.clone(),
    };

    let daemon = run_server(args.bind, daemon_state)
        .await
        .unwrap_or_else(|e| {
            eprintln!("oxplow-daemon: bind {} failed: {e}", args.bind);
            std::process::exit(1);
        });

    tracing::info!(
        addr = %daemon.bind_addr,
        project = %project_dir.display(),
        "oxplow-daemon ready"
    );
    println!("oxplow-daemon listening on http://{}", daemon.bind_addr);
    println!(
        "  tunnel: ssh -L {0}:127.0.0.1:{0} <host>",
        daemon.bind_addr.port()
    );

    // Serve until killed.
    let _ = daemon.task.await;
}
