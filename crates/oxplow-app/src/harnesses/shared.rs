//! What the harnesses share: shell quoting and the launch command's shape.

/// POSIX single-quote escape: wraps `'`, replaces internal `'` with
/// `'\''`.
pub fn shell_escape(s: &str) -> String {
    let escaped = s.replace('\'', r"'\''");
    format!("'{escaped}'")
}

/// `K='v' …` ahead of the command, or nothing.
pub fn env_prefix(env: &[(String, String)]) -> String {
    if env.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = env
        .iter()
        .map(|(k, v)| {
            assert!(
                k.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'),
                "invalid env var name: {k}"
            );
            format!("{k}={}", shell_escape(v))
        })
        .collect();
    format!("{} ", parts.join(" "))
}

/// What to exec, plus a preflight to run before it.
///
/// With a resolved absolute path there's nothing to check — PATH is out of
/// the picture. Without one we keep the bare name, because resolution is a
/// heuristic and the login shell may still find it, but we first test PATH
/// so a miss reports the actual cause: a bare `sh: claude: command not
/// found` lands under the stale-resume notice and reads as a session
/// problem.
pub fn program_and_guard(program: Option<&str>, bin: &str) -> (String, String) {
    match program {
        Some(path) => (shell_escape(path), String::new()),
        None => {
            let msg = format!(
                "[oxplow] agent CLI '{bin}' not found on PATH. If oxplow was launched from the \
                 Finder or the launcher it does not inherit your shell PATH (and `sh -l` does not \
                 read ~/.zshrc) — install {bin} to a standard location, or launch oxplow from a \
                 terminal. See DEV.md."
            );
            let guard = format!(
                "command -v {bin} >/dev/null 2>&1 || {{ echo {} >&2; exit 127; }}; ",
                shell_escape(&msg)
            );
            (bin.to_string(), guard)
        }
    }
}

/// `cd <cwd> && <guard><command>`, as one login shell runs it.
pub fn in_shell(cwd: &str, guard: &str, command: &str) -> String {
    let inner = format!("cd {} && {guard}{command}", shell_escape(cwd));
    format!("sh -lc {}", shell_escape(&inner))
}

/// A TOML string for a `--config key=value` override.
pub fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_escape_handles_apostrophes() {
        assert_eq!(shell_escape("it's"), r"'it'\''s'");
    }

    /// An unresolved program still gets its shot at PATH, with a legible
    /// message instead of a bare `command not found`; a resolved one needs
    /// no guard.
    #[test]
    fn an_unresolved_program_is_guarded_and_a_resolved_one_is_not() {
        let (prog, guard) = program_and_guard(None, "claude");
        assert_eq!(prog, "claude");
        assert!(guard.contains("command -v claude") && guard.contains("launched from the"));
        let (prog, guard) = program_and_guard(Some("/opt/agents/claude"), "claude");
        assert_eq!(
            (prog.as_str(), guard.as_str()),
            ("'/opt/agents/claude'", "")
        );
    }
}
