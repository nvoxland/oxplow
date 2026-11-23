//! A local path, outside oxplow: shown in the OS file manager (Finder,
//! Explorer, the desktop's file manager) or opened with its default app —
//! the right-click Open In ▸ of a path the window shows. The shell does it
//! (it is the person's machine), through `tauri-plugin-opener`'s own
//! functions: the webview gets no opener permission, and the shell
//! plugin's `open` stays http(s) only.

use std::path::{Path, PathBuf};

use crate::error::IpcError;

/// What is done with a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathUse {
    /// Shown in the file manager: any existing path.
    Reveal,
    /// Opened with its default app: never a program. A path in a
    /// terminal's output or a markdown link must not run one.
    Open,
}

/// Extensions the OS runs rather than opens in an app.
const PROGRAM_EXTENSIONS: &[&str] = &[
    "app", "command", "tool", "workflow", "pkg", "mpkg", "exe", "com", "bat", "cmd", "ps1", "msi",
    "scr", "vbs", "vbe", "wsf", "wsh", "lnk", "jar", "desktop", "appimage",
];

/// `path` if `use_` may be done with it: absolute and there, and for
/// [`PathUse::Open`] not a program (a program's extension, or a file with
/// an execute bit).
pub(crate) fn check_local_path(path: &str, use_: PathUse) -> Result<PathBuf, String> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(format!("not an absolute path: {path}"));
    }
    let meta = std::fs::metadata(p).map_err(|e| format!("{path}: {e}"))?;
    if use_ == PathUse::Open {
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        if ext
            .as_deref()
            .is_some_and(|e| PROGRAM_EXTENSIONS.contains(&e))
        {
            return Err(format!("{path} is a program: it is not opened"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
                return Err(format!("{path} is executable: it is not opened"));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = meta;
    Ok(p.to_path_buf())
}

/// Show `path` in the OS file manager, selected.
#[tauri::command]
#[specta::specta]
pub async fn reveal_path(path: String) -> Result<(), IpcError> {
    let p = check_local_path(&path, PathUse::Reveal).map_err(IpcError::invalid)?;
    tauri_plugin_opener::reveal_item_in_dir(&p)
        .map_err(|e| IpcError::internal(format!("reveal {path}: {e}")))
}

/// Open `path` with its default app (a directory: in the file manager).
#[tauri::command]
#[specta::specta]
pub async fn open_path(path: String) -> Result<(), IpcError> {
    let p = check_local_path(&path, PathUse::Open).map_err(IpcError::invalid)?;
    tauri_plugin_opener::open_path(&p, None::<&str>)
        .map_err(|e| IpcError::internal(format!("open {path}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn a_relative_or_missing_path_is_refused() {
        assert!(check_local_path("src/main.rs", PathUse::Reveal).is_err());
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.txt");
        assert!(check_local_path(&s(&missing), PathUse::Reveal).is_err());
    }

    #[test]
    fn any_existing_path_is_revealed() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Thing.app");
        std::fs::create_dir(&app).unwrap();
        assert_eq!(check_local_path(&s(&app), PathUse::Reveal).unwrap(), app);
        assert!(check_local_path(&s(dir.path()), PathUse::Reveal).is_ok());
    }

    #[test]
    fn a_document_or_directory_is_opened() {
        let dir = tempfile::tempdir().unwrap();
        let doc = dir.path().join("notes.md");
        std::fs::write(&doc, "# hi").unwrap();
        assert!(check_local_path(&s(&doc), PathUse::Open).is_ok());
        assert!(check_local_path(&s(dir.path()), PathUse::Open).is_ok());
    }

    #[test]
    fn a_program_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["Thing.app", "run.command", "setup.EXE", "x.bat"] {
            let p = dir.path().join(name);
            std::fs::write(&p, "").unwrap();
            assert!(
                check_local_path(&s(&p), PathUse::Open).is_err(),
                "{name} must not open"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_executable_file_is_not_opened() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("build");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(check_local_path(&s(&script), PathUse::Open).is_err());
        assert!(check_local_path(&s(&script), PathUse::Reveal).is_ok());
    }
}
