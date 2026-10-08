//! The `claude` harness: Claude Code in a terminal. Its runtime is a
//! plugin directory (hooks, skills, commands) and a per-session MCP config;
//! token usage rides its OTEL exporter; a stale resume id is caught before
//! launch by looking for the session's transcript.
//!
//! The plugin (`.oxplow/runtime/claude-plugin/`) has two surfaces:
//!
//! 1. **HTTP hooks** — `hooks/hooks.json` registers PreToolUse,
//!    UserPromptSubmit, Stop, etc. as HTTP POSTs back to the
//!    in-process control plane. Auth is a per-spawn bearer token
//!    threaded through `$OXPLOW_HOOK_TOKEN`; routing context
//!    (stream/thread/session) rides per-spawn env vars too.
//! 2. **MCP server config** — `mcp-config.<session>.json` points Claude at
//!    the same control-plane port via the streamable-HTTP MCP transport,
//!    passed as `--mcp-config <path> --strict-mcp-config` so the only MCP
//!    server in scope is oxplow's.
//!
//! Plus the skills, guide and slash commands it exposes as model-invoked
//! context. The dir is rewritten on every launch, so edits to skill content
//! take effect without a cleanup step; identity rides env-var-interpolated
//! headers, not file contents, so one dir serves every session (the MCP
//! config is the exception: its headers can't read env vars).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;

use oxplow_domain::agent::harness::{
    AgentHarness, Endpoints, Gate, HarnessError, Input, Interact, Launch, LaunchInput, LaunchSpec,
    Transcript,
};
use oxplow_domain::agent::text::AgentText;

use super::shared::{
    env_prefix, in_shell, program_and_guard, runtime, shell_escape, write_commands, write_json,
    write_skills,
};
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
        let ep = input.endpoints;
        let plugin_dir = write_plugin(
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
        let mcp_config = write_mcp_config(
            &plugin_dir,
            &ep.mcp_endpoint_url,
            &ep.hook_token,
            McpIdentity {
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
                    plugin_dir: Some(&plugin_dir.to_string_lossy()),
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

    fn refresh_text(&self, project_dir: &Path, text: &AgentText) -> Result<(), HarnessError> {
        let plugin_dir = project_dir.join(PLUGIN_DIR_REL);
        let skills_dir = plugin_dir.join("skills");
        if skills_dir.is_dir() {
            write_skills(&skills_dir, &text.skills).map_err(runtime)?;
        }
        let commands_dir = plugin_dir.join("commands");
        if commands_dir.is_dir() {
            write_commands(&commands_dir, &text.commands).map_err(runtime)?;
        }
        Ok(())
    }
}

const PLUGIN_DIR_REL: &str = ".oxplow/runtime/claude-plugin";

/// The plugin's name: Claude Code's prefix for its commands and skills
/// (`/oxplow:configure`).
const PLUGIN_NAME: &str = "oxplow";
const PLUGIN_VERSION: &str = "0.0.0";

/// The hook events registered. SessionStart is registered even though
/// Claude Code drops HTTP hooks for it ("HTTP hooks are not supported for
/// SessionStart" in its debug log) — the session id comes from whichever
/// hook fires next instead. The last group is only observed: subagents,
/// Claude's own task list and compaction are acked unread today, and their
/// payloads can be dumped (`OXPLOW_HOOK_DEBUG`) to learn their shapes.
const HOOK_EVENTS: &[&str] = &[
    "PreToolUse",
    "PostToolUse",
    "UserPromptSubmit",
    "SessionStart",
    "SessionEnd",
    "Stop",
    "Notification",
    "SubagentStart",
    "SubagentStop",
    "TaskCreated",
    "TaskCompleted",
    "PreCompact",
];

/// Env vars the hooks header-interpolate from. Claude Code requires
/// explicit allowlisting via `allowedEnvVars`.
const HOOK_ENV_VARS: &[&str] = &[
    "OXPLOW_HOOK_TOKEN",
    "OXPLOW_STREAM_ID",
    "OXPLOW_THREAD_ID",
    "OXPLOW_SESSION",
];

/// Materialize the plugin directory, returning it. `hook_base_url` and
/// `mcp_endpoint_url` are absolute URLs to the in-process control plane
/// (e.g. `http://127.0.0.1:51823/hook` and `…/mcp`). Re-running against
/// the same `project_dir` overwrites in place.
fn write_plugin(
    project_dir: &Path,
    hook_base_url: &str,
    mcp_endpoint_url: &str,
    hook_token: &str,
    text: &AgentText,
) -> io::Result<PathBuf> {
    let plugin_dir = project_dir.join(PLUGIN_DIR_REL);
    let manifest_dir = plugin_dir.join(".claude-plugin");
    let hooks_dir = plugin_dir.join("hooks");
    let commands_dir = plugin_dir.join("commands");
    fs::create_dir_all(&manifest_dir)?;
    fs::create_dir_all(&hooks_dir)?;
    fs::create_dir_all(&commands_dir)?;
    write_json(
        &manifest_dir.join("plugin.json"),
        &json!({
            "name": PLUGIN_NAME,
            "version": PLUGIN_VERSION,
            "description": "Forwards Claude Code lifecycle hooks into the oxplow runtime.",
        }),
    )?;
    write_json(
        &hooks_dir.join("hooks.json"),
        &build_hooks_json(hook_base_url),
    )?;
    write_json(
        &plugin_dir.join("mcp-config.json"),
        &build_mcp_config(mcp_endpoint_url, hook_token, None),
    )?;
    fs::write(
        plugin_dir.join("AGENT_GUIDE.md"),
        include_str!("../assets/AGENT_GUIDE.md"),
    )?;
    write_skills(&plugin_dir.join("skills"), &text.skills)?;
    write_commands(&commands_dir, &text.commands)?;
    Ok(plugin_dir)
}

fn build_hooks_json(hook_base_url: &str) -> serde_json::Value {
    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        let entry = json!({
            "type": "http",
            "url": format!("{}/{}", hook_base_url.trim_end_matches('/'), event),
            // MUST exceed the control plane's HOOK_HANDLING_TIMEOUT (5s): the
            // server races handling against that budget and always answers by
            // then, so the client never actually waits this long — but a
            // client timeout BELOW it makes Claude Code discard output the
            // server was about to deliver ("hook timed out after 3s", seen on
            // the first prompt after a daemon restart while boot work holds
            // the DB writer).
            "timeout": 8,
            "headers": {
                "Authorization": "Bearer $OXPLOW_HOOK_TOKEN",
                "X-Oxplow-Stream": "$OXPLOW_STREAM_ID",
                "X-Oxplow-Thread": "$OXPLOW_THREAD_ID",
                "X-Oxplow-Session": "$OXPLOW_SESSION",
            },
            "allowedEnvVars": HOOK_ENV_VARS,
        });
        // PreToolUse / PostToolUse have a per-tool matcher; everything
        // else is unconditional.
        let outer = if matches!(*event, "PreToolUse" | "PostToolUse") {
            json!([{ "matcher": "*", "hooks": [entry] }])
        } else {
            json!([{ "hooks": [entry] }])
        };
        hooks.insert(event.to_string(), outer);
    }
    json!({ "hooks": serde_json::Value::Object(hooks) })
}

/// The thread, stream and agent session an MCP connection acts for, as the
/// control plane reads them (`X-Oxplow-Thread` / `X-Oxplow-Stream` /
/// `X-Oxplow-Session`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct McpIdentity<'a> {
    thread_id: &'a str,
    stream_id: &'a str,
    session_id: Option<&'a str>,
}

fn build_mcp_config(
    mcp_endpoint_url: &str,
    hook_token: &str,
    identity: Option<McpIdentity<'_>>,
) -> serde_json::Value {
    // Bake the literal token — and the identity — into the file. Claude
    // Code's MCP config schema does not env-var-interpolate `headers`
    // (unlike hooks, which opt in via `allowedEnvVars`), so
    // `"Bearer $VAR"` would be sent verbatim and the control plane would
    // 401. The file lives under `.oxplow/runtime/claude-plugin/`
    // (gitignored) and is rewritten per launch, so it tracks the current
    // boot's token.
    let mut headers = serde_json::Map::new();
    headers.insert(
        "Authorization".into(),
        format!("Bearer {hook_token}").into(),
    );
    if let Some(id) = identity {
        headers.insert("X-Oxplow-Thread".into(), id.thread_id.into());
        headers.insert("X-Oxplow-Stream".into(), id.stream_id.into());
        if let Some(session) = id.session_id {
            headers.insert("X-Oxplow-Session".into(), session.into());
        }
    }
    json!({
        "mcpServers": {
            "oxplow": {
                "type": "http",
                "url": mcp_endpoint_url,
                "headers": headers,
            },
        },
    })
}

/// A per-session MCP config (`mcp-config.<session>.json`, beside the
/// shared `mcp-config.json`) carrying its identity headers, so
/// `run_command` and every audited write know who is acting. Returns the
/// path to pass as `--mcp-config`.
fn write_mcp_config(
    plugin_dir: &Path,
    mcp_endpoint_url: &str,
    hook_token: &str,
    identity: McpIdentity<'_>,
) -> io::Result<PathBuf> {
    let path = plugin_dir.join(format!(
        "mcp-config.{}.json",
        identity.session_id.unwrap_or(identity.thread_id)
    ));
    write_json(
        &path,
        &build_mcp_config(mcp_endpoint_url, hook_token, Some(identity)),
    )?;
    Ok(path)
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
    use crate::test_launch::{harness, launch_in};
    use oxplow_domain::agent::text::Text;
    use tempfile::TempDir;

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
        let l = crate::test_launch::launch_with_home(h.as_ref(), "gone", |home, cwd| {
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

    fn plugin(project: &Path, token: &str, text: &AgentText) -> PathBuf {
        write_plugin(project, "http://h/hook", "http://h/mcp", token, text).unwrap()
    }

    #[test]
    fn write_plugin_emits_expected_files() {
        let tmp = TempDir::new().unwrap();
        let dir = plugin(tmp.path(), "tok", &oxplow_agent_text::core_text());
        for file in [
            ".claude-plugin/plugin.json",
            "hooks/hooks.json",
            "mcp-config.json",
            "AGENT_GUIDE.md",
            "commands/review-comments.md",
            "commands/configure.md",
        ] {
            assert!(dir.join(file).exists(), "missing {file}");
        }
        for skill in oxplow_agent_text::core_text().skills {
            assert!(
                dir.join("skills")
                    .join(&skill.name)
                    .join("SKILL.md")
                    .exists(),
                "missing skill {}",
                skill.name
            );
        }
    }

    /// What's offered is installed; a skill or command oxplow wrote that
    /// isn't offered any more (retired, or its extension's implementation
    /// not active) leaves on the next refresh; a person's own stays.
    #[test]
    fn what_is_no_longer_offered_is_removed() {
        let tmp = TempDir::new().unwrap();
        let mut text = oxplow_agent_text::core_text();
        text.skills.push(Text {
            name: "work-items".into(),
            body: "---\nname: work-items\ndescription: d\n---\n".into(),
        });
        text.commands.push(Text {
            name: "work-next".into(),
            body: "next".into(),
        });
        let dir = plugin(tmp.path(), "tok", &text);
        let skills = dir.join("skills");
        let commands = dir.join("commands");
        fs::create_dir_all(skills.join("someone-elses")).unwrap();
        fs::write(skills.join("someone-elses").join("SKILL.md"), "x").unwrap();
        assert!(skills.join("work-items").join("SKILL.md").exists());
        assert!(commands.join("work-next.md").exists());
        let h = harness("oxplow:claude-code", "claude");
        h.refresh_text(tmp.path(), &oxplow_agent_text::core_text())
            .unwrap();
        assert!(!skills.join("work-items").exists());
        assert!(!commands.join("work-next.md").exists());
        assert!(skills.join("someone-elses").exists());
        assert!(skills.join("oxplow-runtime").join("SKILL.md").exists());
        assert!(commands.join("configure.md").exists());
    }

    /// A refresh rewrites a plugin already on disk and creates none.
    #[test]
    fn refresh_text_rewrites_only_runtimes_already_on_disk() {
        let tmp = TempDir::new().unwrap();
        let h = harness("oxplow:claude-code", "claude");
        let text = oxplow_agent_text::core_text();
        h.refresh_text(tmp.path(), &text).unwrap();
        assert!(!tmp.path().join(PLUGIN_DIR_REL).exists());
        let skills = tmp.path().join(PLUGIN_DIR_REL).join("skills");
        fs::create_dir_all(skills.join("oxplow-extension")).unwrap();
        fs::write(skills.join("oxplow-extension/SKILL.md"), "stale").unwrap();
        h.refresh_text(tmp.path(), &text).unwrap();
        for skill in &text.skills {
            assert_eq!(
                fs::read_to_string(skills.join(&skill.name).join("SKILL.md")).unwrap(),
                skill.body
            );
        }
    }

    #[test]
    fn shipped_plugin_files_never_mention_dot_context() {
        // `.context/` is THIS repo's own docs convention — it must never
        // leak into the skills / prompts / hooks / commands oxplow writes
        // into a user's project (those docs don't exist downstream).
        let tmp = TempDir::new().unwrap();
        plugin(tmp.path(), "tok", &oxplow_agent_text::core_text());
        let mut offenders = Vec::new();
        let mut stack = vec![tmp.path().to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if let Ok(body) = fs::read_to_string(&path) {
                    if body.contains(".context") {
                        offenders.push(path.display().to_string());
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "emitted plugin files mention .context: {offenders:?}"
        );
    }

    #[test]
    fn manifest_name_drives_oxplow_command_prefix() {
        // The plugin `name` is the Claude Code command/skill prefix:
        // commands surface as `/<name>:<command>`. Keep it `oxplow`
        // so users type `/oxplow:configure`, not `/oxplow-runtime:…`.
        let tmp = TempDir::new().unwrap();
        let dir = plugin(tmp.path(), "tok", &oxplow_agent_text::core_text());
        let body = fs::read_to_string(dir.join(".claude-plugin/plugin.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["name"], "oxplow");
    }

    #[test]
    fn hooks_json_contains_pretooluse_with_matcher() {
        let v = build_hooks_json("http://h/hook");
        let pre = &v["hooks"]["PreToolUse"][0];
        assert_eq!(pre["matcher"], "*");
        let entry = &pre["hooks"][0];
        assert_eq!(entry["type"], "http");
        assert_eq!(entry["url"], "http://h/hook/PreToolUse");
        assert_eq!(
            entry["headers"]["Authorization"],
            "Bearer $OXPLOW_HOOK_TOKEN"
        );
    }

    /// The events oxplow only observes (subagents, Claude's own task list,
    /// compaction) are registered too, so their payloads reach the hook
    /// endpoint (acked unread, dumped under `OXPLOW_HOOK_DEBUG`).
    #[test]
    fn hooks_json_registers_the_observed_events() {
        let v = build_hooks_json("http://h/hook");
        for event in [
            "SubagentStart",
            "SubagentStop",
            "TaskCreated",
            "TaskCompleted",
            "PreCompact",
            "Notification",
        ] {
            assert_eq!(
                v["hooks"][event][0]["hooks"][0]["url"],
                format!("http://h/hook/{event}"),
                "{event}"
            );
        }
    }

    #[test]
    fn mcp_config_uses_http_transport() {
        let v = build_mcp_config("http://127.0.0.1:8/mcp", "tok", None);
        assert_eq!(v["mcpServers"]["oxplow"]["type"], "http");
        assert_eq!(v["mcpServers"]["oxplow"]["url"], "http://127.0.0.1:8/mcp");
    }

    #[test]
    fn mcp_config_inlines_literal_bearer_token() {
        // Regression: Claude Code does not env-var-interpolate MCP
        // header values, so the token must land in the file as a
        // literal string — not "Bearer $OXPLOW_HOOK_TOKEN".
        let v = build_mcp_config("http://x/mcp", "abc123", None);
        assert_eq!(
            v["mcpServers"]["oxplow"]["headers"]["Authorization"],
            "Bearer abc123"
        );
    }

    #[test]
    fn write_plugin_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let text = oxplow_agent_text::core_text();
        write_plugin(tmp.path(), "http://h/hook", "http://h/mcp", "t1", &text).unwrap();
        let dir = write_plugin(tmp.path(), "http://h2/hook", "http://h2/mcp", "t2", &text).unwrap();
        let body = fs::read_to_string(dir.join("hooks/hooks.json")).unwrap();
        assert!(body.contains("http://h2/hook"));
        let mcp_body = fs::read_to_string(dir.join("mcp-config.json")).unwrap();
        assert!(mcp_body.contains("Bearer t2"));
    }
}
