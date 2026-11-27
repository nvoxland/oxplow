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

/// Extensions the OS runs, or hands to a program that runs them, rather
/// than opening in an app: bundles and installers, scripts, launcher
/// documents (a `.terminal` runs a shell, a `.fileloc` opens what it
/// names). On Windows, `PATHEXT` adds its own.
const PROGRAM_EXTENSIONS: &[&str] = &[
    // macOS
    "app",
    "command",
    "tool",
    "workflow",
    "action",
    "pkg",
    "mpkg",
    "terminal",
    "term",
    "fileloc",
    "inetloc",
    "webloc",
    "prefpane",
    "kext",
    "scpt",
    "applescript",
    "osascript",
    // Windows
    "exe",
    "com",
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "msi",
    "msp",
    "mst",
    "scr",
    "pif",
    "cpl",
    "msc",
    "reg",
    "hta",
    "js",
    "jse",
    "vb",
    "vbs",
    "vbe",
    "wsf",
    "wsh",
    "wsc",
    "lnk",
    "url",
    "scf",
    "application",
    "appref-ms",
    "gadget",
    "py",
    "pyw",
    // Anywhere
    "jar",
    "desktop",
    "appimage",
];

/// Extensions of a folder that is a program (a macOS bundle): a file
/// inside one is part of it.
const BUNDLE_EXTENSIONS: &[&str] = &[
    "app", "workflow", "action", "prefpane", "kext", "pkg", "mpkg",
];

/// Whether `ext` (lower case) is a program's.
fn is_program_extension(ext: &str) -> bool {
    if PROGRAM_EXTENSIONS.contains(&ext) {
        return true;
    }
    #[cfg(windows)]
    {
        if let Ok(list) = std::env::var("PATHEXT") {
            return list
                .split(';')
                .any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(ext));
        }
    }
    false
}

/// `path` if `use_` may be done with it: absolute and there. For
/// [`PathUse::Open`] it is what the path resolves to (symlinks and `..`
/// followed) that is opened, and that is not a program: it has no
/// program's extension, it is not inside a bundle (a file inside an app
/// is the app's), and it is not a file with an execute bit.
pub(crate) fn check_local_path(path: &str, use_: PathUse) -> Result<PathBuf, String> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(format!("not an absolute path: {path}"));
    }
    let meta = std::fs::metadata(p).map_err(|e| format!("{path}: {e}"))?;
    if use_ == PathUse::Reveal {
        return Ok(p.to_path_buf());
    }
    let real = std::fs::canonicalize(p).map_err(|e| format!("{path}: {e}"))?;
    let ext_of = |p: &Path| {
        p.extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
    };
    // The path itself by every program extension; a folder it is in only
    // by a bundle's (a repo folder named `three.js` holds documents).
    let program = ext_of(&real).is_some_and(|e| is_program_extension(&e))
        || real
            .ancestors()
            .skip(1)
            .any(|a| ext_of(a).is_some_and(|e| BUNDLE_EXTENSIONS.contains(&e.as_str())));
    if program {
        return Err(format!("{path} is a program: it is not opened"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
            return Err(format!("{path} is executable: it is not opened"));
        }
    }
    #[cfg(not(unix))]
    let _ = meta;
    Ok(real)
}

/// Show `path` in the OS file manager, selected.
#[tauri::command]
#[specta::specta]
pub async fn reveal_path(window: tauri::Window, path: String) -> Result<(), IpcError> {
    super::local_window_only(&window)?;
    let p = check_local_path(&path, PathUse::Reveal).map_err(IpcError::invalid)?;
    tauri_plugin_opener::reveal_item_in_dir(&p)
        .map_err(|e| IpcError::internal(format!("reveal {path}: {e}")))
}

/// Open `path` with its default app (a directory: in the file manager).
#[tauri::command]
#[specta::specta]
pub async fn open_path(window: tauri::Window, path: String) -> Result<(), IpcError> {
    super::local_window_only(&window)?;
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

    /// What is opened is what the path resolves to: a symlink to a
    /// program, a `..` back out to one, or a file inside an app bundle is
    /// a program too.
    #[cfg(unix)]
    #[test]
    fn a_path_that_resolves_to_a_program_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Marker.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/Info.plist"), "").unwrap();
        let link = dir.path().join("guide.pdf");
        std::os::unix::fs::symlink(&app, &link).unwrap();
        let back = format!("{}/Contents/..", app.display());
        for p in [s(&link), back, s(&app.join("Contents/Info.plist"))] {
            assert!(
                check_local_path(&p, PathUse::Open).is_err(),
                "{p} must not open"
            );
        }
    }

    #[test]
    fn a_document_in_a_folder_named_like_a_script_is_opened() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("three.js");
        std::fs::create_dir(&repo).unwrap();
        let doc = repo.join("README.md");
        std::fs::write(&doc, "").unwrap();
        assert!(check_local_path(&s(&doc), PathUse::Open).is_ok());
    }

    #[test]
    fn a_launcher_document_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "x.terminal",
            "x.term",
            "x.fileloc",
            "x.inetloc",
            "x.hta",
            "x.js",
            "x.reg",
        ] {
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
