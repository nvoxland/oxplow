//! An agent harness: what runs in an agent session.
//!
//! Its **Launch** seam (`launch`) says how the session's process comes to
//! exist — a PTY command, or an ACP agent's program — writing whatever
//! runtime files its agent reads under `.oxplow/runtime/` (never the
//! worktree). Starting a session only spawns: nothing here prompts.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::observe::{HookAnswer, OtlpRecord, Prompt, TokenReading, Turn};
use super::text::AgentText;
use super::tool::{Subagent, ToolUse};
use crate::ids::{AgentSessionId, StreamId, ThreadId};

/// How a person interacts with a harness's session — what the UI reads,
/// never something core invokes (the Interact seam).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interact {
    pub transcript: Transcript,
    /// Its sessions each name one of the project's ACP agents
    /// (`agent_session.acp_agent`), the program its launch runs.
    pub acp_agent: bool,
}

/// What the person reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transcript {
    /// The harness's own terminal.
    Terminal,
    /// oxplow's structured transcript (an ACP agent's, a Codex thread's).
    Structured,
}

/// The session a launch is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionIds {
    pub stream: StreamId,
    pub thread: ThreadId,
    pub session: AgentSessionId,
}

/// Where the agent reaches oxplow: the control plane's hook and OTLP
/// receivers, its MCP endpoint, and the bearer for all three.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoints {
    pub hook_base_url: String,
    pub mcp_endpoint_url: String,
    pub otlp_base_url: String,
    pub hook_token: String,
}

/// What a launch knows. On the wire (a provider harness's `launch`), these
/// fields as they are.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchInput {
    pub session: SessionIds,
    /// The stream's worktree: where the agent runs.
    pub workspace: PathBuf,
    /// The project: where `.oxplow/runtime/` is.
    pub project_dir: PathBuf,
    pub endpoints: Endpoints,
    /// `OXPLOW_*` — what every process oxplow starts is told about itself.
    pub identity_env: Vec<(String, String)>,
    /// oxplow's system prompt for the session.
    pub system_prompt: Option<String>,
    /// The harness's own session to resume.
    pub resume: Option<String>,
    /// Resume `resume` as a new conversation copied from it (a fork), not
    /// the saved one itself.
    #[serde(default)]
    pub fork: bool,
    /// The skills and commands offered now.
    pub text: AgentText,
    /// The harness's configuration (`agentConfig.<harness>`), or for an
    /// ACP agent its adapter's resolved program.
    pub config: serde_json::Value,
    /// The person's home, where a harness keeps its own sessions.
    pub home: Option<PathBuf>,
    /// Where programs are looked for, in order: the PATH and the
    /// well-known install dirs, so a GUI-launched oxplow's thin PATH
    /// doesn't matter ([`Self::resolve_program`]).
    pub search_path: Vec<PathBuf>,
}

impl LaunchInput {
    /// Where `bin` is: its absolute path in [`Self::search_path`], or
    /// `bin` itself when it's a path to a file.
    pub fn resolve_program(&self, bin: &str) -> Option<String> {
        resolve_program_in(bin, &self.search_path).map(|p| p.to_string_lossy().into_owned())
    }
}

/// The first file named `bin` in `dirs`, in order; `bin` itself when it
/// already names a path (has a separator) and is a file — as `command -v`.
pub fn resolve_program_in(bin: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let as_path = Path::new(bin);
    if as_path.components().count() > 1 {
        return as_path.is_file().then(|| as_path.to_path_buf());
    }
    dirs.iter().map(|dir| dir.join(bin)).find(|c| c.is_file())
}

/// Where a harness's runtimes can be on disk: the project, whose
/// `.oxplow/runtime/` holds what it's pointed at, and every stream's
/// worktree, for a harness that finds its skills only beside the
/// directory it runs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRoots {
    pub project_dir: PathBuf,
    pub workspaces: Vec<PathBuf>,
}

/// How the session's agent is reached. On the wire, `{ "kind": "pty",
/// "command", "env" }`, `{ "kind": "acp", "program", "args", "env",
/// "system_prompt_via_meta" }` or `{ "kind": "codex_app_server", "socket",
/// "program", "method", "params" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchSpec {
    /// A shell command run in a PTY (`sh -lc <command>`), with `env` set
    /// in its environment. Anything secret (the session's bearer) goes in
    /// `env`, never in `command`, whose text any process can list.
    Pty {
        command: String,
        #[serde(default)]
        env: Vec<(String, String)>,
    },
    /// An ACP agent's program, spoken to over its stdio.
    Acp {
        program: PathBuf,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: Vec<(String, String)>,
        /// The system prompt rides `_meta.systemPrompt.append` on
        /// `session/new` rather than ahead of the first prompt.
        #[serde(default)]
        system_prompt_via_meta: bool,
    },
    /// A thread on Codex's shared app-server daemon, opened over its
    /// control socket with one request (`thread/start`, `thread/resume` or
    /// `thread/fork` and its params, which carry the session's bearer).
    CodexAppServer {
        /// The daemon's control socket.
        socket: PathBuf,
        /// `codex`, which starts the daemon when it isn't running.
        program: Option<PathBuf>,
        method: String,
        params: serde_json::Value,
    },
}

/// A launch: how to start the process, and whether the harness found the
/// saved conversation it resumes gone (its transcript is not where it
/// looked). The launch resumes regardless — the harness has the final say
/// — and the flag only lets a failed resume say why. Nothing forgets the
/// id; only `agent_session.start_fresh` does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launch {
    pub spec: LaunchSpec,
    #[serde(default)]
    pub transcript_missing: bool,
}

/// One project setting a harness reads (`agentConfig.<harness>.<key>`), as
/// Settings shows it: a text value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessSetting {
    pub key: String,
    pub title: String,
    /// What it does, under the field.
    #[serde(default)]
    pub hint: String,
    /// An example value, in the empty field.
    #[serde(default)]
    pub placeholder: String,
}

/// What a harness provider declares of itself beyond its features (its
/// `agent_harness` capability's `data`): what a built-in answers from
/// [`AgentHarness::env_markers`] and [`AgentHarness::settings`]. A
/// project's own instruction files are the harness's to read, never
/// oxplow's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HarnessData {
    pub env_markers: Vec<String>,
    pub settings: Vec<HarnessSetting>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HarnessError {
    /// Its runtime files couldn't be written.
    #[error("writing the agent's runtime: {0}")]
    Runtime(String),
    /// Its configuration doesn't hold.
    #[error("{0}")]
    Config(String),
    /// It doesn't do what was asked (a verb it has no feature for).
    #[error("{harness} can't {what}")]
    Unsupported { harness: String, what: String },
    /// It does, but couldn't now (its CLI failed, its storage is unreadable).
    #[error("{0}")]
    Unavailable(String),
}

/// Which of a harness's saved conversations to list: those begun in `cwd`
/// on the machine whose home is `home`, newest first, at most `limit`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationQuery {
    pub cwd: PathBuf,
    pub home: Option<PathBuf>,
    /// Where its program is looked for (as [`LaunchInput::search_path`]).
    pub search_path: Vec<PathBuf>,
    pub limit: usize,
}

impl ConversationQuery {
    pub fn resolve_program(&self, bin: &str) -> Option<String> {
        resolve_program_in(bin, &self.search_path).map(|p| p.to_string_lossy().into_owned())
    }
}

/// One of a harness's saved conversations, as it can tell. What it can't
/// tell is `None`, never guessed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct NativeConversation {
    /// Its id, what a resume names.
    pub id: String,
    /// Its name, set by the person or generated by the harness.
    pub title: Option<String>,
    /// The first thing the person said in it.
    pub first_prompt: Option<String>,
    pub started_at: Option<crate::Timestamp>,
    pub updated_at: Option<crate::Timestamp>,
    #[specta(type = Option<crate::JsNumber>)]
    pub size_bytes: Option<u64>,
    /// The conversation it was forked or spawned from.
    pub parent: Option<String>,
}

/// One harness implementation, registered under the key an agent
/// session's `harness` names: built in, or a provider process
/// (`.context/agent-model.md`, `.context/providers.md`).
///
/// What every harness answers: who it is, how its sessions start and are
/// shown, how its tool calls read in oxplow's vocabulary, and how its hooks
/// expect an answer. The rest is what a harness offers beyond that — its
/// own environment markers, settings, a transcript or
/// telemetry core can read — each with a default of "none", so a harness
/// implements only what it has.
#[async_trait]
pub trait AgentHarness: Send + Sync {
    /// The registry key (`claude`): what `agent_session.harness` names.
    fn id(&self) -> &str;
    /// How a person names it.
    fn title(&self) -> &str;
    fn interact(&self) -> Interact;
    /// How the session's process starts. Spawns only — never prompts.
    async fn launch(&self, input: &LaunchInput) -> Result<Launch, HarnessError>;
    /// One of its tool hooks' bodies (a PreToolUse or PostToolUse) mapped
    /// onto oxplow's vocabulary; `None` when the body names no tool.
    async fn tool_use(&self, body: &serde_json::Value) -> Option<ToolUse>;
    /// `answer` in the shape its hooks expect back.
    async fn render(&self, answer: &HookAnswer) -> serde_json::Value;

    /// Its process's markers an agent or terminal must not inherit from
    /// oxplow's own environment (when oxplow itself runs in one).
    fn env_markers(&self) -> Vec<String> {
        Vec::new()
    }
    /// The project settings its `launch` reads from its `agentConfig`
    /// entry, which Settings → Agents offers for it.
    fn settings(&self) -> Vec<HarnessSetting> {
        Vec::new()
    }
    /// Rewrite the skills and commands of its runtimes already on disk
    /// under `roots` to `text`, creating none: an agent that outlives a
    /// launch (running across an upgrade, or resumed) reads what's offered
    /// now. Nothing to do for a harness that keeps none on disk.
    async fn refresh_text(
        &self,
        _roots: &RuntimeRoots,
        _text: &AgentText,
    ) -> Result<(), HarnessError> {
        Ok(())
    }
    /// The recordable turns in a chunk of its transcript; none when it
    /// keeps no transcript core reads.
    async fn turns(&self, _transcript: &str) -> Vec<Turn> {
        Vec::new()
    }
    /// The token counts in the records of one telemetry export — all of
    /// them at once, one call per export; none for records it doesn't
    /// recognize.
    async fn token_readings(&self, _records: &[OtlpRecord]) -> Vec<TokenReading> {
        Vec::new()
    }
    /// What its prompt hook's body is: a person's prompt (its `prompt`
    /// text, the default) or a subagent's hand-back; `None` for a body
    /// with no prompt.
    async fn prompt(&self, body: &serde_json::Value) -> Option<Prompt> {
        body.get("prompt")
            .and_then(|p| p.as_str())
            .map(|text| Prompt::Person { text: text.into() })
    }
    /// The subagent a `SubagentStart` / `SubagentStop` hook names; none for
    /// a harness that reports no subagents (the default).
    async fn subagent(&self, _body: &serde_json::Value) -> Option<Subagent> {
        None
    }
    /// Its saved conversations (feature `conversation_list`); a harness
    /// without it is [`HarnessError::Unsupported`].
    async fn conversations(
        &self,
        _query: &ConversationQuery,
    ) -> Result<Vec<NativeConversation>, HarnessError> {
        Err(HarnessError::Unsupported {
            harness: self.title().to_string(),
            what: "list its saved conversations".into(),
        })
    }
    /// The name a hook body gives the session's conversation, when it
    /// gives one.
    fn session_title(&self, _body: &serde_json::Value) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A launch's input crosses the wire as it is.
    #[test]
    fn a_launch_input_and_its_answer_round_trip() {
        let input = LaunchInput {
            session: SessionIds {
                stream: StreamId::new(1),
                thread: ThreadId::new(2),
                session: AgentSessionId::new(3),
            },
            workspace: "/w".into(),
            project_dir: "/p".into(),
            endpoints: Endpoints {
                hook_base_url: "h".into(),
                mcp_endpoint_url: "m".into(),
                otlp_base_url: "o".into(),
                hook_token: "t".into(),
            },
            identity_env: vec![("OXPLOW_SESSION".into(), "ses3".into())],
            system_prompt: Some("sp".into()),
            resume: None,
            fork: false,
            text: AgentText::default(),
            config: serde_json::json!({"model": "x"}),
            home: None,
            search_path: vec!["/opt/bin".into()],
        };
        let wire = serde_json::to_value(&input).unwrap();
        assert_eq!(wire["session"]["session"], "ses3");
        assert_eq!(
            wire["identity_env"],
            serde_json::json!([["OXPLOW_SESSION", "ses3"]])
        );
        assert_eq!(serde_json::from_value::<LaunchInput>(wire).unwrap(), input);
        let launch: Launch = serde_json::from_value(serde_json::json!({
            "spec": {"kind": "pty", "command": "agent", "env": [["K", "v"]]}
        }))
        .unwrap();
        assert_eq!(
            launch,
            Launch {
                spec: LaunchSpec::Pty {
                    command: "agent".into(),
                    env: vec![("K".into(), "v".into())]
                },
                transcript_missing: false
            }
        );
    }

    /// A program is found on the search path in order; a path is taken
    /// as it is.
    #[test]
    fn a_program_resolves_on_the_search_path() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(b.path().join("agent"), "").unwrap();
        let dirs = vec![a.path().to_path_buf(), b.path().to_path_buf()];
        assert_eq!(
            resolve_program_in("agent", &dirs),
            Some(b.path().join("agent"))
        );
        assert_eq!(resolve_program_in("nope", &dirs), None);
        let full = b.path().join("agent").to_string_lossy().into_owned();
        assert_eq!(resolve_program_in(&full, &[]), Some(b.path().join("agent")));
    }

    /// A provider's declared data is these two lists, nothing else: a
    /// project's instruction files aren't one of them.
    #[test]
    fn harness_data_refuses_an_unknown_key() {
        let data: HarnessData = serde_json::from_value(serde_json::json!({
            "env_markers": ["ACME_SESSION"],
            "settings": [{"key": "model", "title": "Model"}]
        }))
        .unwrap();
        assert_eq!(data.env_markers, ["ACME_SESSION"]);
        assert_eq!(data.settings[0].hint, "");
        assert!(serde_json::from_value::<HarnessData>(serde_json::json!({"interact": 1})).is_err());
        assert!(serde_json::from_value::<HarnessData>(
            serde_json::json!({"instruction_files": ["AGENTS.md"]})
        )
        .is_err());
    }
}
