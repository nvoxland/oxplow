use std::sync::Arc;

/// The canonical state type registered with `tauri::Builder::manage`.
///
/// Use this exact alias from every `#[tauri::command]` parameter list:
/// `state: tauri::State<'_, AppState>`. A type mismatch is a runtime
/// panic, so consistency matters.
///
/// Managed only by a **project backend**. The desktop shell manages no
/// `Services` at all — every project runs in its own `oxplow-daemon`
/// child — so in the shell process these commands are never reached:
/// the renderer's transport sends everything but the shell surface to
/// the window's daemon.
pub type AppState = Arc<oxplow_app::Services>;

/// Global recent-projects store, managed by the shell so the launcher
/// and every project window can list / open / forget recent projects.
pub type RecentProjectsState = Arc<oxplow_config::RecentProjects>;

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
