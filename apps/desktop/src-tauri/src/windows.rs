//! Shell windows: labels, titles, and the one place their shape is
//! defined.
//!
//! Windows are built at runtime rather than declared in
//! `tauri.conf.json`, because the shell opens a different window for
//! each thing it can be showing (a project, the launcher, the
//! create-project confirmation) and — once the daemon-backed shell
//! lands — several project windows at once.
//!
//! **The label is load-bearing.** It scopes capabilities
//! (`capabilities/oxplow-windows.json` matches `project-*`, `launcher`
//! and `setup`; `capabilities/external-url.json` matches `ext-url-*`),
//! it keys the per-window menu registry, and it tells the window-event
//! handler whether a closing window is a project or a sandboxed
//! external-URL webview. Change the format here and the capability
//! globs move with it.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// Window kinds, as the renderer sees them in `window.__OXPLOW__.kind`.
pub const KIND_PROJECT: &str = "project";
pub const KIND_LAUNCHER: &str = "launcher";
pub const KIND_SETUP: &str = "setup";

/// What the shell tells a window about itself. Injected before any page
/// script runs, so the transport can pick its backend on first call
/// rather than asking over IPC (which would need a backend already).
pub struct WindowContext<'a> {
    /// Base URL of the daemon this window drives, or `None` while the
    /// shell still serves the backend in-process over Tauri IPC.
    pub base: Option<&'a str>,
    /// One of the `KIND_*` constants above.
    pub kind: &'a str,
}

/// The `initialization_script` for a window. Built through `serde_json`
/// so a base URL can't break out of the literal.
pub fn initialization_script(ctx: &WindowContext<'_>) -> String {
    let payload = serde_json::json!({ "base": ctx.base, "kind": ctx.kind });
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

    /// No daemon yet (the shell still serves the backend in-process):
    /// the renderer must see an explicit null, not a missing key, so it
    /// can tell "local" from "the shell forgot to inject anything".
    #[test]
    fn initialization_script_states_a_null_base_for_local_windows() {
        let script = initialization_script(&WindowContext {
            base: None,
            kind: KIND_PROJECT,
        });
        assert!(script.contains("window.__OXPLOW__ ="), "{script}");
        assert!(script.contains("\"base\":null"), "{script}");
        assert!(script.contains("\"kind\":\"project\""), "{script}");
    }

    #[test]
    fn initialization_script_carries_the_daemon_base() {
        let script = initialization_script(&WindowContext {
            base: Some("http://127.0.0.1:60331"),
            kind: KIND_PROJECT,
        });
        assert!(
            script.contains("\"base\":\"http://127.0.0.1:60331\""),
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
