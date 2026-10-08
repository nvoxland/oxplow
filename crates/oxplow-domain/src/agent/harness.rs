//! An agent harness: what runs in an agent session.

/// How a person interacts with a harness's session — flags the UI reads,
/// never something core invokes (the Interact seam).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interact {
    pub transcript: Transcript,
    pub input: Input,
    pub gate: Gate,
}

/// What the person reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transcript {
    /// The harness's own terminal.
    Terminal,
    /// oxplow's structured transcript (an ACP agent's).
    Structured,
}

/// Where the person's input goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// Keystrokes to its PTY.
    Keystrokes,
    /// oxplow's prompt box.
    Prompt,
}

/// Who asks the person to permit a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// The harness's own prompt, in its terminal.
    Harness,
    /// oxplow's permission card.
    Oxplow,
}

/// One harness implementation, registered under the key an agent
/// session's `harness` names.
pub trait AgentHarness: Send + Sync {
    /// The registry key (`claude`): what `agent_session.harness` names.
    fn id(&self) -> &str;
    /// How a person names it.
    fn title(&self) -> &str;
    fn interact(&self) -> Interact;
}
