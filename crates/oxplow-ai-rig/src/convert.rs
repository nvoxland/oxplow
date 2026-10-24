//! oxplow's typed parts (`oxplow_ai::parts`) to Rig's messages and tool
//! definitions, and Rig's reply back to a V1 response.

use std::sync::Arc;

use oxplow_ai::capabilities::CapabilitySupport;
use oxplow_ai::client::AiError;
use oxplow_ai::parts::{
    FinishReasonV1, MessageRoleV1, PartV1, TypedRequestV1, TypedResponseV1, UsageSourceV1, UsageV1,
};
use rig_core::completion::{
    AssistantContent, AssistantMessage, CompletionRequest, CompletionResponse, FinishReason,
    Message, ToolDefinition, Usage,
};
use rig_core::message::{
    CallId, ToolCall, ToolFunction, ToolName, ToolResult, ToolResultContent, UserContent,
};
use serde_json::Value;

/// What a model can't be asked: `feature`, with what is known of it.
fn unsupported(provider: &str, feature: &str, support: CapabilitySupport) -> AiError {
    AiError::UnsupportedFeature {
        provider: provider.to_string(),
        feature: feature.to_string(),
        support,
    }
}

fn tool_name(provider: &str, name: &str) -> Result<ToolName, AiError> {
    ToolName::new(name).map_err(|_| AiError::BadRequest {
        provider: provider.to_string(),
        detail: "a tool needs a name".into(),
    })
}

/// A tool result's content: a string as text, anything else as JSON.
fn result_content(result: &Value) -> ToolResultContent {
    match result {
        Value::String(text) => ToolResultContent::text(text.clone()),
        other => ToolResultContent::json(other.clone()),
    }
}

/// `req` as Rig's request: its messages in order, its tools. A tool result
/// carries its call's tool name, which Rig's wires need; it is taken from
/// the call it answers, earlier in the conversation. Images are refused:
/// nothing resolves an asset here.
pub fn request(provider: &str, req: &TypedRequestV1) -> Result<CompletionRequest, AiError> {
    req.validate_version()?;
    let mut history: Vec<Message> = Vec::new();
    let mut calls: Vec<(String, String)> = Vec::new();
    for message in &req.messages {
        let image = |part: &PartV1| matches!(part, PartV1::Image { .. });
        if message.parts.iter().any(image) {
            return Err(unsupported(provider, "image", CapabilitySupport::Unknown));
        }
        match message.role {
            MessageRoleV1::System => {
                let text = message
                    .parts
                    .iter()
                    .map(|part| match part {
                        PartV1::Text { text } => Ok(text.as_str()),
                        _ => Err(unsupported(
                            provider,
                            "systemContent",
                            CapabilitySupport::Unsupported,
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .join("");
                history.push(Message::system(text));
            }
            MessageRoleV1::Assistant => {
                let mut content = Vec::new();
                for part in &message.parts {
                    match part {
                        PartV1::Text { text } => content.push(AssistantContent::text(text.clone())),
                        PartV1::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            calls.push((id.clone(), name.clone()));
                            content.push(AssistantContent::ToolCall(ToolCall::new(
                                CallId::from_wire(id.clone()),
                                ToolFunction::new(tool_name(provider, name)?, arguments.clone()),
                            )));
                        }
                        _ => {
                            return Err(unsupported(
                                provider,
                                "assistantToolResult",
                                CapabilitySupport::Unsupported,
                            ))
                        }
                    }
                }
                history.push(Message::Assistant(AssistantMessage::new(content)));
            }
            MessageRoleV1::User | MessageRoleV1::Tool => {
                let mut content = Vec::new();
                for part in &message.parts {
                    match part {
                        PartV1::Text { text } => content.push(UserContent::text(text.clone())),
                        PartV1::ToolResult { call_id, result } => {
                            let Some((_, name)) = calls.iter().find(|(id, _)| id == call_id) else {
                                return Err(AiError::BadRequest {
                                    provider: provider.to_string(),
                                    detail: format!(
                                        "a tool result answers `{call_id}`, a call the \
                                         conversation doesn't hold"
                                    ),
                                });
                            };
                            content.push(UserContent::ToolResult(ToolResult {
                                call: CallId::from_wire(call_id.clone()),
                                name: tool_name(provider, name)?,
                                content: vec![result_content(result)],
                                is_error: false,
                            }));
                        }
                        _ => {
                            return Err(unsupported(
                                provider,
                                "userToolCall",
                                CapabilitySupport::Unsupported,
                            ))
                        }
                    }
                }
                history.push(Message::User { content });
            }
        }
    }
    let Some(first) = history.first().cloned() else {
        return Err(unsupported(
            provider,
            "userMessage",
            CapabilitySupport::Unsupported,
        ));
    };
    let mut out = CompletionRequest::new(first);
    out.chat_history = history.into_iter().map(Arc::new).collect();
    let tools = req
        .tools
        .iter()
        .map(|tool| {
            Ok(ToolDefinition::new(
                tool_name(provider, &tool.name)?,
                tool.description.clone().unwrap_or_default(),
                tool.parameters.clone(),
            ))
        })
        .collect::<Result<Vec<_>, AiError>>()?;
    Ok(out.tools(tools))
}

/// oxplow's usage: what the provider counted. Rig's input count already
/// holds cache reads and writes.
pub fn usage(usage: &Usage) -> UsageV1 {
    let source = if usage.input_tokens.is_some() || usage.output_tokens.is_some() {
        UsageSourceV1::Observed
    } else {
        UsageSourceV1::Unknown
    };
    UsageV1 {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        source,
    }
}

pub fn finish(reason: Option<FinishReason>) -> FinishReasonV1 {
    match reason {
        None => FinishReasonV1::Unknown,
        Some(FinishReason::Stop) => FinishReasonV1::Stop,
        Some(FinishReason::Length) => FinishReasonV1::Length,
        Some(FinishReason::ToolCalls) => FinishReasonV1::ToolCalls,
        Some(_) => FinishReasonV1::Other,
    }
}

/// One assistant block as a part: text and tool calls. Reasoning and
/// provider items with no meaning here are left out; an image is refused,
/// as nothing holds image output.
pub fn part(provider: &str, content: &AssistantContent) -> Result<Option<PartV1>, AiError> {
    Ok(match content {
        AssistantContent::Text(text) => Some(PartV1::Text {
            text: text.text.clone(),
        }),
        AssistantContent::ToolCall(call) => Some(PartV1::ToolCall {
            id: call.id.wire().into_owned(),
            name: call.function.name.to_string(),
            arguments: call.function.arguments_value(),
        }),
        AssistantContent::Image(_) => {
            return Err(AiError::BadResponse {
                provider: provider.to_string(),
                detail: "the model answered with an image, which oxplow can't hold".into(),
            })
        }
        // Reasoning, and provider items with no meaning here.
        _ => None,
    })
}

/// Rig's reply as a V1 response. A reply the provider marked failed (a
/// refusal) is an error.
pub fn response(provider: &str, reply: &CompletionResponse) -> Result<TypedResponseV1, AiError> {
    if let Some(error) = &reply.error {
        return Err(AiError::BadResponse {
            provider: provider.to_string(),
            detail: error.clone(),
        });
    }
    let mut parts = Vec::new();
    for content in &reply.choice {
        if let Some(part) = part(provider, content)? {
            parts.push(part);
        }
    }
    Ok(TypedResponseV1 {
        schema_version: 1,
        parts,
        finish_reason: finish(reply.finish_reason()),
        usage: usage(&reply.usage),
    })
}
