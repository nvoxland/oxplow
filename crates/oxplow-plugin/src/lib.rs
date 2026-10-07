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
const OPENCODE_RUNTIME_DIR_REL: &str = ".oxplow/runtime/opencode-plugin";
const PLUGIN_NAME: &str = "oxplow";
const PLUGIN_VERSION: &str = "0.0.0";

/// Hook event names mirrored from main. SessionStart is registered
/// even though Claude Code drops HTTP hooks for it ("HTTP hooks are
/// not supported for SessionStart" in its debug log) — we learn the
/// session id from whichever hook fires next instead. The last group is
/// only observed: subagents, Claude's own task list and compaction are
/// acked unread today, and their payloads can be dumped
/// (`OXPLOW_HOOK_DEBUG`) to learn their shapes.
pub const HOOK_EVENTS: &[&str] = &[
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
    /// The agent doesn't run in a terminal (an ACP agent): it has no
    /// terminal runtime to write.
    #[error("{0} agents don't run in a terminal")]
    NotTerminal(&'static str),
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
    pub wiki_capture_skill: PathBuf,
    pub mermaid_skill: PathBuf,
    pub collection_skill: PathBuf,
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
    pub wiki_capture_skill: PathBuf,
    pub mermaid_skill: PathBuf,
    pub collection_skill: PathBuf,
}

/// Paths emitted by `write_opencode_runtime`. opencode needs no
/// on-disk hooks/MCP config — both ride the per-spawn
/// `OPENCODE_CONFIG_CONTENT` env var the spawn path assembles — so the
/// runtime dir carries the JS hook-bridge plugin and a `prompts/`
/// dir for per-thread instruction files. Skills are the exception:
/// opencode only discovers them from fixed locations (project
/// `.opencode/skills/`, `.claude/skills/`, `.agents/skills/` — no
/// config key), so the oxplow skills land in `.opencode/skills/`
/// with a self-ignoring `.gitignore` per skill dir. Slash commands
/// ride the config env var (`command` key) — see
/// [`opencode_command_definitions`].
#[derive(Debug, Clone)]
pub struct OpencodeRuntimePaths {
    pub runtime_dir: PathBuf,
    pub hooks_plugin: PathBuf,
    pub prompts_dir: PathBuf,
    /// `<project>/.opencode/skills` — where the oxplow skills were
    /// materialized for opencode's fixed-location discovery.
    pub skills_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub enum AgentRuntimePaths {
    Claude(PluginPaths),
    Codex(CodexRuntimePaths),
    Opencode(OpencodeRuntimePaths),
}

pub fn write_agent_runtime(
    agent: AgentKind,
    project_dir: &Path,
    hook_base_url: &str,
    mcp_endpoint_url: &str,
    hook_token: &str,
    text: &AgentText,
) -> Result<AgentRuntimePaths, PluginError> {
    match agent {
        AgentKind::Claude => write_plugin(
            project_dir,
            hook_base_url,
            mcp_endpoint_url,
            hook_token,
            text,
        )
        .map(AgentRuntimePaths::Claude),
        AgentKind::Codex => {
            write_codex_runtime(project_dir, mcp_endpoint_url, text).map(AgentRuntimePaths::Codex)
        }
        AgentKind::Opencode => {
            write_opencode_runtime(project_dir, text).map(AgentRuntimePaths::Opencode)
        }
        AgentKind::Acp => Err(PluginError::NotTerminal("ACP")),
    }
}

/// Materialize the opencode hook-bridge plugin. Idempotent like the
/// other writers; per-spawn identity rides env vars (the JS reads
/// `OXPLOW_*` from its process env), so one dir serves every spawn.
pub fn write_opencode_runtime(
    project_dir: &Path,
    text: &AgentText,
) -> Result<OpencodeRuntimePaths, PluginError> {
    let runtime_dir = project_dir.join(OPENCODE_RUNTIME_DIR_REL);
    let plugin_dir = runtime_dir.join("plugin");
    let prompts_dir = runtime_dir.join("prompts");
    fs::create_dir_all(&plugin_dir)?;
    fs::create_dir_all(&prompts_dir)?;

    let hooks_plugin = plugin_dir.join("oxplow-hooks.js");
    fs::write(&hooks_plugin, include_str!("../assets/opencode-hooks.js"))?;

    // Skills: opencode discovers SKILL.md only from fixed project
    // locations (`.opencode/skills/<name>/SKILL.md` et al) — there is
    // no opencode.json key to point at the .oxplow runtime dir. The
    // assets' frontmatter (name matching the dir, description) is
    // already opencode-compatible. Each generated dir gets a `*`
    // .gitignore so these never land in the user's commits.
    let skills_dir = write_opencode_skills(project_dir, text)?;

    Ok(OpencodeRuntimePaths {
        runtime_dir,
        hooks_plugin,
        prompts_dir,
        skills_dir,
    })
}

/// opencode's skills: `<project>/.opencode/skills/<name>/SKILL.md`, each
/// dir with a `*` `.gitignore`. Returns the skills dir.
fn write_opencode_skills(project_dir: &Path, text: &AgentText) -> Result<PathBuf, PluginError> {
    let skills_dir = project_dir.join(".opencode").join("skills");
    write_skills(&skills_dir, &text.skills)?;
    for skill in &text.skills {
        fs::write(skills_dir.join(&skill.name).join(".gitignore"), "*\n")?;
    }
    Ok(skills_dir)
}

/// Rewrite the skills of every agent runtime already materialized under
/// `project_dir`, creating none. A runtime is written on each spawn, so
/// this is for what outlives one: an agent still running (or resumed)
/// across an oxplow upgrade, or a switch of what's active, reads the
/// current skills and commands (tsk376).
pub fn refresh_skills(project_dir: &Path, text: &AgentText) -> Result<(), PluginError> {
    for rel in [PLUGIN_DIR_REL, CODEX_RUNTIME_DIR_REL] {
        let skills_dir = project_dir.join(rel).join("skills");
        if skills_dir.is_dir() {
            write_skills(&skills_dir, &text.skills)?;
        }
    }
    let commands_dir = project_dir.join(PLUGIN_DIR_REL).join("commands");
    if commands_dir.is_dir() {
        write_commands(&commands_dir, &text.commands)?;
    }
    if project_dir.join(OPENCODE_RUNTIME_DIR_REL).is_dir() {
        write_opencode_skills(project_dir, text)?;
    }
    Ok(())
}

/// The capability answerability questions (`assets/questions/<capability>.yaml`,
/// P5.F1): what an agent should be able to answer, the skill that should
/// lead it there, and what it reaches. `oxplow_sdk::answerability` checks
/// them.
pub const CAPABILITY_QUESTIONS: &[(&str, &str)] = &[
    ("vcs", include_str!("../assets/questions/vcs.yaml")),
    (
        "work_items",
        include_str!("../assets/questions/work_items.yaml"),
    ),
    (
        "knowledge",
        include_str!("../assets/questions/knowledge.yaml"),
    ),
    (
        "code_intel",
        include_str!("../assets/questions/code_intel.yaml"),
    ),
    ("plugins", include_str!("../assets/questions/plugins.yaml")),
];

/// A capability question as the person sees it (P6.D2): offered with an
/// Ask button on the catalog page, and on a page for a ref of `about`'s
/// kind (`file`, `commit`, `effort`, `work_item`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityPrompt {
    pub capability: &'static str,
    pub prompt: String,
    pub about: Option<String>,
}

/// Every capability question, in file order. The files' other keys are
/// the answerability check's (`oxplow_sdk::answerability`).
pub fn capability_prompts() -> Vec<CapabilityPrompt> {
    #[derive(serde::Deserialize)]
    struct Entry {
        question: String,
        #[serde(default)]
        about: Option<String>,
    }
    CAPABILITY_QUESTIONS
        .iter()
        .flat_map(|(capability, yaml)| {
            serde_yaml::from_str::<Vec<Entry>>(yaml)
                .expect("a bundled questions file parses (checked by its test)")
                .into_iter()
                .map(|e| CapabilityPrompt {
                    capability,
                    prompt: e.question,
                    about: e.about,
                })
        })
        .collect()
}

/// The `description:` line of a `---`-fenced frontmatter block.
fn frontmatter_description(body: &str) -> &str {
    let Some(front) = body
        .strip_prefix("---\n")
        .and_then(|rest| rest.split("\n---").next())
    else {
        return "";
    };
    front
        .lines()
        .find_map(|l| l.strip_prefix("description:"))
        .map(str::trim)
        .unwrap_or("")
}

/// The file in each skill folder oxplow writes: how it tells its own
/// from a person's when one is no longer offered.
const SKILL_MARKER: &str = ".oxplow";

/// Write each of `skills` as `<skills_dir>/<name>/SKILL.md`, and remove
/// any other skill folder oxplow wrote there (one it no longer ships, or
/// an extension's no longer offered); a person's own stay.
fn write_skills(skills_dir: &Path, skills: &[Text]) -> Result<(), PluginError> {
    if let Ok(entries) = fs::read_dir(skills_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().join(SKILL_MARKER).is_file() && !skills.iter().any(|s| s.name == name) {
                fs::remove_dir_all(entry.path())?;
            }
        }
    }
    for skill in skills {
        let dir = skills_dir.join(&skill.name);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("SKILL.md"), &skill.body)?;
        fs::write(dir.join(SKILL_MARKER), "")?;
    }
    Ok(())
}

/// Write each of `commands` as `<commands_dir>/<name>.md`, removing every
/// other `.md` there: the folder is oxplow's.
fn write_commands(commands_dir: &Path, commands: &[Text]) -> Result<(), PluginError> {
    if let Ok(entries) = fs::read_dir(commands_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".md") {
                if !commands.iter().any(|c| c.name == stem) {
                    fs::remove_file(entry.path())?;
                }
            }
        }
    }
    for command in commands {
        fs::write(
            commands_dir.join(format!("{}.md", command.name)),
            &command.body,
        )?;
    }
    Ok(())
}

/// One piece of text for the agent: a skill (its `SKILL.md`, whose
/// frontmatter `name:` is `name`) or a slash command (its markdown).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    pub name: String,
    pub body: String,
}

/// Every skill and slash command an agent runtime gets: core's, and what
/// the project's extensions offer now (`oxplow_app::capabilities::agent_text`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentText {
    pub skills: Vec<Text>,
    pub commands: Vec<Text>,
}

impl AgentText {
    /// Core's own skills and commands.
    pub fn core() -> Self {
        let text = |list: &[(&str, &str)]| {
            list.iter()
                .map(|(name, body)| Text {
                    name: (*name).into(),
                    body: (*body).into(),
                })
                .collect()
        };
        Self {
            skills: text(OXPLOW_SKILLS),
            commands: text(CORE_COMMANDS),
        }
    }

    /// Every skill as `(name, description)`, the description taken from
    /// its frontmatter: the index an agent that can't discover skill files
    /// (an ACP agent) is given, to fetch bodies with `get_skill`.
    pub fn skill_index(&self) -> Vec<(&str, &str)> {
        self.skills
            .iter()
            .map(|s| (s.name.as_str(), frontmatter_description(&s.body)))
            .collect()
    }

    /// One skill's `SKILL.md` body by name.
    pub fn skill_body(&self, name: &str) -> Option<&str> {
        self.skills
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.body.as_str())
    }

    /// Whether `name` is taken by a skill or command already.
    pub fn names(&self, name: &str) -> bool {
        self.skills
            .iter()
            .chain(&self.commands)
            .any(|t| t.name == name)
    }
}

/// Core's slash commands, as `(name, markdown)`: `/oxplow:<name>`.
const CORE_COMMANDS: &[(&str, &str)] = &[
    (
        "review-comments",
        include_str!("../assets/review-comments.md"),
    ),
    ("configure", include_str!("../assets/configure.md")),
    ("new-metric", include_str!("../assets/new-metric.md")),
];

/// The oxplow skills every agent runtime ships, as `(dir_name, SKILL.md body)`
/// pairs. The dir name must match the frontmatter `name:` — both Claude and
/// opencode key discovery on it.
const OXPLOW_SKILLS: &[(&str, &str)] = &[
    (
        "oxplow-runtime",
        include_str!("../assets/oxplow-runtime.SKILL.md"),
    ),
    (
        "oxplow-wiki-capture",
        include_str!("../assets/oxplow-wiki-capture.SKILL.md"),
    ),
    (
        "oxplow-mermaid",
        include_str!("../assets/oxplow-mermaid.SKILL.md"),
    ),
    (
        "oxplow-collection",
        include_str!("../assets/oxplow-collection.SKILL.md"),
    ),
    (
        "oxplow-metrics",
        include_str!("../assets/oxplow-metrics.SKILL.md"),
    ),
    (
        "oxplow-extension",
        include_str!("../assets/oxplow-extension.SKILL.md"),
    ),
    (
        "oxplow-codebase",
        include_str!("../assets/oxplow-codebase.SKILL.md"),
    ),
];

/// Slash-command definitions for opencode's inline `command` config
/// key (carried per-spawn in `OPENCODE_CONFIG_CONTENT`, so nothing
/// lands on disk). Mirrors the claude plugin's `commands/` markdown
/// assets: the frontmatter `description` becomes the TUI description
/// and the body becomes the prompt template. Names are prefixed
/// `oxplow-` since opencode has no plugin namespacing (`/oxplow-work-next`
/// vs claude's `/oxplow:work-next`).
pub fn opencode_command_definitions(text: &AgentText) -> serde_json::Value {
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

/// Split a command asset into its frontmatter `description:` (if any)
/// and the markdown body after the closing `---`. Assets without
/// frontmatter come back whole as the template.
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
    text: &AgentText,
) -> Result<PluginPaths, PluginError> {
    let plugin_dir = project_dir.join(PLUGIN_DIR_REL);
    let manifest_dir = plugin_dir.join(".claude-plugin");
    let hooks_dir = plugin_dir.join("hooks");
    let commands_dir = plugin_dir.join("commands");
    let skills_dir = plugin_dir.join("skills");

    fs::create_dir_all(&manifest_dir)?;
    fs::create_dir_all(&hooks_dir)?;
    fs::create_dir_all(&commands_dir)?;

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
    let mcp_body = build_mcp_config(mcp_endpoint_url, hook_token, None);
    write_json(&mcp_config, &mcp_body)?;

    let agent_guide = plugin_dir.join("AGENT_GUIDE.md");
    fs::write(&agent_guide, include_str!("../assets/AGENT_GUIDE.md"))?;

    write_skills(&skills_dir, &text.skills)?;
    let runtime_skill = skills_dir.join("oxplow-runtime").join("SKILL.md");
    let wiki_capture_skill = skills_dir.join("oxplow-wiki-capture").join("SKILL.md");
    let mermaid_skill = skills_dir.join("oxplow-mermaid").join("SKILL.md");
    let collection_skill = skills_dir.join("oxplow-collection").join("SKILL.md");

    write_commands(&commands_dir, &text.commands)?;
    let review_comments_command = commands_dir.join("review-comments.md");
    let configure_command = commands_dir.join("configure.md");

    Ok(PluginPaths {
        plugin_dir,
        manifest,
        hooks,
        mcp_config,
        agent_guide,
        runtime_skill,
        wiki_capture_skill,
        mermaid_skill,
        collection_skill,
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

/// The thread and stream an MCP connection acts for, as the control
/// plane reads them (`X-Oxplow-Thread` / `X-Oxplow-Stream`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpIdentity<'a> {
    pub thread_id: &'a str,
    pub stream_id: &'a str,
}

fn build_mcp_config(
    mcp_endpoint_url: &str,
    hook_token: &str,
    identity: Option<McpIdentity<'_>>,
) -> serde_json::Value {
    // Bake the literal token — and the thread identity — into the file.
    // Claude Code's MCP config schema does not env-var-interpolate
    // `headers` (unlike hooks, which opt in via `allowedEnvVars`), so
    // `"Bearer $VAR"` would be sent verbatim and the control plane would
    // 401. The file lives under `.oxplow/runtime/claude-plugin/`
    // (gitignored) and is rewritten per `open_terminal_session`, so it
    // tracks the current boot's token.
    let mut headers = serde_json::Map::new();
    headers.insert(
        "Authorization".into(),
        format!("Bearer {hook_token}").into(),
    );
    if let Some(id) = identity {
        headers.insert("X-Oxplow-Thread".into(), id.thread_id.into());
        headers.insert("X-Oxplow-Stream".into(), id.stream_id.into());
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

/// A per-thread MCP config for Claude (`mcp-config.<thread>.json` next to
/// the shared `mcp-config.json`) carrying the thread's identity headers,
/// so `run_command` and every audited write know which thread is acting.
/// The shared file stays for spawns with no thread. Returns the path to
/// pass as `--mcp-config`.
pub fn write_claude_mcp_config(
    plugin_dir: &Path,
    mcp_endpoint_url: &str,
    hook_token: &str,
    identity: McpIdentity<'_>,
) -> Result<PathBuf, PluginError> {
    let path = plugin_dir.join(format!("mcp-config.{}.json", identity.thread_id));
    write_json(
        &path,
        &build_mcp_config(mcp_endpoint_url, hook_token, Some(identity)),
    )?;
    Ok(path)
}

pub fn write_codex_runtime(
    project_dir: &Path,
    mcp_endpoint_url: &str,
    text: &AgentText,
) -> Result<CodexRuntimePaths, PluginError> {
    let runtime_dir = project_dir.join(CODEX_RUNTIME_DIR_REL);
    let manifest_dir = runtime_dir.join(".codex-plugin");
    let hooks_dir = runtime_dir.join("hooks");
    let skills_dir = runtime_dir.join("skills");

    fs::create_dir_all(&manifest_dir)?;
    fs::create_dir_all(&hooks_dir)?;

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

    let oxplow_executable = std::env::current_exe()?;

    let hooks = hooks_dir.join("hooks.json");
    write_json(&hooks, &build_codex_hooks_json(&oxplow_executable))?;

    let mcp_config = runtime_dir.join("mcp-config.toml");
    fs::write(&mcp_config, build_codex_mcp_config(mcp_endpoint_url))?;

    write_skills(&skills_dir, &text.skills)?;
    let runtime_skill = skills_dir.join("oxplow-runtime").join("SKILL.md");
    let wiki_capture_skill = skills_dir.join("oxplow-wiki-capture").join("SKILL.md");
    let mermaid_skill = skills_dir.join("oxplow-mermaid").join("SKILL.md");
    let collection_skill = skills_dir.join("oxplow-collection").join("SKILL.md");

    Ok(CodexRuntimePaths {
        runtime_dir,
        manifest,
        hooks,
        oxplow_executable,
        mcp_config,
        runtime_skill,
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
    /// P6.D2: each capability question is a prompt; an `about` names a
    /// registered kind of ref, and the entity pages have some.
    #[test]
    fn capability_prompts_are_about_registered_ref_kinds() {
        let prompts = capability_prompts();
        let kinds = oxplow_domain::refs::kind::core_kinds();
        for p in &prompts {
            if let Some(about) = &p.about {
                assert!(kinds.get(about).is_some(), "{}: `{about}`", p.prompt);
            }
        }
        for kind in ["file", "commit", "effort", "work_item"] {
            assert!(
                prompts.iter().any(|p| p.about.as_deref() == Some(kind)),
                "no prompt about `{kind}`"
            );
        }
        assert!(prompts
            .iter()
            .any(|p| p.capability == "vcs" && p.prompt == "Who has changed this file the most?"));
    }

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
            &AgentText::core(),
        )
        .unwrap();
        assert!(paths.manifest.exists());
        assert!(paths.hooks.exists());
        assert!(paths.mcp_config.exists());
        assert!(paths.runtime_skill.exists());
        assert!(paths.wiki_capture_skill.exists());
        assert!(paths.mermaid_skill.exists());
        assert!(paths.collection_skill.exists());
        for (name, _) in OXPLOW_SKILLS {
            let skill = paths
                .runtime_skill
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(name)
                .join("SKILL.md");
            assert!(skill.exists(), "claude plugin missing skill {name}");
        }
        assert!(paths.review_comments_command.exists());
        assert!(paths.configure_command.exists());
        assert!(paths.agent_guide.exists());
    }

    /// What's offered is installed; a skill or command oxplow wrote that
    /// isn't offered any more (retired, or its extension's implementation
    /// not active) leaves on the next write; a person's own stays.
    #[test]
    fn what_is_no_longer_offered_is_removed() {
        let tmp = TempDir::new().unwrap();
        let mut text = AgentText::core();
        text.skills.push(Text {
            name: "work-items".into(),
            body: "---\nname: work-items\ndescription: d\n---\n".into(),
        });
        text.commands.push(Text {
            name: "work-next".into(),
            body: "next".into(),
        });
        let paths =
            write_plugin(tmp.path(), "http://h/hook", "http://h/mcp", "tok", &text).unwrap();
        let skills = paths.plugin_dir.join("skills");
        let commands = paths.plugin_dir.join("commands");
        fs::create_dir_all(skills.join("someone-elses")).unwrap();
        fs::write(skills.join("someone-elses").join("SKILL.md"), "x").unwrap();
        assert!(skills.join("work-items").join("SKILL.md").exists());
        assert!(commands.join("work-next.md").exists());
        refresh_skills(tmp.path(), &AgentText::core()).unwrap();
        assert!(!skills.join("work-items").exists());
        assert!(!commands.join("work-next.md").exists());
        assert!(skills.join("someone-elses").exists());
        assert!(skills.join("oxplow-runtime").join("SKILL.md").exists());
        assert!(commands.join("configure.md").exists());
    }

    #[test]
    fn shipped_plugin_assets_never_mention_dot_context() {
        // `.context/` is THIS repo's own docs convention — it must never
        // leak into the skills / prompts / hooks / commands oxplow writes
        // into a user's project (those docs don't exist downstream).
        let tmp = TempDir::new().unwrap();
        write_plugin(
            tmp.path(),
            "http://h/hook",
            "http://h/mcp",
            "tok",
            &AgentText::core(),
        )
        .unwrap();
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
        // so users type `/oxplow:work-next`, not `/oxplow-runtime:…`.
        let tmp = TempDir::new().unwrap();
        let paths = write_plugin(
            tmp.path(),
            "http://h/hook",
            "http://h/mcp",
            "tok",
            &AgentText::core(),
        )
        .unwrap();
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
        write_plugin(
            tmp.path(),
            "http://h/hook",
            "http://h/mcp",
            "t1",
            &AgentText::core(),
        )
        .unwrap();
        // Second call must not error.
        let p = write_plugin(
            tmp.path(),
            "http://h2/hook",
            "http://h2/mcp",
            "t2",
            &AgentText::core(),
        )
        .unwrap();
        let body = fs::read_to_string(&p.hooks).unwrap();
        assert!(body.contains("http://h2/hook"));
        let mcp_body = fs::read_to_string(&p.mcp_config).unwrap();
        assert!(mcp_body.contains("Bearer t2"));
    }

    #[test]
    fn write_codex_runtime_emits_expected_files() {
        let tmp = TempDir::new().unwrap();
        let paths =
            write_codex_runtime(tmp.path(), "http://127.0.0.1:51823/mcp", &AgentText::core())
                .unwrap();
        assert!(paths.manifest.exists());
        assert!(paths.hooks.exists());
        assert!(paths.oxplow_executable.exists());
        assert!(paths.mcp_config.exists());
        assert!(paths.runtime_skill.exists());
        assert!(paths.wiki_capture_skill.exists());
        assert!(paths.mermaid_skill.exists());
        assert!(paths.collection_skill.exists());
        for (name, _) in OXPLOW_SKILLS {
            let skill = paths
                .runtime_skill
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(name)
                .join("SKILL.md");
            assert!(skill.exists(), "codex runtime missing skill {name}");
        }
    }

    /// Agents that can't discover skill files get an index instead
    /// (tsk376): every skill, with its frontmatter description.
    #[test]
    fn the_skill_index_names_every_skill_with_its_description() {
        let text = AgentText::core();
        let index = text.skill_index();
        assert_eq!(index.len(), OXPLOW_SKILLS.len());
        let (name, description) = index
            .iter()
            .find(|(n, _)| *n == "oxplow-extension")
            .unwrap();
        assert_eq!(*name, "oxplow-extension");
        assert!(
            description.starts_with("Build oxplow lenses"),
            "{description}"
        );
        assert!(index.iter().all(|(_, d)| !d.is_empty()));
        assert!(text
            .skill_body("oxplow-extension")
            .unwrap()
            .contains("# Building oxplow lenses"));
        assert_eq!(text.skill_body("nope"), None);
    }

    /// Boot refreshes the skills of runtimes already on disk, so a running
    /// or resumed agent reads the current ones; it creates none (tsk376).
    #[test]
    fn refresh_skills_rewrites_existing_runtimes_only() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(PLUGIN_DIR_REL).join("skills");
        std::fs::create_dir_all(claude.join("oxplow-extension")).unwrap();
        std::fs::write(claude.join("oxplow-extension/SKILL.md"), "stale").unwrap();
        refresh_skills(tmp.path(), &AgentText::core()).unwrap();
        for (name, body) in OXPLOW_SKILLS {
            assert_eq!(
                std::fs::read_to_string(claude.join(name).join("SKILL.md")).unwrap(),
                *body
            );
        }
        assert!(!tmp.path().join(CODEX_RUNTIME_DIR_REL).exists());
        assert!(!tmp.path().join(".opencode").exists());
    }

    #[test]
    fn write_opencode_runtime_emits_hook_bridge_plugin() {
        let tmp = TempDir::new().unwrap();
        let paths = write_opencode_runtime(tmp.path(), &AgentText::core()).unwrap();
        assert!(paths.hooks_plugin.exists());
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
    }

    #[test]
    fn write_opencode_runtime_materializes_skills_with_gitignore() {
        let tmp = TempDir::new().unwrap();
        let paths = write_opencode_runtime(tmp.path(), &AgentText::core()).unwrap();
        assert_eq!(paths.skills_dir, tmp.path().join(".opencode/skills"));
        for name in [
            "oxplow-runtime",
            "oxplow-wiki-capture",
            "oxplow-mermaid",
            "oxplow-collection",
            "oxplow-metrics",
            "oxplow-extension",
        ] {
            let skill = paths.skills_dir.join(name).join("SKILL.md");
            let body = fs::read_to_string(&skill).unwrap_or_else(|_| panic!("missing {name}"));
            // opencode keys discovery on frontmatter name == dir name.
            assert!(
                body.contains(&format!("name: {name}")),
                "frontmatter name must match dir for {name}"
            );
            // Generated dirs self-ignore so they never land in commits.
            let ignore =
                fs::read_to_string(paths.skills_dir.join(name).join(".gitignore")).unwrap();
            assert_eq!(ignore.trim(), "*");
        }
    }

    #[test]
    fn opencode_command_definitions_carry_description_and_template() {
        let defs = opencode_command_definitions(&AgentText::core());
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

    #[test]
    fn write_opencode_runtime_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        write_opencode_runtime(tmp.path(), &AgentText::core()).unwrap();
        let paths = write_opencode_runtime(tmp.path(), &AgentText::core()).unwrap();
        assert!(paths.hooks_plugin.exists());
    }

    #[test]
    fn codex_runtime_configures_hooks_and_mcp() {
        let tmp = TempDir::new().unwrap();
        let paths = write_codex_runtime(tmp.path(), "http://h/mcp", &AgentText::core()).unwrap();
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
