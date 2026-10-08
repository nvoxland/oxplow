//! An agent harness: what runs in an agent session.
//!
//! Its **Launch** seam (`launch`) says how the session's process comes to
//! exist — a PTY command, or an ACP agent's program — writing whatever
//! runtime files its agent reads under `.oxplow/runtime/` (never the
//! worktree). Starting a session only spawns: nothing here prompts.

use std::path::{Path, PathBuf};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionIds {
    pub stream: StreamId,
    pub thread: ThreadId,
    pub session: AgentSessionId,
}

/// Where the agent reaches oxplow: the control plane's hook and OTLP
/// receivers, its MCP endpoint, and the bearer for all three.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub hook_base_url: String,
    pub mcp_endpoint_url: String,
    pub otlp_base_url: String,
    pub hook_token: String,
}

/// What a launch knows.
pub struct LaunchInput<'a> {
    pub session: SessionIds,
    /// The stream's worktree: where the agent runs.
    pub workspace: &'a Path,
    /// The project: where `.oxplow/runtime/` is.
    pub project_dir: &'a Path,
    pub endpoints: &'a Endpoints,
    /// `OXPLOW_*` — what every process oxplow starts is told about itself.
    pub identity_env: &'a [(String, String)],
    /// oxplow's system prompt for the session.
    pub system_prompt: Option<&'a str>,
    /// The harness's own session to resume.
    pub resume: Option<&'a str>,
    /// The skills and commands offered now.
    pub text: &'a AgentText,
    /// The harness's configuration (`agentConfig.<harness>`), or for an
    /// ACP agent its adapter's resolved program.
    pub config: &'a serde_json::Value,
    /// The oxplow binary, for a harness whose hooks are commands.
    pub oxplow_executable: &'a Path,
    /// The person's home, where a harness keeps its own sessions.
    pub home: Option<&'a Path>,
    /// Where a program is: its absolute path on PATH or in the well-known
    /// install dirs, so a GUI-launched oxplow's thin PATH doesn't matter.
    pub resolve_program: &'a (dyn Fn(&str) -> Option<String> + Send + Sync),
}

/// How the session's process is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchSpec {
    /// A shell command run in a PTY (`sh -lc <command>`), with `env` set
    /// in its environment. Anything secret (the session's bearer) goes in
    /// `env`, never in `command`, whose text any process can list.
    Pty {
        command: String,
        env: Vec<(String, String)>,
    },
    /// An ACP agent's program, spoken to over its stdio.
    Acp {
        program: PathBuf,
        args: Vec<String>,
        env: Vec<(String, String)>,
        /// The system prompt rides `_meta.systemPrompt.append` on
        /// `session/new` rather than ahead of the first prompt.
        system_prompt_via_meta: bool,
    },
}

/// A launch: how to start the process, and whether the resume id was
/// found stale (core then forgets it, `resume_check::forget_missing`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub spec: LaunchSpec,
    pub resume_dropped: bool,
}

/// One project setting a harness reads (`agentConfig.<harness>.<key>`), as
/// Settings shows it: a text value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessSetting {
    pub key: &'static str,
    pub title: &'static str,
    /// What it does, under the field.
    pub hint: &'static str,
    /// An example value, in the empty field.
    pub placeholder: &'static str,
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
/// session's `harness` names.
///
/// What every harness answers: who it is, how its sessions start and are
/// shown, how its tool calls read in oxplow's vocabulary, and how its hooks
/// expect an answer. The rest is what a harness offers beyond that — its
/// own instruction files, environment markers, settings, a transcript or
/// telemetry core can read — each with a default of "none", so a harness
/// implements only what it has.
pub trait AgentHarness: Send + Sync {
    /// The registry key (`claude`): what `agent_session.harness` names.
    fn id(&self) -> &str;
    /// How a person names it.
    fn title(&self) -> &str;
    fn interact(&self) -> Interact;
    /// How the session's process starts. Spawns only — never prompts.
    fn launch(&self, input: &LaunchInput<'_>) -> Result<Launch, HarnessError>;
    /// One of its tool hooks' bodies (a PreToolUse or PostToolUse) mapped
    /// onto oxplow's vocabulary; `None` when the body names no tool.
    fn tool_use(&self, body: &serde_json::Value) -> Option<ToolUse>;
    /// `answer` in the shape its hooks expect back.
    fn render(&self, answer: &HookAnswer) -> serde_json::Value;

    /// The project files whose text oxplow adds to its system prompt: the
    /// instruction files a session of it wouldn't read by itself.
    fn instruction_files(&self) -> &[&str] {
        &[]
    }
    /// Its process's markers an agent or terminal must not inherit from
    /// oxplow's own environment (when oxplow itself runs in one).
    fn env_markers(&self) -> &[&str] {
        &[]
    }
    /// The project settings its `launch` reads from its `agentConfig`
    /// entry, which Settings → Agents offers for it.
    fn settings(&self) -> &[HarnessSetting] {
        &[]
    }
    /// Rewrite the skills and commands of its runtime already on disk under
    /// `project_dir` to `text`, creating none: an agent that outlives a
    /// launch (running across an upgrade, or resumed) reads what's offered
    /// now. Nothing to do for a harness that keeps none on disk.
    fn refresh_text(&self, _project_dir: &Path, _text: &AgentText) -> Result<(), HarnessError> {
        Ok(())
    }
    /// Its tools that can change the worktree, lowercased: file edits,
    /// shell commands, subagents (which run their own tools). Every other
    /// call only reads or talks.
    fn writing_tools(&self) -> &[&str] {
        &[]
    }
    /// The recordable turns in a chunk of its transcript; none when it
    /// keeps no transcript core reads.
    fn turns(&self, _transcript: &str) -> Vec<Turn> {
        Vec::new()
    }
    /// The token counts in one record of its telemetry export; none for a
    /// record it doesn't recognize.
    fn token_readings(&self, _record: &OtlpRecord<'_>) -> Vec<TokenReading> {
        Vec::new()
    }
}
