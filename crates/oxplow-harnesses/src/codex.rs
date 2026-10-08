//! The `codex` harness: Codex in a terminal. Its hooks are commands
//! (`oxplow hook <event>`) and its MCP server, hooks and OTEL exporter ride
//! `--config` overrides; the MCP identity rides the URL's query string.

use std::fs;
use std::io;
use std::path::Path;

use serde_json::json;

use oxplow_domain::agent::harness::{
    AgentHarness, Gate, HarnessError, Input, Interact, Launch, LaunchInput, LaunchSpec, Transcript,
};
use oxplow_domain::agent::observe::{HookAnswer, OtlpRecord, TokenReading, Turn};
use oxplow_domain::agent::text::AgentText;
use oxplow_domain::events::schema::TokenKind;

use super::shared::{
    env_prefix, in_shell, program_and_guard, runtime, shell_escape, toml_string, write_json,
    write_skills,
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
        let ids = input.session;
        let (thread, stream, session) = (
            ids.thread.to_string(),
            ids.stream.to_string(),
            ids.session.to_string(),
        );
        let mut overrides = config_overrides(
            input.oxplow_executable,
            &ep.mcp_endpoint_url,
            &thread,
            &stream,
        );
        // Codex has no OTEL env vars: its exporter rides `--config otel.*`,
        // to the same receiver as Claude's, with the same attribution.
        overrides.extend(otel_overrides(
            &ep.otlp_base_url,
            &ep.hook_token,
            &thread,
            &session,
        ));
        let program = (input.resolve_program)("codex");
        Ok(Launch {
            spec: LaunchSpec::Pty {
                command: command(
                    &input.workspace.to_string_lossy(),
                    input.resume.filter(|r| !r.is_empty()),
                    program.as_deref(),
                    input.identity_env,
                    &overrides,
                ),
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

    fn refresh_text(&self, project_dir: &Path, text: &AgentText) -> Result<(), HarnessError> {
        let skills_dir = project_dir.join(RUNTIME_DIR_REL).join("skills");
        if skills_dir.is_dir() {
            write_skills(&skills_dir, &text.skills).map_err(runtime)?;
        }
        Ok(())
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
fn command(
    cwd: &str,
    resume: Option<&str>,
    program: Option<&str>,
    env: &[(String, String)],
    overrides: &[String],
) -> String {
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
    in_shell(
        cwd,
        &format!("{guard}{}", env_prefix(env)),
        &format!("exec {base}"),
    )
}

/// The MCP endpoint with the session's identity in the query string: Codex's
/// MCP config has no per-session headers. The control plane reads
/// `?thread=…&stream=…` like the `X-Oxplow-*` headers.
fn mcp_url_with_identity(mcp_endpoint_url: &str, thread: &str, stream: &str) -> String {
    let sep = if mcp_endpoint_url.contains('?') {
        '&'
    } else {
        '?'
    };
    format!("{mcp_endpoint_url}{sep}thread={thread}&stream={stream}")
}

/// Codex's MCP server and command hooks, as `--config` overrides.
fn config_overrides(
    oxplow_executable: &Path,
    mcp_endpoint_url: &str,
    thread: &str,
    stream: &str,
) -> Vec<String> {
    let mut out = vec![
        format!(
            "mcp_servers.oxplow.url={}",
            toml_string(&mcp_url_with_identity(mcp_endpoint_url, thread, stream))
        ),
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
/// is), and the bearer and attribution headers.
fn otel_overrides(
    otlp_base_url: &str,
    hook_token: &str,
    thread: &str,
    session: &str,
) -> Vec<String> {
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
        format!(
            "otel.exporter.otlp-http.headers.x-oxplow-thread={}",
            toml_string(thread)
        ),
        format!(
            "otel.exporter.otlp-http.headers.x-oxplow-session={}",
            toml_string(session)
        ),
    ]
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

    #[test]
    fn launch_builds_the_command_and_env() {
        let h = harness("oxplow:codex-cli", "codex");
        let l = launch_in(h.as_ref(), Some("sess-w"), None, &serde_json::json!({}));
        let LaunchSpec::Pty { command } = &l.launch.spec else {
            panic!("not a PTY launch")
        };
        for want in [
            "OXPLOW_SESSION=",
            "/opt/agents/codex",
            " resume --cd ",
            "sess-w",
            "--config",
            "thread=thr2&stream=str1",
            "x-oxplow-session",
            "/bin/oxplow",
        ] {
            assert!(command.contains(want), "{want}: {command}");
        }
        assert!(!command.contains("--dangerously-bypass-hook-trust"));
    }

    #[test]
    fn the_otel_overrides_configure_the_http_exporter() {
        let ov = otel_overrides("http://127.0.0.1:9", "tok123", "thr1", "ses3");
        assert!(ov.contains(
            &"otel.exporter.otlp-http.endpoint=\"http://127.0.0.1:9/v1/metrics\"".to_string()
        ));
        assert!(ov.contains(&"otel.exporter.otlp-http.protocol=\"binary\"".to_string()));
        assert!(ov.contains(
            &"otel.exporter.otlp-http.headers.authorization=\"Bearer tok123\"".to_string()
        ));
        assert!(
            ov.contains(&"otel.exporter.otlp-http.headers.x-oxplow-thread=\"thr1\"".to_string())
        );
        assert!(
            ov.contains(&"otel.exporter.otlp-http.headers.x-oxplow-session=\"ses3\"".to_string())
        );
        assert!(!ov.iter().any(|o| o.contains("x-oxplow-stream")));
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
        let cmd = command("/repo", None, None, &[], &[]);
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
