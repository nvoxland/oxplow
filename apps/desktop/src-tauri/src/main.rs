// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

mod icon_tint;

use oxplow_app::{AppLayout, Services};
use oxplow_tauri_ipc::{
    specta_builder, AppState, PluginRuntime, PluginRuntimeState, OXPLOW_EVENT_CHANNEL,
};
use tauri::Emitter;

/// Set once the app is quitting as a whole (Cmd-Q / app exit). A
/// single window closing while this is false is a deliberate
/// per-window close (drop it from the restore set); during a full
/// quit we preserve the set so every open window comes back.
static QUITTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn main() {
    if let Some(event) = hook_event_arg() {
        run_hook_command(&event);
        return;
    }

    init_tracing();

    // `generate_context!` embeds the Info.plist and may expand only
    // once per binary — build it here and hand it to whichever mode
    // we boot into.
    let ctx = tauri::generate_context!();

    // Process-per-window model: this process serves exactly one
    // project, chosen at launch. A bare launch (Finder/Spotlight/dock,
    // no arg + no env) has no project and boots the launcher instead.
    // A dir that isn't an Oxplow project yet (no `.oxplow/`) boots the
    // setup-confirmation screen rather than silently initializing —
    // unless `--init` was passed, which creates the project and boots
    // straight in (scripting / profiling a fresh dir).
    match resolve_project_dir() {
        Some(dir) if dir.join(".oxplow").is_dir() => run_project(dir, ctx),
        Some(dir) if init_flag() => {
            if let Err(e) = std::fs::create_dir_all(dir.join(".oxplow")) {
                eprintln!(
                    "oxplow: could not create .oxplow/ in {}: {e}",
                    dir.display()
                );
                std::process::exit(1);
            }
            run_project(dir, ctx)
        }
        Some(dir) => run_setup(dir, ctx),
        // Bare launch: reopen the windows that were open at last exit;
        // if there were none, show the launcher.
        None if restore_session() => {}
        None => run_launcher(ctx),
    }
}

fn hook_event_arg() -> Option<String> {
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next(), args.next()) {
        (Some("hook"), Some(event), None) => Some(event),
        _ => None,
    }
}

/// `--init`: create `.oxplow/` for a fresh project dir and boot straight
/// in, instead of showing the setup-confirmation screen.
fn init_flag() -> bool {
    std::env::args().skip(1).any(|a| a == "--init")
}

fn run_hook_command(event: &str) {
    use std::io::{Read, Write};

    let mut payload = Vec::new();
    if let Err(err) = std::io::stdin().read_to_end(&mut payload) {
        eprintln!("oxplow hook failed to read stdin: {err}");
        return;
    }
    if payload.is_empty() {
        payload.extend_from_slice(b"{}");
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("oxplow hook failed to start runtime: {err}");
            return;
        }
    };

    let result = runtime.block_on(forward_hook(event, payload));
    match result {
        Ok(body) if !body.is_empty() => {
            if let Err(err) = std::io::stdout().write_all(&body) {
                eprintln!("oxplow hook failed to write response: {err}");
            }
        }
        Ok(_) => {}
        Err(err) => eprintln!("oxplow hook forwarding failed: {err}"),
    }
}

async fn forward_hook(event: &str, payload: Vec<u8>) -> Result<Vec<u8>, reqwest::Error> {
    let base_url = std::env::var("OXPLOW_HOOK_BASE_URL").unwrap_or_default();
    let url = format!("{}/{}", base_url.trim_end_matches('/'), event);
    let token = std::env::var("OXPLOW_HOOK_TOKEN").unwrap_or_default();

    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .post(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header(
            "X-Oxplow-Stream",
            std::env::var("OXPLOW_STREAM_ID").unwrap_or_default(),
        )
        .header(
            "X-Oxplow-Thread",
            std::env::var("OXPLOW_THREAD_ID").unwrap_or_default(),
        )
        .header(
            "X-Oxplow-Pane",
            std::env::var("OXPLOW_PANE").unwrap_or_default(),
        )
        .body(payload)
        .send()
        .await?;

    Ok(response.bytes().await?.to_vec())
}

/// The environment [`restore_session_with`] acts on, injected so the
/// restore policy can be tested without real processes, advisory locks,
/// or a global session file.
struct RestoreOps<'a> {
    /// Is this dir still an Oxplow project?
    is_project: &'a dyn Fn(&std::path::Path) -> bool,
    /// Does a live process already hold this project's instance lock?
    is_locked: &'a dyn Fn(&std::path::Path) -> bool,
    /// Raise the existing window; false if it couldn't be reached.
    focus: &'a dyn Fn(&std::path::Path) -> bool,
    /// Launch a fresh window process for this project.
    spawn: &'a dyn Fn(&std::path::Path) -> std::io::Result<()>,
}

/// Reopen the project windows recorded in the global session (the set
/// open at last exit), returning whether at least one window resulted.
/// Entries whose dir is gone or no longer an Oxplow project are
/// skipped; entries already open in another process are focused rather
/// than spawned.
fn restore_session() -> bool {
    let Some(session) = session_store() else {
        return false;
    };
    restore_session_with(
        &session.list(),
        &RestoreOps {
            is_project: &|dir| dir.join(".oxplow").is_dir(),
            is_locked: &|dir| oxplow_app::is_project_locked(dir),
            focus: &|dir| oxplow_app::request_focus(dir),
            spawn: &|dir| oxplow_app::spawn_project_window(dir, true),
        },
    )
}

/// Restore policy over `entries`, returning whether at least one window
/// ended up on screen. The caller shows the launcher when this is false,
/// so "no window resulted" must never report true.
fn restore_session_with(entries: &[String], ops: &RestoreOps<'_>) -> bool {
    let mut restored = 0;
    for path in entries {
        let dir = std::path::Path::new(path);
        if !(ops.is_project)(dir) {
            continue; // gone or never initialized — don't restore
        }
        // Already open in another process (commonly a dev build sharing
        // this session file). Spawning would just hit that project's
        // instance lock and exit, so raise the existing window instead —
        // the same trade `open_project` makes.
        if (ops.is_locked)(dir) {
            if (ops.focus)(dir) {
                restored += 1;
                tracing::info!(project = %path, "focused already-open session window");
            } else {
                tracing::warn!(
                    project = %path,
                    "session project is open in an unreachable window; not restoring"
                );
            }
            continue;
        }
        match (ops.spawn)(dir) {
            Ok(()) => {
                restored += 1;
                tracing::info!(project = %path, "restored session window");
            }
            Err(e) => {
                tracing::warn!(error = %e, project = %path, "failed to restore session window")
            }
        }
    }
    restored > 0
}

/// The global session store (`session.json` in the app-config dir),
/// or `None` if the config dir is undiscoverable.
fn session_store() -> Option<oxplow_config::SessionProjects> {
    oxplow_config::global_config_dir()
        .map(|d| oxplow_config::SessionProjects::new(d.join("session.json")))
}

/// The set of project dirs with a currently-live window: `self_dir`
/// (which holds its own instance lock during boot) plus every recent
/// project whose `.oxplow/instance.lock` is held by a live process.
/// Used to re-snapshot the session on a fresh boot so closed/stale
/// projects drop off and don't accumulate across runs.
fn live_project_dirs(self_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut seen: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    let self_canon = std::fs::canonicalize(self_dir).unwrap_or_else(|_| self_dir.to_path_buf());
    seen.insert(self_canon.clone());
    out.push(self_canon);
    if let Some(cfg) = oxplow_config::global_config_dir() {
        let recents = oxplow_config::RecentProjects::new(cfg.join("recent-projects.json"));
        for r in recents.list() {
            let p = std::path::PathBuf::from(&r.path);
            let canon = std::fs::canonicalize(&p).unwrap_or(p);
            if !seen.insert(canon.clone()) {
                continue;
            }
            if oxplow_app::is_project_locked(&canon) {
                out.push(canon);
            }
        }
    }
    out
}

/// Whether any oxplow project OTHER than `self_dir` currently has a
/// live window (its instance lock is held). Used to decide whether a
/// closing window is "one of several" (drop it from the restore set)
/// vs. the last/only window (keep it to restore next launch).
fn other_window_alive(self_dir: &std::path::Path) -> bool {
    let self_canon = std::fs::canonicalize(self_dir).ok();
    let Some(cfg) = oxplow_config::global_config_dir() else {
        return false;
    };
    let recents = oxplow_config::RecentProjects::new(cfg.join("recent-projects.json"));
    recents.list().into_iter().any(|r| {
        let canon = std::fs::canonicalize(&r.path).ok();
        if canon.is_some() && canon == self_canon {
            return false; // skip self
        }
        oxplow_app::is_project_locked(std::path::Path::new(&r.path))
    })
}

/// Publish a loopback focus channel for this project and serve it on a
/// background thread: a nonce-matching ping raises/focuses the main
/// window. Lets a second `open_project` of the same dir focus the
/// existing window (see `oxplow_app::request_focus`) instead of being
/// turned away by the instance lock. Best-effort — any failure just
/// means the second open falls back to the "already open" error.
fn start_focus_listener(app: &tauri::AppHandle, project_dir: std::path::PathBuf) {
    use std::io::{BufRead, BufReader};
    use tauri::Manager;

    let listener = match std::net::TcpListener::bind(("127.0.0.1", 0)) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "focus listener bind failed");
            return;
        }
    };
    let Ok(port) = listener.local_addr().map(|a| a.port()) else {
        return;
    };
    let nonce = oxplow_app::new_focus_nonce();
    let layout = AppLayout::for_project(&project_dir);
    if let Err(e) = oxplow_app::write_instance_info(
        &layout,
        &oxplow_app::InstanceInfo {
            focus_port: port,
            nonce: nonce.clone(),
        },
    ) {
        tracing::warn!(error = %e, "failed to publish focus channel");
        return;
    }

    let app = app.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { continue };
            let mut line = String::new();
            if BufReader::new(conn).read_line(&mut line).is_ok() && line.trim() == nonce {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.unminimize();
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
        }
    });
}

/// Resolve the project dir for this process:
///   1. first positional CLI arg (`oxplow <dir>`),
///   2. `OXPLOW_PROJECT_DIR` (set by the dev script and by the
///      `open_project` spawn),
///   3. otherwise `None` → launcher mode.
///
/// The cwd fallback was intentionally dropped: a bare launch shows the
/// launcher rather than silently adopting whatever directory it was
/// started from.
fn resolve_project_dir() -> Option<std::path::PathBuf> {
    if let Some(arg) = std::env::args().nth(1) {
        // Skip flag-like args (e.g. macOS may pass `-psn_…`).
        if !arg.starts_with('-') {
            return Some(absolutize_project_dir(std::path::PathBuf::from(arg)));
        }
    }
    std::env::var_os("OXPLOW_PROJECT_DIR")
        .map(std::path::PathBuf::from)
        .map(absolutize_project_dir)
}

/// Make the project dir absolute. A relative root (`oxplow .`) breaks
/// the workspace path-traversal guard, which does a separator-prefix
/// containment check: `normalize_path(".")` collapses to `""`, so every
/// subdirectory read then false-positives as escaping the root and the
/// whole file listing dies. `canonicalize` resolves symlinks and `..`;
/// if it fails (path doesn't exist yet) we keep the original.
fn absolutize_project_dir(p: std::path::PathBuf) -> std::path::PathBuf {
    std::fs::canonicalize(&p).unwrap_or(p)
}

/// Resolve the global recent-projects store, optionally record the
/// just-opened project, and manage it on the app. Shared by both
/// launch modes so the launcher screen and a project window can each
/// list / open / forget recent projects.
fn install_recent_projects(app: &tauri::AppHandle, record: Option<std::path::PathBuf>) {
    use tauri::Manager;
    let cfg_dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    let store: oxplow_tauri_ipc::RecentProjectsState = Arc::new(
        oxplow_config::RecentProjects::new(cfg_dir.join("recent-projects.json")),
    );
    if let Some(dir) = record {
        store.record(&dir);
    }
    app.manage(store);
}

/// Launcher mode — no project, no `Services`. Hosts only the
/// recent-projects surface (`commands::launch`) and the launcher
/// window; the renderer's `<Root>` sees `mode: "launcher"` and renders
/// the start screen.
fn run_launcher(ctx: tauri::Context) {
    tracing::info!("booting in launcher mode (no project dir)");
    let specta = specta_builder();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(specta.invoke_handler())
        .manage(oxplow_tauri_ipc::LaunchInfo::launcher())
        .setup(move |app| {
            specta.mount_events(app);
            install_recent_projects(app.handle(), None);
            oxplow_tauri_ipc::commands::menu::install_menu_handler(app.handle());
            Ok(())
        })
        .run(ctx)
        .expect("error while running tauri application");
}

/// Setup mode — a directory was opened that isn't an Oxplow project
/// yet (no `.oxplow/`). Like the launcher (no `Services`), but the
/// renderer shows the "Create an Oxplow project here?" screen for this
/// dir. Confirming calls `setup_project` (creates `.oxplow/` and
/// relaunches into `run_project`); declining calls `abort_setup`
/// (exits). Nothing is recorded into recents until setup is confirmed.
fn run_setup(project_dir: std::path::PathBuf, ctx: tauri::Context) {
    tracing::info!(project = %project_dir.display(), "booting in setup mode (no .oxplow yet)");
    let specta = specta_builder();
    let launch_info = oxplow_tauri_ipc::LaunchInfo::setup(project_dir.to_string_lossy());
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(specta.invoke_handler())
        .manage(launch_info)
        .setup(move |app| {
            specta.mount_events(app);
            install_recent_projects(app.handle(), None);
            oxplow_tauri_ipc::commands::menu::install_menu_handler(app.handle());
            Ok(())
        })
        .run(ctx)
        .expect("error while running tauri application");
}

/// Project mode — boot `Services` for `project_dir` and run the full
/// app shell, exactly as before the launcher existed.
fn run_project(project_dir: std::path::PathBuf, ctx: tauri::Context) {
    let layout = AppLayout::for_project(&project_dir);

    // Per-project single-instance guard. Two processes on the same
    // `.oxplow/local.sqlite` would double the fs/git watchers and
    // contend on SQLite's writer lock, so refuse the second boot. Held
    // for the life of the process (leaked); the OS frees it on exit.
    match oxplow_app::try_acquire_instance_lock(&layout) {
        Ok(Some(lock)) => {
            Box::leak(Box::new(lock));
        }
        Ok(None) => {
            tracing::error!(
                project = %layout.project_dir.display(),
                "project already open in another oxplow window; exiting"
            );
            eprintln!(
                "oxplow: this project is already open in another window: {}",
                layout.project_dir.display()
            );
            std::process::exit(0);
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to acquire instance lock; continuing without guard");
        }
    }

    // Record this window in the global session so a later bare launch
    // reopens it. A window closing does NOT remove its entry — closing
    // the (last) window is how the user "exits", and they expect it
    // restored. Instead we re-snapshot the live set on each fresh boot:
    //   - restore spawns carry OXPLOW_RESTORING → just add self, so
    //     concurrent restores don't clobber the set being restored;
    //   - a user-initiated open re-snapshots the live window set
    //     (self + any project whose instance lock is still held), which
    //     drops projects closed in a prior run so they don't accumulate.
    if let Some(session) = session_store() {
        if std::env::var_os("OXPLOW_RESTORING").is_some() {
            session.add(&project_dir);
        } else {
            session.replace(&live_project_dirs(&project_dir));
        }
    }

    // Services::boot synchronously calls `tokio::spawn` (PtyManager owner
    // task), which requires an entered Tokio runtime. Tauri builds its
    // own runtime later, so we stand up a dedicated multi-thread runtime
    // here, enter it for the duration of boot, and leak the runtime so
    // background tasks keep running for the life of the process.
    let boot_runtime = Box::leak(Box::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime"),
    ));
    let _enter = boot_runtime.enter();
    let services = Services::boot(layout).expect("services boot");

    let state: AppState = Arc::new(services);
    // Read once here: `RunEvent::Ready` fires on the main thread and must not
    // take the config lock, and the tint is a launch-time identity cue anyway.
    let icon_tint = state.config.read().ok().and_then(|c| c.icon_tint.clone());
    let event_bus = state.events.clone();
    let lsp_sessions = state.lsp_sessions.clone();
    let terminal_sessions = state.terminal_sessions.clone();

    // Recovery + primary-stream seeding + every standard background
    // watcher/indexer. Shared with the headless oxplow-daemon — see
    // `oxplow_app::boot::run_boot_orchestration`.
    boot_runtime.block_on(oxplow_app::boot::run_boot_orchestration(&state));

    // Boot the in-process control plane (axum server hosting the
    // plugin's HTTP hook receiver + the streamable-HTTP MCP transport).
    // The handle's URLs + token feed the per-spawn plugin writer in
    // terminal.rs; nothing here needs to keep the handle around — the
    // axum task is detached.
    let control_plane = boot_runtime
        .block_on(async { oxplow_control_plane::spawn(state.clone()).await })
        .expect("control plane boot");
    let plugin_runtime: PluginRuntimeState = Arc::new(PluginRuntime {
        hook_base_url: control_plane.hook_base_url(),
        mcp_endpoint_url: control_plane.mcp_endpoint_url(),
        otlp_base_url: control_plane.otlp_base_url(),
        hook_token: control_plane.hook_token.clone(),
    });

    let specta = specta_builder();
    let launch_info = oxplow_tauri_ipc::LaunchInfo::project(project_dir.to_string_lossy());
    let project_dir_for_setup = project_dir.clone();
    let project_dir_for_close = project_dir.clone();
    let project_dir_for_focus = project_dir.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(specta.invoke_handler())
        .on_window_event(move |_window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                // Closing one window while OTHER windows are still open
                // means "I'm done with this project" → drop it from the
                // restore set. Closing the last/only window, or quitting
                // the whole app (Cmd-Q → ExitRequested sets QUITTING),
                // preserves the set so it's restored next launch.
                let quitting = QUITTING.load(std::sync::atomic::Ordering::SeqCst);
                if !quitting && other_window_alive(&project_dir_for_close) {
                    if let Some(session) = session_store() {
                        session.remove(&project_dir_for_close);
                    }
                }
                // The instance lock releases on process death, so a
                // stale focus port is never used; clear it anyway.
                oxplow_app::clear_instance_info(&project_dir_for_close);
            }
        })
        .setup(move |app| {
            specta.mount_events(app);
            // Record this project into the global recents so the
            // launcher offers it next time, and expose the store to
            // the in-window "Open Recent" surface.
            install_recent_projects(app.handle(), Some(project_dir_for_setup));
            // Publish a focus channel so a second open of this project
            // raises this window instead of failing on the lock.
            start_focus_listener(app.handle(), project_dir_for_focus.clone());
            spawn_event_bridge(app.handle().clone(), event_bus.clone());
            spawn_lsp_event_bridge(app.handle().clone(), lsp_sessions.clone());
            spawn_terminal_event_bridge(app.handle().clone(), terminal_sessions.clone());
            oxplow_tauri_ipc::commands::menu::install_menu_handler(app.handle());
            Ok(())
        })
        .manage(state)
        .manage(plugin_runtime)
        .manage(launch_info)
        .build(ctx)
        .expect("error while building tauri application")
        .run(move |_app_handle, event| {
            match event {
                // A full app quit (Cmd-Q, OS shutdown) flips QUITTING so the
                // per-window CloseRequested handlers preserve the session set
                // instead of dropping each window as it tears down.
                tauri::RunEvent::ExitRequested { .. } => {
                    QUITTING.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                // Tauri sets the dock icon while mapping its own `Ready`, so
                // this runs strictly after it and the tint isn't overwritten
                // (tsk246).
                tauri::RunEvent::Ready => icon_tint::apply(icon_tint.as_deref()),
                _ => {}
            }
        });
}

/// Forwards every `OxplowEvent` from the in-process bus onto the
/// Tauri event channel so the renderer can `listen("oxplow:event", …)`.
/// Lagging subscribers (slow renderer) are tolerated — they'll see a
/// `RecvError::Lagged` and the renderer typically refetches on
/// reconnect rather than replaying every event.
fn spawn_event_bridge(app: tauri::AppHandle, bus: oxplow_app::EventBus) {
    tauri::async_runtime::spawn(async move {
        let mut rx = bus.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(err) = app.emit(OXPLOW_EVENT_CHANNEL, &event) {
                        tracing::warn!(?err, "failed to emit oxplow event");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "event bridge lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Forwards every `LspSessionEvent` (server notifications + session
/// lifecycle) from the session manager onto `lsp:event` for the
/// renderer's lsp.ts demux.
fn spawn_lsp_event_bridge(
    app: tauri::AppHandle,
    sessions: oxplow_app::lsp_sessions::LspSessionManager,
) {
    tauri::async_runtime::spawn(async move {
        let mut rx = sessions.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(err) = app.emit(oxplow_app::event_channels::LSP, &event) {
                        tracing::warn!(?err, "failed to emit lsp event");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "lsp event bridge lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Forwards every `TerminalBridgeEvent` from the terminal session
/// registry onto `terminal:event` for the renderer's TerminalPane.
fn spawn_terminal_event_bridge(
    app: tauri::AppHandle,
    registry: oxplow_app::terminal_sessions::TerminalSessionRegistry,
) {
    tauri::async_runtime::spawn(async move {
        let mut rx = registry.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(err) = app.emit(oxplow_app::event_channels::TERMINAL, &event) {
                        tracing::warn!(?err, "failed to emit terminal event");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "terminal event bridge lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,oxplow_=debug"));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

#[cfg(test)]
mod restore_session_tests {
    use super::*;
    use std::cell::RefCell;

    /// Records which projects were focused vs. spawned so each test can
    /// assert the *route* taken, not just the return value.
    #[derive(Default)]
    struct Calls {
        focused: RefCell<Vec<String>>,
        spawned: RefCell<Vec<String>>,
    }

    fn name(dir: &std::path::Path) -> String {
        dir.to_string_lossy().into_owned()
    }

    /// `locked` lists the dirs a live process already holds; `focusable`
    /// lists the dirs whose focus channel answers. Every entry is a
    /// valid project unless listed in `missing`.
    fn run(
        entries: &[&str],
        locked: &[&str],
        focusable: &[&str],
        missing: &[&str],
        calls: &Calls,
    ) -> bool {
        restore_session_with(
            &entries.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &RestoreOps {
                is_project: &|dir| !missing.contains(&name(dir).as_str()),
                is_locked: &|dir| locked.contains(&name(dir).as_str()),
                focus: &|dir| {
                    calls.focused.borrow_mut().push(name(dir));
                    focusable.contains(&name(dir).as_str())
                },
                spawn: &|dir| {
                    calls.spawned.borrow_mut().push(name(dir));
                    Ok(())
                },
            },
        )
    }

    #[test]
    fn unlocked_project_is_spawned() {
        let calls = Calls::default();
        assert!(run(&["/a"], &[], &[], &[], &calls));
        assert_eq!(*calls.spawned.borrow(), vec!["/a"]);
        assert!(calls.focused.borrow().is_empty());
    }

    #[test]
    fn already_open_project_is_focused_not_spawned() {
        let calls = Calls::default();
        assert!(run(&["/a"], &["/a"], &["/a"], &[], &calls));
        assert_eq!(*calls.focused.borrow(), vec!["/a"]);
        assert!(
            calls.spawned.borrow().is_empty(),
            "spawning a locked project just hits the instance lock and exits"
        );
    }

    /// The reported bug: the only session entry is already open in
    /// another build, so no window results and the caller must fall
    /// through to the launcher instead of exiting silently.
    #[test]
    fn already_open_but_unreachable_reports_no_window() {
        let calls = Calls::default();
        assert!(!run(&["/a"], &["/a"], &[], &[], &calls));
        assert!(calls.spawned.borrow().is_empty());
    }

    #[test]
    fn missing_project_is_skipped() {
        let calls = Calls::default();
        assert!(!run(&["/a"], &[], &[], &["/a"], &calls));
        assert!(calls.spawned.borrow().is_empty());
        assert!(calls.focused.borrow().is_empty());
    }

    /// One dead entry must not suppress the windows that did open.
    #[test]
    fn one_unreachable_entry_does_not_hide_the_others() {
        let calls = Calls::default();
        assert!(run(&["/a", "/b"], &["/a"], &[], &[], &calls));
        assert_eq!(*calls.spawned.borrow(), vec!["/b"]);
    }

    #[test]
    fn empty_session_reports_no_window() {
        let calls = Calls::default();
        assert!(!run(&[], &[], &[], &[], &calls));
    }
}
