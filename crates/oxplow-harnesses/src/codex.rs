//! The `codex` harness: Codex in a terminal. Its hooks are commands
//! (`oxplow hook <event>`) and its MCP server, hooks and OTEL exporter ride
//! `--config` overrides. Embedded mode is explicit when its CLI supports
//! `--no-daemon`, keeping those overrides and identity in this process.
//! Its skills are the one thing on disk: Codex finds
//! them only beside the directory it runs in, so they're in the worktree's
//! `.agents/skills/`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use oxplow_domain::agent::harness::{
    AgentHarness, HarnessError, Interact, Launch, LaunchInput, LaunchSpec, RuntimeRoots, Transcript,
};
use oxplow_domain::agent::observe::{HookAnswer, OtlpRecord, TokenReading};
use oxplow_domain::agent::text::AgentText;
use oxplow_domain::agent::tool::{ToolKind, ToolUse};
use oxplow_domain::events::schema::TokenKind;

use super::shared::{
    in_shell, program_and_guard, refresh_worktree_skills, runtime, shell_escape, toml_string,
    write_worktree_skills,
};
use super::Named;

pub(super) struct Codex(pub(super) Named);

#[async_trait::async_trait]
impl AgentHarness for Codex {
    fn id(&self) -> &str {
        &self.0.id
    }

    fn title(&self) -> &str {
        &self.0.title
    }

    fn interact(&self) -> Interact {
        Interact {
            transcript: Transcript::Terminal,
        }
    }

    async fn launch(&self, input: &LaunchInput) -> Result<Launch, HarnessError> {
        let ep = &input.endpoints;
        write_worktree_skills(&input.workspace, &input.text.skills).map_err(runtime)?;
        let mut overrides = config_overrides(&input.oxplow_executable, &ep.mcp_endpoint_url);
        // Its exporter rides `--config otel.*`, to the same receiver as
        // Claude's; the session's bearer rides its env.
        overrides.extend(otel_overrides(&ep.otlp_base_url));
        let program = input.resolve_program("codex");
        let embedded = supports_embedded_mode(program.as_deref()).await;
        Ok(Launch {
            spec: LaunchSpec::Pty {
                command: command(
                    &input.workspace.to_string_lossy(),
                    input.resume.as_deref().filter(|r| !r.is_empty()),
                    program.as_deref(),
                    &overrides,
                    embedded,
                ),
                env: input
                    .identity_env
                    .iter()
                    .cloned()
                    .chain([otel_env(&ep.hook_token)])
                    .collect(),
            },
            resume_dropped: false,
        })
    }

    fn instruction_files(&self) -> Vec<String> {
        vec!["CLAUDE.md".into()]
    }

    async fn refresh_text(
        &self,
        roots: &RuntimeRoots,
        text: &AgentText,
    ) -> Result<(), HarnessError> {
        refresh_worktree_skills(&roots.workspaces, &text.skills).map_err(runtime)
    }

    async fn tool_use(&self, body: &serde_json::Value) -> Option<ToolUse> {
        codex_tool_use(body)
    }

    async fn token_readings(&self, records: &[OtlpRecord]) -> Vec<TokenReading> {
        records.iter().flat_map(token_reading).collect()
    }

    async fn render(&self, answer: &HookAnswer) -> serde_json::Value {
        super::shared::render(answer)
    }
}

/// Its token counts: the `codex.turn.token_usage` counter, else a
/// `response.completed` log event's counts.
fn token_reading(record: &OtlpRecord) -> Vec<TokenReading> {
    match record {
        OtlpRecord::Point {
            metric,
            value,
            attributes,
            time_unix_nano,
            start_time_unix_nano,
            ..
        } if metric == TOKEN_METRIC => {
            let kind = match attributes.str("token_type") {
                Some("input") => TokenKind::Input,
                Some("output" | "reasoning_output") => TokenKind::Output,
                Some("cached_input") => TokenKind::CacheRead,
                _ => return Vec::new(),
            };
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
        OtlpRecord::Log {
            attributes,
            time_unix_nano,
            ..
        } if attributes.str("event.kind") == Some(TOKEN_EVENT_KIND) => {
            // An untrusted body: a negative count is none, and the
            // arithmetic saturates rather than overflows.
            let count = |key| attributes.int(key).unwrap_or(0).max(0);
            let input = count("input_token_count");
            let cached = count("cached_token_count");
            // New (uncached) input this request; reasoning folded into
            // output; the cached prefix is its own CacheRead count.
            let readings = [
                (TokenKind::Input, input.saturating_sub(cached).max(0)),
                (TokenKind::CacheRead, cached),
                (
                    TokenKind::Output,
                    count("output_token_count").saturating_add(count("reasoning_token_count")),
                ),
            ];
            readings
                .into_iter()
                .filter(|(_, value)| *value > 0)
                .map(|(kind, value)| TokenReading {
                    model: record.model(),
                    kind,
                    value,
                    at_unix_nano: *time_unix_nano,
                    from_unix_nano: *time_unix_nano,
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Its per-turn token histogram, `token_type` the kind and the point's
/// sum the count. Codex 0.142.0 doesn't emit it (its counts ride the
/// `response.completed` log event), so this reads it in case a later one
/// does. `reasoning_output` folds into output; the `total` rollup is
/// dropped (it would double-count).
const TOKEN_METRIC: &str = "codex.turn.token_usage";

/// The `event.kind` of its `codex.sse_event` log record that carries a
/// request's token counts. `input_token_count` is the full context.
const TOKEN_EVENT_KIND: &str = "response.completed";

/// Codex's hook events, each run as `oxplow hook <event>`.
const HOOK_EVENTS: &[&str] = &[
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "UserPromptSubmit",
    "SessionStart",
    "Stop",
];

/// Whether a hook event takes a per-tool matcher.
fn per_tool(event: &str) -> bool {
    matches!(event, "PreToolUse" | "PermissionRequest" | "PostToolUse")
}

struct EmbeddedModeSupport {
    modified: Option<SystemTime>,
    len: u64,
    supported: bool,
}

/// Probe each resolved executable once, and again after an upgrade. Older
/// CLIs (or a failed probe) keep the existing override-based launch. A
/// slow wrapper must not prevent launch; dropping the timed-out probe
/// kills its child. No login or user configuration is changed.
async fn supports_embedded_mode(program: Option<&str>) -> bool {
    static CACHE: OnceLock<tokio::sync::Mutex<HashMap<String, EmbeddedModeSupport>>> =
        OnceLock::new();
    let Some(program) = program else {
        return false;
    };
    let Ok(metadata) = std::fs::metadata(program) else {
        return false;
    };
    let modified = metadata.modified().ok();
    let mut cache = CACHE.get_or_init(Default::default).lock().await;
    if let Some(entry) = cache.get(program) {
        if entry.modified == modified && entry.len == metadata.len() {
            return entry.supported;
        }
    }
    let Ok(Ok(output)) = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::process::Command::new(program)
            .arg("--help")
            .kill_on_drop(true)
            .output(),
    )
    .await
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let supported = String::from_utf8_lossy(&output.stdout)
        .split_ascii_whitespace()
        .any(|word| word == "--no-daemon");
    cache.insert(
        program.to_owned(),
        EmbeddedModeSupport {
            modified,
            len: metadata.len(),
            supported,
        },
    );
    supported
}

/// `codex --cd <cwd>` with its overrides and explicit embedded mode when
/// supported, or `codex resume` for `resume`.
fn command(
    cwd: &str,
    resume: Option<&str>,
    program: Option<&str>,
    overrides: &[String],
    embedded: bool,
) -> String {
    let (prog, guard) = program_and_guard(program, "codex");
    let mode = if embedded { " --no-daemon" } else { "" };
    let config_args: String = overrides
        .iter()
        .map(|c| format!(" --config {}", shell_escape(c)))
        .collect();
    let base = match resume {
        None => format!("{prog}{mode} --cd {}{config_args}", shell_escape(cwd)),
        Some(id) => format!(
            "{prog}{mode} resume --cd {}{config_args} {}",
            shell_escape(cwd),
            shell_escape(id)
        ),
    };
    in_shell(cwd, &guard, &format!("exec {base}"))
}

/// Codex's MCP server and command hooks, as `--config` overrides. Its MCP
/// connection carries the session's bearer from the env, which is who its
/// calls come from.
fn config_overrides(oxplow_executable: &Path, mcp_endpoint_url: &str) -> Vec<String> {
    let mut out = vec![
        format!("mcp_servers.oxplow.url={}", toml_string(mcp_endpoint_url)),
        "mcp_servers.oxplow.bearer_token_env_var=\"OXPLOW_HOOK_TOKEN\"".into(),
    ];
    for event in HOOK_EVENTS {
        let command = hook_command(oxplow_executable, event);
        let matcher = if per_tool(event) {
            "matcher=\"*\","
        } else {
            ""
        };
        out.push(format!(
            "hooks.{event}=[{{{matcher}hooks=[{{type=\"command\",command={},timeout=30,statusMessage=\"Syncing oxplow runtime\"}}]}}]",
            toml_string(&command)
        ));
    }
    out
}

/// Codex's OTEL config pointing its OTLP metrics exporter at oxplow's
/// receiver: protobuf ("binary") and the FULL signal URL (Codex uses it as
/// is). Its bearer isn't here: see [`otel_env`].
fn otel_overrides(otlp_base_url: &str) -> Vec<String> {
    let endpoint = format!("{otlp_base_url}/v1/metrics");
    vec![
        format!(
            "otel.exporter.otlp-http.endpoint={}",
            toml_string(&endpoint)
        ),
        "otel.exporter.otlp-http.protocol=\"binary\"".to_string(),
    ]
}

/// The session's bearer for its OTLP exporter, in the env: the standard
/// `OTEL_EXPORTER_OTLP_HEADERS`, which its OpenTelemetry exporter adds to
/// the headers its config gives (none) — never on its command line, whose
/// text any process can list.
fn otel_env(hook_token: &str) -> (String, String) {
    (
        "OTEL_EXPORTER_OTLP_HEADERS".into(),
        format!("Authorization=Bearer {hook_token}"),
    )
}

/// A Codex tool hook's body mapped onto oxplow's vocabulary. Codex posts
/// its own tool names in Claude Code's hook fields: `apply_patch` edits the
/// files its patch names (the patch its `command`, or older Codexes'
/// `input` / `patch`), `Bash` (older: `shell` / `exec_command`) runs a
/// command (a string or an argv list), `…spawn_agent` starts a subagent. A
/// result is text whose first line is `Exit code: N`, or (older) an object
/// with an `exit_code`.
fn codex_tool_use(body: &serde_json::Value) -> Option<ToolUse> {
    let name = body.get("tool_name")?.as_str()?.to_string();
    let input = body.get("tool_input").cloned().unwrap_or_default();
    let command = || -> Option<String> {
        let c = input.get("command").or_else(|| input.get("cmd"))?;
        match c {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Array(argv) => Some(
                argv.iter()
                    .filter_map(|a| a.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        }
    };
    let (kind, paths, command) = match name.as_str() {
        "apply_patch" => (ToolKind::Edit, super::shared::patch_paths(&input), None),
        "Bash" | "shell" | "exec_command" | "local_shell" => {
            (ToolKind::Shell, Vec::new(), command())
        }
        n if n.ends_with("spawn_agent") => (ToolKind::Subagent, Vec::new(), None),
        n if n.starts_with("mcp__") => (ToolKind::Mcp, Vec::new(), None),
        _ => (ToolKind::Other, Vec::new(), None),
    };
    let response = body.get("tool_response").filter(|r| !r.is_null());
    let exit_code = response.and_then(|r| match r.as_str() {
        Some(text) => text
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("Exit code: "))
            .and_then(|c| c.trim().parse().ok()),
        None => ["exit_code", "exitCode", "code"]
            .iter()
            .find_map(|k| r.get(*k).and_then(|x| x.as_i64())),
    });
    Some(ToolUse {
        name,
        kind,
        paths,
        detail: command.clone(),
        command,
        call_id: body
            .get("tool_use_id")
            .or_else(|| body.get("call_id"))
            .and_then(|t| t.as_str())
            .map(str::to_string),
        ok: exit_code.map(|c| c == 0),
        exit_code,
        question: None,
        subagent: super::shared::subagent_of(body),
    })
}

/// The command a Codex hook runs: the oxplow binary, which forwards it.
fn hook_command(oxplow_executable: &Path, event: &str) -> String {
    format!(
        "{} hook {}",
        shell_escape(&oxplow_executable.to_string_lossy()),
        shell_escape(event)
    )
}

#[cfg(test)]
mod tests {

    /// A Codex spawned agent's own tool calls carry its `agent_id` /
    /// `agent_type` (the recorded `codex/hooks.jsonl`).
    #[test]
    fn a_spawned_agents_calls_name_it() {
        let call = codex_tool_use(&serde_json::json!({
            "agent_id": "01a11f68-9625-7523-8ead-501bb708eb7d", "agent_type": "default",
            "tool_name": "Bash", "tool_input": {"command": "wc -l codex.txt"}
        }))
        .unwrap();
        assert_eq!(
            call.subagent,
            Some(oxplow_domain::agent::tool::Subagent {
                id: "01a11f68-9625-7523-8ead-501bb708eb7d".into(),
                kind: Some("default".into())
            })
        );
    }

    /// Codex 0.158's tool hooks, as recorded: its shell is `Bash`, its
    /// patch rides `command`, a result's exit code is the first line of
    /// its text, and a subagent is `collaborationspawn_agent`.
    #[test]
    fn codex_0_158_tool_hooks_map() {
        let shell = codex_tool_use(&serde_json::json!({
            "tool_name": "Bash",
            "tool_input": {"command": "cat codex.txt"},
            "tool_response": "2026-10-09 01:44:14 CDT\n"
        }))
        .unwrap();
        assert_eq!(shell.kind, ToolKind::Shell);
        assert_eq!(shell.command.as_deref(), Some("cat codex.txt"));
        let patch = codex_tool_use(&serde_json::json!({
            "tool_name": "apply_patch",
            "tool_input": {"command": "*** Begin Patch\n*** Add File: /w/codex.txt\n+x\n*** End Patch"},
            "tool_response": "Exit code: 0\nWall time: 0 seconds\nOutput:\nSuccess. Updated the following files:\nA /w/codex.txt\n"
        }))
        .unwrap();
        assert_eq!(patch.kind, ToolKind::Edit);
        assert_eq!(patch.paths, ["/w/codex.txt"]);
        assert_eq!(patch.command, None, "a patch isn't a shell command");
        assert_eq!((patch.exit_code, patch.ok), (Some(0), Some(true)));
        let failed = codex_tool_use(&serde_json::json!({
            "tool_name": "Bash",
            "tool_input": {"command": "false"},
            "tool_response": "Exit code: 1\nOutput:\n"
        }))
        .unwrap();
        assert_eq!((failed.exit_code, failed.ok), (Some(1), Some(false)));
        let spawn = codex_tool_use(&serde_json::json!({
            "tool_name": "collaborationspawn_agent",
            "tool_input": {"task_name": "count_lines"}
        }))
        .unwrap();
        assert_eq!(spawn.kind, ToolKind::Subagent);
    }

    use super::*;
    use crate::test_launch::{harness, launch_in};
    use std::fs;
    use tempfile::TempDir;

    /// `apply_patch` is an edit of every file its patch names; `shell` a
    /// command, joined when it's an argv list, with its exit code.
    #[test]
    fn codex_tools_map_onto_the_vocabulary() {
        let patch = codex_tool_use(&serde_json::json!({
            "tool_name": "apply_patch",
            "tool_input": {"input": "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** Add File: src/b.rs\n+z\n*** End Patch"}
        }))
        .unwrap();
        assert_eq!(patch.kind, ToolKind::Edit);
        assert_eq!(
            patch.paths,
            vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
        );
        let shell = codex_tool_use(&serde_json::json!({
            "tool_name": "shell",
            "tool_input": {"command": ["bash", "-lc", "cargo test"]},
            "tool_response": {"exit_code": 0}
        }))
        .unwrap();
        assert_eq!(shell.kind, ToolKind::Shell);
        assert_eq!(shell.command.as_deref(), Some("bash -lc cargo test"));
        assert_eq!((shell.exit_code, shell.ok), (Some(0), Some(true)));
        assert_eq!(
            codex_tool_use(&serde_json::json!({"tool_name": "view_image"}))
                .unwrap()
                .kind,
            ToolKind::Other
        );
    }

    #[test]
    fn launch_builds_the_command_and_env() {
        let h = harness("oxplow:codex-cli", "codex");
        let l = launch_in(h.as_ref(), Some("sess-w"), None, &serde_json::json!({}));
        let LaunchSpec::Pty { command, env } = &l.launch.spec else {
            panic!("not a PTY launch")
        };
        let env: std::collections::HashMap<_, _> = env.iter().cloned().collect();
        assert_eq!(env["OXPLOW_HOOK_TOKEN"], "secret-bearer");
        assert_eq!(env["OXPLOW_SESSION"], "ses3");
        for want in [
            "/opt/agents/codex",
            " resume --cd ",
            "sess-w",
            "--config",
            "mcp_servers.oxplow.url=\"http://127.0.0.1:9/mcp\"",
            "/bin/oxplow",
        ] {
            assert!(command.contains(want), "{want}: {command}");
        }
        // No identity rides the command, and no bearer: its OTLP
        // exporter's header is the standard env variable its exporter
        // reads, as Claude's is.
        assert!(!command.contains("OXPLOW_SESSION=") && !command.contains("OXPLOW_HOOK_TOKEN="));
        assert!(!command.contains("thread=") && !command.contains("x-oxplow"));
        assert!(!command.contains("secret-bearer"), "{command}");
        assert_eq!(
            env["OTEL_EXPORTER_OTLP_HEADERS"],
            "Authorization=Bearer secret-bearer"
        );
        assert!(!command.contains("--dangerously-bypass-hook-trust"));
    }

    #[test]
    fn the_otel_overrides_configure_the_http_exporter() {
        let ov = otel_overrides("http://127.0.0.1:9");
        assert!(ov.contains(
            &"otel.exporter.otlp-http.endpoint=\"http://127.0.0.1:9/v1/metrics\"".to_string()
        ));
        assert!(ov.contains(&"otel.exporter.otlp-http.protocol=\"binary\"".to_string()));
        assert!(!ov
            .iter()
            .any(|o| o.contains("headers") || o.contains("x-oxplow")));
    }

    #[test]
    fn the_hook_command_is_stable_and_url_independent() {
        let command = hook_command(Path::new("/Applications/Oxplow App/oxplow"), "PreToolUse");
        assert_eq!(
            command,
            "'/Applications/Oxplow App/oxplow' hook 'PreToolUse'"
        );
        assert!(!command.contains("http://"));
    }

    #[test]
    fn a_fresh_session_runs_codex_in_its_worktree() {
        let cmd = command("/repo", None, None, &[], true);
        assert!(cmd.starts_with("sh -lc "));
        assert!(cmd.contains("exec codex") && cmd.contains("--cd") && cmd.contains("/repo"));
        assert!(!cmd.contains(" resume "));
        assert!(cmd.contains("codex --no-daemon --cd"));
    }

    /// Execute the generated shell command: mode selection must preserve
    /// resume, quoted paths, overrides and the child's session identity.
    #[cfg(unix)]
    #[tokio::test]
    async fn embedded_mode_is_detected_and_preserves_launch_arguments() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let program = tmp.path().join("codex with spaces");
        let probes = tmp.path().join("probes");
        let help = format!(
            "#!/bin/sh\nif [ \"$1\" = --help ]; then\n echo probe >> {}\n echo 'Options: --no-daemon'\n exit 0\nfi\nprintf '%s\\n' \"$@\" \"$OXPLOW_SESSION\"\n",
            shell_escape(probes.to_str().unwrap())
        );
        fs::write(&program, &help).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        let path = program.to_str().unwrap();
        for resume in [None, Some("session 'quoted'")] {
            let embedded = supports_embedded_mode(Some(path)).await;
            assert!(embedded);
            let cmd = command(
                tmp.path().to_str().unwrap(),
                resume,
                Some(path),
                &["mcp_servers.oxplow.url=\"http://127.0.0.1:9/mcp\"".into()],
                embedded,
            );
            let output = tokio::process::Command::new("sh")
                .args(["-c", &cmd])
                .env("OXPLOW_SESSION", "ses3")
                .output()
                .await
                .unwrap();
            assert!(output.status.success());
            let stdout = String::from_utf8(output.stdout).unwrap();
            let args: Vec<_> = stdout.lines().collect();
            let mut expected = vec!["--no-daemon"];
            if resume.is_some() {
                expected.push("resume");
            }
            expected.extend([
                "--cd",
                tmp.path().to_str().unwrap(),
                "--config",
                "mcp_servers.oxplow.url=\"http://127.0.0.1:9/mcp\"",
            ]);
            expected.extend(resume);
            expected.push("ses3");
            assert_eq!(args, expected);
        }
        assert_eq!(fs::read_to_string(&probes).unwrap().lines().count(), 1);

        // Replacing the executable invalidates the cached capability;
        // an older CLI must still launch without an unknown flag.
        fs::write(
            &program,
            help.replace("Options: --no-daemon", "Options: --cd"),
        )
        .unwrap();
        assert!(!supports_embedded_mode(Some(path)).await);
        assert_eq!(fs::read_to_string(&probes).unwrap().lines().count(), 2);
        assert!(!command("/repo", Some("saved"), Some(path), &[], false).contains("--no-daemon"));

        fs::write(&program, "#!/bin/sh\nexit 1\n").unwrap();
        assert!(!supports_embedded_mode(Some(path)).await);
        assert!(!supports_embedded_mode(None).await);
    }

    /// Codex finds skills only beside the directory it runs in: they land
    /// in its worktree's `.agents/skills`, self-ignored, and nothing else
    /// is written for it — its hooks and MCP server ride `--config`.
    #[test]
    fn launch_puts_the_skills_in_the_worktree() {
        let h = harness("oxplow:codex-cli", "codex");
        let l = launch_in(h.as_ref(), None, None, &serde_json::json!({}));
        let skills = l.workspace.join(".agents/skills");
        for skill in &oxplow_agent_text::core_text().skills {
            let dir = skills.join(&skill.name);
            assert_eq!(
                fs::read_to_string(dir.join("SKILL.md")).unwrap(),
                skill.body
            );
            assert_eq!(
                fs::read_to_string(dir.join(".gitignore")).unwrap().trim(),
                "*"
            );
        }
        assert!(!l.project.join(".oxplow/runtime/codex-plugin").exists());
        assert!(!l.project.join(".agents").exists());
    }

    /// A refresh rewrites the worktrees oxplow put skills in, and creates
    /// none.
    #[test]
    fn refresh_text_rewrites_only_worktrees_with_its_skills() {
        let tmp = TempDir::new().unwrap();
        let (ours, theirs) = (tmp.path().join("a"), tmp.path().join("b"));
        let stale = ours.join(".agents/skills/oxplow-extension");
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("SKILL.md"), "stale").unwrap();
        fs::write(stale.join(".oxplow"), "").unwrap();
        let mine = theirs.join(".agents/skills/mine");
        fs::create_dir_all(&mine).unwrap();
        fs::write(mine.join("SKILL.md"), "x").unwrap();
        let h = harness("oxplow:codex-cli", "codex");
        let text = oxplow_agent_text::core_text();
        let workspaces = [ours.clone(), theirs.clone()];
        crate::test_launch::block(h.refresh_text(
            &RuntimeRoots {
                project_dir: tmp.path().to_path_buf(),
                workspaces: workspaces.to_vec(),
            },
            &text,
        ))
        .unwrap();
        for skill in &text.skills {
            assert_eq!(
                fs::read_to_string(
                    ours.join(".agents/skills")
                        .join(&skill.name)
                        .join("SKILL.md")
                )
                .unwrap(),
                skill.body
            );
        }
        assert!(!theirs.join(".agents/skills/oxplow-extension").exists());
        assert!(mine.join("SKILL.md").is_file());
    }

    use oxplow_domain::agent::observe::{AttrValue, Attrs};

    fn attrs(pairs: &[(&str, AttrValue)]) -> Attrs {
        Attrs(
            pairs
                .iter()
                .map(|(k, v)| ((*k).into(), v.clone()))
                .collect(),
        )
    }

    fn sums(readings: &[TokenReading]) -> [i64; 3] {
        let sum = |k| {
            readings
                .iter()
                .filter(|r| r.kind == k)
                .map(|r| r.value)
                .sum()
        };
        [
            sum(TokenKind::Input),
            sum(TokenKind::Output),
            sum(TokenKind::CacheRead),
        ]
    }

    /// Its real token source: a `response.completed` log record. Input is
    /// the full context, so new input = input − cached; reasoning folds
    /// into output; the cached prefix is a CacheRead count. Counts may be
    /// ints or numeric strings.
    #[test]
    fn a_response_completed_log_maps_new_input_and_folded_output() {
        let h = harness("oxplow:codex-cli", "codex");
        let a = attrs(&[
            ("event.kind", AttrValue::Str("response.completed".into())),
            ("input_token_count", AttrValue::Int(113690)),
            ("cached_token_count", AttrValue::Int(2432)),
            ("output_token_count", AttrValue::Str("254".into())),
            ("reasoning_token_count", AttrValue::Str("42".into())),
            ("model", AttrValue::Str("gpt-5.5".into())),
        ]);
        let none = Attrs::default();
        let other = attrs(&[("event.name", AttrValue::Str("codex.api_request".into()))]);
        // One export's records, read in one call: the other event reads
        // as nothing.
        let r = crate::test_launch::block(h.token_readings(&[
            OtlpRecord::Log {
                attributes: other.clone(),
                resource: none.clone(),
                time_unix_nano: 0,
            },
            OtlpRecord::Log {
                attributes: a.clone(),
                resource: none.clone(),
                time_unix_nano: 7,
            },
        ]));
        assert_eq!(sums(&r), [111258, 296, 2432]);
        assert!(r
            .iter()
            .all(|r| r.model == "gpt-5.5" && r.at_unix_nano == 7));
    }

    /// The body is untrusted: counts at the ends of i64 saturate, and a
    /// negative count is no count.
    #[test]
    fn hostile_counts_saturate_and_negative_ones_are_dropped() {
        let none = Attrs::default();
        let log = |input: i64, cached: i64, output: i64, reasoning: i64| {
            attrs(&[
                ("event.kind", AttrValue::Str("response.completed".into())),
                ("input_token_count", AttrValue::Int(input)),
                ("cached_token_count", AttrValue::Int(cached)),
                ("output_token_count", AttrValue::Int(output)),
                ("reasoning_token_count", AttrValue::Int(reasoning)),
            ])
        };
        let read = |a: &Attrs| -> Vec<(TokenKind, i64)> {
            token_reading(&OtlpRecord::Log {
                attributes: a.clone(),
                resource: none.clone(),
                time_unix_nano: 0,
            })
            .into_iter()
            .map(|r| (r.kind, r.value))
            .collect()
        };
        assert_eq!(
            read(&log(i64::MIN, i64::MAX, i64::MAX, i64::MAX)),
            vec![
                (TokenKind::CacheRead, i64::MAX),
                (TokenKind::Output, i64::MAX)
            ]
        );
        assert!(read(&log(-5, -5, -5, -5)).is_empty());
    }

    /// The per-turn histogram (should a Codex emit it): reasoning folds
    /// into output, cached input is CacheRead, the `total` rollup is
    /// dropped.
    #[test]
    fn the_token_histogram_folds_reasoning_and_drops_the_total() {
        let none = Attrs::default();
        let readings: Vec<TokenReading> = [
            ("input", 100),
            ("output", 20),
            ("reasoning_output", 30),
            ("cached_input", 5000),
            ("total", 5150),
        ]
        .iter()
        .flat_map(|(kind, value)| {
            let a = attrs(&[
                ("token_type", AttrValue::Str((*kind).into())),
                ("model", AttrValue::Str("gpt-5-codex".into())),
            ]);
            token_reading(&OtlpRecord::Point {
                metric: TOKEN_METRIC.into(),
                value: *value,
                attributes: a.clone(),
                resource: none.clone(),
                time_unix_nano: 0,
                start_time_unix_nano: 0,
            })
        })
        .collect();
        assert_eq!(sums(&readings), [100, 50, 5000]);
    }
}
