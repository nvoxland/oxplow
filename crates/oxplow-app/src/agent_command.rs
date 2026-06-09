//! Build the shell command oxplow runs in a tmux pane to launch the
//! agent CLI (Claude or Codex).
//!
//! Pure string-building, no IO. Mirrors the original
//! `src/agent/agent-command.ts` so the launcher signature is stable
//! across the migration.

use oxplow_config::AgentKind;
use oxplow_domain::Stream;

#[derive(Debug, Clone, Default)]
pub struct AgentCommandOptions {
    pub plugin_dir: Option<String>,
    pub allowed_tools: Vec<String>,
    pub append_system_prompt: Option<String>,
    pub mcp_config: Option<String>,
    pub codex_config_overrides: Vec<String>,
    pub env: Vec<(String, String)>,
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
        let config_args = opts
            .codex_config_overrides
            .iter()
            .map(|c| format!(" --config {}", shell_escape(c)))
            .collect::<String>();
        let base = if resume_session_id.is_empty() {
            format!("codex --cd {}{config_args}", shell_escape(cwd))
        } else {
            format!(
                "codex resume --cd {}{config_args} {}",
                shell_escape(cwd),
                shell_escape(resume_session_id)
            )
        };
        let inner = format!("cd {} && {}exec {base}", shell_escape(cwd), env_prefix);
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

    let claude_base = format!("claude{plugin_arg}{allowed_tools_arg}{prompt_arg}{mcp_arg}");
    let fresh_claude = format!("{env_prefix}exec {claude_base}");
    let command = if resume_session_id.is_empty() {
        fresh_claude.clone()
    } else {
        format!(
            "{env_prefix}{claude_base} --resume {} || {{ echo '[oxplow] saved resume id was stale; starting a fresh Claude session' >&2; {fresh_claude}; }}",
            shell_escape(resume_session_id)
        )
    };
    let inner = format!("cd {} && {command}", shell_escape(cwd));
    format!("sh -lc {}", shell_escape(&inner))
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
