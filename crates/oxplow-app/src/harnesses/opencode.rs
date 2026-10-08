//! The `opencode` harness.

use oxplow_domain::agent::harness::{AgentHarness, Gate, Input, Interact, Transcript};

use super::Named;

pub(super) struct Opencode(pub(super) Named);

impl AgentHarness for Opencode {
    fn id(&self) -> &str {
        &self.0.id
    }

    fn title(&self) -> &str {
        &self.0.title
    }

    fn interact(&self) -> Interact {
        Interact {
            transcript: Transcript::Terminal,
            input: Input::Keystrokes,
            gate: Gate::Harness,
        }
    }
}
