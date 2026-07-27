//! Resolve an agent CLI to an absolute path, and widen the PATH handed to
//! spawned terminals (tsk245).
//!
//! # Why this exists
//!
//! The agent runs in a PTY as `sh -lc "<agent cmd>"`. Two facts combine into
//! a launch failure that only shows up for some users:
//!
//! 1. A **GUI-launched** app (Finder, the dock, oxplow's own launcher) gets
//!    macOS's minimal PATH — `/usr/bin:/bin:/usr/sbin:/sbin` — not the user's
//!    shell PATH. A terminal-launched app inherits the shell's PATH and the
//!    PTY child inherits it in turn, which is the only reason this ever works.
//! 2. `sh` is bash-in-sh-mode. Even as a **login** shell it reads
//!    `/etc/profile` and `~/.profile` — never `~/.zshrc` or `~/.zprofile`. So
//!    the `-l` does not recover a zsh user's PATH, which is where most people
//!    put `~/.local/bin`.
//!
//! Net effect: launch oxplow from the launcher and every agent pane dies with
//! `sh: claude: command not found`, while the same build launched from a
//! terminal works. Claude Code's default install location (`~/.local/bin`) is
//! in exactly the gap.
//!
//! # What we do
//!
//! Probe PATH, then a list of bin dirs agent CLIs actually install into, and
//! spawn the **absolute path** so PATH stops mattering for the agent binary.
//! [`augmented_path`] additionally appends those dirs to the PATH the PTY
//! inherits, so tools the agent itself shells out to have a chance too.
//!
//! This is deliberately a fixed-list heuristic and not a login-shell env
//! capture: it costs no subprocess and cannot hang on someone's rc file. The
//! limit is that version-manager shims (mise, nvm, volta installs under a
//! versioned directory) are not on the list and still need a terminal launch.

use oxplow_domain::AgentKind;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The user's home dir, or `None` when `HOME` is unset.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Bin dirs that commonly hold an agent CLI but are missing from a
/// GUI-launched app's PATH. Order is preference order.
///
/// Keep this list to places a CLI is *installed*, not every dir a user might
/// have on PATH — it's a fallback for the GUI-launch gap, not a PATH
/// replacement.
pub fn well_known_bin_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        // Claude Code's default install target, and the general XDG-ish
        // user-local bin. This is the one that bites in practice.
        home.join(".local/bin"),
        // Claude Code's older self-contained install.
        home.join(".claude/local"),
        // Homebrew: Apple Silicon, then Intel/Linuxbrew.
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        // JS runtimes people install agent CLIs through.
        home.join(".bun/bin"),
        home.join(".volta/bin"),
        home.join(".npm-global/bin"),
        home.join(".yarn/bin"),
        // Rust-installed tooling.
        home.join(".cargo/bin"),
    ]
}

/// First readable `bin` found on `path_var`, then in `extra`. `None` when it
/// is nowhere — callers keep the bare name so the shell gets its own try.
///
/// A `bin` that already contains a separator is taken as a path and checked
/// directly, mirroring `command -v` and the LSP installer's `binary_exists`.
pub fn resolve_program_in(
    bin: &str,
    path_var: Option<&OsStr>,
    extra: &[PathBuf],
) -> Option<PathBuf> {
    let as_path = Path::new(bin);
    if as_path.components().count() > 1 {
        return as_path.is_file().then(|| as_path.to_path_buf());
    }
    let from_path = path_var
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(bin))
        .find(|c| c.is_file());
    from_path.or_else(|| extra.iter().map(|dir| dir.join(bin)).find(|c| c.is_file()))
}

/// [`resolve_program_in`] for an agent, against the live environment.
pub fn resolve_agent_program(kind: AgentKind) -> Option<String> {
    let extra = home_dir()
        .map(|h| well_known_bin_dirs(&h))
        .unwrap_or_default();
    resolve_program_in(kind.as_str(), std::env::var_os("PATH").as_deref(), &extra)
        .map(|p| p.to_string_lossy().into_owned())
}

/// `path_var` with every existing dir in `extra` appended, skipping ones
/// already present. `None` when nothing would change, so callers can leave the
/// child's inherited PATH alone rather than pinning a snapshot of it.
pub fn augmented_path_in(path_var: Option<&OsStr>, extra: &[PathBuf]) -> Option<String> {
    let mut dirs: Vec<PathBuf> = path_var
        .into_iter()
        .flat_map(std::env::split_paths)
        .collect();
    let before = dirs.len();
    for dir in extra {
        if !dirs.contains(dir) && dir.is_dir() {
            dirs.push(dir.clone());
        }
    }
    if dirs.len() == before {
        return None;
    }
    std::env::join_paths(dirs)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// [`augmented_path_in`] against the live environment.
pub fn augmented_path() -> Option<String> {
    let extra = home_dir()
        .map(|h| well_known_bin_dirs(&h))
        .unwrap_or_default();
    augmented_path_in(std::env::var_os("PATH").as_deref(), &extra)
}

/// The env every agent/terminal PTY is spawned with. One place so the five
/// spawn sites can't drift on whether they widen PATH.
pub fn base_pty_env() -> Vec<(String, String)> {
    let mut env = vec![
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
    ];
    if let Some(path) = augmented_path() {
        env.push(("PATH".to_string(), path));
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dir containing an executable-ish file named `bin`.
    fn dir_with(root: &Path, name: &str, bin: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(bin), "#!/bin/sh\n").unwrap();
        dir
    }

    #[test]
    fn resolves_from_a_well_known_dir_when_path_does_not_have_it() {
        // The actual tsk245 failure: a GUI-launched app's PATH has no
        // ~/.local/bin, so PATH lookup fails and only the fallback list saves
        // the launch.
        let tmp = tempfile::tempdir().unwrap();
        let local_bin = dir_with(tmp.path(), "local-bin", "claude");
        let empty = dir_with(tmp.path(), "empty", "unrelated");

        let path_var = std::env::join_paths([&empty]).unwrap();
        let got = resolve_program_in(
            "claude",
            Some(path_var.as_os_str()),
            std::slice::from_ref(&local_bin),
        );
        assert_eq!(got, Some(local_bin.join("claude")));
    }

    #[test]
    fn path_wins_over_the_fallback_list() {
        // The fallback must never shadow a binary the user deliberately put on
        // PATH — that would silently run a different install.
        let tmp = tempfile::tempdir().unwrap();
        let on_path = dir_with(tmp.path(), "on-path", "claude");
        let fallback = dir_with(tmp.path(), "fallback", "claude");

        let path_var = std::env::join_paths([&on_path]).unwrap();
        let got = resolve_program_in("claude", Some(path_var.as_os_str()), &[fallback]);
        assert_eq!(got, Some(on_path.join("claude")));
    }

    #[test]
    fn unresolvable_binary_is_none_so_the_caller_can_keep_the_bare_name() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = dir_with(tmp.path(), "empty", "unrelated");
        let path_var = std::env::join_paths([&empty]).unwrap();
        assert_eq!(
            resolve_program_in("claude", Some(path_var.as_os_str()), &[]),
            None
        );
        // No PATH at all is the same story, not a panic.
        assert_eq!(resolve_program_in("claude", None, &[]), None);
    }

    #[test]
    fn an_explicit_path_is_used_verbatim_and_checked() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = dir_with(tmp.path(), "somewhere", "claude").join("claude");
        let spelled = bin.to_string_lossy().into_owned();
        assert_eq!(resolve_program_in(&spelled, None, &[]), Some(bin));
        // A path that doesn't exist resolves to nothing rather than being
        // handed to the shell as if it were fine.
        let missing = tmp.path().join("nope/claude");
        assert_eq!(
            resolve_program_in(&missing.to_string_lossy(), None, &[]),
            None
        );
    }

    #[test]
    fn augmented_path_appends_only_missing_existing_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let already = dir_with(tmp.path(), "already", "x");
        let addition = dir_with(tmp.path(), "addition", "y");
        let absent = tmp.path().join("does-not-exist");

        let path_var = std::env::join_paths([&already]).unwrap();
        let got = augmented_path_in(
            Some(path_var.as_os_str()),
            &[already.clone(), addition.clone(), absent],
        )
        .expect("one dir was added");
        let dirs: Vec<PathBuf> = std::env::split_paths(&got).collect();
        assert_eq!(
            dirs,
            vec![already, addition],
            "existing entry kept once, in front; missing dir not invented"
        );
    }

    #[test]
    fn augmented_path_is_none_when_it_would_change_nothing() {
        // Returning None lets the caller leave PATH unset on the child, so the
        // PTY inherits it live instead of freezing today's value.
        let tmp = tempfile::tempdir().unwrap();
        let already = dir_with(tmp.path(), "already", "x");
        let path_var = std::env::join_paths([&already]).unwrap();
        assert_eq!(
            augmented_path_in(Some(path_var.as_os_str()), &[already]),
            None
        );
        assert_eq!(augmented_path_in(None, &[]), None);
    }

    #[test]
    fn well_known_dirs_lead_with_the_default_claude_install() {
        let dirs = well_known_bin_dirs(Path::new("/home/u"));
        assert_eq!(dirs.first().unwrap(), Path::new("/home/u/.local/bin"));
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
    }

    #[test]
    fn base_pty_env_always_carries_the_terminal_type() {
        let env = base_pty_env();
        assert!(env
            .iter()
            .any(|(k, v)| k == "TERM" && v == "xterm-256color"));
        assert!(env.iter().any(|(k, _)| k == "COLORTERM"));
    }
}
