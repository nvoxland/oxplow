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
    AgentHarness, Endpoints, Gate, HarnessError, HarnessSetting, Input, Interact, Launch,
    LaunchInput, LaunchSpec, Transcript,
};
use oxplow_domain::agent::observe::{HookAnswer, OtlpRecord, TokenReading, Turn, UsageDelta};
use oxplow_domain::agent::text::AgentText;
use oxplow_domain::events::schema::TokenKind;

use super::shared::{
    in_shell, program_and_guard, runtime, shell_escape, write_commands, write_json, write_skills,
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
        let plugin_dir =
            write_plugin(input.project_dir, &ep.hook_base_url, input.text).map_err(runtime)?;
        // Its MCP config can't read env vars, so the session's bearer is
        // written into a per-session file only its owner can read.
        let mcp_config = write_mcp_config(
            &plugin_dir,
            &ep.mcp_endpoint_url,
            &ep.hook_token,
            &input.session.session.to_string(),
        )
        .map_err(runtime)?;
        let mut env = input.identity_env.to_vec();
        env.extend(otel_env(ep));
        let cwd = input.workspace.to_string_lossy();
        // A resume id whose transcript is gone launches fresh, with no raw
        // "No conversation found" error, and core forgets the id.
        let (resume, resume_dropped) = match (input.resume.filter(|r| !r.is_empty()), input.home) {
            (Some(id), Some(home)) if resume_state(home, &cwd, id) == ResumeState::Missing => {
                (None, true)
            }
            (resume, _) => (resume, false),
        };
        let program =
            (input.resolve_program)("claude").or_else(|| input.home.and_then(local_install));
        Ok(Launch {
            spec: LaunchSpec::Pty {
                command: command(&Command {
                    cwd: &cwd,
                    resume,
                    program: program.as_deref(),
                    plugin_dir: Some(&plugin_dir.to_string_lossy()),
                    system_prompt: input.system_prompt,
                    mcp_config: Some(&mcp_config.to_string_lossy()),
                }),
                env,
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

    fn settings(&self) -> &[HarnessSetting] {
        &[]
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

    fn writing_tools(&self) -> &[&str] {
        &[
            "write",
            "edit",
            "multiedit",
            "notebookedit",
            "bash",
            "agent",
            "task",
        ]
    }

    fn turns(&self, transcript: &str) -> Vec<Turn> {
        transcript_turns(transcript)
            .into_iter()
            .filter(Turn::is_recordable)
            .collect()
    }

    /// Its `claude_code.token.usage` counter (delta temporality): the
    /// `type` attribute is the kind.
    fn token_readings(&self, record: &OtlpRecord<'_>) -> Vec<TokenReading> {
        let OtlpRecord::Point {
            metric: TOKEN_METRIC,
            value,
            attributes,
            time_unix_nano,
            start_time_unix_nano,
            ..
        } = record
        else {
            return Vec::new();
        };
        let kind = match attributes.str("type") {
            Some("input") => TokenKind::Input,
            Some("output") => TokenKind::Output,
            Some("cacheRead") => TokenKind::CacheRead,
            Some("cacheCreation") => TokenKind::CacheCreation,
            _ => return Vec::new(),
        };
        // An untrusted body: a negative count is no count.
        if *value <= 0 {
            return Vec::new();
        }
        vec![TokenReading {
            model: record.model(),
            kind,
            value: *value,
            at_unix_nano: *time_unix_nano,
            from_unix_nano: *start_time_unix_nano,
        }]
    }

    fn render(&self, answer: &HookAnswer) -> serde_json::Value {
        super::shared::render(answer)
    }
}

/// Claude Code's per-model token counter.
const TOKEN_METRIC: &str = "claude_code.token.usage";

/// The person's prompt in a `type=="user"` message, or `None` when the line
/// isn't one. Claude reuses `type=="user"` for the prompt typed and for the
/// tool results it injects; a message whose content is only tool_result
/// blocks is the latter. String content is taken whole; array content
/// joins its `text` blocks.
fn user_prompt(msg: &serde_json::Value) -> Option<String> {
    let content = msg.get("content")?;
    if let Some(s) = content.as_str() {
        let s = s.trim();
        return (!s.is_empty()).then(|| s.to_string());
    }
    let parts: Vec<&str> = content
        .as_array()?
        .iter()
        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// A transcript chunk split into turns: each prompt opens one; assistant
/// `usage` blocks accumulate into the current one; tool-result messages
/// fold in. Assistant lines before any prompt form a leading prompt-less
/// turn. Blank and malformed lines are skipped, so a partly written tail
/// line never poisons the sum.
fn transcript_turns(content: &str) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in content.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(msg) = v.get("message") else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => {
                if let Some(prompt) = user_prompt(msg) {
                    turns.push(Turn {
                        prompt: Some(prompt),
                        usage: UsageDelta::default(),
                    });
                }
            }
            Some("assistant") => {
                let Some(usage) = msg.get("usage") else {
                    continue;
                };
                // Claude writes one line per content block (thinking, text,
                // tool_use), each repeating the message's cumulative usage:
                // a message counts once, by its id. A line without one (a
                // synthetic one) counts on its own.
                if let Some(id) = msg.get("id").and_then(|i| i.as_str()) {
                    if !seen.insert(id.to_string()) {
                        continue;
                    }
                }
                if turns.is_empty() {
                    turns.push(Turn::default());
                }
                let d = &mut turns.last_mut().expect("just pushed").usage;
                let get = |k: &str| usage.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
                d.input_tokens += get("input_tokens");
                d.output_tokens += get("output_tokens");
                d.cache_creation_input_tokens += get("cache_creation_input_tokens");
                d.cache_read_input_tokens += get("cache_read_input_tokens");
                d.message_count += 1;
                if let Some(m) = msg.get("model").and_then(|m| m.as_str()) {
                    d.model = Some(m.to_string());
                }
            }
            _ => {}
        }
    }
    turns
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
/// explicit allowlisting via `allowedEnvVars`. The bearer is the whole of
/// who a hook comes from.
const HOOK_ENV_VARS: &[&str] = &["OXPLOW_HOOK_TOKEN"];

/// Materialize the plugin directory, returning it. `hook_base_url` is the
/// control plane's absolute hook URL (e.g. `http://127.0.0.1:51823/hook`).
/// It holds no bearer: each session's is in its own MCP config and env.
/// Re-running against the same `project_dir` overwrites in place.
fn write_plugin(project_dir: &Path, hook_base_url: &str, text: &AgentText) -> io::Result<PathBuf> {
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

fn build_mcp_config(mcp_endpoint_url: &str, hook_token: &str) -> serde_json::Value {
    // Bake the session's literal bearer into the file. Claude Code's MCP
    // config schema does not env-var-interpolate `headers` (unlike hooks,
    // which opt in via `allowedEnvVars`), so `"Bearer $VAR"` would be sent
    // verbatim and the control plane would 401. The file lives under
    // `.oxplow/runtime/claude-plugin/` (gitignored, owner-only) and is
    // rewritten per launch, so it holds the session's current bearer.
    json!({
        "mcpServers": {
            "oxplow": {
                "type": "http",
                "url": mcp_endpoint_url,
                "headers": { "Authorization": format!("Bearer {hook_token}") },
            },
        },
    })
}

/// A session's MCP config (`mcp-config.<session>.json`) carrying its
/// bearer, which is who its MCP calls come from. Returns the path to pass
/// as `--mcp-config`.
fn write_mcp_config(
    plugin_dir: &Path,
    mcp_endpoint_url: &str,
    hook_token: &str,
    session: &str,
) -> io::Result<PathBuf> {
    let path = plugin_dir.join(format!("mcp-config.{session}.json"));
    write_json(&path, &build_mcp_config(mcp_endpoint_url, hook_token))?;
    Ok(path)
}

/// Claude Code's older self-contained install (`~/.claude/local/claude`),
/// which no PATH names: where it looks when the resolver finds nothing.
fn local_install(home: &Path) -> Option<String> {
    let bin = home.join(".claude").join("local").join("claude");
    bin.is_file().then(|| bin.to_string_lossy().into_owned())
}

/// What the `claude` command line is built from.
struct Command<'a> {
    cwd: &'a str,
    resume: Option<&'a str>,
    program: Option<&'a str>,
    plugin_dir: Option<&'a str>,
    system_prompt: Option<&'a str>,
    mcp_config: Option<&'a str>,
}

/// `claude` with its plugin, prompt and MCP config, resuming `resume`
/// with a fallback to a fresh session when the id is stale.
fn command(c: &Command<'_>) -> String {
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
    let fresh = format!("exec {base}");
    let command = match c.resume {
        None => fresh,
        Some(id) => format!(
            "{base} --resume {} || {{ echo '[oxplow] saved resume id was stale; starting a fresh Claude session' >&2; {fresh}; }}",
            shell_escape(id)
        ),
    };
    in_shell(c.cwd, &guard, &command)
}

/// OTEL env that points Claude Code's OTLP metrics exporter at oxplow's
/// receiver. Only metrics are exported. The session's bearer rides the
/// OTLP headers, so the receiver attributes token facts to the session's
/// turn. Temporality is the SDK default (delta): each export is the
/// per-interval increment, additive as `oxplow.tokens` facts.
fn otel_env(ep: &Endpoints) -> Vec<(String, String)> {
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
            format!("Authorization=Bearer {}", ep.hook_token),
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
    use crate::test_launch::{harness, launch_in, owner_only};
    use oxplow_domain::agent::text::Text;
    use tempfile::TempDir;

    fn pty(launch: &Launch) -> &str {
        crate::test_launch::pty(launch).0
    }

    /// The launch writes its plugin and per-session MCP config; its env
    /// carries the session's bearer and identity and the OTEL exporter, its
    /// command the prompt, the plugin and the MCP config — and no secret.
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
        let (cmd, env) = crate::test_launch::pty(&l.launch);
        assert_eq!(env["OXPLOW_HOOK_TOKEN"], "secret-bearer");
        assert_eq!(env["OXPLOW_SESSION"], "ses3");
        assert_eq!(env["CLAUDE_CODE_ENABLE_TELEMETRY"], "1");
        assert_eq!(
            env["OTEL_EXPORTER_OTLP_HEADERS"],
            "Authorization=Bearer secret-bearer"
        );
        assert!(!cmd.contains("OXPLOW_"), "{cmd}");
        for want in [
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
        let config = l
            .project
            .join(".oxplow/runtime/claude-plugin/mcp-config.ses3.json");
        assert!(owner_only(&config));
        let body = fs::read_to_string(&config).unwrap();
        assert!(
            body.contains("Bearer secret-bearer") && !body.contains("X-Oxplow"),
            "{body}"
        );
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
        let env: std::collections::HashMap<String, String> = otel_env(&ep).into_iter().collect();
        assert_eq!(env["OTEL_METRICS_EXPORTER"], "otlp");
        assert_eq!(env["OTEL_EXPORTER_OTLP_PROTOCOL"], "http/protobuf");
        assert_eq!(env["OTEL_EXPORTER_OTLP_ENDPOINT"], "http://127.0.0.1:9");
        assert_eq!(
            env["OTEL_EXPORTER_OTLP_HEADERS"],
            "Authorization=Bearer tok123"
        );
        assert_eq!(env["OTEL_METRIC_EXPORT_INTERVAL"], "10000");
    }

    #[test]
    fn a_fresh_session_has_no_resume() {
        let c = Command {
            cwd: "/repo",
            resume: None,
            program: None,
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

    fn plugin(project: &Path, text: &AgentText) -> PathBuf {
        write_plugin(project, "http://h/hook", text).unwrap()
    }

    #[test]
    fn write_plugin_emits_expected_files() {
        let tmp = TempDir::new().unwrap();
        let dir = plugin(tmp.path(), &oxplow_agent_text::core_text());
        for file in [
            ".claude-plugin/plugin.json",
            "hooks/hooks.json",
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
        let dir = plugin(tmp.path(), &text);
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
        plugin(tmp.path(), &oxplow_agent_text::core_text());
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
        let dir = plugin(tmp.path(), &oxplow_agent_text::core_text());
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
        // The bearer is the whole of who a hook comes from.
        assert_eq!(
            entry["headers"],
            serde_json::json!({ "Authorization": "Bearer $OXPLOW_HOOK_TOKEN" })
        );
        assert_eq!(
            entry["allowedEnvVars"],
            serde_json::json!(["OXPLOW_HOOK_TOKEN"])
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
        let v = build_mcp_config("http://127.0.0.1:8/mcp", "tok");
        assert_eq!(v["mcpServers"]["oxplow"]["type"], "http");
        assert_eq!(v["mcpServers"]["oxplow"]["url"], "http://127.0.0.1:8/mcp");
    }

    #[test]
    fn mcp_config_inlines_literal_bearer_token() {
        // Regression: Claude Code does not env-var-interpolate MCP
        // header values, so the token must land in the file as a
        // literal string — not "Bearer $OXPLOW_HOOK_TOKEN".
        let v = build_mcp_config("http://x/mcp", "abc123");
        assert_eq!(
            v["mcpServers"]["oxplow"]["headers"],
            serde_json::json!({ "Authorization": "Bearer abc123" })
        );
    }

    #[test]
    fn write_plugin_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let text = oxplow_agent_text::core_text();
        write_plugin(tmp.path(), "http://h/hook", &text).unwrap();
        let dir = write_plugin(tmp.path(), "http://h2/hook", &text).unwrap();
        let body = fs::read_to_string(dir.join("hooks/hooks.json")).unwrap();
        assert!(body.contains("http://h2/hook"));
        // No file every session shares holds a bearer.
        assert!(!dir.join("mcp-config.json").exists());
    }

    const ASSISTANT_LINE: &str = r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":100,"output_tokens":20,"cache_creation_input_tokens":50,"cache_read_input_tokens":200}}}"#;

    /// A prompt line (string content).
    fn user_line(text: &str) -> String {
        serde_json::json!({"type": "user", "message": {"content": text}}).to_string()
    }

    /// An assistant line for message `id` with the given cumulative usage.
    /// Claude writes one per content block, each repeating id and usage.
    fn assistant_id_line(id: &str, input: i64, output: i64) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {
                "id": id,
                "model": "claude-opus-4-8",
                "usage": {"input_tokens": input, "output_tokens": output},
            },
        })
        .to_string()
    }

    #[test]
    fn a_turn_sums_its_assistant_lines_and_skips_the_rest() {
        let h = harness("oxplow:claude-code", "claude");
        let content = format!(
            "{}\n{ASSISTANT_LINE}\nnot json at all\n{ASSISTANT_LINE}\n",
            user_line("hi")
        );
        let turns = h.turns(&content);
        assert_eq!(turns.len(), 1);
        let d = &turns[0].usage;
        assert_eq!(
            (
                d.message_count,
                d.input_tokens,
                d.output_tokens,
                d.cache_creation_input_tokens,
                d.cache_read_input_tokens
            ),
            (2, 200, 40, 100, 400)
        );
        assert_eq!(d.model.as_deref(), Some("claude-opus-4-8"));
        assert!(h.turns("{\"type\":\"user\"}\n").is_empty());
    }

    #[test]
    fn a_message_counts_once_however_many_lines_carry_it() {
        let line = assistant_id_line("msg_a", 10, 583);
        let content = format!("{}\n{line}\n{line}\n{line}\n", user_line("prompt"));
        let turns = transcript_turns(&content);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].usage.output_tokens, 583, "counted once");
        assert_eq!(turns[0].usage.input_tokens, 10);
        assert_eq!(turns[0].usage.message_count, 1);
    }

    #[test]
    fn each_prompt_opens_a_turn() {
        let content = format!(
            "{}\n{ASSISTANT_LINE}\n{}\n{ASSISTANT_LINE}\n",
            user_line("prompt A"),
            user_line("prompt B"),
        );
        let turns = harness("oxplow:claude-code", "claude").turns(&content);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].prompt.as_deref(), Some("prompt A"));
        assert_eq!(turns[0].usage.input_tokens, 100);
        assert_eq!(turns[1].prompt.as_deref(), Some("prompt B"));
        assert_eq!(turns[1].usage.model.as_deref(), Some("claude-opus-4-8"));
    }

    #[test]
    fn tool_result_user_messages_do_not_open_a_turn() {
        let tool_result = serde_json::json!({
            "type": "user",
            "message": {"content": [{"type": "tool_result", "content": "ok"}]}
        })
        .to_string();
        let content = format!(
            "{}\n{ASSISTANT_LINE}\n{tool_result}\n{ASSISTANT_LINE}\n",
            user_line("real prompt"),
        );
        let turns = transcript_turns(&content);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].prompt.as_deref(), Some("real prompt"));
        assert_eq!(turns[0].usage.message_count, 2);
    }

    #[test]
    fn a_prompt_joins_text_blocks_and_skips_tool_results() {
        let arr = serde_json::json!({"content": [
            {"type": "text", "text": "hello"},
            {"type": "tool_result", "content": "ignored"},
            {"type": "text", "text": "world"},
        ]});
        assert_eq!(user_prompt(&arr).as_deref(), Some("hello\nworld"));
        let only_tool = serde_json::json!({"content": [{"type": "tool_result"}]});
        assert!(user_prompt(&only_tool).is_none());
        assert!(user_prompt(&serde_json::json!({"content": "   "})).is_none());
    }

    /// Its token counter's four kinds, the model falling back to the
    /// resource; another metric, an unknown kind and a non-positive count
    /// read as nothing.
    #[test]
    fn the_token_counter_maps_its_four_kinds() {
        use oxplow_domain::agent::observe::{AttrValue, Attrs};
        let h = harness("oxplow:claude-code", "claude");
        let resource = Attrs(vec![("model".into(), AttrValue::Str("claude-x".into()))]);
        let read = |metric: &str, kind: &str, value: i64| {
            let a = Attrs(vec![("type".into(), AttrValue::Str(kind.into()))]);
            h.token_readings(&OtlpRecord::Point {
                metric,
                value,
                attributes: &a,
                resource: &resource,
                time_unix_nano: 9,
                start_time_unix_nano: 3,
            })
        };
        for (kind, want) in [
            ("input", TokenKind::Input),
            ("output", TokenKind::Output),
            ("cacheRead", TokenKind::CacheRead),
            ("cacheCreation", TokenKind::CacheCreation),
        ] {
            assert_eq!(
                read(TOKEN_METRIC, kind, 5),
                vec![TokenReading {
                    model: "claude-x".into(),
                    kind: want,
                    value: 5,
                    at_unix_nano: 9,
                    from_unix_nano: 3,
                }]
            );
        }
        assert!(read("claude_code.cost.usage", "input", 5).is_empty());
        assert!(read(TOKEN_METRIC, "total", 5).is_empty());
        assert!(read(TOKEN_METRIC, "input", 0).is_empty());
        assert!(read(TOKEN_METRIC, "input", -5).is_empty());
    }

    /// With nothing on PATH or in the common dirs, Claude Code's own
    /// self-contained install is used.
    #[test]
    fn its_local_install_is_the_fallback() {
        let home = TempDir::new().unwrap();
        assert_eq!(local_install(home.path()), None);
        let bin = home.path().join(".claude/local/claude");
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        fs::write(&bin, "").unwrap();
        assert_eq!(
            local_install(home.path()),
            Some(bin.to_string_lossy().into_owned())
        );
    }
}
