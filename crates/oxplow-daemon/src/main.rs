//! `oxplow-daemon --project <dir> [--bind 127.0.0.1:7420] [--init]`
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
        "usage: oxplow-daemon --project <dir> [--bind 127.0.0.1:7420] [--init]\n\
         \n\
         --init creates the project (`.oxplow/`) if it doesn't exist yet,\n\
         instead of refusing — handy for scripting / profiling a fresh\n\
         project without opening the desktop setup flow first.\n\
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
    /// Create `.oxplow/` if the target dir isn't a project yet.
    init: bool,
}

/// Hand-rolled arg parsing — not worth a clap dependency.
fn parse_args() -> Args {
    let mut project: Option<PathBuf> = None;
    let mut bind: Option<String> = None;
    let mut init = false;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--project" => project = it.next().map(PathBuf::from),
            "--bind" => bind = it.next(),
            "--init" => init = true,
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
    Args {
        project_dir,
        bind,
        init,
    }
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
        if args.init {
            if let Err(e) = std::fs::create_dir_all(project_dir.join(".oxplow")) {
                eprintln!(
                    "oxplow-daemon: could not create .oxplow/ in {}: {e}",
                    project_dir.display()
                );
                std::process::exit(1);
            }
            tracing::info!(
                project = %project_dir.display(),
                "created new oxplow project (.oxplow/) via --init",
            );
        } else {
            eprintln!(
                "oxplow-daemon: {} is not an oxplow project (no .oxplow/). \
                 Pass --init to create it, or open it once in the desktop app.",
                project_dir.display()
            );
            std::process::exit(1);
        }
    }

    let layout = AppLayout::for_project(&project_dir);

    // Same per-project single-instance guard as the desktop shell —
    // two processes on one `.oxplow/local.sqlite` would double the
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
        ctx: oxplow_rpc::RpcContext {
            services: state,
            plugin_runtime: Some(oxplow_rpc::PluginRuntime {
                hook_base_url: control_plane.hook_base_url(),
                mcp_endpoint_url: control_plane.mcp_endpoint_url(),
                otlp_base_url: control_plane.otlp_base_url(),
                hook_token: control_plane.hook_token.clone(),
            }),
        },
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
