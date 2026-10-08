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
    let args = parse_args(name);
    // The UI token: from the supervising shell over stdin (never an env
    // var or a file an agent could read), or fresh for a hand-started
    // daemon, printed below for the person to use. Read before anything
    // else; what follows it on stdin is the lifeline, watched below.
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
        } else {
            eprintln!(
                "{name}: {} is not an oxplow project (no .oxplow/). \
                 Pass --init to create it, or open it once in the desktop app.",
                project_dir.display()
            );
            std::process::exit(1);
        }
    }

    init_logging(&project_dir);
    log_panics();
    if args.init {
        tracing::info!(project = %project_dir.display(), "--init: the project's .oxplow/ is ready");
    }
    tracing::info!(pid = std::process::id(), project = %project_dir.display(), "{name} starting");
    if args.token_stdin {
        // The rest of stdin is the lifeline to the app that started us.
        oxplow_app::daemon_supervisor::stop_when_app_goes(project_dir.clone());
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
    oxplow_app::diagnostics::spawn_watchdog(log_dir(&project_dir));

    // Hook + MCP control plane (agents spawned on this box
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
    // Publish the endpoint for a shell that didn't spawn us: a second
    // app opening this project reads it and defers (tsk1063). The stdout
    // line below is what a supervising shell reads at spawn.
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

    // Serve until the server stops or a signal asks us to; either way the
    // reason is logged (tsk1070). A SIGKILL leaves no trace and the file
    // behind, which is why `DaemonSupervisor::stop` clears it too.
    let reason = tokio::select! {
        served = daemon.task => format!("the server stopped: {served:?}"),
        signal = terminated() => format!("received {signal}"),
    };
    tracing::info!("{name} stopping: {reason}");
    oxplow_app::daemon_supervisor::clear_daemon_info(&project_dir);
    std::process::exit(0);
}

/// Where the daemon's logs and stall samples go: `.oxplow/logs/`.
fn log_dir(project_dir: &std::path::Path) -> PathBuf {
    project_dir.join(".oxplow").join("logs")
}

/// Log to stderr (the supervising app's) and to
/// `.oxplow/logs/daemon.<date>.log`, daily, the last week kept: a packaged
/// app's stderr goes nowhere, and a daemon that stopped or hung must leave
/// a trace (tsk1070). Written synchronously, so a crash loses no line.
fn init_logging(project_dir: &std::path::Path) {
    use tracing_subscriber::prelude::*;
    // The MCP library logs three INFO lines per agent tool connection
    // (opened, input ended, finished): routine, so only its warnings show
    // unless RUST_LOG asks (tsk1077).
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,rmcp=warn"));
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("daemon")
        .filename_suffix("log")
        .max_log_files(7)
        .build(log_dir(project_dir));
    let file_error = file.as_ref().err().map(ToString::to_string);
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(file.ok().map(|f| {
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(f)
        }))
        .init();
    if let Some(e) = file_error {
        tracing::warn!(error = %e, "no log file; logging to stderr only");
    }
}

/// Log a panic, with its thread and backtrace, before the default hook
/// prints it: a panic in a task otherwise reaches only stderr.
fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        tracing::error!(
            thread = thread.name().unwrap_or("unnamed"),
            backtrace = %std::backtrace::Backtrace::force_capture(),
            "panic: {info}"
        );
        default(info);
    }));
}

/// The first of SIGTERM, SIGINT or SIGHUP, by name.
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
