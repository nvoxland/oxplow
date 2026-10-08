//! The `acp` harness: an Agent Client Protocol agent, in oxplow's chat.

use oxplow_domain::agent::harness::{AgentHarness, Gate, Input, Interact, Transcript};

use super::Named;

pub(super) struct Acp(pub(super) Named);

impl AgentHarness for Acp {
    fn id(&self) -> &str {
        &self.0.id
    }

    fn title(&self) -> &str {
        &self.0.title
    }

    fn interact(&self) -> Interact {
        Interact {
            transcript: Transcript::Structured,
            input: Input::Prompt,
            gate: Gate::Oxplow,
        }
    }
}
