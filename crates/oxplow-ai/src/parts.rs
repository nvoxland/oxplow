//! Versioned typed input and output parts for model requests.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::capabilities::{check_v1, SchemaVersionError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type",
    deny_unknown_fields
)]
pub enum PartV1 {
    Text {
        text: String,
    },
    /// `asset_ref` is an opaque host-owned reference, never a path or URL.
    Image {
        asset_ref: String,
        media_type: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        call_id: String,
        result: Value,
    },
}

/// One piece of a streamed reply, as the provider produces it: text as
/// it arrives, a tool call once its arguments are whole (providers send
/// argument JSON in fragments; only the mapping knows when one is done).
/// The stream's final [`TypedResponseV1`] holds the same content, folded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type",
    deny_unknown_fields
)]
pub enum StreamDeltaV1 {
    Text {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
}

impl StreamDeltaV1 {
    /// The deltas a whole reply is, in order: one per text part (a run of
    /// adjacent text parts is one) and one per tool call.
    pub fn of(parts: &[PartV1]) -> Vec<StreamDeltaV1> {
        let mut deltas: Vec<StreamDeltaV1> = Vec::new();
        for part in parts {
            match part {
                PartV1::Text { text } => match deltas.last_mut() {
                    Some(StreamDeltaV1::Text { text: last }) => last.push_str(text),
                    _ => deltas.push(StreamDeltaV1::Text { text: text.clone() }),
                },
                PartV1::ToolCall {
                    id,
                    name,
                    arguments,
                } => deltas.push(StreamDeltaV1::ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                }),
                PartV1::Image { .. } | PartV1::ToolResult { .. } => {}
            }
        }
        deltas
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum MessageRoleV1 {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageV1 {
    pub role: MessageRoleV1,
    pub parts: Vec<PartV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolDefinitionV1 {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TypedRequestV1 {
    pub schema_version: u32,
    pub model: String,
    pub messages: Vec<MessageV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinitionV1>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TypedRequestV1Repr {
    schema_version: u32,
    model: String,
    messages: Vec<MessageV1>,
    #[serde(default)]
    tools: Vec<ToolDefinitionV1>,
}

impl TypedRequestV1 {
    pub fn validate_version(&self) -> Result<(), SchemaVersionError> {
        check_v1("typed request", self.schema_version)
    }
}

impl<'de> Deserialize<'de> for TypedRequestV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error as _;
        let repr = TypedRequestV1Repr::deserialize(deserializer)?;
        check_v1("typed request", repr.schema_version).map_err(D::Error::custom)?;
        Ok(Self {
            schema_version: repr.schema_version,
            model: repr.model,
            messages: repr.messages,
            tools: repr.tools,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum FinishReasonV1 {
    Unknown,
    Stop,
    Length,
    ToolCalls,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum UsageSourceV1 {
    Unknown,
    Observed,
    Estimated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UsageV1 {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub source: UsageSourceV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TypedResponseV1 {
    pub schema_version: u32,
    pub parts: Vec<PartV1>,
    pub finish_reason: FinishReasonV1,
    pub usage: UsageV1,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TypedResponseV1Repr {
    schema_version: u32,
    parts: Vec<PartV1>,
    finish_reason: FinishReasonV1,
    usage: UsageV1,
}

impl TypedResponseV1 {
    pub fn validate_version(&self) -> Result<(), SchemaVersionError> {
        check_v1("typed response", self.schema_version)
    }

    /// What a stream had produced when it stopped early: its parts so far,
    /// no finish reason, usage unknown.
    pub fn partial(parts: Vec<PartV1>) -> Self {
        Self {
            schema_version: 1,
            parts,
            finish_reason: FinishReasonV1::Unknown,
            usage: UsageV1 {
                input_tokens: None,
                output_tokens: None,
                source: UsageSourceV1::Unknown,
            },
        }
    }
}

/// Fold deltas into parts as a stream goes: adjacent text joins, each tool
/// call is its own part. What a cancelled stream reports it had.
#[derive(Debug, Clone, Default)]
pub struct DeltaFold {
    parts: Vec<PartV1>,
}

impl DeltaFold {
    pub fn push(&mut self, delta: &StreamDeltaV1) {
        match delta {
            StreamDeltaV1::Text { text } => match self.parts.last_mut() {
                Some(PartV1::Text { text: last }) => last.push_str(text),
                _ => self.parts.push(PartV1::Text { text: text.clone() }),
            },
            StreamDeltaV1::ToolCall {
                id,
                name,
                arguments,
            } => self.parts.push(PartV1::ToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            }),
        }
    }

    pub fn parts(&self) -> &[PartV1] {
        &self.parts
    }

    pub fn into_partial(self) -> TypedResponseV1 {
        TypedResponseV1::partial(self.parts)
    }
}

impl<'de> Deserialize<'de> for TypedResponseV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error as _;
        let repr = TypedResponseV1Repr::deserialize(deserializer)?;
        check_v1("typed response", repr.schema_version).map_err(D::Error::custom)?;
        Ok(Self {
            schema_version: repr.schema_version,
            parts: repr.parts,
            finish_reason: repr.finish_reason,
            usage: repr.usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> TypedRequestV1 {
        TypedRequestV1 {
            schema_version: 1,
            model: "m".into(),
            messages: vec![
                MessageV1 {
                    role: MessageRoleV1::User,
                    parts: vec![
                        PartV1::Text {
                            text: "look".into(),
                        },
                        PartV1::Image {
                            asset_ref: "asset:1".into(),
                            media_type: "image/png".into(),
                        },
                    ],
                },
                MessageV1 {
                    role: MessageRoleV1::Assistant,
                    parts: vec![PartV1::ToolCall {
                        id: "c1".into(),
                        name: "grep".into(),
                        arguments: json!({ "q": "x" }),
                    }],
                },
                MessageV1 {
                    role: MessageRoleV1::Tool,
                    parts: vec![PartV1::ToolResult {
                        call_id: "c1".into(),
                        result: json!([1, 2]),
                    }],
                },
            ],
            tools: vec![ToolDefinitionV1 {
                name: "grep".into(),
                description: None,
                parameters: json!({ "type": "object" }),
            }],
        }
    }

    /// Every part kind survives a round trip, in order, with the wire
    /// names the scripts read (`toolCall`, `callId`, `assetRef`).
    #[test]
    fn a_request_round_trips_with_its_schema_version() {
        let req = request();
        let wire = serde_json::to_value(&req).unwrap();
        assert_eq!(wire["schemaVersion"], 1);
        let kinds: Vec<_> = wire["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|m| m["parts"].as_array().unwrap())
            .map(|p| p["type"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(kinds, ["text", "image", "toolCall", "toolResult"]);
        assert_eq!(wire["messages"][0]["parts"][1]["assetRef"], "asset:1");
        assert_eq!(wire["messages"][2]["parts"][0]["callId"], "c1");
        assert_eq!(wire["messages"][2]["role"], "tool");
        assert_eq!(serde_json::from_value::<TypedRequestV1>(wire).unwrap(), req);
    }

    #[test]
    fn a_request_without_tools_omits_them_and_reads_back_empty() {
        let req = TypedRequestV1 {
            tools: Vec::new(),
            ..request()
        };
        let wire = serde_json::to_value(&req).unwrap();
        assert!(wire.get("tools").is_none());
        assert_eq!(serde_json::from_value::<TypedRequestV1>(wire).unwrap(), req);
    }

    #[test]
    fn a_response_round_trips_with_finish_reason_and_usage() {
        let response = TypedResponseV1 {
            schema_version: 1,
            parts: vec![
                PartV1::Text { text: "a".into() },
                PartV1::ToolCall {
                    id: "c".into(),
                    name: "n".into(),
                    arguments: json!({}),
                },
            ],
            finish_reason: FinishReasonV1::ToolCalls,
            usage: UsageV1 {
                input_tokens: Some(0),
                output_tokens: None,
                source: UsageSourceV1::Estimated,
            },
        };
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(wire["finishReason"], "toolCalls");
        assert_eq!(wire["usage"]["source"], "estimated");
        // A known zero stays zero; a missing count stays null.
        assert_eq!(wire["usage"]["inputTokens"], 0);
        assert!(wire["usage"]["outputTokens"].is_null());
        assert_eq!(
            serde_json::from_value::<TypedResponseV1>(wire).unwrap(),
            response
        );
    }

    #[test]
    fn an_unknown_schema_version_is_refused() {
        let mut wire = serde_json::to_value(request()).unwrap();
        wire["schemaVersion"] = json!(2);
        let err = serde_json::from_value::<TypedRequestV1>(wire)
            .unwrap_err()
            .to_string();
        assert!(err.contains("schemaVersion 2 is unsupported"), "{err}");

        let wire = json!({ "schemaVersion": 0, "parts": [], "finishReason": "stop",
                           "usage": { "inputTokens": null, "outputTokens": null, "source": "unknown" } });
        let err = serde_json::from_value::<TypedResponseV1>(wire)
            .unwrap_err()
            .to_string();
        assert!(err.contains("typed response schemaVersion 0"), "{err}");

        // A Rust-built value is checked by `validate_version`.
        let stale = TypedRequestV1 {
            schema_version: 7,
            ..request()
        };
        assert_eq!(stale.validate_version().unwrap_err().actual, 7);
        assert!(request().validate_version().is_ok());
    }

    #[test]
    fn an_unknown_part_kind_or_field_is_refused() {
        let part = json!({ "type": "audio", "data": "x" });
        assert!(serde_json::from_value::<PartV1>(part).is_err());
        let part = json!({ "type": "text", "text": "a", "extra": 1 });
        assert!(serde_json::from_value::<PartV1>(part).is_err());
        let wire = json!({ "schemaVersion": 1, "model": "m", "messages": [], "stream": true });
        assert!(serde_json::from_value::<TypedRequestV1>(wire).is_err());
        let wire = json!({ "schemaVersion": 1, "model": "m",
            "messages": [{ "role": "robot", "parts": [] }] });
        assert!(serde_json::from_value::<TypedRequestV1>(wire).is_err());
    }
}
