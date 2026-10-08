//! The `opencode` harness: opencode in a terminal. Its hooks, MCP server,
//! slash commands and the system prompt (an instructions file per
//! session) all ride the `OPENCODE_CONFIG_CONTENT` env var; skills land in
//! `.opencode/skills/`, the one place it discovers them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use oxplow_domain::agent::harness::{
    AgentHarness, Gate, HarnessError, Input, Interact, Launch, LaunchInput, LaunchSpec, Transcript,
};
use oxplow_domain::agent::observe::{HookAnswer, OtlpRecord, TokenReading, Turn};
use oxplow_domain::agent::text::AgentText;

use super::shared::{env_prefix, in_shell, program_and_guard, runtime, shell_escape, write_skills};
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
        let paths = write_runtime(input.project_dir, input.text).map_err(runtime)?;
        // No --append-system-prompt: the prompt is an instructions file,
        // one per session (two sessions of a thread run apart).
        let mut instructions = Vec::new();
        if let Some(prompt) = input.system_prompt.filter(|p| !p.is_empty()) {
            let path = paths
                .prompts_dir
                .join(format!("{}.md", input.session.session));
            fs::write(&path, prompt).map_err(runtime)?;
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

    /// Its skills are on disk; its commands ride each launch's config.
    fn refresh_text(&self, project_dir: &Path, text: &AgentText) -> Result<(), HarnessError> {
        if project_dir.join(RUNTIME_DIR_REL).is_dir() {
            write_opencode_skills(project_dir, text).map_err(runtime)?;
        }
        Ok(())
    }

    /// As its hook bridge names them (`opencode-hooks.js` maps `patch` to
    /// `Edit`).
    fn writing_tools(&self) -> &[&str] {
        &["write", "edit", "bash", "task"]
    }

    /// Its session format isn't read yet.
    fn turns(&self, _: &str) -> Vec<Turn> {
        Vec::new()
    }

    /// It exports no token telemetry.
    fn token_readings(&self, _: &OtlpRecord<'_>) -> Vec<TokenReading> {
        Vec::new()
    }

    fn render(&self, answer: &HookAnswer) -> serde_json::Value {
        super::shared::render(answer)
    }
}

const RUNTIME_DIR_REL: &str = ".oxplow/runtime/opencode-plugin";

/// What [`write_runtime`] wrote. opencode needs no on-disk hooks or MCP
/// config — both ride `OPENCODE_CONFIG_CONTENT` — so the runtime dir
/// carries the JS hook-bridge plugin and a `prompts/` dir for the
/// sessions' instruction files.
struct RuntimePaths {
    hooks_plugin: PathBuf,
    prompts_dir: PathBuf,
}

/// Materialize the hook-bridge plugin and the skills. Idempotent;
/// per-session identity rides env vars (the JS reads `OXPLOW_*` from its
/// process env), so one dir serves every session.
fn write_runtime(project_dir: &Path, text: &AgentText) -> io::Result<RuntimePaths> {
    let runtime_dir = project_dir.join(RUNTIME_DIR_REL);
    let plugin_dir = runtime_dir.join("plugin");
    let prompts_dir = runtime_dir.join("prompts");
    fs::create_dir_all(&plugin_dir)?;
    fs::create_dir_all(&prompts_dir)?;
    let hooks_plugin = plugin_dir.join("oxplow-hooks.js");
    fs::write(&hooks_plugin, include_str!("../assets/opencode-hooks.js"))?;
    write_opencode_skills(project_dir, text)?;
    Ok(RuntimePaths {
        hooks_plugin,
        prompts_dir,
    })
}

/// opencode discovers skills only from fixed project locations
/// (`.opencode/skills/<name>/SKILL.md` et al — no config key points at the
/// runtime dir), so they land in `<project>/.opencode/skills/`, each dir
/// with a `*` `.gitignore` so they never reach the person's commits. The
/// skills' frontmatter (name matching the dir, description) is already
/// opencode-compatible.
fn write_opencode_skills(project_dir: &Path, text: &AgentText) -> io::Result<()> {
    let skills_dir = project_dir.join(".opencode").join("skills");
    write_skills(&skills_dir, &text.skills)?;
    for skill in &text.skills {
        fs::write(skills_dir.join(&skill.name).join(".gitignore"), "*\n")?;
    }
    Ok(())
}

/// Slash-command definitions for opencode's inline `command` config key
/// (carried per launch, so nothing lands on disk): the frontmatter
/// `description` becomes the TUI description and the body the prompt
/// template. Names are prefixed `oxplow-`, since opencode has no plugin
/// namespacing (`/oxplow-configure` vs Claude's `/oxplow:configure`).
fn command_definitions(text: &AgentText) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for command in &text.commands {
        let (description, template) = split_frontmatter_description(&command.body);
        let mut def = serde_json::Map::new();
        def.insert("template".into(), template.into());
        if let Some(description) = description {
            def.insert("description".into(), description.into());
        }
        map.insert(
            format!("oxplow-{}", command.name),
            serde_json::Value::Object(def),
        );
    }
    serde_json::Value::Object(map)
}

/// A command's frontmatter `description:` (if any) and the markdown body
/// after the closing `---`. One without frontmatter comes back whole as
/// the template.
fn split_frontmatter_description(asset: &str) -> (Option<String>, String) {
    let Some(rest) = asset.strip_prefix("---\n") else {
        return (None, asset.trim().to_string());
    };
    let Some((front, body)) = rest.split_once("\n---\n") else {
        return (None, asset.trim().to_string());
    };
    let description = front
        .lines()
        .find_map(|l| l.strip_prefix("description:"))
        .map(|d| d.trim().to_string());
    (description, body.trim().to_string())
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
        "command": command_definitions(text),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_launch::{harness, launch_in};
    use tempfile::TempDir;

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
            &oxplow_agent_text::core_text(),
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

    #[test]
    fn write_runtime_emits_the_hook_bridge_plugin() {
        let tmp = TempDir::new().unwrap();
        let paths = write_runtime(tmp.path(), &oxplow_agent_text::core_text()).unwrap();
        assert!(paths.prompts_dir.is_dir());
        let js = fs::read_to_string(&paths.hooks_plugin).unwrap();
        // The bridge reads its routing identity from env, posts the
        // Claude-shaped lifecycle events, and maps tool names so the
        // write guard matches.
        assert!(js.contains("OXPLOW_HOOK_BASE_URL"));
        assert!(js.contains("OXPLOW_HOOK_TOKEN"));
        assert!(js.contains("X-Oxplow-Thread"));
        for event in ["UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"] {
            assert!(js.contains(event), "missing {event}");
        }
        assert!(js.contains("permissionDecision"));
        assert!(js.contains("session.idle"));
        assert!(js.contains("file_path"));
        // Idempotent.
        write_runtime(tmp.path(), &oxplow_agent_text::core_text()).unwrap();
    }

    #[test]
    fn write_runtime_materializes_skills_with_gitignore() {
        let tmp = TempDir::new().unwrap();
        let text = oxplow_agent_text::core_text();
        write_runtime(tmp.path(), &text).unwrap();
        let skills_dir = tmp.path().join(".opencode/skills");
        for skill in &text.skills {
            let name = &skill.name;
            let body = fs::read_to_string(skills_dir.join(name).join("SKILL.md"))
                .unwrap_or_else(|_| panic!("missing {name}"));
            // opencode keys discovery on frontmatter name == dir name.
            assert!(
                body.contains(&format!("name: {name}")),
                "frontmatter name must match dir for {name}"
            );
            // Generated dirs self-ignore so they never land in commits.
            let ignore = fs::read_to_string(skills_dir.join(name).join(".gitignore")).unwrap();
            assert_eq!(ignore.trim(), "*");
        }
    }

    /// A refresh rewrites a runtime already on disk and creates none.
    #[test]
    fn refresh_text_rewrites_only_runtimes_already_on_disk() {
        let tmp = TempDir::new().unwrap();
        let h = harness("oxplow:opencode", "opencode");
        let text = oxplow_agent_text::core_text();
        h.refresh_text(tmp.path(), &text).unwrap();
        assert!(!tmp.path().join(".opencode").exists());
        fs::create_dir_all(tmp.path().join(RUNTIME_DIR_REL)).unwrap();
        h.refresh_text(tmp.path(), &text).unwrap();
        for skill in &text.skills {
            assert_eq!(
                fs::read_to_string(
                    tmp.path()
                        .join(".opencode/skills")
                        .join(&skill.name)
                        .join("SKILL.md")
                )
                .unwrap(),
                skill.body
            );
        }
    }

    #[test]
    fn command_definitions_carry_description_and_template() {
        let defs = command_definitions(&oxplow_agent_text::core_text());
        for name in [
            "oxplow-review-comments",
            "oxplow-configure",
            "oxplow-new-metric",
        ] {
            let def = &defs[name];
            let template = def["template"].as_str().unwrap_or_default();
            assert!(!template.is_empty(), "{name} template empty");
            assert!(
                !template.starts_with("---"),
                "{name} template must not retain frontmatter"
            );
            assert!(
                !def["description"].as_str().unwrap_or_default().is_empty(),
                "{name} description missing"
            );
        }
    }
}
