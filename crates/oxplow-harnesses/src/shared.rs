//! What the harnesses share: shell quoting, the launch command's shape,
//! writing skills and commands into a runtime, and the hook answer's shape.

use std::fs;
use std::io;
use std::path::Path;

use oxplow_domain::agent::harness::HarnessError;
use oxplow_domain::agent::observe::HookAnswer;
use oxplow_domain::agent::text::Text;
use oxplow_domain::HookKind;
use serde_json::json;

/// POSIX single-quote escape: wraps `'`, replaces internal `'` with
/// `'\''`.
pub fn shell_escape(s: &str) -> String {
    let escaped = s.replace('\'', r"'\''");
    format!("'{escaped}'")
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

/// `answer` in Claude Code's hook-response shape, which Codex's hooks and
/// opencode's bridge speak too: `{}` for nothing to say, else a
/// `hookSpecificOutput` naming the event. The wording inside is core's
/// (pinned by the control plane's goldens).
pub fn render(answer: &HookAnswer) -> serde_json::Value {
    match answer {
        HookAnswer::Ack => json!({}),
        HookAnswer::Deny { reason } => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        }),
        HookAnswer::Context { event, text } => json!({
            "hookSpecificOutput": {
                "hookEventName": event_name(*event),
                "additionalContext": text,
            }
        }),
    }
}

/// The hook event's name on the wire.
fn event_name(kind: HookKind) -> &'static str {
    match kind {
        HookKind::UserPromptSubmit => "UserPromptSubmit",
        HookKind::PreToolUse => "PreToolUse",
        HookKind::PostToolUse => "PostToolUse",
        HookKind::Stop => "Stop",
        HookKind::Interrupt => "Interrupt",
        HookKind::SessionStart => "SessionStart",
        HookKind::SessionEnd => "SessionEnd",
        HookKind::Notification => "Notification",
    }
}

/// A runtime file that couldn't be written.
pub fn runtime(e: io::Error) -> HarnessError {
    HarnessError::Runtime(e.to_string())
}

/// The file in each skill folder oxplow writes: how it tells its own
/// from a person's when one is no longer offered.
const SKILL_MARKER: &str = ".oxplow";

/// Write each of `skills` as `<skills_dir>/<name>/SKILL.md`, and remove
/// any other skill folder oxplow wrote there (one it no longer ships, or
/// an extension's no longer offered); a person's own stay.
pub fn write_skills(skills_dir: &Path, skills: &[Text]) -> io::Result<()> {
    if let Ok(entries) = fs::read_dir(skills_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().join(SKILL_MARKER).is_file() && !skills.iter().any(|s| s.name == name) {
                fs::remove_dir_all(entry.path())?;
            }
        }
    }
    for skill in skills {
        let dir = skills_dir.join(&skill.name);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("SKILL.md"), &skill.body)?;
        fs::write(dir.join(SKILL_MARKER), "")?;
    }
    Ok(())
}

/// Write each of `commands` as `<commands_dir>/<name>.md`, removing every
/// other `.md` there: the folder is oxplow's.
pub fn write_commands(commands_dir: &Path, commands: &[Text]) -> io::Result<()> {
    if let Ok(entries) = fs::read_dir(commands_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".md") {
                if !commands.iter().any(|c| c.name == stem) {
                    fs::remove_file(entry.path())?;
                }
            }
        }
    }
    for command in commands {
        fs::write(
            commands_dir.join(format!("{}.md", command.name)),
            &command.body,
        )?;
    }
    Ok(())
}

/// `value` as pretty JSON, newline-terminated, at `path`.
/// Write `value` as pretty JSON, readable by its owner only: a runtime
/// file may hold the session's bearer.
pub fn write_json(path: &Path, value: &serde_json::Value) -> io::Result<()> {
    let mut s = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    s.push('\n');
    fs::write(path, s)?;
    owner_only(path)
}

/// Make `path` readable and writable by its owner alone.
#[cfg(unix)]
fn owner_only(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn owner_only(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deny and a context are Claude's `hookSpecificOutput`; an ack is
    /// an empty object (anything else prints a warning in Claude's
    /// terminal).
    #[test]
    fn answers_render_in_the_hook_response_shape() {
        assert_eq!(render(&HookAnswer::Ack), json!({}));
        assert_eq!(
            render(&HookAnswer::Deny {
                reason: "no".into()
            }),
            json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": "no",
            }})
        );
        assert_eq!(
            render(&HookAnswer::Context {
                event: HookKind::PostToolUse,
                text: "ctx".into()
            }),
            json!({"hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext": "ctx",
            }})
        );
    }

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
