//! Native menu wiring.
//!
//! The renderer builds a `Vec<MenuGroupSnapshot>` from its
//! `commands.ts` state machine and pushes it through
//! `set_native_menu`. We translate it into a `tauri::menu::Menu`
//! with one submenu per group and one menu item per command, then
//! install it as the app menu. Menu activations forward to the
//! renderer over the `menu:command` event channel as the original
//! command id (e.g. `"file.save"`); the renderer's
//! `subscribeMenuCommand` listener fires the matching handler.
//!
//! Only the macOS native menu bar is exercised today — on Windows
//! and Linux Tauri renders the same items as a window menu.
//! Accelerators come over verbatim from the snapshot; Tauri parses
//! `Ctrl/Cmd+S`-style strings via its own accelerator codec.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::error::IpcError;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MenuItemSnapshot {
    pub id: String,
    pub label: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub checked: Option<bool>,
    /// When present, this item is a nested submenu of these children
    /// (e.g. File ▸ Open Recent ▸ …) rather than a leaf command.
    #[serde(default)]
    pub submenu: Option<Vec<MenuItemSnapshot>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MenuGroupSnapshot {
    pub id: String,
    pub label: String,
    pub items: Vec<MenuItemSnapshot>,
}

/// The menu bar is app-global (`AppHandle::set_menu`) but every window
/// has its own menu state, so the shell keeps each window's latest
/// snapshot here and installs whichever belongs to the focused window.
///
/// A window with no snapshot — an external-URL webview, or a project
/// window whose renderer hasn't pushed yet — never takes the menu bar
/// over: focusing one leaves the current menu in place rather than
/// blanking it.
#[derive(Default)]
pub struct MenuRegistry {
    inner: Mutex<RegistryState>,
}

#[derive(Default)]
struct RegistryState {
    snapshots: HashMap<String, Vec<MenuGroupSnapshot>>,
    /// Label of the window that last took focus, whether or not it has
    /// a snapshot. `None` before the first focus event and after the
    /// focused window closes.
    focused: Option<String>,
}

impl MenuRegistry {
    /// Record `label`'s menu. Returns whether it should be installed
    /// now: only the focused window may write the menu bar, plus the
    /// pre-first-focus case (boot, where the renderer can push before
    /// the OS reports focus — otherwise the app would start menu-less).
    pub fn store(&self, label: &str, groups: Vec<MenuGroupSnapshot>) -> bool {
        let mut state = self.lock();
        state.snapshots.insert(label.to_string(), groups);
        match &state.focused {
            None => true,
            Some(focused) => focused == label,
        }
    }

    /// Mark `label` focused, returning the snapshot to install — `None`
    /// when that window has none, which means "leave the menu alone".
    pub fn focus(&self, label: &str) -> Option<Vec<MenuGroupSnapshot>> {
        let mut state = self.lock();
        state.focused = Some(label.to_string());
        state.snapshots.get(label).cloned()
    }

    /// Drop a closed window's menu. Clearing `focused` matters: macOS
    /// doesn't always focus the next window before the replacement
    /// renderer pushes, and a stale focus label would swallow it.
    pub fn forget(&self, label: &str) {
        let mut state = self.lock();
        state.snapshots.remove(label);
        if state.focused.as_deref() == Some(label) {
            state.focused = None;
        }
    }

    /// The window a menu activation belongs to.
    pub fn focused_label(&self) -> Option<String> {
        self.lock().focused.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RegistryState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Replace the native menu with the calling window's snapshot. Each
/// activation fires `menu:command` with `{ id: "<command-id>" }` back
/// to the window that owns the menu.
///
/// The snapshot is always recorded; it is only *installed* when this
/// window owns the menu bar (see [`MenuRegistry`]), so a background
/// window's menu churn can't overwrite the foreground one's.
#[tauri::command]
#[specta::specta]
pub async fn set_native_menu(
    window: tauri::Window,
    groups: Vec<MenuGroupSnapshot>,
) -> Result<(), IpcError> {
    let app = window.app_handle().clone();
    let registry = app.state::<MenuRegistry>();
    if !registry.store(window.label(), groups.clone()) {
        return Ok(());
    }
    install_menu(&app, &groups)
}

fn install_menu(app: &AppHandle, groups: &[MenuGroupSnapshot]) -> Result<(), IpcError> {
    let menu =
        build_menu(app, groups).map_err(|e| IpcError::internal(format!("menu build: {e}")))?;
    app.set_menu(menu)
        .map_err(|e| IpcError::internal(format!("set menu: {e}")))?;
    Ok(())
}

/// Re-install the menu for a window that just took focus. Called from
/// the shell's window-event handler; a no-op for windows with no
/// snapshot of their own.
pub fn apply_focused_menu(window: &tauri::Window) {
    let app = window.app_handle().clone();
    let Some(groups) = app.state::<MenuRegistry>().focus(window.label()) else {
        return;
    };
    if let Err(e) = install_menu(&app, &groups) {
        tracing::warn!(error = %e.message, label = window.label(), "failed to apply focused menu");
    }
}

/// Forget a closing window's menu.
pub fn forget_window_menu(window: &tauri::Window) {
    window
        .app_handle()
        .state::<MenuRegistry>()
        .forget(window.label());
}

fn build_menu(app: &AppHandle, groups: &[MenuGroupSnapshot]) -> tauri::Result<Menu<Wry>> {
    let mut menu = MenuBuilder::new(app);
    // On macOS the first submenu is always rendered bold under the
    // application name, so prepend a proper app menu (About / Hide /
    // Quit). Without it, the renderer's first group (File) lands under
    // the app-name slot and there's no visible Quit. The label is
    // irrelevant on macOS — the OS substitutes the app name — but the
    // predefined Hide/Show All items are macOS-shaped, so this is
    // macOS-only; off-Mac the in-window Menubar carries File/Edit/etc.
    #[cfg(target_os = "macos")]
    {
        let app_menu = SubmenuBuilder::new(app, "Oxplow")
            .item(&PredefinedMenuItem::about(app, None, None)?)
            .item(&PredefinedMenuItem::separator(app)?)
            .item(&PredefinedMenuItem::hide(app, None)?)
            .item(&PredefinedMenuItem::hide_others(app, None)?)
            .item(&PredefinedMenuItem::show_all(app, None)?)
            .item(&PredefinedMenuItem::separator(app)?)
            .item(&PredefinedMenuItem::quit(app, None)?)
            .build()?;
        menu = menu.item(&app_menu);
    }
    for group in groups {
        let submenu = build_submenu(app, &group.label, &group.items)?;
        menu = menu.item(&submenu);
    }
    menu.build()
}

/// Build one submenu (a menu group, or a nested `submenu` item like
/// File ▸ Open Recent). Recurses for items that carry their own
/// `submenu` children.
fn build_submenu(
    app: &AppHandle,
    label: &str,
    items: &[MenuItemSnapshot],
) -> tauri::Result<tauri::menu::Submenu<Wry>> {
    let mut submenu = SubmenuBuilder::new(app, label);
    for item in items {
        // Items with id "native.<role>" map to the OS predefined
        // menu items (Cut/Copy/Paste/SelectAll/Undo/Redo). These
        // dispatch through the macOS responder chain so the
        // standard keyboard shortcuts (Cmd+V, Cmd+C, …) reach
        // the focused webview — without them, WKWebView swallows
        // Cmd+V and JS keydown handlers never see it.
        if let Some(role) = item.id.strip_prefix("native.") {
            let predefined = match role {
                "undo" => PredefinedMenuItem::undo(app, Some(&item.label))?,
                "redo" => PredefinedMenuItem::redo(app, Some(&item.label))?,
                "cut" => PredefinedMenuItem::cut(app, Some(&item.label))?,
                "copy" => PredefinedMenuItem::copy(app, Some(&item.label))?,
                "paste" => PredefinedMenuItem::paste(app, Some(&item.label))?,
                "selectAll" => PredefinedMenuItem::select_all(app, Some(&item.label))?,
                role if is_separator_role(role) => PredefinedMenuItem::separator(app)?,
                _ => {
                    tracing::warn!(role, "unknown native menu role; skipping");
                    continue;
                }
            };
            submenu = submenu.item(&predefined);
            continue;
        }

        // Nested submenu (e.g. Open Recent ▸ <project>).
        if let Some(children) = &item.submenu {
            let child = build_submenu(app, &item.label, children)?;
            submenu = submenu.item(&child);
            continue;
        }

        let mut builder =
            MenuItemBuilder::with_id(item.id.clone(), &item.label).enabled(item.enabled);
        if let Some(shortcut) = item.shortcut.as_deref().filter(|s| !s.is_empty()) {
            let normalized = normalize_accelerator(shortcut);
            builder = builder.accelerator(normalized);
        }
        let menu_item = builder.build(app)?;
        submenu = submenu.item(&menu_item);
    }
    submenu.build()
}

/// The renderer ships accelerators in human-readable form
/// (`"Ctrl/Cmd+S"`, `"Ctrl/Cmd+Shift+N"`). Tauri's accelerator
/// codec accepts `CmdOrCtrl+S` for the same intent — translate.
fn normalize_accelerator(s: &str) -> String {
    s.replace("Ctrl/Cmd", "CmdOrCtrl")
        .replace("Cmd/Ctrl", "CmdOrCtrl")
}

/// Whether a `native.<role>` id names a separator. Bare `"separator"`
/// is the common case; the renderer also emits numbered variants
/// (`"separator.0"`, `"separator.1"`, …) so several separators can
/// coexist in one menu without colliding on a duplicate id. The digit
/// suffix is purely a uniqueness tag — `"separator."` (empty suffix)
/// or a non-numeric suffix (`"separator.x"`) is NOT a separator and
/// falls through to the unknown-role warning.
fn is_separator_role(role: &str) -> bool {
    role == "separator"
        || role
            .strip_prefix("separator.")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Install the per-window menu registry and the menu-event forwarder.
/// Called once at app startup from `main.rs`, before any window is
/// created — the renderer pushes its menu as soon as it loads, and
/// `set_native_menu` needs the registry managed by then.
///
/// Activations go to the window that owns the menu bar, not to every
/// window: with several projects open, Cmd-S belongs to exactly one.
pub fn install_menu_handler(app: &AppHandle) {
    app.manage(MenuRegistry::default());
    let handle = app.clone();
    app.on_menu_event(move |_app, event| {
        let payload = MenuCommandEvent {
            id: event.id().0.clone(),
        };
        let target = handle.state::<MenuRegistry>().focused_label();
        let result = match target {
            Some(label) => handle.emit_to(label, "menu:command", payload),
            // No window has taken focus yet — broadcast rather than
            // drop the activation.
            None => handle.emit("menu:command", payload),
        };
        if let Err(err) = result {
            tracing::warn!(?err, "failed to emit menu:command");
        }
    });
}

#[derive(Debug, Clone, Serialize)]
struct MenuCommandEvent {
    id: String,
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    fn groups(label: &str) -> Vec<MenuGroupSnapshot> {
        vec![MenuGroupSnapshot {
            id: "file".into(),
            label: label.into(),
            items: vec![],
        }]
    }

    fn labels(groups: &[MenuGroupSnapshot]) -> Vec<String> {
        groups.iter().map(|g| g.label.clone()).collect()
    }

    /// Boot order isn't guaranteed: the renderer can push its menu
    /// before the OS reports the window focused. Suppressing that push
    /// would leave the app with no menu bar until the user clicked away
    /// and back.
    #[test]
    fn the_first_push_is_installed_even_before_any_focus_event() {
        let reg = MenuRegistry::default();
        assert!(reg.store("project-1", groups("File")));
    }

    #[test]
    fn a_push_from_the_focused_window_is_installed() {
        let reg = MenuRegistry::default();
        reg.store("project-1", groups("File"));
        reg.focus("project-1");
        assert!(reg.store("project-1", groups("File*")));
    }

    /// The whole point of the registry: a background window whose menu
    /// state changes must not repaint the foreground window's menu bar.
    #[test]
    fn a_push_from_a_background_window_is_recorded_but_not_installed() {
        let reg = MenuRegistry::default();
        reg.store("project-1", groups("File 1"));
        reg.store("project-2", groups("File 2"));
        reg.focus("project-2");

        assert!(
            !reg.store("project-1", groups("File 1 edited")),
            "project-1 is in the background; it may not write the menu bar"
        );
        // …but the push was kept, so refocusing shows the latest.
        assert_eq!(
            labels(&reg.focus("project-1").expect("stored snapshot")),
            vec!["File 1 edited"]
        );
    }

    #[test]
    fn focusing_a_window_returns_its_own_snapshot() {
        let reg = MenuRegistry::default();
        reg.store("project-1", groups("File 1"));
        reg.store("project-2", groups("File 2"));
        assert_eq!(
            labels(&reg.focus("project-2").expect("stored snapshot")),
            vec!["File 2"]
        );
        assert_eq!(
            labels(&reg.focus("project-1").expect("stored snapshot")),
            vec!["File 1"]
        );
    }

    /// External-URL webviews have no menu of their own. Focusing one
    /// must leave the menu bar as-is rather than clearing it.
    #[test]
    fn focusing_a_menuless_window_leaves_the_menu_alone() {
        let reg = MenuRegistry::default();
        reg.store("project-1", groups("File"));
        assert!(reg.focus("ext-url-abc").is_none());
    }

    #[test]
    fn closing_the_focused_window_lets_the_next_push_install() {
        let reg = MenuRegistry::default();
        reg.store("project-1", groups("File 1"));
        reg.focus("project-1");
        reg.forget("project-1");

        assert!(
            reg.store("project-2", groups("File 2")),
            "with no focused window, the next push owns the menu bar"
        );
        assert!(
            reg.focus("project-1").is_none(),
            "the closed window's snapshot is gone"
        );
    }

    /// Menu activations are emitted to one window, not broadcast —
    /// Cmd-S in window B must not save in window A.
    #[test]
    fn focused_label_names_the_window_activations_go_to() {
        let reg = MenuRegistry::default();
        assert_eq!(reg.focused_label(), None);
        reg.focus("project-2");
        assert_eq!(reg.focused_label().as_deref(), Some("project-2"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_accelerator_translates_both_combo_spellings() {
        assert_eq!(normalize_accelerator("Ctrl/Cmd+S"), "CmdOrCtrl+S");
        assert_eq!(normalize_accelerator("Cmd/Ctrl+S"), "CmdOrCtrl+S");
        assert_eq!(
            normalize_accelerator("Ctrl/Cmd+Shift+N"),
            "CmdOrCtrl+Shift+N"
        );
    }

    #[test]
    fn normalize_accelerator_leaves_plain_shortcuts_untouched() {
        // No combo token to rewrite — passes through verbatim so Tauri's
        // codec sees exactly what the renderer sent.
        assert_eq!(normalize_accelerator("Cmd+K"), "Cmd+K");
        assert_eq!(normalize_accelerator("F2"), "F2");
        assert_eq!(normalize_accelerator(""), "");
    }

    #[test]
    fn is_separator_role_accepts_bare_and_numbered() {
        assert!(is_separator_role("separator"));
        assert!(is_separator_role("separator.0"));
        assert!(is_separator_role("separator.1"));
        assert!(is_separator_role("separator.42"));
    }

    #[test]
    fn is_separator_role_rejects_empty_or_nonnumeric_suffix() {
        // The digit suffix is a uniqueness tag; anything else is a real
        // command id (or a typo) and must fall through, not silently
        // render as a separator.
        assert!(!is_separator_role("separator."));
        assert!(!is_separator_role("separator.x"));
        assert!(!is_separator_role("separator.1a"));
        assert!(!is_separator_role("separatorx"));
        assert!(!is_separator_role("undo"));
        assert!(!is_separator_role(""));
    }
}
