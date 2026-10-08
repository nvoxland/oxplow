//! Drive: oxplow (or an extension) prompting an agent. An interface only —
//! nothing implements or calls it yet. The rule it carries: oxplow never
//! drives an agent through a surface licensed for interactive use. A host
//! says whether it's `programmatic` (true only for API-funded paths); a
//! PTY, or an ACP agent signed in with a plan, never is.

use async_trait::async_trait;

use crate::ids::AgentSessionId;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DriveError {
    /// The surface is licensed for interactive use only.
    #[error("this agent's surface is interactive-only; oxplow doesn't drive it")]
    NotProgrammatic,
    #[error("{0}")]
    Failed(String),
}

#[async_trait]
pub trait Drive: Send + Sync {
    /// Whether this surface may be driven at all.
    fn programmatic(&self) -> bool;
    /// Send `text` to agent session `session`.
    async fn prompt(&self, session: &AgentSessionId, text: &str) -> Result<(), DriveError>;
}
