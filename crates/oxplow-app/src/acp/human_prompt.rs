//! The ONLY way to make a prompt for an ACP agent (tsk281).
//!
//! oxplow never writes to an agent on its own: a prompt exists only
//! because a person pressed Enter. [`HumanPrompt`] has a private field and
//! one constructor, [`compose`], which the session calls only from the
//! human's submit path; `wire::AgentConn::prompt` accepts nothing else.
//! The source-scan guard in `acp/guard_tests.rs` pins who may call
//! `compose` and who may name the SDK's prompt request.

/// A prompt a person typed, with the oxplow context attached to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanPrompt {
    blocks: Vec<String>,
}

impl HumanPrompt {
    /// The text blocks sent, in order: the context block (when there is
    /// one) and then what the person typed.
    pub fn blocks(&self) -> &[String] {
        &self.blocks
    }
}

/// Build the prompt for `text` as the person typed it. `context` (session
/// context, advisories, decisions, queued nudges) rides as a visible block
/// ahead of it, the ACP counterpart of a hook's `additionalContext`.
pub(super) fn compose(text: &str, context: Option<&str>) -> HumanPrompt {
    let mut blocks = Vec::new();
    if let Some(c) = context.filter(|c| !c.trim().is_empty()) {
        blocks.push(c.to_string());
    }
    blocks.push(text.to_string());
    HumanPrompt { blocks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_rides_ahead_of_the_typed_text() {
        assert_eq!(compose("go", Some("ctx")).blocks(), ["ctx", "go"]);
        assert_eq!(compose("go", None).blocks(), ["go"]);
        assert_eq!(compose("go", Some("  ")).blocks(), ["go"]);
    }
}
