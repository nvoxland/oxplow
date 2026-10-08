//! The `codex` harness: Codex in a terminal. Its hooks are commands
//! (`oxplow hook <event>`) and its MCP server, hooks and OTEL exporter ride
//! `--config` overrides; the MCP identity rides the URL's query string.

use std::fs;
use std::io;
use std::path::Path;

use serde_json::json;

use oxplow_domain::agent::harness::{
    AgentHarness, Gate, HarnessError, HarnessSetting, Input, Interact, Launch, LaunchInput,
    LaunchSpec, Transcript,
};
use oxplow_domain::agent::observe::{HookAnswer, OtlpRecord, TokenReading, Turn};
use oxplow_domain::agent::text::AgentText;
use oxplow_domain::agent::tool::{ToolKind, ToolUse};
use oxplow_domain::events::schema::TokenKind;

use super::shared::{
    in_shell, program_and_guard, runtime, shell_escape, toml_string, write_json, write_skills,
};
use super::Named;

pub(super) struct Codex(pub(super) Named);

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
            input: Input::Keystrokes,
            gate: Gate::Harness,
        }
    }

    fn launch(&self, input: &LaunchInput<'_>) -> Result<Launch, HarnessError> {
        let ep = input.endpoints;
        // Its plugin manifest, hooks file and skills, on disk; the hooks and
        // MCP server it runs with ride the overrides below.
        write_runtime(
            input.project_dir,
            &ep.mcp_endpoint_url,
            input.oxplow_executable,
            input.text,
        )
        .map_err(runtime)?;
        let mut overrides = config_overrides(input.oxplow_executable, &ep.mcp_endpoint_url);
        // Codex has no OTEL env vars: its exporter rides `--config otel.*`,
        // to the same receiver as Claude's, with the session's bearer.
        overrides.extend(otel_overrides(&ep.otlp_base_url, &ep.hook_token));
        let program = (input.resolve_program)("codex");
        Ok(Launch {
            spec: LaunchSpec::Pty {
                command: command(
                    &input.workspace.to_string_lossy(),
                    input.resume.filter(|r| !r.is_empty()),
                    program.as_deref(),
                    &overrides,
                ),
                env: input.identity_env.to_vec(),
            },
            resume_dropped: false,
        })
    }

    fn instruction_files(&self) -> &[&str] {
        &["CLAUDE.md"]
    }

    fn env_markers(&self) -> &[&str] {
        &[]
    }

    fn settings(&self) -> &[HarnessSetting] {
        &[]
    }

    fn refresh_text(&self, project_dir: &Path, text: &AgentText) -> Result<(), HarnessError> {
        let skills_dir = project_dir.join(RUNTIME_DIR_REL).join("skills");
        if skills_dir.is_dir() {
            write_skills(&skills_dir, &text.skills).map_err(runtime)?;
        }
        Ok(())
    }

    fn tool_use(&self, body: &serde_json::Value) -> Option<ToolUse> {
        codex_tool_use(body)
    }

    fn writing_tools(&self) -> &[&str] {
        &["apply_patch", "shell", "exec_command"]
    }

    /// Its session format isn't read yet.
    fn turns(&self, _: &str) -> Vec<Turn> {
        Vec::new()
    }

    fn token_readings(&self, record: &OtlpRecord<'_>) -> Vec<TokenReading> {
        match record {
            OtlpRecord::Point {
                metric: TOKEN_METRIC,
                value,
                attributes,
                time_unix_nano,
                start_time_unix_nano,
                ..
            } => {
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

    fn render(&self, answer: &HookAnswer) -> serde_json::Value {
        super::shared::render(answer)
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

const RUNTIME_DIR_REL: &str = ".oxplow/runtime/codex-plugin";

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

/// Materialize the Codex plugin layout (manifest, command hooks, MCP
/// config, skills) under `.oxplow/runtime/codex-plugin/`. Idempotent.
fn write_runtime(
    project_dir: &Path,
    mcp_endpoint_url: &str,
    oxplow_executable: &Path,
    text: &AgentText,
) -> io::Result<()> {
    let runtime_dir = project_dir.join(RUNTIME_DIR_REL);
    let manifest_dir = runtime_dir.join(".codex-plugin");
    let hooks_dir = runtime_dir.join("hooks");
    fs::create_dir_all(&manifest_dir)?;
    fs::create_dir_all(&hooks_dir)?;
    write_json(
        &manifest_dir.join("plugin.json"),
        &json!({
            "name": "oxplow",
            "version": "0.0.0",
            "description": "Forwards Codex lifecycle hooks into the oxplow runtime.",
            "skills": "./skills/"
        }),
    )?;
    write_json(
        &hooks_dir.join("hooks.json"),
        &hooks_json(oxplow_executable),
    )?;
    fs::write(
        runtime_dir.join("mcp-config.toml"),
        format!(
            "[mcp_servers.oxplow]\nurl = {}\nbearer_token_env_var = \"OXPLOW_HOOK_TOKEN\"\n\n",
            toml_string(mcp_endpoint_url)
        ),
    )?;
    write_skills(&runtime_dir.join("skills"), &text.skills)
}

fn hooks_json(oxplow_executable: &Path) -> serde_json::Value {
    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        let entry = json!({
            "type": "command",
            "command": hook_command(oxplow_executable, event),
            "timeout": 30,
            "statusMessage": "Syncing oxplow runtime",
        });
        let outer = if per_tool(event) {
            json!([{ "matcher": "*", "hooks": [entry] }])
        } else {
            json!([{ "hooks": [entry] }])
        };
        hooks.insert(event.to_string(), outer);
    }
    json!({ "hooks": serde_json::Value::Object(hooks) })
}

/// `codex --cd <cwd>` with its overrides, or `codex resume` for `resume`.
fn command(cwd: &str, resume: Option<&str>, program: Option<&str>, overrides: &[String]) -> String {
    let (prog, guard) = program_and_guard(program, "codex");
    let config_args: String = overrides
        .iter()
        .map(|c| format!(" --config {}", shell_escape(c)))
        .collect();
    let base = match resume {
        None => format!("{prog} --cd {}{config_args}", shell_escape(cwd)),
        Some(id) => format!(
            "{prog} resume --cd {}{config_args} {}",
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
/// receiver: protobuf ("binary"), the FULL signal URL (Codex uses it as
/// is), and the session's bearer. Codex reads its exporter's headers only
/// from config, so this bearer is on its command line — the one place a
/// harness's is (`.context/agent-model.md` "Caller identity").
fn otel_overrides(otlp_base_url: &str, hook_token: &str) -> Vec<String> {
    let endpoint = format!("{otlp_base_url}/v1/metrics");
    vec![
        format!(
            "otel.exporter.otlp-http.endpoint={}",
            toml_string(&endpoint)
        ),
        "otel.exporter.otlp-http.protocol=\"binary\"".to_string(),
        format!(
            "otel.exporter.otlp-http.headers.authorization={}",
            toml_string(&format!("Bearer {hook_token}"))
        ),
    ]
}

/// A Codex tool hook's body mapped onto oxplow's vocabulary. Codex posts
/// its own tool names in Claude Code's hook fields: `apply_patch` edits the
/// files its patch names, `shell` / `exec_command` run a command (a string
/// or an argv list).
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
        "apply_patch" => (ToolKind::Edit, patch_paths(&input), None),
        "shell" | "exec_command" | "local_shell" => (ToolKind::Shell, Vec::new(), command()),
        n if n.starts_with("mcp__") => (ToolKind::Mcp, Vec::new(), None),
        _ => (ToolKind::Other, Vec::new(), None),
    };
    let response = body.get("tool_response").filter(|r| !r.is_null());
    let exit_code = response.and_then(|r| {
        ["exit_code", "exitCode", "code"]
            .iter()
            .find_map(|k| r.get(*k).and_then(|x| x.as_i64()))
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
    })
}

/// The files a Codex patch names (`*** Add File: a`, `*** Update File: b`,
/// `*** Delete File: c`, `*** Move to: d`), from its `input` / `patch`
/// text or an explicit `path`.
fn patch_paths(input: &serde_json::Value) -> Vec<String> {
    let text = ["input", "patch"]
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
    use super::*;
    use crate::test_launch::{harness, launch_in};
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
        // No identity rides the command; the bearer only in the one
        // override Codex reads from config alone (its OTLP exporter's).
        assert!(!command.contains("OXPLOW_SESSION=") && !command.contains("OXPLOW_HOOK_TOKEN="));
        assert!(!command.contains("thread=") && !command.contains("x-oxplow"));
        assert_eq!(command.matches("secret-bearer").count(), 1, "{command}");
        assert!(!command.contains("--dangerously-bypass-hook-trust"));
    }

    #[test]
    fn the_otel_overrides_configure_the_http_exporter() {
        let ov = otel_overrides("http://127.0.0.1:9", "tok123");
        assert!(ov.contains(
            &"otel.exporter.otlp-http.endpoint=\"http://127.0.0.1:9/v1/metrics\"".to_string()
        ));
        assert!(ov.contains(&"otel.exporter.otlp-http.protocol=\"binary\"".to_string()));
        assert!(ov.contains(
            &"otel.exporter.otlp-http.headers.authorization=\"Bearer tok123\"".to_string()
        ));
        assert!(!ov.iter().any(|o| o.contains("x-oxplow")));
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
        let cmd = command("/repo", None, None, &[]);
        assert!(cmd.starts_with("sh -lc "));
        assert!(cmd.contains("exec codex") && cmd.contains("--cd") && cmd.contains("/repo"));
        assert!(!cmd.contains(" resume "));
    }

    #[test]
    fn write_runtime_emits_expected_files() {
        let tmp = TempDir::new().unwrap();
        let text = oxplow_agent_text::core_text();
        write_runtime(tmp.path(), "http://h/mcp", Path::new("/bin/oxplow"), &text).unwrap();
        let dir = tmp.path().join(RUNTIME_DIR_REL);
        assert!(dir.join(".codex-plugin/plugin.json").exists());
        for skill in &text.skills {
            assert!(
                dir.join("skills")
                    .join(&skill.name)
                    .join("SKILL.md")
                    .exists(),
                "missing skill {}",
                skill.name
            );
        }
        let hooks = fs::read_to_string(dir.join("hooks/hooks.json")).unwrap();
        assert!(hooks.contains("PreToolUse"));
        assert!(!hooks.contains("http://"));
        assert!(hooks.contains("'/bin/oxplow' hook "));
        let mcp = fs::read_to_string(dir.join("mcp-config.toml")).unwrap();
        assert!(mcp.contains("[mcp_servers.oxplow]"));
        assert!(mcp.contains("url = \"http://h/mcp\""));
        assert!(mcp.contains("OXPLOW_HOOK_TOKEN"));
    }

    /// A refresh rewrites a runtime already on disk and creates none.
    #[test]
    fn refresh_text_rewrites_only_runtimes_already_on_disk() {
        let tmp = TempDir::new().unwrap();
        let h = harness("oxplow:codex-cli", "codex");
        let text = oxplow_agent_text::core_text();
        h.refresh_text(tmp.path(), &text).unwrap();
        assert!(!tmp.path().join(RUNTIME_DIR_REL).exists());
        let skills = tmp.path().join(RUNTIME_DIR_REL).join("skills");
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
        let r = h.token_readings(&OtlpRecord::Log {
            attributes: &a,
            resource: &none,
            time_unix_nano: 7,
        });
        assert_eq!(sums(&r), [111258, 296, 2432]);
        assert!(r
            .iter()
            .all(|r| r.model == "gpt-5.5" && r.at_unix_nano == 7));
        let other = attrs(&[("event.name", AttrValue::Str("codex.api_request".into()))]);
        assert!(h
            .token_readings(&OtlpRecord::Log {
                attributes: &other,
                resource: &none,
                time_unix_nano: 0,
            })
            .is_empty());
    }

    /// The body is untrusted: counts at the ends of i64 saturate, and a
    /// negative count is no count.
    #[test]
    fn hostile_counts_saturate_and_negative_ones_are_dropped() {
        let h = harness("oxplow:codex-cli", "codex");
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
            h.token_readings(&OtlpRecord::Log {
                attributes: a,
                resource: &none,
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
        let h = harness("oxplow:codex-cli", "codex");
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
            h.token_readings(&OtlpRecord::Point {
                metric: TOKEN_METRIC,
                value: *value,
                attributes: &a,
                resource: &none,
                time_unix_nano: 0,
                start_time_unix_nano: 0,
            })
        })
        .collect();
        assert_eq!(sums(&readings), [100, 50, 5000]);
    }
}
