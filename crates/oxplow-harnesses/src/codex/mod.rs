//! The `codex` harness: a thread on Codex's shared app-server daemon, in
//! oxplow's chat (`.context/agent-model.md` "Codex: an app-server
//! client"). Its launch is one request that opens the thread — started,
//! resumed or forked — with oxplow's MCP server (the session's bearer as a
//! header) and system prompt in the thread's own config; oxplow speaks to
//! it as a client of the daemon, so `codex agents`, Codex's own TUI
//! (`codex resume <id>`) and its other clients see the same thread. Its
//! skills are the one thing on disk: Codex finds them only beside the
//! directory it runs in, so they're in the worktree's `.agents/skills/`.

pub mod app_server;

use std::path::PathBuf;

use oxplow_domain::agent::harness::{
    AgentHarness, ConversationQuery, HarnessError, Interact, Launch, LaunchInput, LaunchSpec,
    NativeConversation, RuntimeRoots, Transcript,
};
use oxplow_domain::agent::observe::HookAnswer;
use oxplow_domain::agent::text::{AgentText, Text};
use oxplow_domain::agent::tool::ToolUse;
use serde_json::{json, Value};

use super::shared::{
    in_shell, program_and_guard, refresh_worktree_skills, runtime, saved, shell_escape,
    toml_string, write_worktree_skills, Saved, METADATA_TIMEOUT,
};
use super::Named;

pub(super) struct Codex(pub(super) Named);

/// The turns a resumed thread brings back to show, newest first.
const RESUMED_TURNS: u32 = 50;

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
            transcript: Transcript::Structured,
            acp_agent: false,
        }
    }

    async fn launch(&self, input: &LaunchInput) -> Result<Launch, HarnessError> {
        let home = input.home.as_deref().ok_or_else(|| {
            HarnessError::Unavailable(
                "a Codex session runs on this machine's Codex app server, and this stream isn't on this machine".into(),
            )
        })?;
        write_worktree_skills(&input.workspace, &skills_and_commands(&input.text))
            .map_err(runtime)?;
        let (method, params) = open_thread(input);
        Ok(Launch {
            spec: LaunchSpec::CodexAppServer {
                socket: app_server::control_socket(&app_server::codex_home(home)),
                program: input.resolve_program("codex").map(PathBuf::from),
                method: method.into(),
                params,
            },
            transcript_missing: false,
        })
    }

    async fn refresh_text(
        &self,
        roots: &RuntimeRoots,
        text: &AgentText,
    ) -> Result<(), HarnessError> {
        refresh_worktree_skills(&roots.workspaces, &skills_and_commands(text)).map_err(runtime)
    }

    /// Its threads begun in the worktree, from the daemon (`thread/list`);
    /// a subagent's thread isn't one to resume.
    async fn conversations(
        &self,
        query: &ConversationQuery,
    ) -> Result<Vec<NativeConversation>, HarnessError> {
        let home = query.home.as_deref().ok_or_else(|| {
            HarnessError::Unavailable("Codex's threads are on another machine".into())
        })?;
        let program = query.resolve_program("codex").map(PathBuf::from);
        let socket = app_server::control_socket(&app_server::codex_home(home));
        let list = async {
            let (server, _) = app_server::AppServer::connect(&socket, program.as_deref()).await?;
            server
                .request(
                    "thread/list",
                    json!({ "cwd": query.cwd, "limit": query.limit, "archived": false }),
                )
                .await
        };
        let listed = tokio::time::timeout(METADATA_TIMEOUT, list)
            .await
            .map_err(|_| {
                HarnessError::Unavailable("Codex's app server didn't answer in time".into())
            })?
            .map_err(HarnessError::Unavailable)?;
        Ok(threads_of(&listed))
    }

    /// Codex's own TUI attached to the daemon's thread: `codex resume` with
    /// `--cd` for the worktree. No `-c` override, which would run it
    /// embedded instead of attaching.
    async fn native_view(
        &self,
        query: &ConversationQuery,
        conversation: &str,
    ) -> Result<String, HarnessError> {
        let cwd = query.cwd.to_string_lossy();
        let (program, guard) =
            program_and_guard(query.resolve_program("codex").as_deref(), "codex");
        let resume = format!(
            "exec {program} resume --cd {} {}",
            shell_escape(&cwd),
            shell_escape(conversation)
        );
        Ok(in_shell(&cwd, &guard, &resume))
    }

    /// It posts no hooks: its calls arrive as the thread's items, mapped
    /// where the client reads them.
    async fn tool_use(&self, _: &Value) -> Option<ToolUse> {
        None
    }

    async fn render(&self, answer: &HookAnswer) -> Value {
        super::shared::render(answer)
    }
}

/// The request that opens the session's thread: `thread/start`, or
/// `thread/resume` / `thread/fork` of the saved one. Each carries the
/// worktree, oxplow's system prompt as its developer instructions and
/// oxplow's MCP server with the session's bearer — per thread, so the
/// person's own Codex config stays theirs. Its sandbox is the client's to
/// set: what the thread may write when it opens.
fn open_thread(input: &LaunchInput) -> (&'static str, Value) {
    let ep = &input.endpoints;
    let mut params = json!({
        "cwd": input.workspace,
        "config": {
            "mcp_servers": {
                "oxplow": {
                    "url": ep.mcp_endpoint_url,
                    "http_headers": {"Authorization": format!("Bearer {}", ep.hook_token)},
                },
            },
        },
    });
    if let Some(prompt) = input.system_prompt.as_deref() {
        params["developerInstructions"] = json!(prompt);
    }
    match saved(input) {
        None => ("thread/start", params),
        Some(Saved { id, fork: true }) => {
            params["threadId"] = json!(id);
            ("thread/fork", params)
        }
        Some(Saved { id, fork: false }) => {
            params["threadId"] = json!(id);
            params["initialTurnsPage"] =
                json!({"itemsView": "full", "sortDirection": "desc", "limit": RESUMED_TURNS});
            ("thread/resume", params)
        }
    }
}

/// Codex has no slash-command files a project can carry (its prompts live
/// in the person's `~/.codex/prompts`, which oxplow doesn't write), so each
/// command is a skill named `oxplow-<command>` in the worktree's skills
/// folder, described by the command's own description and holding its
/// prompt; `$oxplow-<command>` runs it.
fn skills_and_commands(text: &AgentText) -> Vec<Text> {
    let mut all = text.skills.clone();
    all.extend(text.commands.iter().map(|command| {
        let (description, prompt) = split_description(&command.body);
        let name = format!("oxplow-{}", command.name);
        Text {
            body: format!(
                "---\nname: {name}\ndescription: {}\n---\n\n{prompt}\n",
                toml_string(
                    &description.unwrap_or_else(|| format!("The oxplow {} command.", command.name))
                )
            ),
            name,
            references: Vec::new(),
        }
    }));
    all
}

/// A command's frontmatter `description:` and the prompt after it.
fn split_description(asset: &str) -> (Option<String>, String) {
    let Some((front, body)) = asset
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
    else {
        return (None, asset.trim().to_string());
    };
    let description = front
        .lines()
        .find_map(|l| l.strip_prefix("description:"))
        .map(|d| d.trim().to_string());
    (description, body.trim().to_string())
}

/// The threads a `thread/list` result holds, a subagent's left out.
fn threads_of(listed: &serde_json::Value) -> Vec<NativeConversation> {
    let at = |t: &serde_json::Value, key: &str| {
        t.get(key)
            .and_then(|v| v.as_i64())
            .map(|secs| oxplow_domain::Timestamp::from_unix_ms(secs.saturating_mul(1000)))
    };
    let text = |t: &serde_json::Value, key: &str| {
        t.get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    listed
        .get("data")
        .and_then(|d| d.as_array())
        .into_iter()
        .flatten()
        .filter(|t| t.get("parentThreadId").is_none_or(|p| p.is_null()))
        .filter_map(|t| {
            Some(NativeConversation {
                id: t.get("id")?.as_str()?.to_string(),
                title: text(t, "name"),
                first_prompt: text(t, "preview"),
                started_at: at(t, "createdAt"),
                updated_at: at(t, "updatedAt"),
                size_bytes: None,
                parent: text(t, "forkedFromId"),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::test_launch::{harness, launch_at_home};

    fn opened(l: &crate::test_launch::Launched) -> (&std::path::Path, &str, &Value) {
        let LaunchSpec::CodexAppServer {
            socket,
            method,
            params,
            ..
        } = &l.launch.spec
        else {
            panic!("not an app-server launch: {:?}", l.launch.spec)
        };
        (socket, method, params)
    }

    /// A fresh session starts a thread in its worktree on the daemon under
    /// the person's Codex home, with oxplow's MCP server and the session's
    /// bearer in the thread's own config.
    #[test]
    fn a_fresh_session_starts_a_thread_on_the_daemon() {
        let h = harness("oxplow:codex-cli", "codex");
        let l = launch_at_home(h.as_ref(), None, false, Some("Line one\n\"quoted\""));
        let (socket, method, params) = opened(&l);
        assert_eq!(
            socket,
            l.project
                .join("home/.codex/app-server-control/app-server-control.sock")
        );
        assert_eq!(method, "thread/start");
        assert_eq!(params["cwd"], json!(l.workspace));
        assert_eq!(params["developerInstructions"], "Line one\n\"quoted\"");
        let oxplow = &params["config"]["mcp_servers"]["oxplow"];
        assert_eq!(oxplow["url"], "http://127.0.0.1:9/mcp");
        assert_eq!(
            oxplow["http_headers"]["Authorization"],
            "Bearer secret-bearer"
        );
        assert!(params.get("threadId").is_none());
    }

    /// A saved conversation is resumed with its latest turns to show, or
    /// forked; neither starts a new thread.
    #[test]
    fn a_saved_conversation_resumes_or_forks_its_thread() {
        let h = harness("oxplow:codex-cli", "codex");
        let l = launch_at_home(h.as_ref(), Some("t-1"), false, None);
        let (_, method, params) = opened(&l);
        assert_eq!(method, "thread/resume");
        assert_eq!(params["threadId"], "t-1");
        assert_eq!(params["initialTurnsPage"]["itemsView"], "full");
        assert!(params["config"]["mcp_servers"]["oxplow"].is_object());
        let l = launch_at_home(h.as_ref(), Some("t-1"), true, None);
        let (_, method, params) = opened(&l);
        assert_eq!(method, "thread/fork");
        assert_eq!(params["threadId"], "t-1");
        assert!(params.get("initialTurnsPage").is_none());
    }

    /// A stream not on this machine has no home here: there is no daemon
    /// to reach.
    #[test]
    fn a_stream_with_no_home_here_cant_launch() {
        let h = harness("oxplow:codex-cli", "codex");
        let err = crate::test_launch::try_launch_in(h.as_ref(), &json!({}))
            .err()
            .unwrap();
        assert!(matches!(err, HarnessError::Unavailable(_)), "{err}");
    }

    /// Each slash command is a worktree skill carrying the command's
    /// prompt.
    #[test]
    fn launch_hands_codex_its_commands_as_skills() {
        let h = harness("oxplow:codex-cli", "codex");
        let l = launch_at_home(h.as_ref(), None, false, None);
        let skill =
            std::fs::read_to_string(l.workspace.join(".agents/skills/oxplow-configure/SKILL.md"))
                .unwrap();
        assert!(
            skill.starts_with("---\nname: oxplow-configure\ndescription: \""),
            "{skill}"
        );
        let text = oxplow_agent_text::core_text();
        let configure = text
            .commands
            .iter()
            .find(|c| c.name == "configure")
            .unwrap();
        let (_, prompt) = split_description(&configure.body);
        assert!(skill.contains(&prompt));
    }

    /// Its threads come from the daemon's `thread/list` for the worktree,
    /// a subagent's left out; a daemon that refuses is unavailable.
    #[tokio::test]
    async fn its_conversations_come_from_the_daemon() {
        // Short: a unix socket's path has a length limit.
        let home = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
        let refuse = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let r = refuse.clone();
        let fake = app_server::fake::FakeAppServer::start(
            &home.path().join(".codex"),
            move |method, params| {
                if r.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err("no".into());
                }
                assert_eq!(
                    (method, &params["cwd"]),
                    ("thread/list", &json!("/repo/wt"))
                );
                Ok(json!({"data": [
                {"id":"t1","name":"Login fix","preview":"fix the login","createdAt":100,"updatedAt":200,"parentThreadId":null,"forkedFromId":"t0"},
                {"id":"t2","name":null,"preview":"","createdAt":1,"updatedAt":2,"parentThreadId":"t1"}
            ], "nextCursor": null}))
            },
        );
        let h = harness("oxplow:codex-cli", "codex");
        let query = ConversationQuery {
            cwd: "/repo/wt".into(),
            home: Some(home.path().to_path_buf()),
            search_path: vec![],
            limit: 10,
        };
        let found = h.conversations(&query).await.unwrap();
        assert_eq!(found.len(), 1, "the subagent's thread isn't listed");
        assert_eq!(
            (
                found[0].id.as_str(),
                found[0].title.as_deref(),
                found[0].first_prompt.as_deref(),
                found[0].parent.as_deref()
            ),
            ("t1", Some("Login fix"), Some("fix the login"), Some("t0"))
        );
        assert_eq!(
            found[0].updated_at,
            Some(oxplow_domain::Timestamp::from_unix_ms(200_000))
        );
        refuse.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            h.conversations(&query).await,
            Err(HarnessError::Unavailable(_))
        ));
        drop(fake);
    }

    /// Its own view of a conversation attaches to the daemon's thread:
    /// `codex resume --cd <cwd> <id>` in the worktree, the program as
    /// resolved, everything shell-escaped, and no `-c` override (one would
    /// run Codex embedded instead of attaching).
    #[tokio::test]
    async fn its_native_view_is_codex_resume_on_the_thread() {
        let bin = tempfile::tempdir().unwrap();
        std::fs::write(bin.path().join("codex"), "").unwrap();
        let h = harness("oxplow:codex-cli", "codex");
        let query = ConversationQuery {
            cwd: "/repo/it's wt".into(),
            home: None,
            search_path: vec![bin.path().to_path_buf()],
            limit: 1,
        };
        let cmd = h.native_view(&query, "thr-1; rm -rf /").await.unwrap();
        let program = bin.path().join("codex");
        let cwd = "/repo/it's wt";
        let want = crate::shared::in_shell(
            cwd,
            "",
            &format!(
                "exec {} resume --cd {} {}",
                crate::shared::shell_escape(&program.to_string_lossy()),
                crate::shared::shell_escape(cwd),
                crate::shared::shell_escape("thr-1; rm -rf /"),
            ),
        );
        assert_eq!(cmd, want);
        for flag in [" -c ", "--config", "--remote"] {
            assert!(!cmd.contains(flag), "{flag}: {cmd}");
        }
        // Without Codex on the path, the launch says so rather than failing blind.
        let none = ConversationQuery {
            search_path: vec![],
            ..query
        };
        let cmd = h.native_view(&none, "t1").await.unwrap();
        assert!(cmd.contains("not found on PATH"), "{cmd}");
    }

    /// The other harnesses have no view of their own to attach.
    #[tokio::test]
    async fn the_others_have_no_native_view() {
        let query = ConversationQuery {
            cwd: "/repo".into(),
            home: None,
            search_path: vec![],
            limit: 1,
        };
        for (entry, id) in [
            ("oxplow:claude-code", "claude"),
            ("oxplow:opencode", "opencode"),
        ] {
            assert!(matches!(
                super::super::built_in(entry, id, id)
                    .unwrap()
                    .native_view(&query, "c1")
                    .await,
                Err(HarnessError::Unsupported { .. })
            ));
        }
    }

    #[test]
    fn a_toml_string_escapes_what_would_end_or_break_the_line() {
        assert_eq!(toml_string("a\nb\t\"c\"\\"), "\"a\\nb\\t\\\"c\\\"\\\\\"");
    }

    /// Codex finds skills only beside the directory it runs in: they land
    /// in its worktree's `.agents/skills`, self-ignored, and nothing else
    /// is written for it — its MCP server rides the thread's config.
    #[test]
    fn launch_puts_the_skills_in_the_worktree() {
        let h = harness("oxplow:codex-cli", "codex");
        let l = launch_at_home(h.as_ref(), None, false, None);
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
}
