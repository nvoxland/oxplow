//! "Install `oxplow` command line…": put the `oxplow` command on the
//! person's PATH.
//!
//! Shell surface, like [`super::launch`]: the running app's own binary is
//! the command line (`oxplow extension test`, `oxplow <dir>`), so
//! installing is one symlink to it. `/usr/local/bin` first; a person who
//! can't write there gets `~/.local/bin`, with a hint when that folder
//! isn't on their PATH.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::IpcError;

/// Where the command went, and what the person still has to do.
#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledCli {
    /// The symlink, `<folder>/oxplow`.
    pub path: String,
    /// Set when the folder isn't on the PATH: what to add.
    pub hint: Option<String>,
}

/// A folder the command may go in, and whether to create it.
struct Target {
    dir: PathBuf,
    create: bool,
}

/// Link `exe` as `oxplow` in the first of `targets` that takes it.
/// `path_var` is the PATH to check the folder against.
fn install_into(
    exe: &Path,
    targets: &[Target],
    path_var: &std::ffi::OsStr,
) -> Result<InstalledCli, String> {
    let mut last = "no folder to put it in".to_string();
    for target in targets {
        match link_in(exe, target) {
            Ok(link) => {
                let on_path = std::env::split_paths(path_var).any(|p| p == target.dir);
                return Ok(InstalledCli {
                    path: link.display().to_string(),
                    hint: (!on_path).then(|| {
                        format!(
                            "{} isn't on your PATH: add `export PATH=\"{}:$PATH\"` to your shell profile.",
                            target.dir.display(),
                            target.dir.display()
                        )
                    }),
                });
            }
            Err(e) => last = format!("{}: {e}", target.dir.display()),
        }
    }
    Err(last)
}

#[cfg(unix)]
fn link_in(exe: &Path, target: &Target) -> std::io::Result<PathBuf> {
    if target.create {
        std::fs::create_dir_all(&target.dir)?;
    }
    let link = target.dir.join("oxplow");
    // Only ever replace a link (an older install); never a file a person
    // or a package manager put there.
    if std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink()) {
        std::fs::remove_file(&link)?;
    }
    std::os::unix::fs::symlink(exe, &link)?;
    Ok(link)
}

#[cfg(not(unix))]
fn link_in(_exe: &Path, _target: &Target) -> std::io::Result<PathBuf> {
    Err(std::io::Error::other(
        "installing the command line isn't supported on this system",
    ))
}

/// Make `oxplow` run this app: a symlink in `/usr/local/bin`, else in
/// `~/.local/bin`.
#[tauri::command]
#[specta::specta]
pub async fn install_cli() -> Result<InstalledCli, IpcError> {
    let exe = std::env::current_exe()
        .map_err(|e| IpcError::internal(format!("find the running app: {e}")))?;
    let mut targets = vec![Target {
        dir: PathBuf::from("/usr/local/bin"),
        create: false,
    }];
    if let Some(home) = std::env::var_os("HOME") {
        targets.push(Target {
            dir: PathBuf::from(home).join(".local/bin"),
            create: true,
        });
    }
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    install_into(&exe, &targets, &path_var).map_err(IpcError::internal)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn target(dir: &Path, create: bool) -> Target {
        Target {
            dir: dir.to_path_buf(),
            create,
        }
    }

    /// The first folder that takes it gets a link to the app; a folder on
    /// the PATH needs no hint.
    #[test]
    fn the_command_links_to_the_running_app() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let exe = tmp.path().join("Oxplow");
        std::fs::write(&exe, "app").unwrap();
        let done = install_into(&exe, &[target(&bin, false)], bin.as_os_str()).unwrap();
        assert_eq!(done.path, bin.join("oxplow").display().to_string());
        assert_eq!(done.hint, None);
        assert_eq!(std::fs::read_link(bin.join("oxplow")).unwrap(), exe);
    }

    /// A folder that is missing (and not ours to create) is skipped for
    /// the next; one off the PATH says what to add.
    #[test]
    fn a_folder_that_will_not_do_falls_back_with_a_path_hint() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("Oxplow");
        std::fs::write(&exe, "app").unwrap();
        let missing = tmp.path().join("usr/local/bin");
        let home_bin = tmp.path().join("home/.local/bin");
        let done = install_into(
            &exe,
            &[target(&missing, false), target(&home_bin, true)],
            std::ffi::OsStr::new("/usr/bin"),
        )
        .unwrap();
        assert!(!missing.exists());
        assert_eq!(
            std::fs::read_link(home_bin.join("oxplow")).unwrap(),
            exe,
            "linked in the fallback"
        );
        let hint = done.hint.expect("not on the PATH");
        assert!(hint.contains(&home_bin.display().to_string()), "{hint}");
    }

    /// Installing again replaces the old link; a real file is left alone.
    #[test]
    fn it_replaces_an_old_link_and_never_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let (old, new) = (tmp.path().join("Old"), tmp.path().join("New"));
        std::fs::write(&old, "").unwrap();
        std::fs::write(&new, "").unwrap();
        let targets = [target(&bin, false)];
        install_into(&old, &targets, bin.as_os_str()).unwrap();
        install_into(&new, &targets, bin.as_os_str()).unwrap();
        assert_eq!(std::fs::read_link(bin.join("oxplow")).unwrap(), new);

        std::fs::remove_file(bin.join("oxplow")).unwrap();
        std::fs::write(bin.join("oxplow"), "a package manager's").unwrap();
        let err = install_into(&new, &targets, bin.as_os_str()).unwrap_err();
        assert!(err.contains("exists"), "{err}");
        assert_eq!(
            std::fs::read_to_string(bin.join("oxplow")).unwrap(),
            "a package manager's"
        );
    }
}
