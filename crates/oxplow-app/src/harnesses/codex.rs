//! The `codex` harness: Codex in a terminal. Its hooks are commands
//! (`oxplow hook <event>`) and its MCP server, hooks and OTEL exporter ride
//! `--config` overrides; the MCP identity rides the URL's query string.

use std::path::Path;

use oxplow_domain::agent::harness::{
    AgentHarness, Gate, HarnessError, Input, Interact, Launch, LaunchInput, LaunchSpec, Transcript,
};

use super::shared::{env_prefix, in_shell, program_and_guard, shell_escape, toml_string};
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
        oxplow_plugin::write_codex_runtime(input.project_dir, &ep.mcp_endpoint_url, input.text)
            .map_err(|e| HarnessError::Runtime(e.to_string()))?;
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
    for event in [
        "PreToolUse",
        "PermissionRequest",
        "PostToolUse",
        "UserPromptSubmit",
        "SessionStart",
        "Stop",
    ] {
        let command = hook_command(oxplow_executable, event);
        let matcher = if matches!(event, "PreToolUse" | "PermissionRequest" | "PostToolUse") {
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
    use crate::harnesses::test_launch::{harness, launch_in};

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
}
