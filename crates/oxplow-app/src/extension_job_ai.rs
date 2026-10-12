//! Bounded, role-based typed AI input for extension jobs.

use std::io::{self, Write};

use oxplow_ai::config::Role;
use oxplow_ai::parts::{MessageV1, ToolDefinitionV1, TypedRequestV1};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_INPUT_BYTES: usize = 1024 * 1024;
const MAX_INPUT_DEPTH: usize = 64;

/// A job supplies content and a role; the host supplies the selected model.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypedAiJobRequest {
    schema_version: u32,
    role: Role,
    messages: Vec<MessageV1>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ToolDefinitionV1>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    schema_version: u32,
    role: Role,
    messages: Vec<MessageV1>,
    #[serde(default)]
    tools: Vec<ToolDefinitionV1>,
}

#[derive(Debug, thiserror::Error)]
pub enum TypedAiJobRequestError {
    #[error("typed AI job input exceeds the size limit")]
    TooLarge,
    #[error("typed AI job input exceeds the depth limit")]
    TooDeep,
    #[error("invalid typed AI job input")]
    Invalid,
}

impl TypedAiJobRequest {
    pub fn from_value(value: &Value) -> Result<Self, TypedAiJobRequestError> {
        // Check nesting before invoking serde's recursive serializer. Walk
        // children one at a time so a wide input cannot grow an auxiliary stack.
        let mut remaining = MAX_INPUT_BYTES;
        check_shape(value, 1, &mut remaining)?;
        let mut writer = CountingWriter { bytes: 0 };
        serde_json::to_writer(&mut writer, value).map_err(|_| TypedAiJobRequestError::TooLarge)?;

        let input: Input =
            serde_json::from_value(value.clone()).map_err(|_| TypedAiJobRequestError::Invalid)?;
        if input.schema_version != 1 {
            return Err(TypedAiJobRequestError::Invalid);
        }
        Ok(Self {
            schema_version: 1,
            role: input.role,
            messages: input.messages,
            tools: input.tools,
        })
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn request_for_model(&self, model: &str) -> TypedRequestV1 {
        TypedRequestV1 {
            schema_version: 1,
            model: model.to_owned(),
            messages: self.messages.clone(),
            tools: self.tools.clone(),
        }
    }
}

fn check_shape(
    value: &Value,
    depth: usize,
    remaining: &mut usize,
) -> Result<(), TypedAiJobRequestError> {
    if depth > MAX_INPUT_DEPTH {
        return Err(TypedAiJobRequestError::TooDeep);
    }
    let size = match value {
        Value::String(text) => text.len().max(1),
        _ => 1,
    };
    *remaining = remaining
        .checked_sub(size)
        .ok_or(TypedAiJobRequestError::TooLarge)?;
    match value {
        Value::Array(items) => {
            for item in items {
                check_shape(item, depth + 1, remaining)?;
            }
        }
        Value::Object(fields) => {
            for (key, item) in fields {
                *remaining = remaining
                    .checked_sub(key.len())
                    .ok_or(TypedAiJobRequestError::TooLarge)?;
                check_shape(item, depth + 1, remaining)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Count encoded JSON without allocating a second copy of the input.
struct CountingWriter {
    bytes: usize,
}

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_INPUT_BYTES - self.bytes {
            return Err(io::Error::other("input size limit"));
        }
        self.bytes += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
