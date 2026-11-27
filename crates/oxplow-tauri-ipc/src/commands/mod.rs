//! Tauri command surface — split by area.
//!
//! Adding a new command: drop it in the relevant submodule, then
//! add the function name to `specta_builder()`'s `collect_commands![]`
//! list in `lib.rs`. The TS bindings regenerate on the next
//! `cargo test`.

/// Refuse a shell command that acts on this Mac (opens a folder or a file
/// here, writes its settings, pairs it) when a window on another machine's
/// host asks: that host's daemon could otherwise drive this Mac.
pub(crate) fn local_window_only(window: &tauri::Window) -> Result<(), crate::error::IpcError> {
    use tauri::Manager;
    let shell = window.app_handle().state::<crate::windows::ShellWindows>();
    if shell.registry.is_remote(window.label()) {
        return Err(crate::error::IpcError::invalid(
            "a window on another computer's host cannot do that on this Mac",
        ));
    }
    Ok(())
}

pub mod app;
pub mod background;
pub mod branch;
pub mod config;
pub mod generated;
pub mod hooks;
pub mod install_cli;
pub mod launch;
pub mod log;
pub mod lsp;
pub mod menu;
pub mod metrics;
pub mod oauth_redirect;
pub mod os_open;
pub mod page_refs;
pub mod page_visit;
pub mod remote_hosts;
pub mod search;
pub mod streams;
pub mod threads;
pub mod usage;
pub mod webview;
