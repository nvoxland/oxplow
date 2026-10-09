//! The fake as an **agent harness** (`OXPLOW_FAKE_CAPABILITY=agent_harness`):
//! the fake harness (`oxplow-harness-fake`) behind the provider protocol.
//! Its verbs answer with that harness's own launch and mappings:
//!
//! - `launch` — the launch's input as it is → a `Launch` (its binary, run
//!   in a terminal, found on the input's search path);
//! - `tool_use { body }` → `{ tool }` (`null` when the body names none);
//! - `render { answer }` → `{ body }`, in its own `{"fake": …}` shape;
//! - `token_readings { records }` → `{ readings }` (its `telemetry`).
//!
//! It declares itself as a harness provider does: features (`terminal`,
//! `telemetry`) and `data` (instruction files, an environment marker, a
//! setting).

use oxplow_domain::agent::harness::LaunchInput;
use oxplow_domain::agent::observe::{HookAnswer, OtlpRecord};
use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::ProtocolError;
use serde_json::{json, Value};

/// The environment marker it declares.
pub const MARKER: &str = "OXPLOW_FAKE_HARNESS_SESSION";

/// What it declares in harness mode: the `agent_harness` capability, its
/// verbs, no event types, no collectors; the same config as its work list.
pub fn declarations() -> InitializeResult {
    InitializeResult {
        protocol_version: PROTOCOL_VERSION.into(),
        provider: Party {
            name: crate::PROVIDER.into(),
            version: "1".into(),
        },
        capabilities: vec![CapabilityDecl {
            capability: "agent_harness".into(),
            features: json!({ "terminal": true, "telemetry": true }),
            data: json!({
                "instruction_files": ["AGENTS.md"],
                "env_markers": [MARKER],
                "settings": [{
                    "key": "model",
                    "title": "Model",
                    "hint": "The model its scripted session names.",
                    "placeholder": "fake-1"
                }]
            }),
        }],
        commands: vec![
            crate::command(
                "launch",
                "How a session of it starts.",
                json!({ "type": "object", "required": ["session", "endpoints"] }),
            ),
            crate::command(
                "tool_use",
                "A tool hook's body in oxplow's vocabulary.",
                json!({ "type": "object", "required": ["body"] }),
            ),
            crate::command(
                "render",
                "A hook answer in its hooks' shape.",
                json!({ "type": "object", "required": ["answer"] }),
            ),
            crate::command(
                "token_readings",
                "The token counts in one telemetry export.",
                json!({ "type": "object", "required": ["records"] }),
            ),
        ],
        event_types: Vec::new(),
        collectors: Vec::new(),
        config_schema: json!({ "type": "object", "required": ["team"],
                               "properties": { "team": { "type": "string" } } }),
    }
}

fn invalid(field: &str, e: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::InvalidInput {
        field: field.into(),
        message: e.to_string(),
    }
}

/// `verb`'s answer to `input`.
pub(crate) fn answer(verb: &str, input: &Value) -> Result<Value, ProtocolError> {
    match verb {
        "launch" => {
            let input: LaunchInput =
                serde_json::from_value(input.clone()).map_err(|e| invalid("", e))?;
            let launch = oxplow_harness_fake::launch(&input).map_err(|e| invalid("", e))?;
            Ok(serde_json::to_value(launch).expect("a launch serializes"))
        }
        "tool_use" => Ok(json!({ "tool": oxplow_harness_fake::tool_use(&input["body"]) })),
        "render" => {
            let answer: HookAnswer =
                serde_json::from_value(input["answer"].clone()).map_err(|e| invalid("/answer", e))?;
            Ok(json!({ "body": oxplow_harness_fake::render(&answer) }))
        }
        "token_readings" => {
            let records: Vec<OtlpRecord> = serde_json::from_value(input["records"].clone())
                .map_err(|e| invalid("/records", e))?;
            let readings: Vec<_> = records
                .iter()
                .flat_map(oxplow_harness_fake::token_reading)
                .collect();
            Ok(json!({ "readings": readings }))
        }
        other => Err(invalid(
            "/command",
            format!("an agent harness answers launch, tool_use, render or token_readings, not `{other}`"),
        )),
    }
}
