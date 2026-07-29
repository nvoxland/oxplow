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
    open: Mutex<Vec<(String, PathBuf)>>,
}

impl WindowRegistry {
    pub fn insert(&self, label: &str, project_dir: &Path) {
        let dir = canonical(project_dir);
        let mut open = self.lock();
        open.retain(|(l, _)| l != label);
        open.push((label.to_string(), dir));
    }

    /// Forget a closed window, returning the project it was showing.
    pub fn remove(&self, label: &str) -> Option<PathBuf> {
        let mut open = self.lock();
        let idx = open.iter().position(|(l, _)| l == label)?;
        Some(open.remove(idx).1)
    }

    /// The window already showing `project_dir`, if any. Canonicalized
    /// on both sides — the same project reached by two paths (a symlink,
    /// `/tmp` vs `/private/tmp`) is one window.
    pub fn label_for(&self, project_dir: &Path) -> Option<String> {
        let wanted = canonical(project_dir);
        self.lock()
            .iter()
            .find(|(_, dir)| *dir == wanted)
            .map(|(label, _)| label.clone())
    }

    /// Every open project, in the order the windows were opened. This is
    /// the session set.
    pub fn project_dirs(&self) -> Vec<PathBuf> {
        self.lock().iter().map(|(_, dir)| dir.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(String, PathBuf)>> {
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
    /// One of the `KIND_*` constants above.
    pub kind: &'a str,
    /// The project this window is for — the dir being opened, or the
    /// dir being offered for setup.
    pub project_dir: Option<&'a str>,
}

/// The `initialization_script` for a window. Built through `serde_json`
/// so a base URL or path can't break out of the literal.
pub fn initialization_script(ctx: &WindowContext<'_>) -> String {
    let payload = serde_json::json!({
        "base": ctx.base,
        "kind": ctx.kind,
        "projectDir": ctx.project_dir,
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

/// How long a killed orphan daemon gets to release the project lock
/// before we try to start its replacement anyway.
const ORPHAN_EXIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The shell's lifecycle: which windows are open, the daemon behind
/// each, and the global session that follows from them.
///
/// Every project window is one `oxplow-daemon` child. Opening a project
/// starts the daemon, waits for its endpoint, then creates a window
/// carrying that endpoint — the base has to be known before the window
/// exists, because it is injected at creation.
pub struct ShellWindows {
    pub registry: WindowRegistry,
    supervisor: oxplow_app::daemon_supervisor::DaemonSupervisor,
    /// The global restore set, or `None` when the config dir can't be
    /// resolved (then the session simply isn't persisted).
    session: Option<oxplow_config::SessionProjects>,
    /// Set once the whole app is quitting, so the per-window teardown
    /// stops rewriting the restore set — Cmd-Q with three windows open
    /// must bring all three back.
    quitting: std::sync::atomic::AtomicBool,
}

impl Default for ShellWindows {
    fn default() -> Self {
        Self {
            registry: WindowRegistry::default(),
            supervisor: oxplow_app::daemon_supervisor::DaemonSupervisor::default(),
            session: oxplow_config::global_config_dir()
                .map(|d| oxplow_config::SessionProjects::new(d.join("session.json"))),
            quitting: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl ShellWindows {
    /// Open `project_dir` in a window, or focus the window that already
    /// has it. Returns the window label.
    ///
    /// **Blocking**: the daemon boots (recovery, watchers, indexers)
    /// before it reports an endpoint, and the endpoint is injected at
    /// window creation. Call it off the main thread for anything but
    /// the first window.
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
            // Registered but gone — a window we lost track of. Drop it
            // and open a fresh one rather than refusing.
            self.registry.remove(&label);
            self.supervisor.stop(&dir);
        }

        // A daemon left behind by a shell that died without unwinding
        // (crash, SIGKILL, `kill` from a terminal) still holds this
        // project's instance lock, so a fresh daemon can't start and the
        // project would look permanently broken. Sweep it first — the
        // endpoint file it published is how we find it. Deliberately
        // kill rather than adopt: reattaching to a backend whose window
        // is gone is a feature nobody has asked for yet.
        if oxplow_app::daemon_supervisor::kill_orphan_daemon(&dir) {
            tracing::warn!(project = %dir.display(), "reclaimed an orphaned daemon");
            // The signal returns long before the process does. Starting
            // the replacement now would just hit the lock the dying one
            // still holds.
            if !oxplow_app::wait_for_project_unlock(&dir, ORPHAN_EXIT_TIMEOUT) {
                tracing::warn!(
                    project = %dir.display(),
                    "orphaned daemon still holds the project lock; starting anyway"
                );
            }
        }

        let base = self.supervisor.start(&dir).map_err(|e| {
            let binary = oxplow_app::daemon_supervisor::BundledDaemon::binary_path();
            if !binary.is_file() {
                // The usual cause in a dev checkout: the shell was built
                // but its sidecar wasn't. Say so instead of surfacing a
                // bare "No such file or directory".
                format!(
                    "no oxplow-daemon next to the app ({}). \
                     In a dev checkout: cargo build -p oxplow-daemon",
                    binary.display()
                )
            } else {
                format!("could not start the backend for {}: {e}", dir.display())
            }
        })?;

        let label = next_project_label();
        let title = project_window_title(&dir);
        let dir_str = dir.to_string_lossy().into_owned();
        let built = build_shell_window(
            app,
            &label,
            &title,
            &WindowContext {
                base: Some(&base),
                kind: KIND_PROJECT,
                project_dir: Some(&dir_str),
            },
        );
        if let Err(e) = built {
            // No window means nothing will ever close this daemon.
            self.supervisor.stop(&dir);
            return Err(format!("could not create the window: {e}"));
        }

        self.registry.insert(&label, &dir);
        recents.record(&dir);
        self.write_session();
        tracing::info!(project = %dir.display(), label, base, "opened project window");
        Ok(label)
    }

    /// A project window closed: forget it, stop its daemon, and update
    /// the restore set. Returns whether it was a project window at all
    /// (the launcher, setup and external-URL webviews are not).
    ///
    /// **Stopping the daemon kills that project's agents.** Deliberate:
    /// closing the window is the user saying they're done with the
    /// project, and a backend still churning behind a window that no
    /// longer exists is worse than a clean stop. Reattaching to a
    /// surviving daemon is the feature to build if that changes.
    pub fn on_window_closed(&self, label: &str) -> bool {
        let Some(dir) = self.registry.remove(label) else {
            return false; // not a project window (launcher, setup, ext-url)
        };
        self.supervisor.stop(&dir);
        tracing::info!(project = %dir.display(), label, "closed project window");
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

    /// The restore set recorded at last exit.
    pub fn session_entries(&self) -> Vec<String> {
        self.session.as_ref().map(|s| s.list()).unwrap_or_default()
    }

    /// Mark a full app quit, freezing the restore set.
    pub fn begin_quit(&self) {
        self.quitting
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether a full app quit is under way.
    pub fn is_quitting(&self) -> bool {
        self.quitting.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Stop every daemon (the shell is exiting).
    pub fn stop_all_daemons(&self) {
        self.supervisor.stop_all();
    }

    fn write_session(&self) {
        if let Some(session) = &self.session {
            session.replace(&self.registry.project_dirs());
        }
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn a_window_can_be_found_by_the_project_it_shows() {
        let reg = WindowRegistry::default();
        reg.insert("project-1", Path::new("/a"));
        reg.insert("project-2", Path::new("/b"));
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
        reg.insert("project-1", &real);
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
        reg.insert("project-1", Path::new("/a"));
        assert_eq!(reg.remove("project-1").as_deref(), Some(Path::new("/a")));
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
            reg.insert(label, Path::new(dir));
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
            kind: KIND_LAUNCHER,
            project_dir: None,
        });
        assert!(script.contains("window.__OXPLOW__ ="), "{script}");
        assert!(script.contains("\"base\":null"), "{script}");
        assert!(script.contains("\"kind\":\"launcher\""), "{script}");
        assert!(script.contains("\"projectDir\":null"), "{script}");
    }

    #[test]
    fn initialization_script_carries_the_daemon_base_and_project() {
        let script = initialization_script(&WindowContext {
            base: Some("http://127.0.0.1:60331"),
            kind: KIND_PROJECT,
            project_dir: Some("/Users/me/src/oxplow"),
        });
        assert!(
            script.contains("\"base\":\"http://127.0.0.1:60331\""),
            "{script}"
        );
        assert!(
            script.contains("\"projectDir\":\"/Users/me/src/oxplow\""),
            "{script}"
        );
    }

    /// The base is interpolated into a page script, so it goes through
    /// JSON encoding rather than string formatting: a quote in the URL
    /// must not be able to close the literal and run as code.
    #[test]
    fn initialization_script_escapes_its_payload() {
        let hostile = "http://x\";alert(1);//";
        let script = initialization_script(&WindowContext {
            base: Some(hostile),
            kind: KIND_LAUNCHER,
            project_dir: None,
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
