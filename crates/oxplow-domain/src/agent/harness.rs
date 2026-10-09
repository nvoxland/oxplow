//! An agent harness: what runs in an agent session.
//!
//! Its **Launch** seam (`launch`) says how the session's process comes to
//! exist — a PTY command, or an ACP agent's program — writing whatever
//! runtime files its agent reads under `.oxplow/runtime/` (never the
//! worktree). Starting a session only spawns: nothing here prompts.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::observe::{HookAnswer, OtlpRecord, TokenReading, Turn};
use super::text::AgentText;
use super::tool::ToolUse;
use crate::ids::{AgentSessionId, StreamId, ThreadId};

/// How a person interacts with a harness's session — what the UI reads,
/// never something core invokes (the Interact seam).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interact {
    pub transcript: Transcript,
}

/// What the person reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transcript {
    /// The harness's own terminal.
    Terminal,
    /// oxplow's structured transcript (an ACP agent's).
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
    /// The skills and commands offered now.
    pub text: AgentText,
    /// The harness's configuration (`agentConfig.<harness>`), or for an
    /// ACP agent its adapter's resolved program.
    pub config: serde_json::Value,
    /// The oxplow binary, for a harness whose hooks are commands.
    pub oxplow_executable: PathBuf,
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

/// How the session's process is started. On the wire, `{ "kind": "pty",
/// "command", "env" }` or `{ "kind": "acp", "program", "args", "env",
/// "system_prompt_via_meta" }`.
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
}

/// A launch: how to start the process, and whether the resume id was
/// found stale (core then forgets it, `resume_check::forget_missing`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launch {
    pub spec: LaunchSpec,
    #[serde(default)]
    pub resume_dropped: bool,
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
/// [`AgentHarness::instruction_files`], [`AgentHarness::env_markers`] and
/// [`AgentHarness::settings`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HarnessData {
    pub instruction_files: Vec<String>,
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
}

/// One harness implementation, registered under the key an agent
/// session's `harness` names: built in, or a provider process
/// (`.context/agent-model.md`, `.context/providers.md`).
///
/// What every harness answers: who it is, how its sessions start and are
/// shown, how its tool calls read in oxplow's vocabulary, and how its hooks
/// expect an answer. The rest is what a harness offers beyond that — its
/// own instruction files, environment markers, settings, a transcript or
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

    /// The project files whose text oxplow adds to its system prompt: the
    /// instruction files a session of it wouldn't read by itself.
    fn instruction_files(&self) -> Vec<String> {
        Vec::new()
    }
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
            text: AgentText::default(),
            config: serde_json::json!({"model": "x"}),
            oxplow_executable: "/bin/oxplow".into(),
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
                resume_dropped: false
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

    /// A provider's declared data is these three lists, nothing else.
    #[test]
    fn harness_data_refuses_an_unknown_key() {
        let data: HarnessData = serde_json::from_value(serde_json::json!({
            "instruction_files": ["AGENTS.md"],
            "settings": [{"key": "model", "title": "Model"}]
        }))
        .unwrap();
        assert_eq!(data.instruction_files, ["AGENTS.md"]);
        assert_eq!(data.settings[0].hint, "");
        assert!(serde_json::from_value::<HarnessData>(serde_json::json!({"interact": 1})).is_err());
    }
}
