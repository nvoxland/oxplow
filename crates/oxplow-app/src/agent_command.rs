//! Build the shell command oxplow runs in a tmux pane to launch the
//! agent CLI (Claude, Codex, or opencode).
//!
//! Pure string-building, no IO. Mirrors the original
//! `src/agent/agent-command.ts` so the launcher signature is stable
//! across the migration.

use oxplow_config::AgentKind;
use oxplow_domain::Stream;

/// Model opencode launches with (`-m provider/model`) when the project
/// config doesn't override it (`agentModels: { opencode: … }` in
/// .oxplow/project.yaml). Assumes GitHub Copilot auth in opencode's own auth
/// store.
pub const OPENCODE_MODEL: &str = "github-copilot/gpt-5-mini";

#[derive(Debug, Clone, Default)]
pub struct AgentCommandOptions {
    pub plugin_dir: Option<String>,
    pub allowed_tools: Vec<String>,
    pub append_system_prompt: Option<String>,
    pub mcp_config: Option<String>,
    pub codex_config_overrides: Vec<String>,
    pub env: Vec<(String, String)>,
    /// `agentModels.opencode` from .oxplow/project.yaml — overrides
    /// [`OPENCODE_MODEL`] when set. Claude/codex launch with their
    /// own defaults and ignore this.
    pub opencode_model: Option<String>,
    /// Absolute path to the agent CLI, from `agent_path::resolve_agent_program`
    /// (tsk245). `None` falls back to the bare binary name plus a preflight
    /// that explains the GUI-launch PATH gap — see [`program_and_guard`].
    pub program: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Working,
    Talking,
}

pub fn build_agent_command(
    agent: AgentKind,
    stream: &Stream,
    pane: PaneKind,
    opts: &AgentCommandOptions,
) -> String {
    let resume_session_id = match pane {
        PaneKind::Working => stream.working_session_id.as_str(),
        PaneKind::Talking => stream.talking_session_id.as_str(),
    };
    build_agent_command_for_session(agent, &stream.worktree_path, resume_session_id, opts)
}

pub fn build_agent_command_for_session(
    agent: AgentKind,
    cwd: &str,
    resume_session_id: &str,
    opts: &AgentCommandOptions,
) -> String {
    let env_prefix = build_env_prefix(&opts.env);

    if matches!(agent, AgentKind::Codex) {
        let (prog, guard) = program_and_guard(opts, "codex");
        let config_args = opts
            .codex_config_overrides
            .iter()
            .map(|c| format!(" --config {}", shell_escape(c)))
            .collect::<String>();
        let base = if resume_session_id.is_empty() {
            format!("{prog} --cd {}{config_args}", shell_escape(cwd))
        } else {
            format!(
                "{prog} resume --cd {}{config_args} {}",
                shell_escape(cwd),
                shell_escape(resume_session_id)
            )
        };
        let inner = format!(
            "cd {} && {guard}{}exec {base}",
            shell_escape(cwd),
            env_prefix
        );
        return format!("sh -lc {}", shell_escape(&inner));
    }

    if matches!(agent, AgentKind::Opencode) {
        // Hooks + MCP + the per-spawn system prompt all ride the
        // OPENCODE_CONFIG_CONTENT env var (set by the caller via
        // `opts.env`); the CLI itself only needs the model and an
        // optional session to resume. cwd comes from the `cd` (opencode
        // starts in the working directory).
        let (prog, guard) = program_and_guard(opts, "opencode");
        let model = opts.opencode_model.as_deref().unwrap_or(OPENCODE_MODEL);
        let base = format!("{prog} -m {}", shell_escape(model));
        let fresh = format!("{env_prefix}exec {base}");
        let command = if resume_session_id.is_empty() {
            fresh.clone()
        } else {
            format!(
                "{env_prefix}{base} -s {} || {{ echo '[oxplow] saved resume id was stale; starting a fresh opencode session' >&2; {fresh}; }}",
                shell_escape(resume_session_id)
            )
        };
        let inner = format!("cd {} && {guard}{command}", shell_escape(cwd));
        return format!("sh -lc {}", shell_escape(&inner));
    }

    let plugin_arg = opts
        .plugin_dir
        .as_deref()
        .map(|p| format!(" --plugin-dir {}", shell_escape(p)))
        .unwrap_or_default();
    let allowed_tools_arg = if opts.allowed_tools.is_empty() {
        String::new()
    } else {
        let joined: Vec<String> = opts.allowed_tools.iter().map(|t| shell_escape(t)).collect();
        format!(" --allowedTools {}", joined.join(" "))
    };
    let prompt_arg = opts
        .append_system_prompt
        .as_deref()
        .map(|p| format!(" --append-system-prompt {}", shell_escape(p)))
        .unwrap_or_default();
    let mcp_arg = opts
        .mcp_config
        .as_deref()
        .map(|p| format!(" --mcp-config {} --strict-mcp-config", shell_escape(p)))
        .unwrap_or_default();

    let (prog, guard) = program_and_guard(opts, "claude");
    let claude_base = format!("{prog}{plugin_arg}{allowed_tools_arg}{prompt_arg}{mcp_arg}");
    let fresh_claude = format!("{env_prefix}exec {claude_base}");
    let command = if resume_session_id.is_empty() {
        fresh_claude.clone()
    } else {
        format!(
            "{env_prefix}{claude_base} --resume {} || {{ echo '[oxplow] saved resume id was stale; starting a fresh Claude session' >&2; {fresh_claude}; }}",
            shell_escape(resume_session_id)
        )
    };
    let inner = format!("cd {} && {guard}{command}", shell_escape(cwd));
    format!("sh -lc {}", shell_escape(&inner))
}

/// What to exec, plus a preflight to run before it.
///
/// With a resolved absolute path there's nothing to check — PATH is out of the
/// picture. Without one we keep the bare name, because resolution is a
/// heuristic and the login shell may still find it, but we first test PATH so
/// a miss reports the actual cause. The bare `sh: claude: command not found`
/// it replaces lands directly under the stale-resume notice, which makes a
/// PATH problem read as a session problem (tsk245).
fn program_and_guard(opts: &AgentCommandOptions, bin: &str) -> (String, String) {
    match opts.program.as_deref() {
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

fn build_env_prefix(env: &[(String, String)]) -> String {
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

/// POSIX single-quote escape: wraps `'`, replaces internal `'` with
/// `'\\''`. Matches the TS impl.
pub fn shell_escape(s: &str) -> String {
    let escaped = s.replace('\'', r"'\''");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{StreamId, StreamKind, Timestamp};

    fn stream() -> Stream {
        Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/repo".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: "sess-w".into(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        }
    }

    #[test]
    fn shell_escape_handles_apostrophes() {
        assert_eq!(shell_escape("it's"), r"'it'\''s'");
    }

    /// `sh -lc '<inner>'` form: the outer single-quote wraps the whole
    /// inner command, escaping any inner quotes as `'\''`. The asserts
    /// below check substrings of the inner command that survive that
    /// transform unchanged.
    #[test]
    fn codex_command_uses_codex_cli() {
        let s = stream();
        let cmd = build_agent_command(AgentKind::Codex, &s, PaneKind::Working, &Default::default());
        assert!(cmd.starts_with("sh -lc "));
        assert!(cmd.contains("exec codex"));
        assert!(cmd.contains("--cd"));
        assert!(cmd.contains("/repo"));
    }

    #[test]
    fn codex_command_includes_config_overrides() {
        let s = stream();
        let opts = AgentCommandOptions {
            codex_config_overrides: vec!["mcp_servers.oxplow.url=\"http://127.0.0.1/mcp\"".into()],
            ..Default::default()
        };
        let cmd = build_agent_command(AgentKind::Codex, &s, PaneKind::Working, &opts);
        assert!(cmd.contains("--config"));
        assert!(cmd.contains("mcp_servers.oxplow.url"));
        assert!(!cmd.contains("--dangerously-bypass-hook-trust"));
    }

    #[test]
    fn claude_command_resumes_when_session_id_set() {
        let s = stream();
        let cmd = build_agent_command(
            AgentKind::Claude,
            &s,
            PaneKind::Working,
            &Default::default(),
        );
        assert!(cmd.contains("--resume "));
        assert!(cmd.contains("sess-w"));
        // Falls back to a fresh session on stale id.
        assert!(cmd.contains("oxplow"));
        assert!(cmd.contains("exec claude"));
    }

    #[test]
    fn claude_command_fresh_when_no_session() {
        let s = stream();
        let cmd = build_agent_command(
            AgentKind::Claude,
            &s,
            PaneKind::Talking,
            &Default::default(),
        );
        assert!(!cmd.contains("--resume"));
        assert!(cmd.contains("exec claude"));
    }

    #[test]
    fn opencode_command_fresh_when_no_session() {
        let s = stream();
        let cmd = build_agent_command(
            AgentKind::Opencode,
            &s,
            PaneKind::Talking,
            &Default::default(),
        );
        assert!(cmd.starts_with("sh -lc "));
        assert!(cmd.contains("exec opencode"));
        assert!(cmd.contains(" -m "));
        assert!(cmd.contains(OPENCODE_MODEL));
        assert!(!cmd.contains(" -s "));
        assert!(cmd.contains("/repo"));
    }

    #[test]
    fn opencode_command_honors_configured_model_override() {
        let s = stream();
        let opts = AgentCommandOptions {
            opencode_model: Some("anthropic/claude-sonnet-4-6".into()),
            ..Default::default()
        };
        let cmd = build_agent_command(AgentKind::Opencode, &s, PaneKind::Talking, &opts);
        assert!(cmd.contains("anthropic/claude-sonnet-4-6"));
        assert!(!cmd.contains(OPENCODE_MODEL));
    }

    #[test]
    fn opencode_command_resumes_with_stale_fallback() {
        let s = stream();
        let cmd = build_agent_command(
            AgentKind::Opencode,
            &s,
            PaneKind::Working,
            &Default::default(),
        );
        assert!(cmd.contains(" -s "));
        assert!(cmd.contains("sess-w"));
        // Falls back to a fresh session on stale id, like claude.
        assert!(cmd.contains("stale"));
        assert!(cmd.contains("exec opencode"));
    }

    #[test]
    fn a_resolved_program_replaces_the_bare_binary_name() {
        // tsk245: a GUI-launched oxplow has a minimal PATH, so the agent is
        // spawned by absolute path and PATH stops mattering. Every agent has
        // to honour it — codex builds its command in two places (fresh and
        // `resume`), so a partial fix is easy to miss.
        let s = stream();
        for (agent, bin) in [
            (AgentKind::Claude, "claude"),
            (AgentKind::Codex, "codex"),
            (AgentKind::Opencode, "opencode"),
        ] {
            let opts = AgentCommandOptions {
                program: Some(format!("/opt/agents/{bin}")),
                ..Default::default()
            };
            for pane in [PaneKind::Working, PaneKind::Talking] {
                let cmd = build_agent_command(agent, &s, pane, &opts);
                assert!(
                    cmd.contains(&format!("/opt/agents/{bin}")),
                    "{bin} ({pane:?}) should exec the resolved path: {cmd}"
                );
                assert!(
                    !cmd.contains(&format!("exec {bin}")),
                    "{bin} ({pane:?}) should not fall back to the bare name: {cmd}"
                );
                assert!(
                    !cmd.contains("command -v"),
                    "{bin} ({pane:?}) needs no not-found guard once resolved: {cmd}"
                );
            }
        }
    }

    #[test]
    fn an_unresolved_program_keeps_the_bare_name_but_explains_the_failure() {
        // Resolution is a heuristic (version-manager shims aren't on the
        // list), so an unresolved agent must still get its shot at PATH —
        // just with a legible message instead of a bare `command not found`,
        // which reads as a session problem when it lands under the
        // stale-resume line.
        let s = stream();
        for (agent, bin) in [
            (AgentKind::Claude, "claude"),
            (AgentKind::Codex, "codex"),
            (AgentKind::Opencode, "opencode"),
        ] {
            let cmd = build_agent_command(agent, &s, PaneKind::Working, &Default::default());
            assert!(
                cmd.contains(&format!("command -v {bin}")),
                "{bin} should preflight PATH: {cmd}"
            );
            assert!(
                cmd.contains("launched from the"),
                "{bin} should name the GUI-launch cause: {cmd}"
            );
            assert!(
                cmd.contains(bin),
                "{bin} still runs by bare name so PATH can win: {cmd}"
            );
        }
    }

    #[test]
    fn opencode_command_carries_config_content_env() {
        let s = stream();
        let opts = AgentCommandOptions {
            env: vec![(
                "OPENCODE_CONFIG_CONTENT".into(),
                r#"{"mcp":{"oxplow":{"url":"http://x/mcp"}}}"#.into(),
            )],
            ..Default::default()
        };
        let cmd = build_agent_command(AgentKind::Opencode, &s, PaneKind::Talking, &opts);
        assert!(cmd.contains("OPENCODE_CONFIG_CONTENT="));
        assert!(cmd.contains("http://x/mcp"));
    }

    #[test]
    fn append_system_prompt_is_quoted() {
        let s = stream();
        let opts = AgentCommandOptions {
            append_system_prompt: Some("be terse".into()),
            ..Default::default()
        };
        let cmd = build_agent_command(AgentKind::Claude, &s, PaneKind::Working, &opts);
        assert!(cmd.contains("--append-system-prompt"));
        assert!(cmd.contains("be terse"));
    }
}
