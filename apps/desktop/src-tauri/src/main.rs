// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

mod icon_tint;

use oxplow_tauri_ipc::{specta_builder, windows};
use tauri::Manager;

fn main() {
    if let Some(event) = hook_event_arg() {
        run_hook_command(&event);
        return;
    }

    init_tracing();

    // `generate_context!` embeds the Info.plist and may expand only
    // once per binary.
    let ctx = tauri::generate_context!();
    run_shell(ctx);
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

/// What the shell shows when it starts.
enum Startup {
    /// A project dir was named on the command line (or in the env).
    Project(PathBuf),
    /// …but it isn't an Oxplow project yet: offer to create one.
    Setup(PathBuf),
    /// Bare launch: reopen last session's windows, else the launcher.
    Restore,
}

/// Resolve the project dir for this launch:
///   1. first positional CLI arg (`oxplow <dir>`),
///   2. `OXPLOW_PROJECT_DIR` (set by the dev script),
///   3. otherwise `None` → restore/launcher.
///
/// The cwd fallback was intentionally dropped: a bare launch shows the
/// launcher rather than silently adopting whatever directory it was
/// started from.
fn resolve_project_dir() -> Option<PathBuf> {
    if let Some(arg) = std::env::args().nth(1) {
        // Skip flag-like args (e.g. macOS may pass `-psn_…`).
        if !arg.starts_with('-') {
            return Some(absolutize_project_dir(PathBuf::from(arg)));
        }
    }
    std::env::var_os("OXPLOW_PROJECT_DIR")
        .map(PathBuf::from)
        .map(absolutize_project_dir)
}

/// Make the project dir absolute. A relative root (`oxplow .`) breaks
/// the workspace path-traversal guard, which does a separator-prefix
/// containment check: `normalize_path(".")` collapses to `""`, so every
/// subdirectory read then false-positives as escaping the root and the
/// whole file listing dies. `canonicalize` resolves symlinks and `..`;
/// if it fails (path doesn't exist yet) we keep the original.
fn absolutize_project_dir(p: PathBuf) -> PathBuf {
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn resolve_startup() -> Startup {
    match resolve_project_dir() {
        Some(dir) if dir.join(".oxplow").is_dir() => Startup::Project(dir),
        // `--init` creates the project and boots straight in (scripting
        // / profiling a fresh dir) rather than asking.
        Some(dir) if init_flag() => match oxplow_app::ensure_state_dir(&dir.join(".oxplow")) {
            Ok(()) => Startup::Project(dir),
            Err(e) => {
                eprintln!(
                    "oxplow: could not create .oxplow/ in {}: {e}",
                    dir.display()
                );
                std::process::exit(1);
            }
        },
        Some(dir) => Startup::Setup(dir),
        None => Startup::Restore,
    }
}

/// The one boot path. A single process owns every window; each project
/// window is backed by its own `oxplow-daemon` child (see
/// `oxplow_tauri_ipc::windows::ShellWindows`), so the shell itself holds
/// no project state at all.
fn run_shell(ctx: tauri::Context) {
    let specta = specta_builder();
    let startup = resolve_startup();
    // Read before the windows open: the dock icon is app-global, so the
    // tint can only reflect one project — the one this launch was for.
    let icon_tint = match &startup {
        Startup::Project(dir) => project_icon_tint(dir),
        _ => None,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(specta.invoke_handler())
        .manage(windows::ShellWindows::default())
        .on_window_event(handle_window_event)
        .setup(move |app| {
            specta.mount_events(app);
            install_recent_projects(app.handle());
            oxplow_tauri_ipc::commands::menu::install_menu_handler(app.handle());
            open_startup_windows(app.handle(), startup);
            Ok(())
        })
        .build(ctx)
        .expect("error while building tauri application")
        .run(move |app, event| match event {
            // Every route out of the app comes through here (Cmd-Q, OS
            // shutdown, the launcher closing). Freeze the restore set so
            // the per-window teardown doesn't empty it window by window.
            //
            // Nothing is prevented: closing the last *project* window
            // already put the launcher up (see `handle_window_event`), so
            // reaching this point means the user closed the launcher
            // itself — which is how you quit.
            tauri::RunEvent::ExitRequested { .. } => {
                app.state::<windows::ShellWindows>().begin_quit();
            }
            // Dock-icon click with nothing on screen (macOS).
            tauri::RunEvent::Reopen {
                has_visible_windows,
                ..
            } => {
                if !has_visible_windows {
                    show_launcher(app);
                }
            }
            // Tauri sets the dock icon while mapping its own `Ready`, so
            // this runs strictly after it and the tint isn't overwritten
            // (tsk246).
            tauri::RunEvent::Ready => icon_tint::apply(icon_tint.as_deref()),
            // Last chance to take the daemons down with us.
            tauri::RunEvent::Exit => app.state::<windows::ShellWindows>().stop_all_daemons(),
            _ => {}
        });
}

/// The configured dock tint for `dir`, read straight off disk — the
/// shell has no `Services` to ask.
fn project_icon_tint(dir: &Path) -> Option<String> {
    oxplow_config::load_project_config(dir)
        .ok()
        .and_then(|c| c.icon_tint)
}

/// Open whatever this launch asked for. Never leaves the shell with no
/// window: a project that can't be opened falls back to the launcher,
/// which at least says what happened and offers another project.
fn open_startup_windows(app: &tauri::AppHandle, startup: Startup) {
    match startup {
        Startup::Project(dir) => {
            if let Err(e) = open_project_window(app, &dir) {
                tracing::error!(error = %e, project = %dir.display(), "could not open project");
                eprintln!("oxplow: {e}");
                show_launcher(app);
            }
        }
        Startup::Setup(dir) => show_setup(app, &dir),
        Startup::Restore => {
            let entries = app.state::<windows::ShellWindows>().session_entries();
            let dirs =
                windows::ShellWindows::restorable(&entries, &|dir| dir.join(".oxplow").is_dir());

            let mut opened = 0;
            for dir in dirs {
                match open_project_window(app, &dir) {
                    Ok(_) => opened += 1,
                    Err(e) => {
                        tracing::warn!(error = %e, project = %dir.display(), "session window not restored")
                    }
                }
            }
            if opened == 0 {
                show_launcher(app);
            }
        }
    }
}

/// Open (or focus) a project window, starting its daemon first.
fn open_project_window(app: &tauri::AppHandle, dir: &Path) -> Result<String, String> {
    let shell = app.state::<windows::ShellWindows>();
    let recents = app.state::<oxplow_tauri_ipc::RecentProjectsState>();
    shell.open_project(app, dir, recents.inner())
}

fn show_launcher(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window(windows::LAUNCHER_LABEL) {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }
    if let Err(e) = windows::build_shell_window(
        app,
        windows::LAUNCHER_LABEL,
        "Oxplow",
        &windows::WindowContext {
            base: None,
            kind: windows::KIND_LAUNCHER,
            project_dir: None,
        },
    ) {
        tracing::error!(error = %e, "could not open the launcher window");
    }
}

/// The "create an Oxplow project here?" confirmation for a dir that
/// isn't one yet. Only the CLI path (`oxplow <fresh-dir>`) reaches it —
/// New Project… inside the app creates without asking.
fn show_setup(app: &tauri::AppHandle, dir: &Path) {
    let dir_str = dir.to_string_lossy().into_owned();
    if let Err(e) = windows::build_shell_window(
        app,
        windows::SETUP_LABEL,
        "Oxplow",
        &windows::WindowContext {
            base: None,
            kind: windows::KIND_SETUP,
            project_dir: Some(&dir_str),
        },
    ) {
        tracing::error!(error = %e, "could not open the setup window");
    }
}

/// Events every shell window handles the same way.
fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    match event {
        // The native menu bar is app-global, so it has to be re-installed
        // for whichever window the OS just handed focus to.
        tauri::WindowEvent::Focused(true) => {
            oxplow_tauri_ipc::commands::menu::apply_focused_menu(window)
        }
        tauri::WindowEvent::Destroyed => {
            let app = window.app_handle().clone();
            oxplow_tauri_ipc::commands::menu::forget_window_menu(window);
            // False for anything that isn't a project window (the
            // launcher, setup, a sandboxed external-URL webview).
            let shell = app.state::<windows::ShellWindows>();
            let was_project = shell.on_window_closed(window.label());
            let quitting = shell.is_quitting();

            // Closing your last project drops you back to the picker
            // rather than to an app with no windows and a dead menu bar.
            // Decided here rather than at `ExitRequested`, so a quit can
            // never be mistaken for a last-window close — and so closing
            // the launcher, which reaches `ExitRequested` unimpeded, is
            // simply how you quit.
            let remaining = app
                .webview_windows()
                .keys()
                .filter(|label| label.as_str() != window.label())
                .count();
            if windows::ShellWindows::should_show_launcher(was_project, remaining, quitting) {
                // Deferred to the next main-thread tick rather than built
                // here: we're inside a window-event callback, and creating
                // a window re-enters the window manager we were called
                // from.
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || show_launcher(&handle));
            }
        }
        _ => {}
    }
}

/// Manage the global recent-projects store. Recording happens when a
/// project window actually opens, not here.
fn install_recent_projects(app: &tauri::AppHandle) {
    let cfg_dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    let store: oxplow_tauri_ipc::RecentProjectsState = Arc::new(
        oxplow_config::RecentProjects::new(cfg_dir.join("recent-projects.json")),
    );
    app.manage(store);
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
