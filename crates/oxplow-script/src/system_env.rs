//! What a child process started with a cleared environment still needs.
//!
//! oxplow starts script workers, `exec` programs, collectors and providers
//! with `env_clear()`, so nothing of the host's environment leaks to them.
//! On Windows a process with none of the system variables below often can't
//! start (`SystemRoot` is how DLLs, `cmd` and the network stack are found),
//! so those are passed on; they name system folders, not the person's
//! files or secrets. Unix has no such list.

use std::ffi::OsString;

/// The system variables a Windows child needs; empty elsewhere.
#[cfg(windows)]
pub const SYSTEM_ENV: &[&str] = &[
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATHEXT",
    "TEMP",
    "TMP",
];

/// The system variables a Windows child needs; empty elsewhere.
#[cfg(not(windows))]
pub const SYSTEM_ENV: &[&str] = &[];

/// Each of [`SYSTEM_ENV`] that the host has set, with its value.
pub fn system_env() -> impl Iterator<Item = (&'static str, OsString)> {
    SYSTEM_ENV
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|v| (*name, v)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_children_keep_the_system_root() {
        assert!(system_env().any(|(name, _)| name.eq_ignore_ascii_case("SystemRoot")));
    }

    #[cfg(not(windows))]
    #[test]
    fn other_children_keep_nothing() {
        assert_eq!(system_env().count(), 0);
    }
}
