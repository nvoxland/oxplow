//! The fake as an **effort policy** (`OXPLOW_FAKE_CAPABILITY=effort_policy`):
//! one verb, `react { event }`, answered as an effect's script answers —
//! `{ commands }` to run, or `{ skip }`:
//!
//! - an item moved to `in_progress` with a thread (the event's anchors)
//!   opens an effort on that thread linked to the item;
//! - an item moved to `done` or `canceled` closes its open efforts, which
//!   it reads of oxplow through `host/call` (`sql.read` over `v_effort`,
//!   named with the invoke's key);
//! - anything else is skipped.

use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::{Peer, ProtocolError};
use serde_json::{json, Value};

/// What it declares in policy mode: the `effort_policy` capability and its
/// verb, no event types, no collectors; the same config as its work list.
pub fn declarations() -> InitializeResult {
    InitializeResult {
        protocol_version: PROTOCOL_VERSION.into(),
        provider: Party {
            name: crate::PROVIDER.into(),
            version: "1".into(),
        },
        capabilities: vec![CapabilityDecl {
            capability: "effort_policy".into(),
            features: json!({}),
        }],
        commands: vec![crate::command(
            "react",
            "Compose the commands an event calls for.",
            json!({ "type": "object" }),
        )],
        event_types: Vec::new(),
        collectors: Vec::new(),
        config_schema: json!({ "type": "object", "required": ["team"],
                               "properties": { "team": { "type": "string" } } }),
    }
}

fn skip(why: &str) -> Value {
    json!({ "skip": why })
}

/// `react`'s answer to `input` (`{ event }`); `key` is the invoke's
/// idempotency key, which its `host/call` names.
pub(crate) async fn react(
    peer: &Peer,
    key: Option<String>,
    input: &Value,
) -> Result<Value, ProtocolError> {
    let event = &input["event"];
    if event["type"] != "work_item.state_changed" {
        return Ok(skip("not an item's move"));
    }
    let item = event["payload"]["work_item"].as_str().unwrap_or_default();
    match event["payload"]["to"].as_str() {
        Some("in_progress") => match event["anchors"]["thread_id"].as_str() {
            Some(thread) => Ok(json!({ "commands": [{
                "name": "oxplow.effort.open",
                "input": { "thread": format!("thread:{thread}"), "work_item": item },
            }] })),
            None => Ok(skip("started on no thread")),
        },
        Some("done" | "canceled") => {
            let rows = peer
                .request(
                    method::HOST_CALL,
                    json!({
                        "key": key,
                        "scope": "sql.read",
                        "args": {
                            "sql": "SELECT id FROM v_effort WHERE work_item = :work_item AND ended_at IS NULL",
                            "params": { "work_item": item },
                        },
                    }),
                )
                .await?;
            let commands: Vec<Value> = rows
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| row["id"].as_i64())
                .map(|id| {
                    json!({ "name": "oxplow.effort.close",
                            "input": { "effort": format!("effort:eff{id}"), "reason": "switch" } })
                })
                .collect();
            Ok(json!({ "commands": commands }))
        }
        _ => Ok(skip("a move that changes no effort")),
    }
}
