//! Launcher + project-open commands.
//!
//! Shell surface: these run in the shell process, never against a
//! project backend, because they are what happens *before* there is a
//! backend. Opening a project starts an `oxplow-daemon` for it and
//! creates a window pointed at that daemon (see
//! [`crate::windows::ShellWindows`]); "replace this window" closes the
//! window the command ran from once the new one exists.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::Manager;

use crate::error::IpcError;
use crate::state::RecentProjectsState;
use crate::windows::ShellWindows;

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

/// Open `path` as an existing project: start its daemon and put a
/// window in front of it, or focus the window that already has it. When
/// `new_window` is false the calling window is closed once the new one
/// is up. A folder that isn't a project yet is an error, not an implicit
/// create; see `create_project`.
#[tauri::command]
#[specta::specta]
pub async fn open_project(
    window: tauri::Window,
    path: String,
    new_window: bool,
) -> Result<(), IpcError> {
    resolve_open_target(&path)?;
    open_and_maybe_replace(window, PathBuf::from(&path), new_window).await
}

/// Create a new project in `path`: initialize `.oxplow/` and open the
/// result in a **new** window. Backs File ▸ New Project… and the
/// launcher's New Project button — the only two ways a folder becomes a
/// project from inside the app.
///
/// Always a new window, so creating a project can never close the
/// launcher or the window the user ran the command from. `path` must not
/// already be a project.
#[tauri::command]
#[specta::specta]
pub async fn create_project(window: tauri::Window, path: String) -> Result<(), IpcError> {
    let dir = resolve_create_target(&path)?;
    oxplow_app::ensure_state_dir(&dir.join(".oxplow"))
        .map_err(|e| IpcError::internal(format!("create .oxplow: {e}")))?;
    open_and_maybe_replace(window, PathBuf::from(&path), true).await
}

/// Confirm first-run setup: create `.oxplow/` in `path` and open it,
/// replacing this setup window.
#[tauri::command]
#[specta::specta]
pub async fn setup_project(window: tauri::Window, path: String) -> Result<(), IpcError> {
    let dir = Path::new(&path);
    if !dir.is_dir() {
        return Err(IpcError::invalid(format!(
            "project path is not a directory: {path}"
        )));
    }
    oxplow_app::ensure_state_dir(&dir.join(".oxplow"))
        .map_err(|e| IpcError::internal(format!("create .oxplow: {e}")))?;
    open_and_maybe_replace(window, PathBuf::from(&path), false).await
}

/// Decline first-run setup: close this window.
#[tauri::command]
#[specta::specta]
pub async fn abort_setup(window: tauri::Window) -> Result<(), IpcError> {
    window
        .close()
        .map_err(|e| IpcError::internal(format!("close window: {e}")))
}

/// Start `dir`'s daemon, open a window on it, and — unless the caller
/// asked for a new window — close the window the command came from.
///
/// The daemon runs a full boot (recovery, watchers, indexers) before it
/// reports an endpoint, and the endpoint has to be known before the
/// window is created, so the work happens on a blocking thread rather
/// than tying up an async worker for seconds.
async fn open_and_maybe_replace(
    window: tauri::Window,
    dir: PathBuf,
    new_window: bool,
) -> Result<(), IpcError> {
    let app = window.app_handle().clone();
    let opened = tauri::async_runtime::spawn_blocking(move || {
        let shell = app.state::<ShellWindows>();
        let recents = app.state::<RecentProjectsState>();
        shell.open_project(&app, &dir, recents.inner())
    })
    .await
    .map_err(|e| IpcError::internal(format!("open project: {e}")))?
    .map_err(IpcError::invalid)?;

    if !new_window && opened != window.label() {
        // Replace the caller: the new window exists, so closing this one
        // can't leave the user with nothing.
        window
            .close()
            .map_err(|e| IpcError::internal(format!("close window: {e}")))?;
    }
    Ok(())
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
