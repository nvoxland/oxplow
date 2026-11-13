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
use oxplow_domain::agent::observe::{HookAnswer, Prompt};
use oxplow_domain::agent::text::AgentText;
use oxplow_domain::agent::tool::{Subagent, ToolKind, ToolUse};
use oxplow_domain::hook::{TurnOutcome, TurnRun};

use super::shared::{
    claude_shaped_tool_use, in_shell, program_and_guard, resume_or_fresh, runtime, shell_escape,
    write_skills, Resume,
};
use super::Named;

pub(super) struct Opencode(pub(super) Named);

/// What tells the bridge to post opencode's raw events and hook inputs for
/// the hook recording (`OXPLOW_HOOK_DEBUG`, which an agent's environment
/// never holds): set on the launch only while a recording is on.
fn raw_recording_env(hook_debug: Option<std::ffi::OsString>) -> Option<(String, String)> {
    hook_debug
        .filter(|path| !path.is_empty())
        .map(|_| ("OXPLOW_OPENCODE_RAW".into(), "1".into()))
}

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
        env.extend(raw_recording_env(std::env::var_os("OXPLOW_HOOK_DEBUG")));
        // Only a model the person set: opencode runs its own default
        // otherwise (a model oxplow named itself once vanished from the
        // provider's list, and opencode quietly ran another).
        let model = super::shared::setting(&input.config, "model");
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
            hint: "The model OpenCode launches with (`provider/model`, as `opencode models` lists them). Blank uses your own OpenCode default; applies to sessions started after Save. OpenCode gives no error for a model it doesn't have, and runs another.".into(),
            placeholder: "provider/model".into(),
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
        // Its list is of the project the program runs in, so it runs in the
        // worktree (a directory outside any git checkout is opencode's
        // `global` project).
        let listed = super::shared::program_output(
            &program,
            &["session", "list", "--format", "json", "-n", LIST_SCAN],
            Some(&query.cwd),
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

    /// The bridge's Stop names the model, variant and tokens of the
    /// session's last assistant message; a variant is opencode's reasoning
    /// effort, and the context it ended on is the input plus the cache's
    /// reads and writes. The bridge has no window to name.
    async fn turn_run(&self, body: &serde_json::Value) -> Option<TurnRun> {
        let text = |k: &str| {
            body.get(k)
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        let (model, effort) = (text("model"), text("variant"));
        let count = |k: &str| {
            body.pointer(&format!("/tokens/{k}"))
                .and_then(|n| n.as_i64())
        };
        let context_tokens = count("input").map(|input| {
            input + count("cache/read").unwrap_or(0) + count("cache/write").unwrap_or(0)
        });
        let left_running = super::shared::left_running(body);
        // How the harness says it ended: the bridge names an error (an
        // abort is `MessageAbortedError`); none means it finished.
        let failure = body.get("error").filter(|e| e.is_object());
        let name = failure.and_then(|e| e["name"].as_str());
        let (outcome, error) = match name {
            Some("MessageAbortedError") => (Some(TurnOutcome::Interrupted), None),
            Some(name) => {
                let message = failure.and_then(|e| e["message"].as_str());
                let said = match message.filter(|m| !m.is_empty()) {
                    Some(m) => format!("{name}: {m}"),
                    None => name.to_string(),
                };
                (Some(TurnOutcome::Errored), Some(said))
            }
            None => (None, None),
        };
        (model.is_some()
            || effort.is_some()
            || context_tokens.is_some()
            || left_running.is_some()
            || outcome.is_some())
        .then_some(TurnRun {
            left_running,
            model,
            effort,
            context_tokens,
            context_window: None,
            outcome,
            error,
        })
    }

    fn reading(&self, body: &serde_json::Value) -> oxplow_domain::hook::HookReading {
        super::shared::claude_shaped_reading(body)
    }

    /// A background subagent's result arrives as a synthetic user part,
    /// `<task id="<child session>" state="completed|error">`: it opens a
    /// turn from that subagent, not from the person.
    async fn prompt(&self, body: &serde_json::Value) -> Option<Prompt> {
        let text = body.get("prompt")?.as_str()?;
        let Some(id) = task_result_from(text) else {
            return Some(Prompt::person(text));
        };
        Some(Prompt::Handback {
            subagent: Subagent {
                id: id.into(),
                kind: Some("task".into()),
            },
            text: text.into(),
        })
    }

    /// The user message the turn opens, which the bridge names.
    fn harness_turn_id(&self, body: &serde_json::Value) -> Option<String> {
        body.get("message_id")
            .and_then(|v| v.as_str())
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    }

    /// The bridge posts the child session as `agent_id` / `agent_type`.
    async fn subagent(&self, body: &serde_json::Value) -> Option<Subagent> {
        super::shared::subagent_of(body)
    }

    async fn render(&self, answer: &HookAnswer) -> serde_json::Value {
        super::shared::render(answer)
    }
}

/// What opencode reports for each fact and what oxplow reads from it: the
/// table in `.context/agent-model.md` ("Subagents"), kept equal by a test.
#[cfg(test)]
const OPENCODE_FACTS: &[(&str, &str, &str)] = &[
    (
        r#"A subagent starts"#,
        r#"`session.created` with `info.parentID`"#,
        r#"`SubagentStart` for the parent, `agent_id` the child session"#,
    ),
    (
        r#"A subagent ends"#,
        r#"`session.idle` / `session.error` of the child"#,
        r#"`SubagentStop` with the child's last message (and the error)"#,
    ),
    (
        r#"A subagent's result returns"#,
        r#"a synthetic `<task id="…" state="…">` part on a new user message"#,
        r#"`Prompt::Handback` from the child, not the person"#,
    ),
    (
        r#"A turn's id"#,
        r#"the user message id"#,
        r#"`harness_turn_id` (`message_id`)"#,
    ),
    (
        r#"An aborted turn"#,
        r#"`session.error` `MessageAbortedError`, then `session.idle`"#,
        r#"`TurnRun.outcome` `interrupted`"#,
    ),
    (
        r#"An errored turn"#,
        r#"`session.error` with another error, then `session.idle`"#,
        r#"`TurnRun.outcome` `errored`, `TurnRun.error` `name: message`"#,
    ),
    (
        r#"What a turn left running"#,
        r#"the children not yet idle at the `session.idle`"#,
        r#"`TurnRun.left_running`"#,
    ),
    (
        r#"A call's duration"#,
        r#"the gap between `tool.execute.before` and `.after`"#,
        r#"`duration_ms` on the PostToolUse"#,
    ),
];

/// The child session id a `<task id="…" state="…">` result part names.
fn task_result_from(text: &str) -> Option<&str> {
    let rest = text.trim_start().strip_prefix("<task id=\"")?;
    rest.split_once('"')
        .map(|(id, _)| id)
        .filter(|id| !id.is_empty())
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

/// `opencode [-m <model>]`, resuming `resume`.
fn command(
    cwd: &str,
    resume: Option<super::shared::Saved<'_>>,
    program: Option<&str>,
    model: Option<&str>,
) -> String {
    let (prog, guard) = program_and_guard(program, "opencode");
    let base = match model {
        Some(model) => format!("{prog} -m {}", shell_escape(model)),
        None => prog,
    };
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
    // A project with no sessions prints nothing (not `[]`) and succeeds.
    let listed = listed.trim();
    if listed.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<serde_json::Value> = serde_json::from_str(listed)
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
    /// which the write guard and the turn's reported files read.
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
  {{ info: {{ role: "assistant", modelID: "gpt-5.6", variant: "high", tokens: {{ input: 100, output: 7, reasoning: 0, cache: {{ read: 4000, write: 50 }} }} }}, parts: [{{ type: "tool", tool: "bash" }}, {{ type: "text", text: "Which one" }}, {{ type: "text", text: "should I use?" }}] }},
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
        // And the model, variant and token counts of that message.
        assert_eq!(stop["payload"]["model"], "gpt-5.6");
        assert_eq!(stop["payload"]["variant"], "high");
        assert_eq!(stop["payload"]["tokens"]["cache"]["read"], 4000);
    }

    /// The bridge forwards a session's compaction as a PostCompact. Run
    /// under bun with `fetch` answering and a stubbed plugin client.
    #[test]
    fn its_bridge_reports_a_compaction() {
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
const hooks = await OxplowHooks({{ client: {{ session: {{ get: async () => ({{ data: {{}} }}) }} }} }});
await hooks.event({{ event: {{ type: "session.compacted", properties: {{ sessionID: "ses1" }} }} }});
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
        let posts: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(posts[0]["event"], "PostCompact");
        assert_eq!(posts[0]["payload"]["session_id"], "ses1");
    }

    /// `session.compacted` names only the session; what started the
    /// compaction is the `compaction` part opencode adds to the session
    /// just before (`auto: false` for `/compact`). The bridge puts it on the
    /// PostCompact as `trigger`, once; a compaction with no such part names
    /// none.
    #[test]
    fn its_bridge_names_what_triggered_a_compaction() {
        let part = |auto: bool| {
            format!(
                r#"await ev("message.part.updated", {{ sessionID: "ses1", part: {{ id: "prt_1", sessionID: "ses1", messageID: "msg_1", type: "compaction", auto: {auto} }}, time: 1 }});"#
            )
        };
        let run = bridge_run(&format!(
            r#"
{manual}
await ev("session.compacted", {{ sessionID: "ses1" }});
{auto}
await ev("session.compacted", {{ sessionID: "ses1" }});
await ev("session.compacted", {{ sessionID: "ses1" }});
"#,
            manual = part(false),
            auto = part(true),
        ));
        let triggers: Vec<_> = run
            .iter()
            .map(|p| {
                assert_eq!(p["event"], "PostCompact");
                p["payload"].get("trigger").cloned()
            })
            .collect();
        assert_eq!(triggers, [Some("manual".into()), Some("auto".into()), None]);
    }

    /// A Stop's model and variant are the turn's model and effort, and its
    /// message's tokens (input, cache reads and writes) the context fill.
    /// The bridge cannot say the window.
    #[test]
    fn a_stop_names_the_model_and_the_variant_as_the_effort() {
        let h = harness("oxplow:opencode", "opencode");
        let run = crate::test_launch::block(h.turn_run(&serde_json::json!({
            "model": "gpt-5.6",
            "variant": "high",
            "tokens": {"input": 100, "output": 7, "reasoning": 0, "cache": {"read": 4000, "write": 50}}
        })));
        assert_eq!(
            run,
            Some(TurnRun {
                model: Some("gpt-5.6".into()),
                effort: Some("high".into()),
                context_tokens: Some(4150),
                context_window: None,
                ..TurnRun::default()
            })
        );
        // A message with neither (the bridge couldn't read it) says nothing.
        assert_eq!(
            crate::test_launch::block(h.turn_run(&serde_json::json!({"model": ""}))),
            None
        );
    }

    /// A Stop that names an error ends the turn as that error, an abort as
    /// interrupted; the children left running ride along.
    #[test]
    fn a_stop_reports_how_the_turn_ended_and_what_it_left_running() {
        let h = harness("oxplow:opencode", "opencode");
        let run = |body: serde_json::Value| crate::test_launch::block(h.turn_run(&body));
        let aborted = run(serde_json::json!({"error": {"name": "MessageAbortedError"}})).unwrap();
        assert_eq!(aborted.outcome, Some(TurnOutcome::Interrupted));
        assert_eq!(aborted.error, None);
        let errored = run(serde_json::json!({
            "error": {"name": "APIError", "message": "overloaded"},
            "background_tasks": [{"id": "ses_c", "type": "subagent", "status": "running", "description": "dig"}]
        }))
        .unwrap();
        assert_eq!(errored.outcome, Some(TurnOutcome::Errored));
        assert_eq!(errored.error.as_deref(), Some("APIError: overloaded"));
        assert_eq!(errored.left_running.as_ref().map(Vec::len), Some(1));
        // A plain finish names no outcome (the ingest reads it as completed).
        assert_eq!(
            run(serde_json::json!({"model": "m"})).unwrap().outcome,
            None
        );
    }

    /// A background subagent's result part opens a hand-back turn from that
    /// child; the user message id is the harness's turn id.
    #[test]
    fn a_background_result_is_a_handback_from_the_child() {
        let h = harness("oxplow:opencode", "opencode");
        let body = serde_json::json!({
            "prompt": "<task id=\"ses_child\" state=\"completed\">\n<summary>x</summary>",
            "message_id": "msg_9"
        });
        match crate::test_launch::block(h.prompt(&body)) {
            Some(Prompt::Handback { subagent, .. }) => {
                assert_eq!(subagent.id, "ses_child");
                assert_eq!(subagent.kind.as_deref(), Some("task"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(h.harness_turn_id(&body).as_deref(), Some("msg_9"));
        let typed = serde_json::json!({"prompt": "fix the bug"});
        assert_eq!(
            crate::test_launch::block(h.prompt(&typed)),
            Some(Prompt::person("fix the bug"))
        );
    }

    /// A child session starts and stops as its parent's subagent, an abort
    /// or error rides the parent's Stop, and a child still running at the
    /// Stop is in its `background_tasks`. Run under bun.
    #[test]
    fn its_bridge_reports_subagents_aborts_and_errors() {
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
const hooks = await OxplowHooks({{ client: {{ session: {{
  get: async () => ({{ data: {{}} }}),
  messages: async () => ({{ data: [{{ info: {{ role: "assistant" }}, parts: [{{ type: "text", text: "done" }}] }}] }}),
}} }} }});
const ev = (type, properties) => hooks.event({{ event: {{ type, properties }} }});
await ev("session.created", {{ info: {{ id: "ses_a", parentID: "ses1", title: "dig" }} }});
await ev("session.created", {{ info: {{ id: "ses_b", parentID: "ses1", title: "other" }} }});
await ev("session.idle", {{ sessionID: "ses_a" }});
await ev("session.error", {{ sessionID: "ses1", error: {{ name: "MessageAbortedError", data: {{ message: "aborted" }} }} }});
await ev("session.idle", {{ sessionID: "ses1" }});
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
        let events: Vec<&str> = posts.iter().map(|p| p["event"].as_str().unwrap()).collect();
        assert_eq!(
            events,
            ["SubagentStart", "SubagentStart", "SubagentStop", "Stop"]
        );
        let h = harness("oxplow:opencode", "opencode");
        let sub = |i: usize| crate::test_launch::block(h.subagent(&posts[i]["payload"])).unwrap();
        assert_eq!(sub(0).id, "ses_a");
        assert_eq!(sub(2).id, "ses_a");
        assert_eq!(posts[2]["payload"]["session_id"], "ses1");
        assert_eq!(posts[2]["payload"]["last_assistant_message"], "done");
        let run = crate::test_launch::block(h.turn_run(&posts[3]["payload"])).unwrap();
        assert_eq!(run.outcome, Some(TurnOutcome::Interrupted));
        let left = run.left_running.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, "ses_b");
    }

    /// The fact table in agent-model.md says what the bridge reports and
    /// oxplow reads, row for row as the code's.
    #[test]
    fn the_documented_fact_table_is_the_adapters() {
        let doc = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.context/agent-model.md"),
        )
        .unwrap();
        for (fact, reports, reads) in OPENCODE_FACTS {
            let row = format!("| {fact} | {reports} | {reads} |");
            assert!(doc.contains(&row), "agent-model.md lacks the row {row}");
        }
    }

    /// What opencode 1.17.4 emits when it waits on the person: a permission
    /// asked (a Notification of a permission prompt naming it) and answered
    /// (a PostToolUse naming no tool, which clears the wait). Only
    /// forwarded: the bridge answers nothing. A question is its tool call:
    /// the question events post nothing (the tool hooks already carry the
    /// wait), except a rejected question, which ends the call (its tool
    /// throws, so no `tool.execute.after` follows). Run under bun.
    #[test]
    fn its_bridge_reports_a_wait_on_a_permission_or_a_question() {
        let run = bridge_run(
            r#"
await ev("permission.asked", { id: "per_1", sessionID: "ses1", permission: "bash", patterns: ["rm -rf build"], metadata: {}, always: [] });
await ev("permission.replied", { sessionID: "ses1", requestID: "per_1", reply: "once" });
await hooks["tool.execute.before"]({ tool: "question", sessionID: "ses1", callID: "call_q" }, { args: { questions: [{ question: "Which branch?", header: "Branch", options: [] }] } });
await ev("question.asked", { id: "que_1", sessionID: "ses1", questions: [{ question: "Which branch?", header: "Branch", options: [] }], tool: { messageID: "msg_1", callID: "call_q" } });
await ev("question.rejected", { sessionID: "ses1", requestID: "que_1" });
"#,
        );
        let events: Vec<&str> = run.iter().map(|p| p["event"].as_str().unwrap()).collect();
        assert_eq!(
            events,
            ["Notification", "PostToolUse", "PreToolUse", "PostToolUse"]
        );
        assert_eq!(run[0]["payload"]["notification_type"], "permission_prompt");
        assert_eq!(run[0]["payload"]["message"], "bash: rm -rf build");
        assert_eq!(run[0]["payload"]["session_id"], "ses1");
        // The permission's answer names no tool: it only clears the wait.
        assert!(run[1]["payload"].get("tool_name").is_none());
        // The question is one call, asked and ended under the tool's id.
        assert_eq!(run[2]["payload"]["tool_name"], "AskUserQuestion");
        assert_eq!(run[2]["payload"]["tool_use_id"], "call_q");
        assert_eq!(
            run[2]["payload"]["tool_input"]["questions"][0]["question"],
            "Which branch?"
        );
        assert_eq!(run[3]["payload"]["tool_name"], "AskUserQuestion");
        assert_eq!(run[3]["payload"]["tool_use_id"], "call_q");
    }

    /// The recorded question (`fixtures/opencode/question.jsonl`): opencode's
    /// question tool hooks and its `question.asked` / `question.replied`
    /// events all report the one call. Fed the recording's raw inputs, the
    /// bridge posts the fixture's two tool hooks and nothing for the
    /// events.
    #[test]
    fn its_bridge_reports_an_answered_question_as_one_call() {
        let questions = serde_json::json!([{"question": "Which function would you like to add to calc.py?", "header": "Choose a function", "options": [{"label": "div", "description": "Division function"}, {"label": "pow", "description": "Power/exponentiation function"}]}]);
        let session = "ses_e378b4fb3ffe9OTzA7zAg24TOq";
        let call = "toolu_01QjZXqw5gT9YXLhcvCgjNyu";
        let asked = serde_json::json!({"id": "que_1c874b8af001qgLmdF79Z6rcQ8", "sessionID": session, "questions": questions, "tool": {"messageID": "msg_1c874b074001uEH1Axhii7AQQz", "callID": call}});
        let replied = serde_json::json!({"sessionID": session, "requestID": "que_1c874b8af001qgLmdF79Z6rcQ8", "answers": [["pow"]]});
        let before_input =
            serde_json::json!({"tool": "question", "sessionID": session, "callID": call});
        let before_output = serde_json::json!({"args": {"questions": questions}});
        let after_input = serde_json::json!({"tool": "question", "sessionID": session, "callID": call, "args": {"questions": questions}});
        let after_output = serde_json::json!({"title": "Asked 1 question", "output": "User has answered your questions: \"Which function would you like to add to calc.py?\"=\"pow\". You can now continue with the user's answers in mind.", "metadata": {"answers": [["pow"]], "truncated": false}});
        let run = bridge_run(&format!(
            r#"
await hooks["tool.execute.before"]({before_input}, {before_output});
await ev("question.asked", {asked});
await ev("question.replied", {replied});
await hooks["tool.execute.after"]({after_input}, {after_output});
"#
        ));
        let fixture = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/opencode/question.jsonl"),
        )
        .unwrap();
        let recorded: Vec<serde_json::Value> = fixture
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .filter(|l| l["event"] == "PreToolUse" || l["event"] == "PostToolUse")
            .collect();
        // The bridge times the call itself; the recording's duration is
        // the person's thinking time.
        let without_duration = |mut v: serde_json::Value| {
            v["payload"].as_object_mut().unwrap().remove("duration_ms");
            v
        };
        assert_eq!(
            run.into_iter().map(without_duration).collect::<Vec<_>>(),
            recorded
                .into_iter()
                .map(without_duration)
                .collect::<Vec<_>>()
        );
    }

    /// Run the bridge under bun with `fetch` caught and a stub client: the
    /// `steps` (JS; `hooks` the plugin's hooks, `ev(type, properties)` an
    /// event) run in order, and what the bridge posted comes back as
    /// `{event, payload}` pairs, its raw recording excluded.
    fn bridge_run(steps: &str) -> Vec<serde_json::Value> {
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
const hooks = await OxplowHooks({{ client: {{ session: {{ get: async () => ({{ data: {{}} }}), messages: async () => ({{ data: [] }}) }} }} }});
const ev = (type, properties) => hooks.event({{ event: {{ type, properties }} }});
{steps}
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
        serde_json::from_slice(&out.stdout).unwrap()
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

    /// A project with no sessions makes `session list --format json` print
    /// nothing at all, exit code 0 (not `[]`; recorded with 1.17.4): that is
    /// an empty list, not an error.
    #[test]
    fn a_blank_session_list_is_no_sessions() {
        for blank in ["", "\n", "  \r\n "] {
            assert!(sessions_in(blank, "/repo/wt", 10).unwrap().is_empty());
        }
        assert!(sessions_in("[]", "/repo/wt", 10).unwrap().is_empty());
        // A cut-off list is still an error.
        assert!(sessions_in("[{\"id\":\"ses_a\"", "/repo/wt", 10).is_err());
    }

    /// It lists the sessions of the project the program runs in, and the
    /// project is the worktree's: the CLI runs there, and a program that
    /// prints nothing lists none.
    #[cfg(unix)]
    #[tokio::test]
    async fn its_session_list_runs_in_the_worktree_and_may_be_empty() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let wt = tmp.path().join("wt");
        fs::create_dir(&wt).unwrap();
        let wt = wt.canonicalize().unwrap();
        let bin = tmp.path().join("opencode");
        // It lists a session of the directory it runs in.
        fs::write(
            &bin,
            "#!/bin/sh\nprintf '[{\"id\":\"ses_here\",\"title\":\"H\",\"updated\":1,\"created\":1,\"directory\":\"%s\"}]' \"$(pwd -P)\"\n",
        )
        .unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        let h = harness("oxplow:opencode", "opencode");
        let query = oxplow_domain::agent::harness::ConversationQuery {
            cwd: wt.clone(),
            home: None,
            search_path: vec![tmp.path().to_path_buf()],
            limit: 10,
        };
        let found = h.conversations(&query).await.unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].id, "ses_here");
        // A project with no sessions: it prints nothing and succeeds.
        fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
        assert!(h.conversations(&query).await.unwrap().is_empty());
    }

    /// Through its CLI: a program that prints the list.
    #[cfg(unix)]
    #[tokio::test]
    async fn its_conversations_come_from_its_cli() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let wt = tmp.path().join("wt");
        fs::create_dir(&wt).unwrap();
        let wt = wt.canonicalize().unwrap();
        let bin = tmp.path().join("opencode");
        fs::write(
            &bin,
            "#!/bin/sh\n[ \"$1 $2 $3 $4\" = 'session list --format json' ] || exit 2\nprintf '[{\"id\":\"ses_a\",\"title\":\"A\",\"updated\":1,\"created\":1,\"directory\":\"%s\"}]' \"$(pwd -P)\"\n",
        )
        .unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        let h = harness("oxplow:opencode", "opencode");
        let query = oxplow_domain::agent::harness::ConversationQuery {
            cwd: wt,
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

    /// The bridge records raw only while a hook recording is on.
    #[test]
    fn the_bridge_records_raw_only_while_a_hook_recording_is_on() {
        assert_eq!(
            raw_recording_env(Some("/tmp/hooks.jsonl".into())),
            Some(("OXPLOW_OPENCODE_RAW".into(), "1".into()))
        );
        assert_eq!(raw_recording_env(Some("".into())), None);
        assert_eq!(raw_recording_env(None), None);
        let js = include_str!("../assets/opencode-hooks.js");
        assert!(js.contains("OXPLOW_OPENCODE_RAW") && js.contains("opencode.raw"));
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

    /// opencode picks the model itself unless the person set one: oxplow
    /// forces none (a model it named once vanished from the provider's
    /// list, and opencode then ran a different one with no error).
    #[test]
    fn with_no_model_configured_it_passes_none() {
        let h = harness("oxplow:opencode", "opencode");
        for config in [serde_json::json!({}), serde_json::json!({ "model": "  " })] {
            let l = launch_in(h.as_ref(), None, None, &config);
            let (command, _) = crate::test_launch::pty(&l.launch);
            assert!(command.contains("exec "), "{command}");
            assert!(!command.contains(" -m "), "{command}");
            assert!(!command.contains(" -s "), "{command}");
        }
        assert_eq!(h.settings()[0].placeholder, "provider/model");
        assert!(h.settings()[0].hint.contains("Blank uses"));
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
