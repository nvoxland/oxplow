//! The `claude` harness: Claude Code in a terminal. Its runtime is a
//! plugin directory (hooks, skills, commands) and a per-session MCP config;
//! token usage rides its OTEL exporter; a stale resume id is caught before
//! launch by looking for the session's transcript.

use std::path::Path;

use oxplow_domain::agent::harness::{
    AgentHarness, Endpoints, Gate, HarnessError, Input, Interact, Launch, LaunchInput, LaunchSpec,
    Transcript,
};

use super::shared::{env_prefix, in_shell, program_and_guard, shell_escape};
use super::Named;

pub(super) struct Claude(pub(super) Named);

/// Claude Code's markers for a session and the processes it starts: an
/// agent or terminal oxplow starts must not inherit them from oxplow's own
/// environment, or Claude treats it as a child session (transcripts off,
/// which breaks resume and token counts).
const MARKERS: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_BRIDGE_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
];

impl AgentHarness for Claude {
    fn id(&self) -> &str {
        &self.0.id
    }

    fn title(&self) -> &str {
        &self.0.title
    }

    fn interact(&self) -> Interact {
        Interact {
            transcript: Transcript::Terminal,
            input: Input::Keystrokes,
            gate: Gate::Harness,
        }
    }

    fn launch(&self, input: &LaunchInput<'_>) -> Result<Launch, HarnessError> {
        let runtime = |e: oxplow_plugin::PluginError| HarnessError::Runtime(e.to_string());
        let ep = input.endpoints;
        let paths = oxplow_plugin::write_plugin(
            input.project_dir,
            &ep.hook_base_url,
            &ep.mcp_endpoint_url,
            &ep.hook_token,
            input.text,
        )
        .map_err(runtime)?;
        let ids = input.session;
        let (thread, stream, session) = (
            ids.thread.to_string(),
            ids.stream.to_string(),
            ids.session.to_string(),
        );
        // Its MCP config can't read env vars, so the session's identity is
        // baked into a per-session file (like the token).
        let mcp_config = oxplow_plugin::write_claude_mcp_config(
            &paths.plugin_dir,
            &ep.mcp_endpoint_url,
            &ep.hook_token,
            oxplow_plugin::McpIdentity {
                thread_id: &thread,
                stream_id: &stream,
                session_id: Some(&session),
            },
        )
        .map_err(runtime)?;
        let mut env = input.identity_env.to_vec();
        env.extend(otel_env(ep, &thread, &session));
        let cwd = input.workspace.to_string_lossy();
        // A resume id whose transcript is gone launches fresh, with no raw
        // "No conversation found" error, and core forgets the id.
        let (resume, resume_dropped) = match (input.resume.filter(|r| !r.is_empty()), input.home) {
            (Some(id), Some(home)) if resume_state(home, &cwd, id) == ResumeState::Missing => {
                (None, true)
            }
            (resume, _) => (resume, false),
        };
        let program = (input.resolve_program)("claude");
        Ok(Launch {
            spec: LaunchSpec::Pty {
                command: command(&Command {
                    cwd: &cwd,
                    resume,
                    program: program.as_deref(),
                    env: &env,
                    plugin_dir: Some(&paths.plugin_dir.to_string_lossy()),
                    system_prompt: input.system_prompt,
                    mcp_config: Some(&mcp_config.to_string_lossy()),
                }),
            },
            resume_dropped,
        })
    }

    fn instruction_files(&self) -> &[&str] {
        &["CLAUDE.md"]
    }

    fn env_markers(&self) -> &[&str] {
        MARKERS
    }
}

/// What the `claude` command line is built from.
struct Command<'a> {
    cwd: &'a str,
    resume: Option<&'a str>,
    program: Option<&'a str>,
    env: &'a [(String, String)],
    plugin_dir: Option<&'a str>,
    system_prompt: Option<&'a str>,
    mcp_config: Option<&'a str>,
}

/// `claude` with its plugin, prompt and MCP config, resuming `resume`
/// with a fallback to a fresh session when the id is stale.
fn command(c: &Command<'_>) -> String {
    let prefix = env_prefix(c.env);
    let plugin_arg = c
        .plugin_dir
        .map(|p| format!(" --plugin-dir {}", shell_escape(p)))
        .unwrap_or_default();
    let prompt_arg = c
        .system_prompt
        .filter(|p| !p.is_empty())
        .map(|p| format!(" --append-system-prompt {}", shell_escape(p)))
        .unwrap_or_default();
    let mcp_arg = c
        .mcp_config
        .map(|p| format!(" --mcp-config {} --strict-mcp-config", shell_escape(p)))
        .unwrap_or_default();
    let (prog, guard) = program_and_guard(c.program, "claude");
    let base = format!("{prog}{plugin_arg}{prompt_arg}{mcp_arg}");
    let fresh = format!("{prefix}exec {base}");
    let command = match c.resume {
        None => fresh,
        Some(id) => format!(
            "{prefix}{base} --resume {} || {{ echo '[oxplow] saved resume id was stale; starting a fresh Claude session' >&2; {fresh}; }}",
            shell_escape(id)
        ),
    };
    in_shell(c.cwd, &guard, &command)
}

/// OTEL env that points Claude Code's OTLP metrics exporter at oxplow's
/// receiver. Only metrics are exported. The owning thread and agent
/// session ride custom OTLP headers so the receiver attributes token facts
/// to the session's turn. Temporality is the SDK default (delta): each
/// export is the per-interval increment, additive as `oxplow.tokens`
/// facts.
fn otel_env(ep: &Endpoints, thread: &str, session: &str) -> Vec<(String, String)> {
    vec![
        ("CLAUDE_CODE_ENABLE_TELEMETRY".into(), "1".into()),
        ("OTEL_METRICS_EXPORTER".into(), "otlp".into()),
        ("OTEL_EXPORTER_OTLP_PROTOCOL".into(), "http/protobuf".into()),
        // Base URL; the SDK appends the `/v1/metrics` signal path.
        (
            "OTEL_EXPORTER_OTLP_ENDPOINT".into(),
            ep.otlp_base_url.clone(),
        ),
        (
            "OTEL_EXPORTER_OTLP_HEADERS".into(),
            format!(
                "Authorization=Bearer {},X-Oxplow-Thread={thread},X-Oxplow-Session={session}",
                ep.hook_token
            ),
        ),
        // 10s (default is 60s) — snappier token updates in the UI.
        ("OTEL_METRIC_EXPORT_INTERVAL".into(), "10000".into()),
    ]
}

/// Whether a Claude session's transcript still exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResumeState {
    /// The session `.jsonl` is on disk — `--resume` will work.
    Present,
    /// The project dir exists but the session `.jsonl` is gone: stale.
    Missing,
    /// Can't tell (no `~/.claude/projects/<cwd>` dir): leave the id alone;
    /// the shell `||` net still protects the launch.
    Unknown,
}

/// Claude encodes a session's cwd into its projects-dir name by replacing
/// every non-alphanumeric byte with `-` (`/Users/x/src/oxplow` →
/// `-Users-x-src-oxplow`).
fn encode_cwd(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Whether the transcript of `session_id` (launched in `cwd`) is under
/// `<home>/.claude/projects/`.
fn resume_state(home: &Path, cwd: &str, session_id: &str) -> ResumeState {
    if session_id.is_empty() {
        return ResumeState::Unknown;
    }
    let project_dir = home.join(".claude").join("projects").join(encode_cwd(cwd));
    if !project_dir.is_dir() {
        return ResumeState::Unknown;
    }
    if project_dir.join(format!("{session_id}.jsonl")).is_file() {
        ResumeState::Present
    } else {
        ResumeState::Missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harnesses::test_launch::{harness, launch_in};
    use std::fs;

    fn pty(launch: &Launch) -> &str {
        match &launch.spec {
            LaunchSpec::Pty { command } => command,
            other => panic!("not a PTY launch: {other:?}"),
        }
    }

    /// The launch writes its plugin and per-session MCP config, and its
    /// command carries the identity, the OTEL exporter, the prompt, the
    /// plugin and the MCP config.
    #[test]
    fn launch_builds_the_command_and_env() {
        let h = harness("oxplow:claude-code", "claude");
        let l = launch_in(
            h.as_ref(),
            Some("sess-w"),
            Some("be terse"),
            &serde_json::json!({}),
        );
        assert!(!l.launch.resume_dropped);
        let cmd = pty(&l.launch);
        for want in [
            "OXPLOW_SESSION=",
            "X-Oxplow-Session=ses3",
            "CLAUDE_CODE_ENABLE_TELEMETRY=",
            "--plugin-dir",
            "--append-system-prompt",
            "be terse",
            "--mcp-config",
            "mcp-config.ses3.json",
            "--resume ",
            "sess-w",
            "/opt/agents/claude",
        ] {
            assert!(cmd.contains(want), "{want}: {cmd}");
        }
        assert!(l
            .project
            .join(".oxplow/runtime/claude-plugin/mcp-config.ses3.json")
            .is_file());
    }

    /// A resume id whose transcript is gone launches fresh and is dropped.
    #[test]
    fn a_missing_transcript_drops_the_resume_id() {
        let h = harness("oxplow:claude-code", "claude");
        let l = launch_in(h.as_ref(), Some("gone"), None, &serde_json::json!({}));
        assert!(
            !l.launch.resume_dropped,
            "no projects dir: can't tell, kept"
        );
        let l = crate::harnesses::test_launch::launch_with_home(h.as_ref(), "gone", |home, cwd| {
            fs::create_dir_all(home.join(".claude/projects").join(encode_cwd(cwd))).unwrap();
        });
        assert!(l.launch.resume_dropped);
        assert!(!pty(&l.launch).contains("--resume"));
    }

    #[test]
    fn the_otel_env_points_the_exporter_at_the_receiver() {
        let ep = Endpoints {
            hook_base_url: "http://127.0.0.1:9/hook".into(),
            mcp_endpoint_url: "http://127.0.0.1:9/mcp".into(),
            otlp_base_url: "http://127.0.0.1:9".into(),
            hook_token: "tok123".into(),
        };
        let env: std::collections::HashMap<String, String> =
            otel_env(&ep, "thr1", "ses3").into_iter().collect();
        assert_eq!(env["OTEL_METRICS_EXPORTER"], "otlp");
        assert_eq!(env["OTEL_EXPORTER_OTLP_PROTOCOL"], "http/protobuf");
        assert_eq!(env["OTEL_EXPORTER_OTLP_ENDPOINT"], "http://127.0.0.1:9");
        assert_eq!(
            env["OTEL_EXPORTER_OTLP_HEADERS"],
            "Authorization=Bearer tok123,X-Oxplow-Thread=thr1,X-Oxplow-Session=ses3"
        );
        assert_eq!(env["OTEL_METRIC_EXPORT_INTERVAL"], "10000");
    }

    #[test]
    fn a_fresh_session_has_no_resume() {
        let c = Command {
            cwd: "/repo",
            resume: None,
            program: None,
            env: &[],
            plugin_dir: None,
            system_prompt: None,
            mcp_config: None,
        };
        let cmd = command(&c);
        assert!(cmd.starts_with("sh -lc ") && cmd.contains("exec claude"));
        assert!(!cmd.contains("--resume"));
        assert!(cmd.contains("command -v claude"));
    }

    #[test]
    fn encodes_path_separators_and_dots() {
        assert_eq!(
            encode_cwd("/Users/nv/src/nvoxland/oxplow"),
            "-Users-nv-src-nvoxland-oxplow"
        );
        assert_eq!(encode_cwd("/a/b.c_d"), "-a-b-c-d");
    }

    #[test]
    fn resume_state_reads_the_transcript_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = "/repo/wt";
        let dir = tmp.path().join(".claude/projects").join(encode_cwd(cwd));
        assert_eq!(resume_state(tmp.path(), cwd, "x"), ResumeState::Unknown);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(resume_state(tmp.path(), cwd, "gone"), ResumeState::Missing);
        fs::write(dir.join("sess-123.jsonl"), "{}").unwrap();
        assert_eq!(
            resume_state(tmp.path(), cwd, "sess-123"),
            ResumeState::Present
        );
        assert_eq!(resume_state(tmp.path(), cwd, ""), ResumeState::Unknown);
    }
}
