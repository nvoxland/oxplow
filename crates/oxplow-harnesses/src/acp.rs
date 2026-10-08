//! The `acp` harness: an Agent Client Protocol agent, in oxplow's chat.
//! Its launch is the ACP agent's program — what its adapter declares, as
//! resolved for this project (`acp::agents`) and handed in as the config.

use std::path::PathBuf;

use oxplow_domain::agent::harness::{
    AgentHarness, HarnessError, Interact, Launch, LaunchInput, LaunchSpec, Transcript,
};

use oxplow_domain::agent::observe::HookAnswer;
use oxplow_domain::agent::tool::ToolUse;

use super::Named;

pub(super) struct Acp(pub(super) Named);

/// The resolved program a launch is handed.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Program {
    program: PathBuf,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: Vec<(String, String)>,
    #[serde(default)]
    system_prompt_via_meta: bool,
}

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
        }
    }

    fn launch(&self, input: &LaunchInput<'_>) -> Result<Launch, HarnessError> {
        let p: Program = serde_json::from_value(input.config.clone())
            .map_err(|e| HarnessError::Config(format!("an ACP agent's program: {e}")))?;
        Ok(Launch {
            spec: LaunchSpec::Acp {
                program: p.program,
                args: p.args,
                env: p.env,
                system_prompt_via_meta: p.system_prompt_via_meta,
            },
            resume_dropped: false,
        })
    }

    fn instruction_files(&self) -> &[&str] {
        &["CLAUDE.md"]
    }

    /// The canonical names its tool calls are recorded under
    /// (`acp::mapping::canonical_name`).
    /// An ACP agent's calls arrive already mapped, by the protocol's tool
    /// kinds (`oxplow_app::acp::mapping`); it posts no hook bodies.
    fn tool_use(&self, _: &serde_json::Value) -> Option<ToolUse> {
        None
    }

    /// It posts no hooks (its tool gate answers in-process); one naming its
    /// session gets the common shape.
    fn render(&self, answer: &HookAnswer) -> serde_json::Value {
        super::shared::render(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_launch::{harness, launch_in};

    #[test]
    fn launch_is_the_agents_program() {
        let h = harness("oxplow:acp", "acp");
        let l = launch_in(
            h.as_ref(),
            None,
            None,
            &serde_json::json!({ "program": "/bin/gemini", "args": ["--acp"], "systemPromptViaMeta": false }),
        );
        assert_eq!(
            l.launch.spec,
            LaunchSpec::Acp {
                program: "/bin/gemini".into(),
                args: vec!["--acp".into()],
                env: vec![],
                system_prompt_via_meta: false,
            }
        );
    }
}
