//! A daemon binary's run: arguments, boot, serve (`oxplow-daemon`, and
//! the browser suite's `oxplow-daemon-sim`, which differ only in where
//! their secrets live).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use oxplow_app::{AppLayout, Services};

use crate::{new_token, run_server, DaemonState};

/// Default port; chosen to be memorable and outside common dev-server
/// ranges. Override with `--bind`.
const DEFAULT_BIND: &str = "127.0.0.1:7420";

fn usage(name: &str) -> ! {
    eprintln!(
        "usage: {name} --project <dir> [--bind 127.0.0.1:7420] [--init] [--token-stdin]\n\
         \n\
         --init creates the project (`.oxplow/`) if it doesn't exist yet,\n\
         instead of refusing — handy for scripting / profiling a fresh\n\
         project without opening the desktop setup flow first.\n\
         \n\
         --token-stdin reads the UI token from the first line of stdin\n\
         (how the desktop shell hands it over). Otherwise a fresh token\n\
         is generated and printed; the UI must present it (launcher\n\
         connect, or VITE_OXPLOW_REMOTE_TOKEN in dev).\n\
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
    /// Read the UI token from stdin instead of generating one.
    token_stdin: bool,
}

/// Hand-rolled arg parsing — not worth a clap dependency.
fn parse_args(name: &str) -> Args {
    let mut project: Option<PathBuf> = None;
    let mut bind: Option<String> = None;
    let mut init = false;
    let mut token_stdin = false;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--project" => project = it.next().map(PathBuf::from),
            "--bind" => bind = it.next(),
            "--init" => init = true,
            "--token-stdin" => token_stdin = true,
            "--help" | "-h" => usage(name),
            other => {
                eprintln!("unknown argument: {other}");
                usage(name);
            }
        }
    }
    let project_dir = project
        .or_else(|| std::env::var_os("OXPLOW_PROJECT_DIR").map(PathBuf::from))
        .unwrap_or_else(|| usage(name));
    let bind = bind
        .unwrap_or_else(|| DEFAULT_BIND.to_string())
        .parse()
        .unwrap_or_else(|e| {
            eprintln!("invalid --bind address: {e}");
            usage(name);
        });
    Args {
        project_dir,
        bind,
        init,
        token_stdin,
    }
}

/// A daemon binary's whole run: parse the arguments, boot the project
/// with `secrets`, serve until killed. `name` is the binary's, for its
/// messages. The shipped `oxplow-daemon` passes the OS keychain, always;
/// `oxplow-daemon-sim` (the browser suite's, dev-only) passes memory.
pub async fn run_main(name: &str, secrets: Arc<dyn oxplow_ai::secrets::SecretStore>) {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = parse_args(name);
    // The UI token: from the supervising shell over stdin (never an env
    // var or a file an agent could read), or fresh for a hand-started
    // daemon, printed below for the person to use.
    let (token, generated) = if args.token_stdin {
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() || line.trim().is_empty() {
            eprintln!("{name}: --token-stdin but no token on stdin");
            std::process::exit(2);
        }
        (line.trim().to_string(), false)
    } else {
        (new_token(), true)
    };
    let project_dir = args.project_dir.canonicalize().unwrap_or_else(|e| {
        eprintln!(
            "{name}: project dir {} not accessible: {e}",
            args.project_dir.display()
        );
        std::process::exit(1);
    });
    if !project_dir.join(".oxplow").is_dir() {
        if args.init {
            if let Err(e) = std::fs::create_dir_all(project_dir.join(".oxplow")) {
                eprintln!(
                    "{name}: could not create .oxplow/ in {}: {e}",
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
                "{name}: {} is not an oxplow project (no .oxplow/). \
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
                "{name}: project already open in another oxplow process: {}",
                layout.project_dir.display()
            );
            std::process::exit(1);
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to acquire instance lock; continuing without guard");
        }
    }

    let services = Services::boot(layout, secrets).unwrap_or_else(|e| {
        eprintln!("{name}: services boot failed: {e}");
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
            eprintln!("{name}: control plane boot failed: {e}");
            std::process::exit(1);
        });

    let daemon_state = DaemonState {
        token: token.clone(),
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
            eprintln!("{name}: bind {} failed: {e}", args.bind);
            std::process::exit(1);
        });

    tracing::info!(
        addr = %daemon.bind_addr,
        project = %project_dir.display(),
        "daemon ready"
    );
    // Publish the endpoint for a shell that didn't spawn us — the
    // orphan sweep after a shell crash reads this (tsk256). The
    // stdout line below is what a supervising shell reads at spawn.
    let info = oxplow_app::daemon_supervisor::DaemonInfo {
        base_url: format!("http://{}", daemon.bind_addr),
        pid: std::process::id(),
    };
    if let Err(e) = oxplow_app::daemon_supervisor::write_daemon_info(&project_dir, &info) {
        tracing::warn!(error = %e, "could not publish daemon.json");
    }
    println!("{name} listening on http://{}", daemon.bind_addr);
    if generated {
        println!("  ui token: {token}");
    }
    println!(
        "  tunnel: ssh -L {0}:127.0.0.1:{0} <host>",
        daemon.bind_addr.port()
    );

    // Serve until killed.
    let _ = daemon.task.await;
    // Only reached on a clean server exit; a SIGTERM/SIGKILL from the
    // supervising shell leaves the file behind, which is why both
    // `DaemonSupervisor::stop` and the boot-time orphan sweep clear it
    // rather than trusting the daemon to.
    oxplow_app::daemon_supervisor::clear_daemon_info(&project_dir);
}
