//! Materializes the Claude Code plugin oxplow ships at
//! `<projectDir>/.oxplow/runtime/claude-plugin/`.
//!
//! Two surfaces:
//!
//! 1. **HTTP hooks** — `hooks/hooks.json` registers PreToolUse,
//!    UserPromptSubmit, Stop, etc. as HTTP POSTs back to the
//!    in-process control plane. Auth is a per-spawn bearer token
//!    threaded through `$OXPLOW_HOOK_TOKEN`; routing context
//!    (stream/thread/pane) rides per-spawn env vars too.
//! 2. **MCP server config** — `mcp-config.json` points Claude at the
//!    same control-plane port via the streamable-HTTP MCP transport.
//!    Passed to claude as `--mcp-config <path> --strict-mcp-config`
//!    so the only MCP server in scope is oxplow's.
//!
//! Plus the static skill / guide / slash-command files the plugin
//! exposes as model-invoked context.
//!
//! `write_plugin` is idempotent — the dir is rewritten on every spawn
//! so live edits to skill content take effect without a manual
//! cleanup step. Per-(stream, thread) identity rides env-var-
//! interpolated headers, not file contents, so the same dir is
//! reusable across spawns.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;
use thiserror::Error;

use oxplow_domain::AgentKind;

const PLUGIN_DIR_REL: &str = ".oxplow/runtime/claude-plugin";
const CODEX_RUNTIME_DIR_REL: &str = ".oxplow/runtime/codex-plugin";
const PLUGIN_NAME: &str = "oxplow";
const PLUGIN_VERSION: &str = "0.0.0";

/// Hook event names mirrored from main. SessionStart is registered
/// even though Claude Code drops HTTP hooks for it ("HTTP hooks are
/// not supported for SessionStart" in its debug log) — we learn the
/// session id from whichever hook fires next instead.
pub const HOOK_EVENTS: &[&str] = &[
    "PreToolUse",
    "PostToolUse",
    "UserPromptSubmit",
    "SessionStart",
    "SessionEnd",
    "Stop",
    "Notification",
];

/// Env vars the plugin's hooks header-interpolates from. Claude Code
/// requires explicit allowlisting via `allowedEnvVars`.
pub const PLUGIN_ENV_VARS: &[&str] = &[
    "OXPLOW_HOOK_TOKEN",
    "OXPLOW_STREAM_ID",
    "OXPLOW_THREAD_ID",
    "OXPLOW_PANE",
];

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("serialize: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Paths emitted by `write_plugin`. The only required output for the
/// caller is `plugin_dir` (passed to `claude --plugin-dir`) and
/// `mcp_config` (passed to `claude --mcp-config`); the rest are
/// returned for tests and diagnostics.
#[derive(Debug, Clone)]
pub struct PluginPaths {
    pub plugin_dir: PathBuf,
    pub manifest: PathBuf,
    pub hooks: PathBuf,
    pub mcp_config: PathBuf,
    pub agent_guide: PathBuf,
    pub runtime_skill: PathBuf,
    pub subagent_skill: PathBuf,
    pub wiki_capture_skill: PathBuf,
    pub mermaid_skill: PathBuf,
    pub collection_skill: PathBuf,
    pub work_next_command: PathBuf,
    pub review_comments_command: PathBuf,
    pub configure_command: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CodexRuntimePaths {
    pub runtime_dir: PathBuf,
    pub manifest: PathBuf,
    pub hooks: PathBuf,
    pub oxplow_executable: PathBuf,
    pub mcp_config: PathBuf,
    pub runtime_skill: PathBuf,
    pub subagent_skill: PathBuf,
    pub wiki_capture_skill: PathBuf,
    pub mermaid_skill: PathBuf,
    pub collection_skill: PathBuf,
}

#[derive(Debug, Clone)]
pub enum AgentRuntimePaths {
    Claude(PluginPaths),
    Codex(CodexRuntimePaths),
}

pub fn write_agent_runtime(
    agent: AgentKind,
    project_dir: &Path,
    hook_base_url: &str,
    mcp_endpoint_url: &str,
    hook_token: &str,
) -> Result<AgentRuntimePaths, PluginError> {
    match agent {
        AgentKind::Claude => write_plugin(project_dir, hook_base_url, mcp_endpoint_url, hook_token)
            .map(AgentRuntimePaths::Claude),
        AgentKind::Codex => {
            write_codex_runtime(project_dir, mcp_endpoint_url).map(AgentRuntimePaths::Codex)
        }
    }
}

/// Materialize the plugin directory. `hook_base_url` and
/// `mcp_endpoint_url` are absolute URLs to the in-process control
/// plane (e.g. `http://127.0.0.1:51823/hook` and `…/mcp`). Re-running
/// against the same `project_dir` overwrites in place — every spawn
/// can call this safely.
pub fn write_plugin(
    project_dir: &Path,
    hook_base_url: &str,
    mcp_endpoint_url: &str,
    hook_token: &str,
) -> Result<PluginPaths, PluginError> {
    let plugin_dir = project_dir.join(PLUGIN_DIR_REL);
    let manifest_dir = plugin_dir.join(".claude-plugin");
    let hooks_dir = plugin_dir.join("hooks");
    let commands_dir = plugin_dir.join("commands");
    let skills_dir = plugin_dir.join("skills");

    fs::create_dir_all(&manifest_dir)?;
    fs::create_dir_all(&hooks_dir)?;
    fs::create_dir_all(&commands_dir)?;
    fs::create_dir_all(skills_dir.join("oxplow-runtime"))?;
    fs::create_dir_all(skills_dir.join("oxplow-subagent-work-protocol"))?;
    fs::create_dir_all(skills_dir.join("oxplow-wiki-capture"))?;
    fs::create_dir_all(skills_dir.join("oxplow-mermaid"))?;
    fs::create_dir_all(skills_dir.join("oxplow-collection"))?;

    let manifest = manifest_dir.join("plugin.json");
    let manifest_body = json!({
        "name": PLUGIN_NAME,
        "version": PLUGIN_VERSION,
        "description": "Forwards Claude Code lifecycle hooks into the oxplow runtime.",
    });
    write_json(&manifest, &manifest_body)?;

    let hooks = hooks_dir.join("hooks.json");
    let hooks_body = build_hooks_json(hook_base_url);
    write_json(&hooks, &hooks_body)?;

    let mcp_config = plugin_dir.join("mcp-config.json");
    let mcp_body = build_mcp_config(mcp_endpoint_url, hook_token);
    write_json(&mcp_config, &mcp_body)?;

    let agent_guide = plugin_dir.join("AGENT_GUIDE.md");
    fs::write(&agent_guide, include_str!("../assets/AGENT_GUIDE.md"))?;

    let runtime_skill = skills_dir.join("oxplow-runtime").join("SKILL.md");
    fs::write(
        &runtime_skill,
        include_str!("../assets/oxplow-runtime.SKILL.md"),
    )?;

    let subagent_skill = skills_dir
        .join("oxplow-subagent-work-protocol")
        .join("SKILL.md");
    fs::write(
        &subagent_skill,
        include_str!("../assets/oxplow-subagent.SKILL.md"),
    )?;

    let wiki_capture_skill = skills_dir.join("oxplow-wiki-capture").join("SKILL.md");
    fs::write(
        &wiki_capture_skill,
        include_str!("../assets/oxplow-wiki-capture.SKILL.md"),
    )?;

    let mermaid_skill = skills_dir.join("oxplow-mermaid").join("SKILL.md");
    fs::write(
        &mermaid_skill,
        include_str!("../assets/oxplow-mermaid.SKILL.md"),
    )?;

    let collection_skill = skills_dir.join("oxplow-collection").join("SKILL.md");
    fs::write(
        &collection_skill,
        include_str!("../assets/oxplow-collection.SKILL.md"),
    )?;

    let work_next_command = commands_dir.join("work-next.md");
    fs::write(&work_next_command, include_str!("../assets/work-next.md"))?;

    let review_comments_command = commands_dir.join("review-comments.md");
    fs::write(
        &review_comments_command,
        include_str!("../assets/review-comments.md"),
    )?;

    let configure_command = commands_dir.join("configure.md");
    fs::write(&configure_command, include_str!("../assets/configure.md"))?;

    Ok(PluginPaths {
        plugin_dir,
        manifest,
        hooks,
        mcp_config,
        agent_guide,
        runtime_skill,
        subagent_skill,
        wiki_capture_skill,
        mermaid_skill,
        collection_skill,
        work_next_command,
        review_comments_command,
        configure_command,
    })
}

fn build_hooks_json(hook_base_url: &str) -> serde_json::Value {
    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        let entry = json!({
            "type": "http",
            "url": format!("{}/{}", hook_base_url.trim_end_matches('/'), event),
            "timeout": 3,
            "headers": {
                "Authorization": "Bearer $OXPLOW_HOOK_TOKEN",
                "X-Oxplow-Stream": "$OXPLOW_STREAM_ID",
                "X-Oxplow-Thread": "$OXPLOW_THREAD_ID",
                "X-Oxplow-Pane": "$OXPLOW_PANE",
            },
            "allowedEnvVars": PLUGIN_ENV_VARS,
        });
        // PreToolUse / PostToolUse have a per-tool matcher; everything
        // else is unconditional. Mirrors main.
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
    // Bake the literal token into the file. Claude Code's MCP config
    // schema does not env-var-interpolate `headers` (unlike hooks,
    // which opt in via `allowedEnvVars`), so `"Bearer $VAR"` would be
    // sent verbatim and the control plane would 401. The file lives
    // under `.oxplow/runtime/claude-plugin/` (gitignored) and is
    // rewritten per `open_terminal_session`, so it tracks the current
    // boot's token.
    json!({
        "mcpServers": {
            "oxplow": {
                "type": "http",
                "url": mcp_endpoint_url,
                "headers": {
                    "Authorization": format!("Bearer {hook_token}"),
                },
            },
        },
    })
}

pub fn write_codex_runtime(
    project_dir: &Path,
    mcp_endpoint_url: &str,
) -> Result<CodexRuntimePaths, PluginError> {
    let runtime_dir = project_dir.join(CODEX_RUNTIME_DIR_REL);
    let manifest_dir = runtime_dir.join(".codex-plugin");
    let hooks_dir = runtime_dir.join("hooks");
    let skills_dir = runtime_dir.join("skills");

    fs::create_dir_all(&manifest_dir)?;
    fs::create_dir_all(&hooks_dir)?;
    fs::create_dir_all(skills_dir.join("oxplow-runtime"))?;
    fs::create_dir_all(skills_dir.join("oxplow-subagent-work-protocol"))?;
    fs::create_dir_all(skills_dir.join("oxplow-wiki-capture"))?;
    fs::create_dir_all(skills_dir.join("oxplow-mermaid"))?;
    fs::create_dir_all(skills_dir.join("oxplow-collection"))?;

    let manifest = manifest_dir.join("plugin.json");
    write_json(
        &manifest,
        &json!({
            "name": PLUGIN_NAME,
            "version": PLUGIN_VERSION,
            "description": "Forwards Codex lifecycle hooks into the oxplow runtime.",
            "skills": "./skills/"
        }),
    )?;

    let legacy_hook_bridge = runtime_dir.join("scripts/hook_bridge.py");
    if legacy_hook_bridge.exists() {
        fs::remove_file(legacy_hook_bridge)?;
    }
    let oxplow_executable = std::env::current_exe()?;

    let hooks = hooks_dir.join("hooks.json");
    write_json(&hooks, &build_codex_hooks_json(&oxplow_executable))?;

    let mcp_config = runtime_dir.join("mcp-config.toml");
    fs::write(&mcp_config, build_codex_mcp_config(mcp_endpoint_url))?;

    let runtime_skill = skills_dir.join("oxplow-runtime").join("SKILL.md");
    fs::write(
        &runtime_skill,
        include_str!("../assets/oxplow-runtime.SKILL.md"),
    )?;

    let subagent_skill = skills_dir
        .join("oxplow-subagent-work-protocol")
        .join("SKILL.md");
    fs::write(
        &subagent_skill,
        include_str!("../assets/oxplow-subagent.SKILL.md"),
    )?;

    let wiki_capture_skill = skills_dir.join("oxplow-wiki-capture").join("SKILL.md");
    fs::write(
        &wiki_capture_skill,
        include_str!("../assets/oxplow-wiki-capture.SKILL.md"),
    )?;

    let mermaid_skill = skills_dir.join("oxplow-mermaid").join("SKILL.md");
    fs::write(
        &mermaid_skill,
        include_str!("../assets/oxplow-mermaid.SKILL.md"),
    )?;

    let collection_skill = skills_dir.join("oxplow-collection").join("SKILL.md");
    fs::write(
        &collection_skill,
        include_str!("../assets/oxplow-collection.SKILL.md"),
    )?;

    Ok(CodexRuntimePaths {
        runtime_dir,
        manifest,
        hooks,
        oxplow_executable,
        mcp_config,
        runtime_skill,
        subagent_skill,
        wiki_capture_skill,
        mermaid_skill,
        collection_skill,
    })
}

fn build_codex_hooks_json(oxplow_executable: &Path) -> serde_json::Value {
    let command = |event: &str| {
        format!(
            "{} hook {}",
            shell_quote_path(oxplow_executable),
            shell_quote(event)
        )
    };
    let mut hooks = serde_json::Map::new();
    for event in [
        "PreToolUse",
        "PermissionRequest",
        "PostToolUse",
        "UserPromptSubmit",
        "SessionStart",
        "Stop",
    ] {
        let entry = json!({
            "type": "command",
            "command": command(event),
            "timeout": 30,
            "statusMessage": "Syncing oxplow runtime",
        });
        let outer = if matches!(event, "PreToolUse" | "PermissionRequest" | "PostToolUse") {
            json!([{ "matcher": "*", "hooks": [entry] }])
        } else {
            json!([{ "hooks": [entry] }])
        };
        hooks.insert(event.to_string(), outer);
    }
    json!({ "hooks": serde_json::Value::Object(hooks) })
}

fn build_codex_mcp_config(mcp_endpoint_url: &str) -> String {
    format!(
        "[mcp_servers.oxplow]\nurl = \"{}\"\nbearer_token_env_var = \"OXPLOW_HOOK_TOKEN\"\n\n",
        escape_toml_string(mcp_endpoint_url)
    )
}

fn shell_quote_path(path: &Path) -> String {
    shell_quote(&path.to_string_lossy())
}

fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', r"'\''");
    format!("'{escaped}'")
}

fn escape_toml_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), PluginError> {
    let mut s = serde_json::to_string_pretty(value)?;
    s.push('\n');
    fs::write(path, s)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn write_plugin_emits_expected_files() {
        let tmp = TempDir::new().unwrap();
        let paths = write_plugin(
            tmp.path(),
            "http://127.0.0.1:51823/hook",
            "http://127.0.0.1:51823/mcp",
            "test-token",
        )
        .unwrap();
        assert!(paths.manifest.exists());
        assert!(paths.hooks.exists());
        assert!(paths.mcp_config.exists());
        assert!(paths.runtime_skill.exists());
        assert!(paths.subagent_skill.exists());
        assert!(paths.wiki_capture_skill.exists());
        assert!(paths.mermaid_skill.exists());
        assert!(paths.collection_skill.exists());
        assert!(paths.work_next_command.exists());
        assert!(paths.review_comments_command.exists());
        assert!(paths.configure_command.exists());
        assert!(paths.agent_guide.exists());
    }

    #[test]
    fn manifest_name_drives_oxplow_command_prefix() {
        // The plugin `name` is the Claude Code command/skill prefix:
        // commands surface as `/<name>:<command>`. Keep it `oxplow`
        // so users type `/oxplow:work-next`, not `/oxplow-runtime:…`.
        let tmp = TempDir::new().unwrap();
        let paths = write_plugin(tmp.path(), "http://h/hook", "http://h/mcp", "tok").unwrap();
        let body = fs::read_to_string(&paths.manifest).unwrap();
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
            v["mcpServers"]["oxplow"]["headers"]["Authorization"],
            "Bearer abc123"
        );
    }

    #[test]
    fn write_plugin_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        write_plugin(tmp.path(), "http://h/hook", "http://h/mcp", "t1").unwrap();
        // Second call must not error.
        let p = write_plugin(tmp.path(), "http://h2/hook", "http://h2/mcp", "t2").unwrap();
        let body = fs::read_to_string(&p.hooks).unwrap();
        assert!(body.contains("http://h2/hook"));
        let mcp_body = fs::read_to_string(&p.mcp_config).unwrap();
        assert!(mcp_body.contains("Bearer t2"));
    }

    #[test]
    fn write_codex_runtime_emits_expected_files() {
        let tmp = TempDir::new().unwrap();
        let paths = write_codex_runtime(tmp.path(), "http://127.0.0.1:51823/mcp").unwrap();
        assert!(paths.manifest.exists());
        assert!(paths.hooks.exists());
        assert!(paths.oxplow_executable.exists());
        assert!(paths.mcp_config.exists());
        assert!(paths.runtime_skill.exists());
        assert!(paths.subagent_skill.exists());
        assert!(paths.wiki_capture_skill.exists());
        assert!(paths.mermaid_skill.exists());
        assert!(paths.collection_skill.exists());
    }

    #[test]
    fn codex_runtime_configures_hooks_and_mcp() {
        let tmp = TempDir::new().unwrap();
        let paths = write_codex_runtime(tmp.path(), "http://h/mcp").unwrap();
        let hooks = fs::read_to_string(paths.hooks).unwrap();
        assert!(hooks.contains("PreToolUse"));
        assert!(!hooks.contains("http://"));
        assert!(hooks.contains(" hook "));
        assert!(!hooks.contains("python"));
        let mcp = fs::read_to_string(paths.mcp_config).unwrap();
        assert!(mcp.contains("[mcp_servers.oxplow]"));
        assert!(mcp.contains("http://h/mcp"));
        assert!(mcp.contains("OXPLOW_HOOK_TOKEN"));
    }
}
