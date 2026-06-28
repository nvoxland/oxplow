//! Launcher + project-open commands.
//!
//! These are the only commands the launcher window invokes. They
//! depend on the global [`RecentProjectsState`] + [`LaunchInfo`],
//! never on [`crate::AppState`] (`Services`), so they work whether or
//! not this process booted a project. `open_project` implements the
//! IntelliJ-style "process per window" model: each project window is
//! its own OS process, so opening a project = spawning a fresh
//! process with `OXPLOW_PROJECT_DIR` set, and "replace this window" =
//! spawn + exit the current process.

use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::IpcError;
use crate::state::{LaunchInfo, RecentProjectsState};

/// A recent-projects row plus a freshness flag for the UI.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecentProjectView {
    pub path: String,
    pub title: String,
    pub last_opened_at: i64,
    /// Whether the directory still exists on disk (drives the
    /// launcher's "missing" badge).
    pub exists: bool,
}

/// Whether this process booted into the launcher or a project, so the
/// renderer can pick the right top-level screen.
#[tauri::command]
#[specta::specta]
pub async fn get_launch_mode(launch: tauri::State<'_, LaunchInfo>) -> Result<LaunchInfo, IpcError> {
    Ok(launch.inner().clone())
}

/// Recent projects, most-recently-opened first, each tagged with
/// whether its directory still exists.
#[tauri::command]
#[specta::specta]
pub async fn list_recent_projects(
    recent: tauri::State<'_, RecentProjectsState>,
) -> Result<Vec<RecentProjectView>, IpcError> {
    let views = recent
        .list()
        .into_iter()
        .map(|p| RecentProjectView {
            exists: Path::new(&p.path).is_dir(),
            path: p.path,
            title: p.title,
            last_opened_at: p.last_opened_at,
        })
        .collect();
    Ok(views)
}

/// Forget a recent project (exact match on the stored path).
#[tauri::command]
#[specta::specta]
pub async fn remove_recent_project(
    recent: tauri::State<'_, RecentProjectsState>,
    path: String,
) -> Result<(), IpcError> {
    recent.remove(&path);
    Ok(())
}

/// Open `path` as a project. Spawns a new oxplow process pinned to
/// that directory. When `new_window` is false the current window is
/// replaced — we spawn the new process and then exit this one.
#[tauri::command]
#[specta::specta]
pub async fn open_project(
    app: tauri::AppHandle,
    recent: tauri::State<'_, RecentProjectsState>,
    path: String,
    new_window: bool,
) -> Result<(), IpcError> {
    let dir = Path::new(&path);
    if !dir.is_dir() {
        return Err(IpcError::invalid(format!(
            "project path is not a directory: {path}"
        )));
    }
    // Already open in another window? Focus that window instead of
    // spawning a duplicate (which would just hit the instance lock and
    // exit). If the running instance can't be reached (stale state),
    // fall back to a clear error.
    if oxplow_app::is_project_locked(dir) {
        if oxplow_app::request_focus(dir) {
            return Ok(());
        }
        return Err(IpcError::invalid(format!(
            "\"{path}\" is already open in another oxplow window"
        )));
    }
    recent.record(dir);
    spawn_project_process(dir)?;

    if !new_window {
        // Replace this window: the freshly spawned process owns the
        // new project; exiting ends our process (and its window). The
        // child is already detached and survives our exit.
        app.exit(0);
    }
    Ok(())
}

/// Whether `path` still needs first-run setup — i.e. it has no
/// `.oxplow/` dir yet. The launcher/app calls this before opening so a
/// declined setup never replaces an existing window.
#[tauri::command]
#[specta::specta]
pub async fn project_needs_setup(path: String) -> Result<bool, IpcError> {
    Ok(!Path::new(&path).join(".oxplow").is_dir())
}

/// Create the `.oxplow/` project structure in `path`, then relaunch
/// into it. The fresh process sees `.oxplow/` present and boots the
/// full app shell (via `run_project`); this setup window then exits.
#[tauri::command]
#[specta::specta]
pub async fn setup_project(app: tauri::AppHandle, path: String) -> Result<(), IpcError> {
    let dir = Path::new(&path);
    if !dir.is_dir() {
        return Err(IpcError::invalid(format!(
            "project path is not a directory: {path}"
        )));
    }
    oxplow_app::ensure_state_dir(&dir.join(".oxplow"))
        .map_err(|e| IpcError::internal(format!("create .oxplow: {e}")))?;
    spawn_project_process(dir)?;
    app.exit(0);
    Ok(())
}

/// Decline first-run setup: close this window by exiting the process.
#[tauri::command]
#[specta::specta]
pub async fn abort_setup(app: tauri::AppHandle) -> Result<(), IpcError> {
    app.exit(0);
    Ok(())
}

/// Spawn a fresh oxplow process pinned to `dir` (process-per-window),
/// mapping IO failures into the frontend error envelope.
fn spawn_project_process(dir: &Path) -> Result<(), IpcError> {
    oxplow_app::spawn_project_window(dir, false)
        .map_err(|e| IpcError::internal(format!("spawn project window: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory with no `.oxplow/` needs first-run setup; once the
    /// dir exists, it doesn't. The launcher gates window replacement on
    /// this, so a wrong answer either re-runs setup over a real project
    /// or boots an un-initialized dir straight into the app shell.
    #[tokio::test]
    async fn project_needs_setup_tracks_oxplow_dir_presence() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().to_string_lossy().into_owned();

        assert!(
            project_needs_setup(path.clone()).await.unwrap(),
            "fresh dir with no .oxplow should need setup"
        );

        std::fs::create_dir(tmp.path().join(".oxplow")).unwrap();
        assert!(
            !project_needs_setup(path).await.unwrap(),
            "dir with .oxplow should not need setup"
        );
    }

    /// A `.oxplow` *file* (not a directory) doesn't count as an
    /// initialized project — `is_dir()` must drive the decision.
    #[tokio::test]
    async fn project_needs_setup_ignores_a_non_dir_oxplow_entry() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join(".oxplow"), b"not a dir").unwrap();
        let path = tmp.path().to_string_lossy().into_owned();
        assert!(project_needs_setup(path).await.unwrap());
    }
}
