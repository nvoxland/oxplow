//! The `opencode` harness: opencode in a terminal. Its hooks, MCP server,
//! slash commands, skills folder and the system prompt (an instructions
//! file per session) all ride the `OPENCODE_CONFIG_CONTENT` env var; the
//! files they name are under `.oxplow/runtime/opencode-plugin/`, so
//! nothing lands in a checkout.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use oxplow_domain::agent::harness::{
    AgentHarness, ConversationQuery, HarnessError, HarnessSetting, Launch, LaunchInput, LaunchSpec,
    NativeConversation, RuntimeRoots,
};
use oxplow_domain::agent::observe::HookAnswer;
use oxplow_domain::agent::text::AgentText;
use oxplow_domain::agent::tool::{ToolKind, ToolUse};

use super::shared::{
    claude_shaped_tool_use, in_shell, program_and_guard, resume_or_fresh, runtime, shell_escape,
    write_skills, Resume,
};
use super::Named;

pub(super) struct Opencode(pub(super) Named);

/// The model opencode launches with (`-m provider/model`) when its config
/// names none (`{ model }`). Assumes GitHub Copilot auth in opencode's own
/// auth store.
pub const DEFAULT_MODEL: &str = "github-copilot/gpt-5-mini";

#[async_trait::async_trait]
impl AgentHarness for Opencode {
    fn id(&self) -> &str {
        &self.0.id
    }

    fn title(&self) -> &str {
        &self.0.title
    }

    async fn launch(&self, input: &LaunchInput) -> Result<Launch, HarnessError> {
        let paths = write_runtime(&input.project_dir, &input.text).map_err(runtime)?;
        // No --append-system-prompt: the prompt is an instructions file,
        // one per session (two sessions of a thread run apart).
        let mut instructions = Vec::new();
        if let Some(prompt) = input.system_prompt.as_deref().filter(|p| !p.is_empty()) {
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
                &paths.skills_dir,
                &input.text,
            ),
        ));
        let model = input
            .config
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(DEFAULT_MODEL);
        let program = input.resolve_program("opencode");
        Ok(Launch {
            spec: LaunchSpec::Pty {
                command: command(
                    &input.workspace.to_string_lossy(),
                    super::shared::saved(input),
                    program.as_deref(),
                    model,
                ),
                env,
            },
            transcript_missing: false,
        })
    }

    fn settings(&self) -> Vec<HarnessSetting> {
        vec![HarnessSetting {
            key: "model".into(),
            title: "Model".into(),
            hint: "The model OpenCode launches with (`provider/model`). Blank uses its default; applies to sessions started after Save.".into(),
            placeholder: DEFAULT_MODEL.into(),
        }]
    }

    /// Its skills are on disk; its commands ride each launch's config.
    async fn refresh_text(
        &self,
        roots: &RuntimeRoots,
        text: &AgentText,
    ) -> Result<(), HarnessError> {
        let skills_dir = roots.project_dir.join(RUNTIME_DIR_REL).join("skills");
        if skills_dir.is_dir() {
            write_skills(&skills_dir, &text.skills).map_err(runtime)?;
        }
        Ok(())
    }

    /// Its bridge posts Claude Code's shape, its tools renamed to Claude's
    /// (`assets/opencode-hooks.js`), except its patch tool: `apply_patch`
    /// with the patch as `patchText`, an edit of every file it names.
    /// `opencode session list --format json`, those begun in the worktree.
    async fn conversations(
        &self,
        query: &ConversationQuery,
    ) -> Result<Vec<NativeConversation>, HarnessError> {
        let program = query.resolve_program("opencode").ok_or_else(|| {
            HarnessError::Unavailable("`opencode` isn't installed where oxplow looks".into())
        })?;
        let listed = super::shared::program_output(
            &program,
            &["session", "list", "--format", "json", "-n", LIST_SCAN],
        )
        .await?;
        sessions_in(&listed, &query.cwd.to_string_lossy(), query.limit)
    }

    async fn tool_use(&self, body: &serde_json::Value) -> Option<ToolUse> {
        let mut tool = claude_shaped_tool_use(body)?;
        if tool.name == "apply_patch" {
            tool.kind = ToolKind::Edit;
            tool.paths = super::shared::patch_paths(&body["tool_input"]);
        }
        Some(tool)
    }

    async fn render(&self, answer: &HookAnswer) -> serde_json::Value {
        super::shared::render(answer)
    }
}

const RUNTIME_DIR_REL: &str = ".oxplow/runtime/opencode-plugin";

/// What [`write_runtime`] wrote. opencode needs no on-disk hooks or MCP
/// config — both ride `OPENCODE_CONFIG_CONTENT` — so the runtime dir
/// carries the JS hook-bridge plugin, the skills, and a `prompts/` dir for
/// the sessions' instruction files.
struct RuntimePaths {
    hooks_plugin: PathBuf,
    /// Named by the config's `skills.paths`. The skills' frontmatter (name
    /// matching the folder, description) is already opencode's.
    skills_dir: PathBuf,
    prompts_dir: PathBuf,
}

/// Materialize the hook-bridge plugin and the skills. Idempotent;
/// per-session identity rides env vars (the JS reads `OXPLOW_*` from its
/// process env), so one dir serves every session of every stream.
fn write_runtime(project_dir: &Path, text: &AgentText) -> io::Result<RuntimePaths> {
    let runtime_dir = project_dir.join(RUNTIME_DIR_REL);
    let plugin_dir = runtime_dir.join("plugin");
    let skills_dir = runtime_dir.join("skills");
    let prompts_dir = runtime_dir.join("prompts");
    fs::create_dir_all(&plugin_dir)?;
    fs::create_dir_all(&prompts_dir)?;
    let hooks_plugin = plugin_dir.join("oxplow-hooks.js");
    fs::write(&hooks_plugin, include_str!("../assets/opencode-hooks.js"))?;
    write_skills(&skills_dir, &text.skills)?;
    Ok(RuntimePaths {
        hooks_plugin,
        skills_dir,
        prompts_dir,
    })
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

/// `opencode -m <model>`, resuming `resume`.
fn command(
    cwd: &str,
    resume: Option<super::shared::Saved<'_>>,
    program: Option<&str>,
    model: &str,
) -> String {
    let (prog, guard) = program_and_guard(program, "opencode");
    let base = format!("{prog} -m {}", shell_escape(model));
    let resume = match resume {
        None => Resume::Fresh,
        Some(saved) => Resume::Saved(format!(
            "-s {}{}",
            shell_escape(saved.id),
            if saved.fork { " --fork" } else { "" }
        )),
    };
    in_shell(cwd, &guard, &resume_or_fresh(&base, resume))
}

/// How many of its newest sessions are read to find a worktree's.
const LIST_SCAN: &str = "500";

/// The sessions `listed` (its `session list --format json`) begun in
/// `cwd`, newest first, at most `limit`.
fn sessions_in(
    listed: &str,
    cwd: &str,
    limit: usize,
) -> Result<Vec<NativeConversation>, HarnessError> {
    let rows: Vec<serde_json::Value> = serde_json::from_str(listed.trim())
        .map_err(|e| HarnessError::Unavailable(format!("reading its session list: {e}")))?;
    let at = |r: &serde_json::Value, key: &str| {
        r.get(key)
            .and_then(|v| v.as_i64())
            .map(oxplow_domain::Timestamp::from_unix_ms)
    };
    let mut out: Vec<NativeConversation> = rows
        .iter()
        .filter(|r| r.get("directory").and_then(|d| d.as_str()) == Some(cwd))
        .filter_map(|r| {
            Some(NativeConversation {
                id: r.get("id")?.as_str()?.to_string(),
                title: r
                    .get("title")
                    .and_then(|t| t.as_str())
                    .filter(|t| !t.is_empty())
                    .map(str::to_string),
                first_prompt: None,
                started_at: at(r, "created"),
                updated_at: at(r, "updated"),
                size_bytes: None,
                parent: None,
            })
        })
        .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
    out.truncate(limit);
    Ok(out)
}

/// The inline opencode config carried per spawn (merged on top of the
/// person's own): the oxplow MCP server (the bearer interpolated by
/// opencode itself from `{env:…}`), the hook-bridge plugin, the session's
/// instructions file, oxplow's skills folder and its slash commands.
fn config_content(
    mcp_endpoint_url: &str,
    hooks_plugin: &Path,
    instructions: &[String],
    skills_dir: &Path,
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
                },
            },
        },
        "plugin": [hooks_plugin.to_string_lossy()],
        "instructions": instructions,
        "skills": { "paths": [skills_dir.to_string_lossy()] },
        "command": command_definitions(text),
    })
    .to_string()
}

#[cfg(test)]
mod tests {

    /// Its edit tool is `apply_patch`, the patch as `patchText` (a
    /// recorded OpenCode 2026-10 body): an edit of every file it names,
    /// which the write guard and the effort's claims read.
    #[test]
    fn its_apply_patch_is_an_edit_of_the_files_it_names() {
        let h = harness("oxplow:opencode", "opencode");
        let body = serde_json::json!({
            "session_id": "ses_ee095c454ffep2jtN6LDC21YZJ",
            "tool_name": "apply_patch",
            "tool_input": {"patchText": "*** Begin Patch\n*** Add File: opencode.txt\n+2026-10-09 01:47:04 CDT\n+\n*** End Patch"}
        });
        let tool = crate::test_launch::block(h.tool_use(&body)).expect("a call");
        assert_eq!(tool.kind, oxplow_domain::agent::tool::ToolKind::Edit);
        assert_eq!(tool.paths, ["opencode.txt"]);
        assert_eq!(tool.name, "apply_patch");
    }

    /// The bridge (`assets/opencode-hooks.js`, run under bun with `fetch`
    /// caught) posts each call's id (OpenCode's `callID`) and a shell
    /// call's exit code (its bash tool's `metadata.exit`): what it posts maps
    /// to a call oxplow can pair and judge.
    #[test]
    fn its_bridge_sends_the_call_id_the_exit_code_and_a_childs_parent() {
        let dir = tempfile::tempdir().unwrap();
        let driver = dir.path().join("drive.mjs");
        let bridge =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/opencode-hooks.js");
        std::fs::write(
            &driver,
            format!(
                r#"import {{ OxplowHooks }} from {bridge};
const posts = [];
globalThis.fetch = async (url, init) => {{
  posts.push({{ event: url.split("/").pop(), payload: JSON.parse(init.body) }});
  return {{ json: async () => ({{}}) }};
}};
process.env.OXPLOW_HOOK_BASE_URL = "http://hook.invalid";
const hooks = await OxplowHooks({{ client: {{ session: {{ get: async ({{ path }}) => ({{ data: path.id === "ses_child" ? {{ parentID: "ses1" }} : {{}} }}) }} }} }});
await hooks["tool.execute.before"]({{ tool: "bash", sessionID: "ses1", callID: "call_1" }}, {{ args: {{ command: "false" }} }});
await hooks["tool.execute.after"](
  {{ tool: "bash", sessionID: "ses1", callID: "call_1", args: {{ command: "false" }} }},
  {{ title: "false", output: "", metadata: {{ output: "", exit: 1, description: "fails" }} }},
);
await hooks["tool.execute.after"](
  {{ tool: "read", sessionID: "ses1", callID: "call_2", args: {{ filePath: "a.txt" }} }},
  {{ title: "a.txt", output: "x", metadata: {{}} }},
);
await hooks["tool.execute.after"](
  {{ tool: "read", sessionID: "ses_child", callID: "call_3", args: {{ filePath: "b.txt" }} }},
  {{ title: "b.txt", output: "y", metadata: {{}} }},
);
console.log(JSON.stringify(posts));
"#,
                bridge = serde_json::to_string(&format!("file://{}", bridge.display())).unwrap()
            ),
        )
        .unwrap();
        let out = std::process::Command::new("bun")
            .arg(&driver)
            .output()
            .expect("bun runs the bridge (the repo's JS runtime)");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let posts: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
        let h = harness("oxplow:opencode", "opencode");
        let tool = |i: usize| crate::test_launch::block(h.tool_use(&posts[i]["payload"])).unwrap();

        assert_eq!(posts[0]["event"], "PreToolUse");
        let pre = tool(0);
        assert_eq!(pre.call_id.as_deref(), Some("call_1"));
        assert_eq!(pre.exit_code, None);

        assert_eq!(posts[1]["event"], "PostToolUse");
        let ran = tool(1);
        assert_eq!(ran.kind, oxplow_domain::agent::tool::ToolKind::Shell);
        assert_eq!(ran.call_id.as_deref(), Some("call_1"));
        assert_eq!((ran.exit_code, ran.ok), (Some(1), Some(false)));

        // A tool with no exit code sends none.
        let read = tool(2);
        assert_eq!(read.call_id.as_deref(), Some("call_2"));
        assert_eq!(read.exit_code, None);
        assert_eq!(posts[2]["payload"]["tool_response"].get("exit_code"), None);
        assert_eq!(tool(2).subagent, None);

        // A child session's tool hook is posted as its parent's, run by the
        // child as a `task` subagent.
        assert_eq!(posts[3]["payload"]["session_id"], "ses1");
        let child = tool(3);
        assert_eq!(
            child.subagent,
            Some(oxplow_domain::agent::tool::Subagent {
                id: "ses_child".into(),
                kind: Some("task".into()),
            })
        );
    }

    /// The bridge hands what oxplow answers to the model — a prompt hook's
    /// context as a synthetic part of the user's message, a tool hook's
    /// appended to the tool's result — and sends the agent's final message
    /// with the Stop, so ending a reply with a question sets waiting. Run
    /// under bun with `fetch` answering and a stubbed plugin client.
    #[test]
    fn its_bridge_delivers_hook_context_and_sends_the_final_message() {
        let dir = tempfile::tempdir().unwrap();
        let driver = dir.path().join("drive.mjs");
        let bridge =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/opencode-hooks.js");
        std::fs::write(
            &driver,
            format!(
                r#"import {{ OxplowHooks }} from {bridge};
const posts = [];
const context = {{ UserPromptSubmit: "CTX-PROMPT", PostToolUse: "CTX-TOOL" }};
globalThis.fetch = async (url, init) => {{
  const event = url.split("/").pop();
  posts.push({{ event, payload: JSON.parse(init.body) }});
  const text = context[event];
  return {{ json: async () => text ? {{ hookSpecificOutput: {{ hookEventName: event, additionalContext: text }} }} : {{}} }};
}};
process.env.OXPLOW_HOOK_BASE_URL = "http://hook.invalid";
const messages = [
  {{ info: {{ role: "user" }}, parts: [{{ type: "text", text: "do it" }}] }},
  {{ info: {{ role: "assistant" }}, parts: [{{ type: "text", text: "first" }}] }},
  {{ info: {{ role: "assistant" }}, parts: [{{ type: "tool", tool: "bash" }}, {{ type: "text", text: "Which one" }}, {{ type: "text", text: "should I use?" }}] }},
];
const hooks = await OxplowHooks({{ client: {{ session: {{
  get: async () => ({{ data: {{}} }}),
  messages: async () => ({{ data: messages }}),
}} }} }});
const prompt = {{ message: {{ id: "msg_1", sessionID: "ses1" }}, parts: [{{ type: "text", text: "hello" }}] }};
await hooks["chat.message"]({{ sessionID: "ses1" }}, prompt);
const tool = {{ title: "t", output: "result", metadata: {{}} }};
await hooks["tool.execute.after"]({{ tool: "read", sessionID: "ses1", callID: "c1", args: {{}} }}, tool);
await hooks.event({{ event: {{ type: "session.idle", properties: {{ sessionID: "ses1" }} }} }});
console.log(JSON.stringify({{ posts, prompt, tool }}));
"#,
                bridge = serde_json::to_string(&format!("file://{}", bridge.display())).unwrap()
            ),
        )
        .unwrap();
        let out = std::process::Command::new("bun")
            .arg(&driver)
            .output()
            .expect("bun runs the bridge (the repo's JS runtime)");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let run: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

        // The prompt's context is a synthetic text part after the person's.
        let parts = run["prompt"]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2, "{parts:?}");
        assert_eq!(parts[0]["text"], "hello");
        assert_eq!(parts[1]["type"], "text");
        assert_eq!(parts[1]["text"], "CTX-PROMPT");
        assert_eq!(parts[1]["synthetic"], true);
        assert_eq!(parts[1]["sessionID"], "ses1");
        assert_eq!(parts[1]["messageID"], "msg_1");
        assert!(parts[1]["id"].as_str().unwrap().starts_with("prt_"));

        // The tool's context follows its result.
        let result = run["tool"]["output"].as_str().unwrap();
        assert!(
            result.starts_with("result") && result.ends_with("CTX-TOOL"),
            "{result}"
        );

        // The Stop carries the last assistant message, its text parts joined.
        let stop = run["posts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["event"] == "Stop")
            .expect("a Stop");
        assert_eq!(
            stop["payload"]["last_assistant_message"],
            "Which one\nshould I use?"
        );
    }

    use super::*;
    use crate::test_launch::{harness, launch_in, project_only};
    use tempfile::TempDir;

    /// Its session list, filtered to the worktree's, newest first.
    #[test]
    fn its_conversations_are_the_worktrees_sessions() {
        let listed = r#"[
          {"id":"ses_old","title":"Older","updated":1000,"created":900,"directory":"/repo/wt"},
          {"id":"ses_new","title":"Newer","updated":3000,"created":2900,"directory":"/repo/wt"},
          {"id":"ses_else","title":"Elsewhere","updated":5000,"created":4900,"directory":"/repo/other"}
        ]"#;
        let found = sessions_in(listed, "/repo/wt", 10).unwrap();
        assert_eq!(
            found
                .iter()
                .map(|c| (c.id.as_str(), c.title.as_deref()))
                .collect::<Vec<_>>(),
            [("ses_new", Some("Newer")), ("ses_old", Some("Older"))]
        );
        assert_eq!(
            found[0].started_at,
            Some(oxplow_domain::Timestamp::from_unix_ms(2900))
        );
        assert_eq!(sessions_in(listed, "/repo/wt", 1).unwrap().len(), 1);
        assert!(sessions_in("not json", "/repo/wt", 1).is_err());
    }

    /// Through its CLI: a program that prints the list.
    #[tokio::test]
    async fn its_conversations_come_from_its_cli() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let bin = tmp.path().join("opencode");
        fs::write(
            &bin,
            "#!/bin/sh\n[ \"$1 $2 $3 $4\" = 'session list --format json' ] || exit 2\necho '[{\"id\":\"ses_a\",\"title\":\"A\",\"updated\":1,\"created\":1,\"directory\":\"/repo/wt\"}]'\n",
        )
        .unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        let h = harness("oxplow:opencode", "opencode");
        let query = oxplow_domain::agent::harness::ConversationQuery {
            cwd: "/repo/wt".into(),
            home: None,
            search_path: vec![tmp.path().to_path_buf()],
            limit: 10,
        };
        let found = h.conversations(&query).await.unwrap();
        assert_eq!(found[0].id, "ses_a");
        let none = oxplow_domain::agent::harness::ConversationQuery {
            search_path: vec![],
            ..query
        };
        assert!(matches!(
            h.conversations(&none).await,
            Err(HarnessError::Unavailable(_))
        ));
    }

    /// A fork continues the saved session as a new one (`--fork`).
    #[test]
    fn a_fork_continues_a_copy() {
        let h = harness("oxplow:opencode", "opencode");
        let l = crate::test_launch::launch_fork(h.as_ref(), "sess-w");
        let (command, _) = crate::test_launch::pty(&l.launch);
        assert!(
            command.contains(" -s ") && command.contains("sess-w") && command.contains(" --fork"),
            "{command}"
        );
    }

    #[test]
    fn launch_builds_the_command_and_env() {
        let h = harness("oxplow:opencode", "opencode");
        let l = launch_in(
            h.as_ref(),
            Some("sess-w"),
            Some("be terse"),
            &serde_json::json!({ "model": "anthropic/claude-sonnet-4-6" }),
        );
        let (command, env) = crate::test_launch::pty(&l.launch);
        assert_eq!(env["OXPLOW_HOOK_TOKEN"], "secret-bearer");
        assert_eq!(env["OXPLOW_SESSION"], "ses3");
        assert!(env["OPENCODE_CONFIG_CONTENT"].contains("oxplow-hooks.js"));
        assert!(!command.contains("OXPLOW_") && !command.contains("OPENCODE_CONFIG"));
        for want in [
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
        // Its skills are the runtime's, which the config names; nothing
        // lands in a checkout.
        let config: serde_json::Value =
            serde_json::from_str(env["OPENCODE_CONFIG_CONTENT"]).unwrap();
        let skills = l.project.join(".oxplow/runtime/opencode-plugin/skills");
        assert_eq!(
            config["skills"]["paths"],
            serde_json::json!([skills.to_string_lossy()])
        );
        assert!(skills.join("oxplow-runtime/SKILL.md").is_file());
        assert!(!l.project.join(".opencode").exists());
        assert!(!l.workspace.join(".opencode").exists());
    }

    #[test]
    fn with_no_model_configured_it_runs_the_default() {
        let cmd = command("/repo", None, None, DEFAULT_MODEL);
        assert!(cmd.contains("exec opencode") && cmd.contains(DEFAULT_MODEL));
        assert!(!cmd.contains(" -s "));
    }

    #[test]
    fn the_config_content_wires_mcp_plugin_and_instructions() {
        let content = config_content(
            "http://127.0.0.1:9/mcp",
            Path::new("/proj/.oxplow/runtime/opencode-plugin/plugin/oxplow-hooks.js"),
            &["/proj/.oxplow/runtime/opencode-plugin/prompts/ses3.md".to_string()],
            Path::new("/proj/.oxplow/runtime/opencode-plugin/skills"),
            &oxplow_agent_text::core_text(),
        );
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["mcp"]["oxplow"]["type"], "remote");
        assert_eq!(v["mcp"]["oxplow"]["url"], "http://127.0.0.1:9/mcp");
        // opencode interpolates {env:VAR} itself: the bearer isn't baked
        // in, and it's the whole of who the calls come from.
        assert_eq!(
            v["mcp"]["oxplow"]["headers"],
            serde_json::json!({ "Authorization": "Bearer {env:OXPLOW_HOOK_TOKEN}" })
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
        assert_eq!(
            v["skills"]["paths"],
            serde_json::json!(["/proj/.oxplow/runtime/opencode-plugin/skills"])
        );
    }

    #[test]
    fn write_runtime_emits_the_hook_bridge_plugin() {
        let tmp = TempDir::new().unwrap();
        let paths = write_runtime(tmp.path(), &oxplow_agent_text::core_text()).unwrap();
        assert!(paths.prompts_dir.is_dir());
        let js = fs::read_to_string(&paths.hooks_plugin).unwrap();
        // The bridge posts with the session's bearer from env, sends the
        // Claude-shaped lifecycle events, and maps tool names so the
        // write guard matches.
        assert!(js.contains("OXPLOW_HOOK_BASE_URL"));
        assert!(js.contains("OXPLOW_HOOK_TOKEN"));
        assert!(!js.contains("X-Oxplow"));
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
    fn write_runtime_materializes_the_skills() {
        let tmp = TempDir::new().unwrap();
        let text = oxplow_agent_text::core_text();
        let paths = write_runtime(tmp.path(), &text).unwrap();
        assert_eq!(
            paths.skills_dir,
            tmp.path().join(RUNTIME_DIR_REL).join("skills")
        );
        for skill in &text.skills {
            let name = &skill.name;
            let body = fs::read_to_string(paths.skills_dir.join(name).join("SKILL.md"))
                .unwrap_or_else(|_| panic!("missing {name}"));
            // opencode keys discovery on frontmatter name == dir name.
            assert!(
                body.contains(&format!("name: {name}")),
                "frontmatter name must match dir for {name}"
            );
        }
    }

    /// A refresh rewrites a runtime already on disk and creates none.
    #[test]
    fn refresh_text_rewrites_only_runtimes_already_on_disk() {
        let tmp = TempDir::new().unwrap();
        let h = harness("oxplow:opencode", "opencode");
        let text = oxplow_agent_text::core_text();
        crate::test_launch::block(h.refresh_text(&project_only(tmp.path()), &text)).unwrap();
        assert!(!tmp.path().join(RUNTIME_DIR_REL).exists());
        let skills = tmp.path().join(RUNTIME_DIR_REL).join("skills");
        fs::create_dir_all(skills.join("oxplow-extension")).unwrap();
        fs::write(skills.join("oxplow-extension/SKILL.md"), "stale").unwrap();
        crate::test_launch::block(h.refresh_text(&project_only(tmp.path()), &text)).unwrap();
        for skill in &text.skills {
            assert_eq!(
                fs::read_to_string(skills.join(&skill.name).join("SKILL.md")).unwrap(),
                skill.body
            );
        }
        assert!(!tmp.path().join(".opencode").exists());
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
