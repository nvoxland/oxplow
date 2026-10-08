//! The `opencode` harness: opencode in a terminal. Its hooks, MCP server,
//! slash commands and the system prompt (an instructions file per
//! session) all ride the `OPENCODE_CONFIG_CONTENT` env var; skills land in
//! `.opencode/skills/`, the one place it discovers them.

use std::path::Path;

use oxplow_domain::agent::harness::{
    AgentHarness, Gate, HarnessError, Input, Interact, Launch, LaunchInput, LaunchSpec, Transcript,
};
use oxplow_domain::agent::text::AgentText;

use super::shared::{env_prefix, in_shell, program_and_guard, shell_escape};
use super::Named;

pub(super) struct Opencode(pub(super) Named);

/// The model opencode launches with (`-m provider/model`) when its config
/// names none (`{ model }`). Assumes GitHub Copilot auth in opencode's own
/// auth store.
pub const DEFAULT_MODEL: &str = "github-copilot/gpt-5-mini";

impl AgentHarness for Opencode {
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
        let runtime = |e: String| HarnessError::Runtime(e);
        let paths = oxplow_plugin::write_opencode_runtime(input.project_dir, input.text)
            .map_err(|e| runtime(e.to_string()))?;
        // No --append-system-prompt: the prompt is an instructions file,
        // one per session (two sessions of a thread run apart).
        let mut instructions = Vec::new();
        if let Some(prompt) = input.system_prompt.filter(|p| !p.is_empty()) {
            let path = paths
                .prompts_dir
                .join(format!("{}.md", input.session.session));
            std::fs::write(&path, prompt).map_err(|e| runtime(e.to_string()))?;
            instructions.push(path.to_string_lossy().into_owned());
        }
        let mut env = input.identity_env.to_vec();
        env.push((
            "OPENCODE_CONFIG_CONTENT".into(),
            config_content(
                &input.endpoints.mcp_endpoint_url,
                &paths.hooks_plugin,
                &instructions,
                input.text,
            ),
        ));
        let model = input
            .config
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(DEFAULT_MODEL);
        let program = (input.resolve_program)("opencode");
        Ok(Launch {
            spec: LaunchSpec::Pty {
                command: command(
                    &input.workspace.to_string_lossy(),
                    input.resume.filter(|r| !r.is_empty()),
                    program.as_deref(),
                    &env,
                    model,
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

/// `opencode -m <model>`, resuming `resume` with a fallback to a fresh
/// session when the id is stale.
fn command(
    cwd: &str,
    resume: Option<&str>,
    program: Option<&str>,
    env: &[(String, String)],
    model: &str,
) -> String {
    let prefix = env_prefix(env);
    let (prog, guard) = program_and_guard(program, "opencode");
    let base = format!("{prog} -m {}", shell_escape(model));
    let fresh = format!("{prefix}exec {base}");
    let command = match resume {
        None => fresh,
        Some(id) => format!(
            "{prefix}{base} -s {} || {{ echo '[oxplow] saved resume id was stale; starting a fresh opencode session' >&2; {fresh}; }}",
            shell_escape(id)
        ),
    };
    in_shell(cwd, &guard, &command)
}

/// The inline opencode config carried per spawn (merged on top of the
/// person's own): the oxplow MCP server (the bearer interpolated by
/// opencode itself from `{env:…}`), the hook-bridge plugin, the session's
/// instructions file, and oxplow's slash commands.
fn config_content(
    mcp_endpoint_url: &str,
    hooks_plugin: &Path,
    instructions: &[String],
    text: &AgentText,
) -> String {
    serde_json::json!({
        "mcp": {
            "oxplow": {
                "type": "remote",
                "url": mcp_endpoint_url,
                "enabled": true,
                "headers": {
                    "Authorization": "Bearer {env:OXPLOW_HOOK_TOKEN}",
                    "X-Oxplow-Thread": "{env:OXPLOW_THREAD_ID}",
                    "X-Oxplow-Stream": "{env:OXPLOW_STREAM_ID}",
                    "X-Oxplow-Session": "{env:OXPLOW_SESSION}",
                },
            },
        },
        "plugin": [hooks_plugin.to_string_lossy()],
        "instructions": instructions,
        "command": oxplow_plugin::opencode_command_definitions(text),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harnesses::test_launch::{harness, launch_in};

    #[test]
    fn launch_builds_the_command_and_env() {
        let h = harness("oxplow:opencode", "opencode");
        let l = launch_in(
            h.as_ref(),
            Some("sess-w"),
            Some("be terse"),
            &serde_json::json!({ "model": "anthropic/claude-sonnet-4-6" }),
        );
        let LaunchSpec::Pty { command } = &l.launch.spec else {
            panic!("not a PTY launch")
        };
        for want in [
            "OXPLOW_SESSION=",
            "OPENCODE_CONFIG_CONTENT=",
            "anthropic/claude-sonnet-4-6",
            " -s ",
            "sess-w",
            "/opt/agents/opencode",
        ] {
            assert!(command.contains(want), "{want}: {command}");
        }
        assert!(l
            .project
            .join(".oxplow/runtime/opencode-plugin/prompts/ses3.md")
            .is_file());
    }

    #[test]
    fn with_no_model_configured_it_runs_the_default() {
        let cmd = command("/repo", None, None, &[], DEFAULT_MODEL);
        assert!(cmd.contains("exec opencode") && cmd.contains(DEFAULT_MODEL));
        assert!(!cmd.contains(" -s "));
    }

    #[test]
    fn the_config_content_wires_mcp_plugin_and_instructions() {
        let content = config_content(
            "http://127.0.0.1:9/mcp",
            Path::new("/proj/.oxplow/runtime/opencode-plugin/plugin/oxplow-hooks.js"),
            &["/proj/.oxplow/runtime/opencode-plugin/prompts/ses3.md".to_string()],
            &oxplow_plugin::core_text(),
        );
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["mcp"]["oxplow"]["type"], "remote");
        assert_eq!(v["mcp"]["oxplow"]["url"], "http://127.0.0.1:9/mcp");
        // opencode interpolates {env:VAR} itself: the token isn't baked in.
        assert_eq!(
            v["mcp"]["oxplow"]["headers"]["Authorization"],
            "Bearer {env:OXPLOW_HOOK_TOKEN}"
        );
        assert_eq!(
            v["mcp"]["oxplow"]["headers"]["X-Oxplow-Session"],
            "{env:OXPLOW_SESSION}"
        );
        assert_eq!(
            v["plugin"][0],
            "/proj/.oxplow/runtime/opencode-plugin/plugin/oxplow-hooks.js"
        );
        assert!(v["command"]["oxplow-review-comments"]["template"]
            .as_str()
            .is_some());
        assert_eq!(
            v["instructions"][0],
            "/proj/.oxplow/runtime/opencode-plugin/prompts/ses3.md"
        );
    }
}
