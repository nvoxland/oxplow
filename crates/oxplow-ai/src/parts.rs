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
