//! A fake agent harness, for the observe conformance suite
//! (`oxplow_app::observe_conformance`, `.context/agent-model.md` "Observe
//! conformance"): a test double, never a production dependency. Its launch
//! runs this crate's binary, which posts a scripted session to oxplow's
//! hook route and OTLP receiver the way a real harness's hooks do. Its
//! answers have a shape of its own (`{"fake": …}`) and its binary fails on
//! any other, so a pass shows core let the harness render every answer.

use oxplow_domain::agent::harness::{
    AgentHarness, HarnessError, Interact, Launch, LaunchInput, LaunchSpec, Transcript,
};
use oxplow_domain::agent::observe::{HookAnswer, OtlpRecord, TokenReading};
use oxplow_domain::agent::tool::{ToolKind, ToolUse};
use oxplow_domain::events::schema::TokenKind;

/// Its registry key.
pub const ID: &str = "fake";
/// Its binary's name, which `LaunchInput::resolve_program` resolves.
pub const BIN: &str = "oxplow-harness-fake";
/// The metric its binary exports token counts on; the `kind` attribute is
/// `input` or `output`.
pub const TOKEN_METRIC: &str = "oxplow_fake.tokens";
/// The file its scripted session edits, in the workspace.
pub const EDITED: &str = "fake.txt";
/// The input and output tokens its scripted session reports.
pub const TOKENS: (i64, i64) = (7, 3);

pub struct FakeHarness;

/// POSIX single-quoting.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

impl AgentHarness for FakeHarness {
    fn id(&self) -> &str {
        ID
    }

    fn title(&self) -> &str {
        "Fake"
    }

    fn interact(&self) -> Interact {
        Interact {
            transcript: Transcript::Terminal,
        }
    }

    fn launch(&self, input: &LaunchInput<'_>) -> Result<Launch, HarnessError> {
        let program = (input.resolve_program)(BIN)
            .ok_or_else(|| HarnessError::Config(format!("{BIN} isn't built")))?;
        let ep = input.endpoints;
        // The identity env carries the hook URL and token; the OTLP
        // receiver is the fake's own addition, as each harness's is.
        let mut env = input.identity_env.to_vec();
        env.push((
            "OXPLOW_FAKE_OTLP_URL".to_string(),
            format!("{}/v1/metrics", ep.otlp_base_url),
        ));
        Ok(Launch {
            spec: LaunchSpec::Pty {
                command: format!(
                    "cd {} && exec {}",
                    quote(&input.workspace.to_string_lossy()),
                    quote(&program)
                ),
                env,
            },
            resume_dropped: false,
        })
    }

    /// Its scripted session posts one edit, `{"tool_name": "Edit",
    /// "tool_input": {"file_path": …}}`.
    fn tool_use(&self, body: &serde_json::Value) -> Option<ToolUse> {
        let name = body.get("tool_name")?.as_str()?.to_string();
        let kind = if name == "Edit" {
            ToolKind::Edit
        } else {
            ToolKind::Other
        };
        Some(ToolUse {
            paths: body["tool_input"]["file_path"]
                .as_str()
                .map(|p| vec![p.to_string()])
                .unwrap_or_default(),
            ok: body
                .get("tool_response")
                .map(|r| r["success"].as_bool().unwrap_or(false)),
            name,
            kind,
            ..ToolUse::default()
        })
    }

    fn token_readings(&self, record: &OtlpRecord<'_>) -> Vec<TokenReading> {
        let OtlpRecord::Point {
            metric: TOKEN_METRIC,
            value,
            attributes,
            time_unix_nano,
            start_time_unix_nano,
            ..
        } = record
        else {
            return Vec::new();
        };
        let kind = match attributes.str("kind") {
            Some("input") => TokenKind::Input,
            Some("output") => TokenKind::Output,
            _ => return Vec::new(),
        };
        vec![TokenReading {
            model: record.model(),
            kind,
            value: *value,
            at_unix_nano: *time_unix_nano,
            from_unix_nano: *start_time_unix_nano,
        }]
    }

    fn render(&self, answer: &HookAnswer) -> serde_json::Value {
        match answer {
            HookAnswer::Ack => serde_json::json!({ "fake": "ack" }),
            HookAnswer::Deny { reason } => serde_json::json!({ "fake": "deny", "reason": reason }),
            HookAnswer::Context { text, .. } => {
                serde_json::json!({ "fake": "context", "text": text })
            }
        }
    }
}
