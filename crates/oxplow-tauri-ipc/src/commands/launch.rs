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

/// Whether `dir` has been initialized as an Oxplow project — i.e. it
/// has a `.oxplow/` **directory** (a plain `.oxplow` file doesn't count).
fn is_project_dir(dir: &Path) -> bool {
    dir.join(".oxplow").is_dir()
}

/// Resolve `path` for **opening**: an existing directory that is already
/// an Oxplow project. Opening never initializes a folder — that is
/// `create_project`'s job — so a plain folder is refused with a message
/// naming the command that does create one.
fn resolve_open_target(path: &str) -> Result<&Path, IpcError> {
    let dir = Path::new(path);
    if !dir.is_dir() {
        return Err(IpcError::invalid(format!(
            "project path is not a directory: {path}"
        )));
    }
    if !is_project_dir(dir) {
        return Err(IpcError::invalid(format!(
            "\"{path}\" is not an Oxplow project (no .oxplow directory). Use File ▸ New Project… to create one."
        )));
    }
    Ok(dir)
}

/// Resolve `path` for **creating**: an existing directory that is not
/// already an Oxplow project. The mirror of [`resolve_open_target`] —
/// creating never adopts an existing project.
fn resolve_create_target(path: &str) -> Result<&Path, IpcError> {
    let dir = Path::new(path);
    if !dir.is_dir() {
        return Err(IpcError::invalid(format!(
            "project path is not a directory: {path}"
        )));
    }
    if is_project_dir(dir) {
        return Err(IpcError::invalid(format!(
            "\"{path}\" is already an Oxplow project. Use File ▸ Open Project… to open it."
        )));
    }
    Ok(dir)
}

/// Open `path` as an existing project. Spawns a new oxplow process
/// pinned to that directory. When `new_window` is false the current
/// window is replaced — we spawn the new process and then exit this one.
/// A folder that isn't a project yet is an error, not an implicit
/// create; see `create_project`.
#[tauri::command]
#[specta::specta]
pub async fn open_project(
    app: tauri::AppHandle,
    recent: tauri::State<'_, RecentProjectsState>,
    path: String,
    new_window: bool,
) -> Result<(), IpcError> {
    let dir = resolve_open_target(&path)?;
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

/// Create a new project in `path`: initialize `.oxplow/` and open the
/// result in a **new** window. Backs File ▸ New Project… and the
/// launcher's New Project button — the only two ways a folder becomes a
/// project from inside the app.
///
/// Always a new window (never `app.exit(0)`), so creating a project can
/// never close the launcher or the window the user ran the command
/// from. `path` must not already be a project.
#[tauri::command]
#[specta::specta]
pub async fn create_project(path: String) -> Result<(), IpcError> {
    let dir = resolve_create_target(&path)?;
    oxplow_app::ensure_state_dir(&dir.join(".oxplow"))
        .map_err(|e| IpcError::internal(format!("create .oxplow: {e}")))?;
    spawn_project_process(dir)
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

    /// A `.oxplow` *file* (not a directory) doesn't count as an
    /// initialized project — `is_dir()` must drive both decisions, or
    /// opening boots an un-initialized dir into the full app shell.
    #[test]
    fn a_non_dir_oxplow_entry_is_not_a_project() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join(".oxplow"), b"not a dir").unwrap();
        let path = tmp.path().to_string_lossy().into_owned();
        assert!(resolve_open_target(&path).is_err());
        assert!(resolve_create_target(&path).is_ok());
    }

    /// Opening never creates: a plain folder is refused, and the message
    /// names the command that *does* create one.
    #[test]
    fn open_refuses_a_folder_that_is_not_a_project() {
        let tmp = tempfile::TempDir::new().unwrap();
        let err = resolve_open_target(&tmp.path().to_string_lossy()).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("New Project"),
            "open error should point at New Project…, got: {msg}"
        );
    }

    #[test]
    fn open_accepts_an_initialized_project() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".oxplow")).unwrap();
        assert!(resolve_open_target(&tmp.path().to_string_lossy()).is_ok());
    }

    /// Creating never adopts: a folder that is already a project is
    /// refused, pointing at Open Project… instead.
    #[test]
    fn create_refuses_a_folder_that_is_already_a_project() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".oxplow")).unwrap();
        let err = resolve_create_target(&tmp.path().to_string_lossy()).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("Open Project"),
            "create error should point at Open Project…, got: {msg}"
        );
    }

    #[test]
    fn create_accepts_a_plain_folder() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(resolve_create_target(&tmp.path().to_string_lossy()).is_ok());
    }

    /// Both paths reject a non-directory before they get as far as the
    /// `.oxplow/` probe.
    #[test]
    fn neither_path_accepts_a_missing_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let file = tmp.path().join("a-file");
        std::fs::write(&file, b"x").unwrap();
        let path = file.to_string_lossy().into_owned();
        assert!(resolve_open_target(&path).is_err());
        assert!(resolve_create_target(&path).is_err());
    }
}
