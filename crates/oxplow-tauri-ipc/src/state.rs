use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;

/// The canonical state type registered with `tauri::Builder::manage`.
///
/// Use this exact alias from every `#[tauri::command]` parameter list:
/// `state: tauri::State<'_, AppState>`. A type mismatch is a runtime
/// panic, so consistency matters.
///
/// Present only in **project mode** (a window booted against a project
/// dir). In **launcher mode** there is no `Services`, so commands that
/// take `tauri::State<'_, AppState>` must not be invoked — the launcher
/// UI calls only the `commands::launch` surface, which depends on
/// [`RecentProjectsState`] / [`LaunchInfo`] instead.
pub type AppState = Arc<oxplow_app::Services>;

/// Global recent-projects store, managed in **both** launch modes so
/// the launcher screen and a running project window can both list /
/// open / forget recent projects.
pub type RecentProjectsState = Arc<oxplow_config::RecentProjects>;

/// Which mode the current process booted in. Managed (and returned by
/// `get_launch_mode`) so the renderer's `<Root>` can decide between the
/// launcher screen and the full app shell without guessing.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LaunchInfo {
    /// `"launcher"` or `"project"`.
    pub mode: String,
    /// The project dir when `mode == "project"`, else `None`.
    pub project_dir: Option<String>,
}

impl LaunchInfo {
    pub fn launcher() -> Self {
        Self {
            mode: "launcher".into(),
            project_dir: None,
        }
    }

    pub fn project(dir: impl Into<String>) -> Self {
        Self {
            mode: "project".into(),
            project_dir: Some(dir.into()),
        }
    }

    /// A directory was opened that isn't an Oxplow project yet (no
    /// `.oxplow/`). The renderer shows the setup-confirmation screen
    /// rather than booting straight into the app shell.
    pub fn setup(dir: impl Into<String>) -> Self {
        Self {
            mode: "setup".into(),
            project_dir: Some(dir.into()),
        }
    }
}

/// Tauri-managed handle to the in-process control plane (axum server
/// hosting hook + MCP routes) plus the per-spawn token. terminal.rs
/// reads it at agent-spawn time so it can materialize the plugin dir
/// and thread the URLs / token into the agent process env.
///
/// The struct itself lives in `oxplow-rpc` (the daemon constructs the
/// same shape from its own control plane); this re-export keeps the
/// historical `oxplow_tauri_ipc::PluginRuntime` path working.
///
/// Decoupled from `Services` so the boot order doesn't gain a new
/// dependency: control-plane spawn happens after `Services::boot`,
/// inside the Tauri shell, and is registered via `.manage(…)`
/// alongside `AppState`.
pub use oxplow_rpc::PluginRuntime;

pub type PluginRuntimeState = Arc<PluginRuntime>;
