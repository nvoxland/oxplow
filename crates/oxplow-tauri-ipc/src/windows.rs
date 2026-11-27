//! Shell windows: labels, titles, the one place their shape is defined,
//! and the registry of which project each open window is showing.
//!
//! Windows are built at runtime rather than declared in
//! `tauri.conf.json`, because the shell opens a different window for
//! each thing it can be showing (a project, the launcher, the
//! create-project confirmation) and several projects at once.
//!
//! **The label is load-bearing.** It scopes capabilities
//! (`capabilities/oxplow-windows.json` matches `project-*`, `launcher`
//! and `setup`; `capabilities/external-url.json` matches `ext-url-*`),
//! it keys the per-window menu registry, and it tells the window-event
//! handler whether a closing window is a project or a sandboxed
//! external-URL webview. Change the format here and the capability
//! globs move with it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// The project picker.
pub const LAUNCHER_LABEL: &str = "launcher";
/// The "create an Oxplow project here?" confirmation.
pub const SETUP_LABEL: &str = "setup";

/// Project windows are `project-<n>`; the glob in
/// `capabilities/oxplow-windows.json` matches this prefix.
const PROJECT_LABEL_PREFIX: &str = "project-";

static NEXT_PROJECT: AtomicUsize = AtomicUsize::new(1);

/// Allocate the next project-window label. Unique per process; the
/// number carries no meaning beyond that.
pub fn next_project_label() -> String {
    format!(
        "{PROJECT_LABEL_PREFIX}{}",
        NEXT_PROJECT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Whether `label` names a project window — i.e. one that shows a
/// project and owns that project's session bookkeeping. Deliberately
/// stricter than the capability glob: only `project-<digits>`, so an
/// unrelated label that happens to start with `project-` can't pass
/// itself off as one.
pub fn is_project_label(label: &str) -> bool {
    label
        .strip_prefix(PROJECT_LABEL_PREFIX)
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Title for a project window. The title bar is hidden (overlay
/// style), so this is what the macOS Window menu and Mission Control
/// show — with several projects open, the bare project name is the
/// thing that distinguishes them.
pub fn project_window_title(project_dir: &Path) -> String {
    project_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| project_dir.to_string_lossy().into_owned())
}

/// Which project each open project window is showing.
///
/// This is what replaced "infer the open set from recent projects whose
/// instance lock is held": the shell opens the windows, so it simply
/// knows. Insertion order is the open order, which is the order the
/// session restores in.
#[derive(Default)]
pub struct WindowRegistry {
    /// A Vec, not a map: the order is the open order, and the session
    /// restores in it.
    open: Mutex<Vec<OpenWindow>>,
}

/// One open project window: its label, its folder, and the host's id for
/// the project (what the window holds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWindow {
    pub label: String,
    /// The project's folder here; for a project on another machine's host,
    /// its address (`<origin>/p/<id>`), which is also its restore entry.
    pub dir: PathBuf,
    pub project: String,
    /// The origin of the host on another machine it is on; `None`: the
    /// app's own host.
    pub remote: Option<String>,
}

/// The restore entry of a project on another machine's host. Pure.
pub fn remote_entry(origin: &str, project: &str) -> String {
    format!("{origin}/p/{project}")
}

/// A restore entry that names a project on another machine's host: its
/// origin and project id. Pure.
pub fn parse_remote_entry(entry: &str) -> Option<(String, String)> {
    if !(entry.starts_with("https://") || entry.starts_with("http://")) {
        return None;
    }
    let (origin, project) = entry.rsplit_once("/p/")?;
    (!project.is_empty() && !project.contains('/'))
        .then(|| (origin.to_string(), project.to_string()))
}

impl WindowRegistry {
    pub fn insert(&self, label: &str, project_dir: &Path, project: &str) {
        let dir = canonical(project_dir);
        let mut open = self.lock();
        open.retain(|w| w.label != label);
        open.push(OpenWindow {
            label: label.to_string(),
            dir,
            project: project.to_string(),
            remote: None,
        });
    }

    /// A window on `project` of the host at `origin` (another machine).
    pub fn insert_remote(&self, label: &str, origin: &str, project: &str) {
        let mut open = self.lock();
        open.retain(|w| w.label != label);
        open.push(OpenWindow {
            label: label.to_string(),
            dir: PathBuf::from(remote_entry(origin, project)),
            project: project.to_string(),
            remote: Some(origin.to_string()),
        });
    }

    /// Forget a closed window, returning what it was showing.
    pub fn remove(&self, label: &str) -> Option<OpenWindow> {
        let mut open = self.lock();
        let idx = open.iter().position(|w| w.label == label)?;
        Some(open.remove(idx))
    }

    /// Whether the window `label` is on a host on another machine.
    pub fn is_remote(&self, label: &str) -> bool {
        self.lock()
            .iter()
            .any(|w| w.label == label && w.remote.is_some())
    }

    /// Every open window, in the order they were opened.
    pub fn windows(&self) -> Vec<OpenWindow> {
        self.lock().clone()
    }

    /// The window already showing `project_dir`, if any. Canonicalized
    /// on both sides — the same project reached by two paths (a symlink,
    /// `/tmp` vs `/private/tmp`) is one window.
    pub fn label_for(&self, project_dir: &Path) -> Option<String> {
        let wanted = canonical(project_dir);
        self.lock()
            .iter()
            .find(|w| w.dir == wanted)
            .map(|w| w.label.clone())
    }

    /// Every open project, in the order the windows were opened. This is
    /// the session set.
    pub fn project_dirs(&self) -> Vec<PathBuf> {
        self.lock().iter().map(|w| w.dir.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<OpenWindow>> {
        self.open.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Resolve symlinks and `..` so one project has one identity. A path
/// that doesn't exist (yet) is kept as-is rather than dropped.
fn canonical(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())
}

/// Window kinds, as the renderer sees them in `window.__OXPLOW__.kind`.
pub const KIND_PROJECT: &str = "project";
pub const KIND_LAUNCHER: &str = "launcher";
pub const KIND_SETUP: &str = "setup";

/// What the shell tells a window about itself. Injected before any page
/// script runs, so the renderer knows which screen it is and which
/// backend to call without an IPC round trip (which would need a
/// backend already).
pub struct WindowContext<'a> {
    /// Base URL of the daemon this window drives. `None` for the
    /// launcher and setup screens, which have no project and therefore
    /// no backend beyond the shell itself.
    pub base: Option<&'a str>,
    /// The daemon's UI token (see `oxplow_daemon`): only this window's
    /// renderer gets it, so only the person's UI can call the daemon.
    pub token: Option<&'a str>,
    /// One of the `KIND_*` constants above.
    pub kind: &'a str,
    /// The project this window is for — the dir being opened, or the
    /// dir being offered for setup.
    pub project_dir: Option<&'a str>,
    /// This install's device (`oxplow_config::this_device`): the Mac app is
    /// one device for all its windows, and a project window sends it on
    /// every request. `None` for the launcher and setup screens.
    pub device: Option<&'a oxplow_config::ThisDevice>,
}

/// The `initialization_script` for a window. Built through `serde_json`
/// so a base URL or path can't break out of the literal.
pub fn initialization_script(ctx: &WindowContext<'_>) -> String {
    let payload = serde_json::json!({
        "base": ctx.base,
        "token": ctx.token,
        "kind": ctx.kind,
        "projectDir": ctx.project_dir,
        "device": ctx.device,
    });
    // Frozen: this is the shell's statement about the window, not
    // renderer state.
    format!("window.__OXPLOW__ = Object.freeze({payload});")
}

/// Build a shell window. Every window the shell owns has the same
/// shape; only the label, title and injected context differ.
pub fn build_shell_window(
    app: &tauri::AppHandle,
    label: &str,
    title: &str,
    ctx: &WindowContext<'_>,
) -> tauri::Result<tauri::WebviewWindow> {
    #[allow(unused_mut)]
    let mut builder = tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::default())
        .initialization_script(initialization_script(ctx))
        .title(title)
        .inner_size(1400.0, 900.0)
        .min_inner_size(800.0, 500.0)
        .resizable(true)
        .decorations(true)
        // The renderer runs its own drag-and-drop (files onto panes);
        // Tauri's native handler would swallow the events first.
        .disable_drag_drop_handler();

    // Content runs under the title bar with the traffic lights floating
    // over it — matches the renderer's own chrome.
    #[cfg(target_os = "macos")]
    {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true);
    }

    builder.build()
}

/// The shell's lifecycle: which windows are open, the project each holds
/// on the app's host, and the global session that follows from them.
///
/// The app's host is the app's child (`oxplow_host::client::start_own_host`,
/// see `.context/host.md` "The Mac app"), started at the first open, with a
/// token made for this run. Opening a project registers and holds it there
/// and waits for its daemon, then creates a window carrying `<host>/p/<id>`
/// and the token: the base has to be known before the window exists,
/// because it is injected at creation. Closing a window releases its hold;
/// the host stops a daemon no device holds. Quitting ends the host (its
/// stdin lifeline closes), and every device's projects with it: the app is
/// the whole of oxplow to the person.
pub struct ShellWindows {
    pub registry: WindowRegistry,
    /// The app's host once started; started again at the next open if it
    /// has ended (crashed).
    host: Mutex<Option<oxplow_host::client::OwnHost>>,
    /// The "Allow other devices" settings the running host started with.
    started_with: Mutex<Option<oxplow_host_access::settings::HostSettings>>,
    /// The global restore set, or `None` when the config dir can't be
    /// resolved (then the session simply isn't persisted).
    session: Option<oxplow_config::SessionProjects>,
    /// This install's device, made once and kept in the global config dir;
    /// `None` when that dir can't be resolved or written.
    device: Option<oxplow_config::ThisDevice>,
    /// Set once the whole app is quitting, so the per-window teardown
    /// stops rewriting the restore set — Cmd-Q with three windows open
    /// must bring all three back.
    quitting: std::sync::atomic::AtomicBool,
    /// The person said Quit (or nothing needed asking): the exit goes on.
    quit_confirmed: std::sync::atomic::AtomicBool,
    /// A quit is being asked about now: a second Quit asks nothing more.
    asking_quit: std::sync::atomic::AtomicBool,
    /// Restore entries of windows on another machine that did not open.
    unrestored: Mutex<Vec<String>>,
}

/// The name a person gave this Mac (System Settings › General › About),
/// else its host name: the device's name until the person renames it.
fn computer_name() -> String {
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
    {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && !name.is_empty() {
            return name;
        }
    }
    std::env::var("HOSTNAME")
        .ok()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "This computer".into())
}

impl Default for ShellWindows {
    fn default() -> Self {
        Self {
            registry: WindowRegistry::default(),
            host: Mutex::new(None),
            started_with: Mutex::new(None),
            session: oxplow_config::global_config_dir()
                .map(|d| oxplow_config::SessionProjects::new(d.join("session.json"))),
            device: oxplow_config::global_config_dir().and_then(|d| {
                oxplow_config::this_device(&d.join("device.json"), computer_name)
                    .map_err(|e| tracing::warn!(error = %e, "could not keep this install's device"))
                    .ok()
            }),
            quitting: std::sync::atomic::AtomicBool::new(false),
            quit_confirmed: std::sync::atomic::AtomicBool::new(false),
            asking_quit: std::sync::atomic::AtomicBool::new(false),
            unrestored: Mutex::new(Vec::new()),
        }
    }
}

impl ShellWindows {
    /// Open `project_dir` in a window, or focus the window that already
    /// has it. Returns the window label.
    ///
    /// **Blocking**: the host may have to start, and the project's daemon
    /// boots (recovery, watchers, indexers) before it runs, and the base is
    /// injected at window creation. Call it off the main thread for
    /// anything but the first window.
    pub fn open_project(
        &self,
        app: &tauri::AppHandle,
        project_dir: &Path,
        recents: &oxplow_config::RecentProjects,
    ) -> Result<String, String> {
        use tauri::Manager;

        let dir = canonical(project_dir);
        if let Some(label) = self.registry.label_for(&dir) {
            if let Some(win) = app.get_webview_window(&label) {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
                return Ok(label);
            }
            // Registered but gone — a window we lost track of. Let go of
            // it and open a fresh one rather than refusing.
            if let Some(lost) = self.registry.remove(&label) {
                self.release(lost.project);
            }
        }

        let device = self
            .device
            .as_ref()
            .ok_or("this install has no device (its config folder can't be written)")?;
        let (host, opened) = tauri::async_runtime::block_on(self.open_on_host(&dir, device))?;

        let label = next_project_label();
        let title = project_window_title(&dir);
        let dir_str = dir.to_string_lossy().into_owned();
        let built = build_shell_window(
            app,
            &label,
            &title,
            &WindowContext {
                base: Some(&opened.base),
                token: Some(host.token()),
                kind: KIND_PROJECT,
                project_dir: Some(&dir_str),
                device: Some(device),
            },
        );
        if let Err(e) = built {
            // No window means nothing will ever let go of this hold.
            self.release(opened.id);
            return Err(format!("could not create the window: {e}"));
        }

        self.registry.insert(&label, &dir, &opened.id);
        recents.record(&dir);
        self.write_session();
        tracing::info!(project = %dir.display(), label, base = %opened.base, "opened project window");
        Ok(label)
    }

    /// Hold `dir` on the app's host and wait for its daemon.
    async fn open_on_host(
        &self,
        dir: &Path,
        device: &oxplow_config::ThisDevice,
    ) -> Result<
        (
            oxplow_host::client::HostClient,
            oxplow_host::client::OpenedProject,
        ),
        String,
    > {
        let host = self.host()?;
        let opened = host
            .open(dir, device.id.as_str())
            .await
            .map_err(|e| e.to_string())?;
        Ok((host, opened))
    }

    /// The app's host, started the first time it is needed, and again when
    /// it has ended (the windows it served stay on their dead socket).
    /// **Blocking** while it starts.
    fn host(&self) -> Result<oxplow_host::client::HostClient, String> {
        let mut slot = self.host.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(own) = slot.as_mut() {
            if !own.ended() {
                return Ok(own.client.clone());
            }
            tracing::warn!("the host ended; starting another (see logs/host.*.log)");
        }
        let binary = oxplow_host::client::binary_beside_exe();
        if !binary.is_file() {
            // The usual cause in a dev checkout: the shell was built but
            // its sidecars weren't staged.
            return Err(format!(
                "no oxplow-host next to the app ({}). In a dev checkout: bun run build:dev",
                binary.display()
            ));
        }
        // "Allow other devices", as saved: a setting that cannot work is
        // said, and the host starts for this machine only.
        let settings = oxplow_config::global_config_dir()
            .map(|dir| oxplow_host_access::settings::HostSettings::load(&dir))
            .unwrap_or_default();
        let extra = settings.serve_args().unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Allow other devices is on but cannot work; this machine only");
            Vec::new()
        });
        tracing::info!(binary = %binary.display(), others = !extra.is_empty(), "starting the host");
        let own = oxplow_host::client::start_own_host(std::process::Command::new(&binary), &extra)
            .map_err(|e| e.to_string())?;
        let client = own.client.clone();
        *slot = Some(own);
        *self.started_with.lock().unwrap_or_else(|e| e.into_inner()) = Some(settings);
        Ok(client)
    }

    /// The app's host, started if it is not running (the Devices window, or
    /// "Allow other devices" at launch). **Blocking** while it starts.
    pub fn host_client(&self) -> Result<oxplow_host::client::HostClient, String> {
        self.host()
    }

    /// The app's host if it runs now; nothing is started, and a host that is
    /// starting now (its lock held) is none: nothing waits on it.
    pub fn running_host(&self) -> Option<oxplow_host::client::HostClient> {
        let mut slot = match self.host.try_lock() {
            Ok(slot) => slot,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
        let own = slot.as_mut()?;
        (!own.ended()).then(|| own.client.clone())
    }

    /// Whether the running host started with `settings` (a change applies
    /// at the next start).
    pub fn settings_applied(&self, settings: &oxplow_host_access::settings::HostSettings) -> bool {
        self.running_host().is_some()
            && self
                .started_with
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                == Some(settings)
    }

    /// This install's device (the Mac app is one device).
    pub fn device(&self) -> Option<&oxplow_config::ThisDevice> {
        self.device.as_ref()
    }

    /// Open (or focus) `project` of the host at `origin` on another machine,
    /// as the device this Mac is there. **Blocking** while its daemon starts.
    pub fn open_remote(
        &self,
        app: &tauri::AppHandle,
        origin: &str,
        project: &str,
    ) -> Result<String, String> {
        use tauri::Manager;
        let entry = PathBuf::from(remote_entry(origin, project));
        if let Some(label) = self.registry.label_for(&entry) {
            if let Some(win) = app.get_webview_window(&label) {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
                return Ok(label);
            }
            self.registry.remove(&label);
        }
        let config_dir = oxplow_config::global_config_dir()
            .ok_or("no config folder for this user; set OXPLOW_HOME")?;
        let paired = oxplow_host::remote::list(&config_dir)
            .into_iter()
            .find(|h| h.origin == origin)
            .ok_or_else(|| format!("this Mac is not paired with {origin}; connect to it first"))?;
        let host = oxplow_host::remote::client(&oxplow_ai::secrets::KeychainSecrets, origin)?;
        let opened = tauri::async_runtime::block_on(host.open_held(project, ""))
            .map_err(|e| e.to_string())?;
        let label = next_project_label();
        let device = oxplow_config::ThisDevice {
            id: oxplow_domain::DeviceId::parse(&paired.device_id)
                .map_err(|_| format!("the id {origin} gave this Mac is not a device id"))?,
            name: paired.device_name.clone(),
        };
        let host_name = origin.split("://").nth(1).unwrap_or(origin);
        let built = build_shell_window(
            app,
            &label,
            &format!("{} — {host_name}", opened.name),
            &WindowContext {
                base: Some(&opened.base),
                token: Some(host.token()),
                kind: KIND_PROJECT,
                // Its files are on that machine, not this one.
                project_dir: None,
                device: Some(&device),
            },
        );
        if let Err(e) = built {
            self.release_remote(origin.to_string(), opened.id);
            return Err(format!("could not create the window: {e}"));
        }
        self.registry.insert_remote(&label, origin, &opened.id);
        self.write_session();
        tracing::info!(origin, project = %opened.id, label, "opened a project on another machine");
        Ok(label)
    }

    /// Let go of `project` on the host at `origin`, in the background.
    fn release_remote(&self, origin: String, project: String) {
        let Ok(host) = oxplow_host::remote::client(&oxplow_ai::secrets::KeychainSecrets, &origin)
        else {
            return;
        };
        tauri::async_runtime::spawn(async move {
            if let Err(e) = host.release(&project, "").await {
                tracing::warn!(origin, project, error = %e, "could not release the project there");
            }
        });
    }

    /// Let go of `project` on the host, in the background.
    fn release(&self, project: String) {
        let host = self.running_host();
        let (Some(host), Some(device)) = (host, self.device.clone()) else {
            return;
        };
        tauri::async_runtime::spawn(async move {
            if let Err(e) = host.release(&project, device.id.as_str()).await {
                tracing::warn!(project, error = %e, "could not release the project on the host");
            }
        });
    }

    /// A project window closed: forget it, release its hold, and update
    /// the restore set. Returns whether it was a project window at all
    /// (the launcher, setup and external-URL webviews are not).
    ///
    /// **The project's agents stop** when no other device holds it: closing
    /// the window is the person saying they're done with the project here.
    /// A tablet that holds it keeps it running. During a quit nothing is
    /// released: the host ends with the app.
    pub fn on_window_closed(&self, label: &str) -> bool {
        let Some(closed) = self.registry.remove(label) else {
            return false; // not a project window (launcher, setup, ext-url)
        };
        match &closed.remote {
            // On another machine's host: closing the window is letting go
            // there; a quit lets go of nothing (the work goes on).
            Some(origin) if !self.is_quitting() => {
                self.release_remote(origin.clone(), closed.project.clone())
            }
            Some(_) => {}
            None if !self.is_quitting() => self.release(closed.project.clone()),
            None => {}
        }
        tracing::info!(project = %closed.dir.display(), label, "closed project window");
        // Closing one window while others are open means "I'm done with
        // this project" → drop it. Closing the LAST window is how you
        // exit, and the set you left behind is what should come back —
        // as is a full quit, which sets `quitting` first.
        if !self.is_quitting() && !self.registry.is_empty() {
            self.write_session();
        }
        true
    }

    /// Whether a closing window should leave the launcher in its place.
    ///
    /// The IntelliJ/Xcode rule: closing your last project drops you back
    /// to the picker rather than to an app with no windows and a dead
    /// menu bar. Closing the *launcher* is how you quit, so it doesn't
    /// reopen itself; and during a full quit nothing reopens at all.
    /// Pure; exported for tests.
    pub fn should_show_launcher(
        was_project: bool,
        remaining_windows: usize,
        quitting: bool,
    ) -> bool {
        was_project && remaining_windows == 0 && !quitting
    }

    /// Session entries worth reopening, in order: still on disk, still a
    /// project, no duplicates. Pure — the caller opens them.
    pub fn restorable(entries: &[String], is_project: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
        let mut seen = std::collections::HashSet::new();
        entries
            .iter()
            .map(|e| canonical(Path::new(e)))
            .filter(|dir| is_project(dir))
            .filter(|dir| seen.insert(dir.clone()))
            .collect()
    }

    /// Session entries that name a project on another machine's host, in
    /// order, once each. Pure.
    pub fn remote_restorable(entries: &[String]) -> Vec<(String, String)> {
        let mut seen = std::collections::HashSet::new();
        entries
            .iter()
            .filter_map(|e| parse_remote_entry(e))
            .filter(|r| seen.insert(r.clone()))
            .collect()
    }

    /// The restore set recorded at last exit.
    pub fn session_entries(&self) -> Vec<String> {
        self.session.as_ref().map(|s| s.list()).unwrap_or_default()
    }

    /// Mark a full app quit, freezing the restore set.
    pub fn begin_quit(&self) {
        self.quitting
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The quit was asked and said yes (or needed no asking).
    pub fn confirm_quit(&self) {
        self.quit_confirmed
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Start asking about a quit; false when one is being asked already.
    pub fn begin_asking_quit(&self) -> bool {
        !self
            .asking_quit
            .swap(true, std::sync::atomic::Ordering::SeqCst)
    }

    /// The question was answered "Cancel".
    pub fn end_asking_quit(&self) {
        self.asking_quit
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn quit_confirmed(&self) -> bool {
        self.quit_confirmed
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// What to ask before quitting, if another device has a window open
    /// on the app's host now. **Blocking** (one call to the host).
    pub fn quit_question_now(&self) -> Option<String> {
        let host = self.running_host()?;
        let connected = tauri::async_runtime::block_on(host.connected()).ok()?;
        quit_question(&connected, self.device().map(|d| d.id.as_str()))
    }

    /// Whether a full app quit is under way.
    pub fn is_quitting(&self) -> bool {
        self.quitting.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The app is ending: so is its host. Dropping it closes the host's
    /// lifeline; the host stops every daemon on its own. Nothing waits.
    pub fn end(&self) {
        // A host starting now holds the lock: never waited on. It ends with
        // the app anyway (its lifeline is this process's).
        let mut slot = match self.host.try_lock() {
            Ok(slot) => slot,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return,
        };
        if slot.take().is_some() {
            tracing::info!("the app is ending; so is its host");
        }
    }

    /// A window on another machine that did not open at the start (its host
    /// was away): kept in the restore set, so the next start tries again.
    pub fn keep_unrestored(&self, origin: &str, project: &str) {
        let entry = remote_entry(origin, project);
        let mut kept = self.unrestored.lock().unwrap_or_else(|e| e.into_inner());
        if !kept.contains(&entry) {
            kept.push(entry);
        }
    }

    /// Forget the windows of `origin` that did not open (the host was
    /// forgotten).
    pub fn forget_unrestored(&self, origin: &str) {
        self.unrestored
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|e| parse_remote_entry(e).is_none_or(|(o, _)| o != origin));
        self.write_session();
    }

    fn write_session(&self) {
        if let Some(session) = &self.session {
            let mut entries = self.registry.project_dirs();
            let opened: Vec<String> = entries.iter().map(|d| d.display().to_string()).collect();
            for kept in self
                .unrestored
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
            {
                if !opened.contains(kept) {
                    entries.push(PathBuf::from(kept));
                }
            }
            session.replace(&entries);
        }
    }
}

/// What quitting asks first, when another device than this Mac (`own`) has
/// a window open on its host: quitting ends the host, and their projects
/// with it. `None`: nothing to ask. Pure.
pub fn quit_question(
    connected: &[oxplow_host::access::Connected],
    own: Option<&str>,
) -> Option<String> {
    let others: Vec<&str> = connected
        .iter()
        .filter(|c| Some(c.device.as_str()) != own)
        .map(|c| c.name.as_str())
        .collect();
    let names = match others.as_slice() {
        [] => return None,
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    let verb = if others.len() == 1 { "is" } else { "are" };
    Some(format!(
        "{names} {verb} using Oxplow on this computer. Quitting stops their projects and agents too."
    ))
}

#[cfg(test)]
mod remote_tests {
    use super::*;

    /// A window on another machine's project is kept in the session as its
    /// address, and only such an entry reads back as one.
    #[test]
    fn a_remote_window_is_its_address_in_the_session() {
        let entry = remote_entry("https://server.ts.net:7400", "prj-1");
        assert_eq!(entry, "https://server.ts.net:7400/p/prj-1");
        assert_eq!(
            parse_remote_entry(&entry),
            Some(("https://server.ts.net:7400".into(), "prj-1".into()))
        );
        assert_eq!(parse_remote_entry("/Users/me/src/p"), None);
        assert_eq!(parse_remote_entry("https://x/p/"), None);
        let entries = vec![entry.clone(), "/Users/me/a".into(), entry];
        assert_eq!(ShellWindows::remote_restorable(&entries).len(), 1);
        assert!(ShellWindows::restorable(&entries, &|_| false).is_empty());

        let reg = WindowRegistry::default();
        reg.insert_remote("project-9", "https://server.ts.net:7400", "prj-1");
        reg.insert("project-1", Path::new("/a"), "prj-2");
        // What may act on this Mac asks this (`local_window_only`).
        assert!(reg.is_remote("project-9"));
        assert!(!reg.is_remote("project-1"));
        let w = reg.remove("project-9").unwrap();
        assert_eq!(w.remote.as_deref(), Some("https://server.ts.net:7400"));
    }
}

#[cfg(test)]
mod quit_tests {
    use super::*;
    use oxplow_host::access::Connected;

    fn on(device: &str, name: &str) -> Connected {
        Connected {
            device: device.into(),
            name: name.into(),
            kind: None,
        }
    }

    /// Quitting ends the host, every device's projects with it: when
    /// another device has a window open, the app asks first and names it.
    #[test]
    fn quit_asks_first_only_when_another_device_is_connected() {
        assert_eq!(quit_question(&[], Some("dev-mac")), None);
        assert_eq!(
            quit_question(&[on("dev-mac", "Studio")], Some("dev-mac")),
            None,
            "the Mac itself"
        );
        let asked = quit_question(
            &[
                on("dev-mac", "Studio"),
                on("dev-tab", "Tab S9"),
                on("dev-phone", "Phone"),
            ],
            Some("dev-mac"),
        )
        .unwrap();
        assert!(asked.contains("Tab S9 and Phone"), "{asked}");
        assert!(!asked.contains("Studio"), "{asked}");
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn a_window_can_be_found_by_the_project_it_shows() {
        let reg = WindowRegistry::default();
        reg.insert("project-1", Path::new("/a"), "prj-1");
        reg.insert("project-2", Path::new("/b"), "prj-1");
        assert_eq!(reg.label_for(Path::new("/b")).as_deref(), Some("project-2"));
        assert_eq!(reg.label_for(Path::new("/nope")), None);
    }

    /// The same project reached by two paths is one window. On macOS
    /// `/tmp` is a symlink to `/private/tmp`, so a dir picked in the
    /// file dialog and the same dir from a recents entry routinely
    /// differ as strings — without canonicalizing, "focus the window
    /// that already has this project" opens a second one, and the
    /// project's own instance lock then rejects its daemon.
    #[cfg(unix)]
    #[test]
    fn the_same_project_through_a_symlink_is_the_same_window() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("proj");
        std::fs::create_dir(&real).unwrap();
        let link = tmp.path().join("link-to-proj");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let reg = WindowRegistry::default();
        reg.insert("project-1", &real, "prj-1");
        assert_eq!(
            reg.label_for(&link).as_deref(),
            Some("project-1"),
            "{} should resolve to the window holding {}",
            link.display(),
            real.display()
        );
    }

    #[test]
    fn closing_a_window_returns_its_project_and_forgets_it() {
        let reg = WindowRegistry::default();
        reg.insert("project-1", Path::new("/a"), "prj-1");
        let closed = reg.remove("project-1").unwrap();
        assert_eq!(closed.dir, Path::new("/a"));
        assert_eq!(closed.project, "prj-1", "the hold it let go of");
        assert!(reg.is_empty());
        assert_eq!(reg.remove("project-1"), None, "removing twice is a no-op");
    }

    #[test]
    fn closing_your_last_project_window_brings_up_the_launcher() {
        assert!(ShellWindows::should_show_launcher(true, 0, false));
    }

    /// Closing one of several is just closing a window — the others are
    /// still there to work in.
    #[test]
    fn closing_one_of_several_project_windows_opens_nothing() {
        assert!(!ShellWindows::should_show_launcher(true, 1, false));
        assert!(!ShellWindows::should_show_launcher(true, 3, false));
    }

    /// Closing the launcher is how you quit. Reopening it there would
    /// make the app unquittable by any route but Cmd-Q.
    #[test]
    fn closing_the_launcher_does_not_reopen_it() {
        assert!(!ShellWindows::should_show_launcher(false, 0, false));
    }

    /// During a full quit every window is torn down in turn; the last
    /// project window closing must not resurrect the app.
    #[test]
    fn quitting_never_reopens_the_launcher() {
        assert!(!ShellWindows::should_show_launcher(true, 0, true));
    }

    /// A session entry whose project was deleted (or was never one)
    /// must not be reopened — the daemon would refuse it and the user
    /// would get an error for a project they don't have any more.
    #[test]
    fn restore_skips_entries_that_are_no_longer_projects() {
        let entries = vec!["/gone".to_string(), "/still-here".to_string()];
        let kept = ShellWindows::restorable(&entries, &|dir| dir == Path::new("/still-here"));
        assert_eq!(kept, vec![PathBuf::from("/still-here")]);
    }

    /// The same project listed twice must open one window, not two —
    /// the second would find the project's instance lock held by the
    /// first one's daemon and fail.
    #[test]
    fn restore_opens_each_project_once() {
        let entries = vec!["/a".to_string(), "/b".to_string(), "/a".to_string()];
        let kept = ShellWindows::restorable(&entries, &|_| true);
        assert_eq!(kept, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn restore_of_an_empty_session_opens_nothing() {
        assert!(ShellWindows::restorable(&[], &|_| true).is_empty());
    }

    /// The session restores in the order the windows were opened, so
    /// the registry has to preserve it — a hash map's iteration order
    /// would shuffle the restore set on every run.
    #[test]
    fn open_projects_come_back_in_the_order_they_were_opened() {
        let reg = WindowRegistry::default();
        for (label, dir) in [
            ("project-1", "/one"),
            ("project-2", "/two"),
            ("project-3", "/three"),
        ] {
            reg.insert(label, Path::new(dir), "prj-1");
        }
        assert_eq!(
            reg.project_dirs(),
            vec![
                PathBuf::from("/one"),
                PathBuf::from("/two"),
                PathBuf::from("/three")
            ]
        );

        reg.remove("project-2");
        assert_eq!(
            reg.project_dirs(),
            vec![PathBuf::from("/one"), PathBuf::from("/three")],
            "removing the middle window must not disturb the rest"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_labels_are_unique_and_recognized() {
        let first = next_project_label();
        let second = next_project_label();
        assert_ne!(first, second);
        assert!(is_project_label(&first));
        assert!(is_project_label(&second));
    }

    #[test]
    fn is_project_label_rejects_the_other_window_kinds() {
        assert!(!is_project_label(LAUNCHER_LABEL));
        assert!(!is_project_label(SETUP_LABEL));
        // Sandboxed external-URL webviews close all the time; treating
        // one as a project window would drop the project from the
        // session restore set.
        assert!(!is_project_label("ext-url-9f2c"));
    }

    #[test]
    fn is_project_label_rejects_near_misses() {
        assert!(!is_project_label("project-"));
        assert!(!is_project_label("project-x"));
        assert!(!is_project_label("project-1a"));
        assert!(!is_project_label("projectile"));
        assert!(!is_project_label(""));
    }

    #[test]
    fn project_window_title_is_the_project_directory_name() {
        assert_eq!(
            project_window_title(Path::new("/Users/me/src/oxplow")),
            "oxplow"
        );
    }

    /// A root path has no file name; falling back to the whole path
    /// beats an empty title.
    #[test]
    fn project_window_title_falls_back_to_the_path() {
        assert_eq!(project_window_title(Path::new("/")), "/");
    }

    /// The launcher has no project and no daemon. The renderer must see
    /// an explicit null, not a missing key, so it can tell "no backend"
    /// from "the shell forgot to inject anything".
    #[test]
    fn initialization_script_states_a_null_base_for_the_launcher() {
        let script = initialization_script(&WindowContext {
            base: None,
            token: None,
            kind: KIND_LAUNCHER,
            project_dir: None,
            device: None,
        });
        assert!(script.contains("window.__OXPLOW__ ="), "{script}");
        assert!(script.contains("\"device\":null"), "{script}");
        assert!(script.contains("\"base\":null"), "{script}");
        assert!(script.contains("\"kind\":\"launcher\""), "{script}");
        assert!(script.contains("\"projectDir\":null"), "{script}");
    }

    #[test]
    fn initialization_script_carries_the_daemon_base_and_project() {
        let device = oxplow_config::ThisDevice {
            id: oxplow_domain::DeviceId::parse("dev-1b4e28ba-2fa1-11d2-883f-0016d3cca427").unwrap(),
            name: "Studio".into(),
        };
        let script = initialization_script(&WindowContext {
            base: Some("http://127.0.0.1:60331"),
            token: Some("t0k"),
            kind: KIND_PROJECT,
            project_dir: Some("/Users/me/src/oxplow"),
            device: Some(&device),
        });
        // The Mac app is one device for all its windows: each project
        // window is told which, and sends it on every request.
        assert!(
            script.contains(
                "\"device\":{\"id\":\"dev-1b4e28ba-2fa1-11d2-883f-0016d3cca427\",\"name\":\"Studio\"}"
            ),
            "{script}"
        );
        assert!(
            script.contains("\"base\":\"http://127.0.0.1:60331\""),
            "{script}"
        );
        assert!(
            script.contains("\"projectDir\":\"/Users/me/src/oxplow\""),
            "{script}"
        );
        assert!(script.contains("\"token\":\"t0k\""), "{script}");
    }

    /// The base is interpolated into a page script, so it goes through
    /// JSON encoding rather than string formatting: a quote in the URL
    /// must not be able to close the literal and run as code.
    #[test]
    fn initialization_script_escapes_its_payload() {
        let hostile = "http://x\";alert(1);//";
        let script = initialization_script(&WindowContext {
            base: Some(hostile),
            token: None,
            kind: KIND_LAUNCHER,
            project_dir: None,
            device: None,
        });
        let payload = script
            .strip_prefix("window.__OXPLOW__ = Object.freeze(")
            .and_then(|s| s.strip_suffix(");"))
            .expect("script shape");
        let parsed: serde_json::Value =
            serde_json::from_str(payload).expect("payload is valid JSON");
        assert_eq!(parsed["base"], hostile, "the URL survives intact");
        assert!(
            !payload.contains("x\";"),
            "the quote must be escaped, not closing the literal: {payload}"
        );
    }
}
