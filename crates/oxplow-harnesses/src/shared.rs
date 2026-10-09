//! What the harnesses share: shell quoting, the launch command's shape,
//! writing skills and commands into a runtime, and the hook answer's shape.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use oxplow_domain::agent::harness::HarnessError;
use oxplow_domain::agent::observe::HookAnswer;
use oxplow_domain::agent::text::Text;
use oxplow_domain::agent::tool::{ToolKind, ToolUse};
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

/// A session started fresh, or resumed: the harness's own session `args`
/// (`--resume <id>`), `known` when the harness found that session on disk.
pub enum Resume<'a> {
    Fresh,
    Known(String),
    /// A resume it couldn't check; a fresh session stands in when it fails.
    Unchecked {
        args: String,
        harness: &'a str,
    },
}

/// `exec <base>`, resuming as `resume` says. A known session is exec'd
/// directly: the session's process is the agent's, and whatever it exits
/// with ends the PTY. Only an unchecked one falls back to a fresh session,
/// on a failure to start it, and says so.
pub fn resume_or_fresh(base: &str, resume: Resume<'_>) -> String {
    match resume {
        Resume::Fresh => format!("exec {base}"),
        Resume::Known(args) => format!("exec {base} {args}"),
        Resume::Unchecked { args, harness } => format!(
            "{base} {args} || {{ echo {} >&2; exec {base}; }}",
            shell_escape(&format!(
                "[oxplow] the saved {harness} session couldn't be resumed; starting a fresh one"
            ))
        ),
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

/// The files an `apply_patch` names (`*** Add File: a`, `*** Update File:
/// b`, `*** Delete File: c`, `*** Move to: d`), from its patch text —
/// Codex's `input` or `patch`, OpenCode's `patchText` — or an explicit
/// `path`. Codex's and OpenCode's edit tools both take this format.
pub fn patch_paths(input: &serde_json::Value) -> Vec<String> {
    let text = ["input", "patch", "patchText"]
        .iter()
        .find_map(|k| input.get(*k).and_then(|v| v.as_str()))
        .unwrap_or_default();
    let mut paths: Vec<String> = text
        .lines()
        .filter_map(|l| {
            [
                "*** Add File: ",
                "*** Update File: ",
                "*** Delete File: ",
                "*** Move to: ",
            ]
            .iter()
            .find_map(|p| l.strip_prefix(p))
        })
        .map(|p| p.trim().to_string())
        .collect();
    if let Some(p) = input.get("path").and_then(|p| p.as_str()) {
        paths.push(p.to_string());
    }
    paths
}

/// A tool hook's body in Claude Code's shape (`tool_name`, `tool_input`,
/// `tool_response`, `tool_use_id`) mapped onto oxplow's vocabulary — what
/// Claude Code sends, and what opencode's bridge translates its calls to.
pub fn claude_shaped_tool_use(body: &serde_json::Value) -> Option<ToolUse> {
    let name = body.get("tool_name")?.as_str()?.to_string();
    let kind = match name.as_str() {
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => ToolKind::Edit,
        "Bash" => ToolKind::Shell,
        "Task" | "Agent" => ToolKind::Subagent,
        "AskUserQuestion" => ToolKind::Ask,
        "ExitPlanMode" => ToolKind::Plan,
        "Read" => ToolKind::Read,
        "Grep" | "Glob" | "List" => ToolKind::Search,
        "WebFetch" | "WebSearch" => ToolKind::Fetch,
        n if n.starts_with("mcp__") => ToolKind::Mcp,
        _ => ToolKind::Other,
    };
    let input = body.get("tool_input");
    let field = |k: &str| input.and_then(|i| i.get(k)).and_then(|v| v.as_str());
    // Every key naming a file, not just the first: a null `file_path`
    // mustn't hide a `notebook_path`.
    let paths = ["file_path", "notebook_path", "path"]
        .iter()
        .filter_map(|k| field(k))
        .map(str::to_string)
        .collect();
    let command = (kind == ToolKind::Shell)
        .then(|| field("command"))
        .flatten()
        .map(str::to_string);
    let detail = field("command")
        .or_else(|| field("pattern"))
        .or_else(|| field("query"))
        .or_else(|| field("url"))
        .map(str::to_string);
    let question = field("question")
        .or_else(|| {
            input
                .and_then(|i| i.get("questions"))
                .and_then(|q| q.get(0))
                .and_then(|q| q.get("question"))
                .and_then(|q| q.as_str())
        })
        .map(str::to_string);
    let response = body.get("tool_response").filter(|r| !r.is_null());
    let exit_code = response.and_then(|r| {
        ["exit_code", "exitCode", "returnCode", "code"]
            .iter()
            .find_map(|k| r.get(*k).and_then(|x| x.as_i64()))
    });
    let ok = response.map(|r| {
        let failed = r
            .get("is_error")
            .or_else(|| r.get("isError"))
            .and_then(|e| e.as_bool())
            .unwrap_or(false);
        !failed && exit_code.is_none_or(|c| c == 0)
    });
    // A shell call with no exit code and no error says nothing of success.
    let ok = match (kind, ok, exit_code) {
        (ToolKind::Shell, Some(true), None) => None,
        (_, ok, _) => ok,
    };
    Some(ToolUse {
        name,
        kind,
        paths,
        command,
        detail,
        call_id: body
            .get("tool_use_id")
            .and_then(|t| t.as_str())
            .map(str::to_string),
        ok,
        exit_code,
        question,
    })
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

/// Where a harness that finds skills only beside the directory it runs in
/// keeps oxplow's: the worktree's `.agents/skills/`, where agents look
/// for a repo's skills.
pub const WORKTREE_SKILLS_REL: &str = ".agents/skills";

/// [`write_skills`] into `workspace`'s [`WORKTREE_SKILLS_REL`], each
/// folder with a `*` `.gitignore` so it never reaches the person's
/// commits.
pub fn write_worktree_skills(workspace: &Path, skills: &[Text]) -> io::Result<()> {
    let skills_dir = workspace.join(WORKTREE_SKILLS_REL);
    write_skills(&skills_dir, skills)?;
    for skill in skills {
        fs::write(skills_dir.join(&skill.name).join(".gitignore"), "*\n")?;
    }
    Ok(())
}

/// [`write_worktree_skills`] for each of `workspaces` where oxplow wrote
/// skills before, creating none.
pub fn refresh_worktree_skills(workspaces: &[PathBuf], skills: &[Text]) -> io::Result<()> {
    for workspace in workspaces {
        let ours = fs::read_dir(workspace.join(WORKTREE_SKILLS_REL)).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|e| e.path().join(SKILL_MARKER).is_file())
        });
        if ours {
            write_worktree_skills(workspace, skills)?;
        }
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

    /// Claude Code's names map onto oxplow's kinds, with the files, the
    /// command, the call id and the outcome read from its shape.
    #[test]
    fn claude_shaped_bodies_map_onto_the_vocabulary() {
        let edit = claude_shaped_tool_use(&json!({
            "tool_name": "NotebookEdit", "tool_use_id": "tu1",
            "tool_input": {"file_path": null, "notebook_path": "nb.ipynb"}
        }))
        .unwrap();
        assert_eq!(
            (edit.kind, edit.paths.clone(), edit.call_id.as_deref()),
            (ToolKind::Edit, vec!["nb.ipynb".to_string()], Some("tu1"))
        );
        assert_eq!(edit.ok, None, "a request has no outcome");
        let ran = claude_shaped_tool_use(&json!({
            "tool_name": "Bash",
            "tool_input": {"command": "cargo test"},
            "tool_response": {"exit_code": 101}
        }))
        .unwrap();
        assert_eq!(ran.kind, ToolKind::Shell);
        assert_eq!(ran.command.as_deref(), Some("cargo test"));
        assert_eq!((ran.exit_code, ran.ok), (Some(101), Some(false)));
        let silent = claude_shaped_tool_use(&json!({
            "tool_name": "Bash", "tool_input": {"command": "ls"}, "tool_response": {}
        }))
        .unwrap();
        assert_eq!(silent.ok, None, "no exit code: success unknown");
        let asked = claude_shaped_tool_use(&json!({
            "tool_name": "AskUserQuestion",
            "tool_input": {"questions": [{"question": "Which one?"}]}
        }))
        .unwrap();
        assert_eq!(
            (asked.kind, asked.question.as_deref()),
            (ToolKind::Ask, Some("Which one?"))
        );
        for (name, kind) in [
            ("Write", ToolKind::Edit),
            ("MultiEdit", ToolKind::Edit),
            ("Task", ToolKind::Subagent),
            ("Agent", ToolKind::Subagent),
            ("ExitPlanMode", ToolKind::Plan),
            ("Read", ToolKind::Read),
            ("Grep", ToolKind::Search),
            ("WebFetch", ToolKind::Fetch),
            ("mcp__oxplow__run_command", ToolKind::Mcp),
            ("TodoWrite", ToolKind::Other),
        ] {
            assert_eq!(
                claude_shaped_tool_use(&json!({ "tool_name": name }))
                    .unwrap()
                    .kind,
                kind,
                "{name}"
            );
        }
        assert!(claude_shaped_tool_use(&json!({})).is_none());
    }

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
