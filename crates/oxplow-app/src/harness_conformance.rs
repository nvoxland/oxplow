//! The agent-harness conformance suite (`.context/agent-model.md` "What a
//! harness implements"): what every harness must do, checked through its
//! own operations — the built-ins in code and a provider's process alike,
//! the same way (core never special-cases its own implementations). It
//! runs in-tree over every registered harness and against a provider
//! process through the conformance kit (`oxplow extension test`).
//!
//! The checks are the floor every harness keeps:
//! - its answers render as objects, and a refusal or added context reads
//!   differently from a plain acknowledgement;
//! - a launch keeps the session's bearer out of the command line (any
//!   process can list it) and, for a terminal, in its environment;
//! - a hook body naming no tool maps to none;
//! - the instruction files it names are paths inside the worktree.

use std::path::{Component, Path, PathBuf};

use oxplow_domain::agent::harness::{AgentHarness, Endpoints, LaunchInput, LaunchSpec, SessionIds};
use oxplow_domain::agent::observe::HookAnswer;
use oxplow_domain::hook::HookKind;
use oxplow_domain::{AgentSessionId, StreamId, ThreadId};
use serde_json::json;

pub use crate::work_items_conformance::{Finding, SuiteRun};

/// The bearer the suite's launch is given: a value no harness would
/// produce on its own, so finding it anywhere means it came from the
/// endpoints.
const BEARER: &str = "oxplow-conformance-bearer-7f3c";

/// Run every check against the registered harness `harness`. Its launch
/// gets `config` (what `agentConfig.<harness>` would give it) and looks
/// for programs on `search_path`, in a scratch worktree.
pub async fn suite(
    svc: &crate::Services,
    harness: &str,
    config: &serde_json::Value,
    search_path: &[PathBuf],
) -> SuiteRun {
    let mut findings = Vec::new();
    match svc.harnesses.get(harness) {
        Ok(h) => run(&*h, config, search_path, &mut findings).await,
        Err(e) => findings.push(Finding {
            check: "setup",
            message: e.to_string(),
        }),
    }
    SuiteRun {
        findings,
        left: Vec::new(),
    }
}

async fn run(
    h: &dyn AgentHarness,
    config: &serde_json::Value,
    search_path: &[PathBuf],
    findings: &mut Vec<Finding>,
) {
    let mut fail = |check: &'static str, message: String| {
        findings.push(Finding { check, message });
    };

    let ack = h.render(&HookAnswer::Ack).await;
    if !ack.is_object() {
        fail(
            "an_answer_renders_as_an_object",
            format!("rendering an acknowledgement gave {ack}, not an object"),
        );
    }
    let deny = h
        .render(&HookAnswer::Deny {
            reason: "conformance".into(),
        })
        .await;
    let context = h
        .render(&HookAnswer::Context {
            event: HookKind::PostToolUse,
            text: "conformance".into(),
        })
        .await;
    for (what, rendered) in [("a refusal", &deny), ("added context", &context)] {
        if !rendered.is_object() {
            fail(
                "an_answer_renders_as_an_object",
                format!("rendering {what} gave {rendered}, not an object"),
            );
        } else if *rendered == ack {
            fail(
                "a_refusal_and_context_read_unlike_an_acknowledgement",
                format!("{what} renders as an acknowledgement does ({ack}): the agent can't tell them apart"),
            );
        }
    }

    if let Some(tool) = h.tool_use(&json!({})).await {
        fail(
            "a_body_naming_no_tool_maps_to_none",
            format!("an empty hook body mapped to {tool:?}"),
        );
    }

    for file in h.instruction_files() {
        let path = Path::new(&file);
        if file.is_empty()
            || !path.is_relative()
            || path.components().any(|c| matches!(c, Component::ParentDir))
        {
            fail(
                "instruction_files_are_inside_the_worktree",
                format!("instruction file `{file}` isn't a path inside the worktree"),
            );
        }
    }

    match scratch_launch(h, config, search_path).await {
        Err(e) => fail(
            "a_launch_keeps_the_bearer_in_its_env",
            format!("launch failed: {e}"),
        ),
        Ok(LaunchSpec::Pty { command, env }) => {
            if command.contains(BEARER) {
                fail(
                    "a_launch_keeps_the_bearer_in_its_env",
                    "the session's bearer is in the command line, which any process can list — \
                     put it in the launch's env"
                        .into(),
                );
            }
            if !env.iter().any(|(_, v)| v.contains(BEARER)) {
                fail(
                    "a_launch_keeps_the_bearer_in_its_env",
                    "a terminal launch's env doesn't carry the session's bearer, so its hooks \
                     can't reach oxplow — pass the launch's identity env on"
                        .into(),
                );
            }
        }
        Ok(LaunchSpec::Acp { program, args, .. }) => {
            if program.to_string_lossy().contains(BEARER) || args.iter().any(|a| a.contains(BEARER))
            {
                fail(
                    "a_launch_keeps_the_bearer_in_its_env",
                    "the session's bearer is in the agent's program or arguments, which any \
                     process can list"
                        .into(),
                );
            }
        }
    }
}

/// `h`'s launch for a placeholder session in a scratch worktree, the
/// endpoints unreachable and the bearer [`BEARER`].
async fn scratch_launch(
    h: &dyn AgentHarness,
    config: &serde_json::Value,
    search_path: &[PathBuf],
) -> Result<LaunchSpec, String> {
    let scratch = tempfile::tempdir().map_err(|e| e.to_string())?;
    let root = scratch.path().to_path_buf();
    let input = LaunchInput {
        session: SessionIds {
            stream: StreamId::new(1),
            thread: ThreadId::new(1),
            session: AgentSessionId::new(1),
        },
        workspace: root.clone(),
        project_dir: root.clone(),
        endpoints: Endpoints {
            hook_base_url: "http://127.0.0.1:9/hook".into(),
            mcp_endpoint_url: "http://127.0.0.1:9/mcp".into(),
            otlp_base_url: "http://127.0.0.1:9".into(),
            hook_token: BEARER.into(),
        },
        identity_env: vec![
            ("OXPLOW_HOOK_TOKEN".into(), BEARER.into()),
            (
                "OXPLOW_HOOK_BASE_URL".into(),
                "http://127.0.0.1:9/hook".into(),
            ),
        ],
        system_prompt: None,
        resume: None,
        text: Default::default(),
        config: config.clone(),
        oxplow_executable: "/bin/false".into(),
        home: Some(root),
        search_path: search_path.to_vec(),
    };
    h.launch(&input)
        .await
        .map(|l| l.spec)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::services_with_effort;

    /// Every built-in harness keeps the floor: run over each as any
    /// harness is (the ACP one launches the program its config names).
    #[tokio::test]
    async fn every_built_in_harness_passes() {
        let fx = services_with_effort().await;
        for id in ["claude", "codex", "opencode", "acp"] {
            let config = if id == "acp" {
                json!({ "program": "/bin/true" })
            } else {
                json!({})
            };
            let run = suite(&fx.svc, id, &config, &[]).await;
            assert!(run.findings.is_empty(), "{id}: {:#?}", run.findings);
        }
    }

    /// A harness that breaks the floor is named check by check: one that
    /// renders everything alike, maps an empty body to a tool, names an
    /// instruction file outside the worktree, and puts the bearer on its
    /// command line.
    #[tokio::test]
    async fn a_harness_off_the_floor_is_named_check_by_check() {
        use oxplow_domain::agent::harness::{HarnessError, Interact, Launch, Transcript};
        use oxplow_domain::agent::tool::{ToolKind, ToolUse};

        struct Sloppy;
        #[async_trait::async_trait]
        impl AgentHarness for Sloppy {
            fn id(&self) -> &str {
                "sloppy"
            }
            fn title(&self) -> &str {
                "Sloppy"
            }
            fn interact(&self) -> Interact {
                Interact {
                    transcript: Transcript::Terminal,
                }
            }
            fn instruction_files(&self) -> Vec<String> {
                vec!["../outside.md".into()]
            }
            async fn launch(&self, input: &LaunchInput) -> Result<Launch, HarnessError> {
                Ok(Launch {
                    spec: LaunchSpec::Pty {
                        command: format!("agent --token {}", input.endpoints.hook_token),
                        env: Vec::new(),
                    },
                    resume_dropped: false,
                })
            }
            async fn tool_use(&self, _: &serde_json::Value) -> Option<ToolUse> {
                Some(ToolUse {
                    kind: ToolKind::Read,
                    ..ToolUse::default()
                })
            }
            async fn render(&self, _: &HookAnswer) -> serde_json::Value {
                json!({})
            }
        }

        let fx = services_with_effort().await;
        fx.svc.harnesses.register(std::sync::Arc::new(Sloppy));
        let run = suite(&fx.svc, "sloppy", &json!({}), &[]).await;
        let mut checks: Vec<&str> = run.findings.iter().map(|f| f.check).collect();
        checks.dedup();
        assert_eq!(
            checks,
            [
                "a_refusal_and_context_read_unlike_an_acknowledgement",
                "a_body_naming_no_tool_maps_to_none",
                "instruction_files_are_inside_the_worktree",
                "a_launch_keeps_the_bearer_in_its_env",
            ],
            "{:#?}",
            run.findings
        );
        let unknown = suite(&fx.svc, "nope", &json!({}), &[]).await;
        assert_eq!(unknown.findings[0].check, "setup");
    }
}
