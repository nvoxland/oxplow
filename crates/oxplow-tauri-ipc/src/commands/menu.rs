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

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Wry};

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

/// Replace the app's native menu with the supplied snapshot. Each
/// activation fires `menu:command` with `{ id: "<command-id>" }` to
/// the renderer.
#[tauri::command]
#[specta::specta]
pub async fn set_native_menu(
    app: AppHandle,
    groups: Vec<MenuGroupSnapshot>,
) -> Result<(), IpcError> {
    let menu =
        build_menu(&app, &groups).map_err(|e| IpcError::internal(format!("menu build: {e}")))?;
    app.set_menu(menu)
        .map_err(|e| IpcError::internal(format!("set menu: {e}")))?;
    Ok(())
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
                role if role == "separator"
                    || role
                        .strip_prefix("separator.")
                        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) =>
                {
                    PredefinedMenuItem::separator(app)?
                }
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

/// Install the menu-event forwarder. Called once at app startup
/// from `main.rs`. Each menu activation re-emits as a Tauri event
/// the renderer subscribes to.
pub fn install_menu_handler(app: &AppHandle) {
    let handle = app.clone();
    app.on_menu_event(move |_app, event| {
        let id = event.id().0.clone();
        if let Err(err) = handle.emit("menu:command", MenuCommandEvent { id }) {
            tracing::warn!(?err, "failed to emit menu:command");
        }
    });
}

#[derive(Debug, Clone, Serialize)]
struct MenuCommandEvent {
    id: String,
}
