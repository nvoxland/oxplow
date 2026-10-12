//! A model that answers from a list, in-process: what a preview of an
//! extension (`oxplow extension test`, `preview_extension`) and the tests
//! run their model calls against, so nothing is sent and nothing is spent.
//! Registered as a host kind (`ModelProviders::register_host`), never
//! declared by an extension.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::client::{
    unless_cancelled, AiError, Cancel, CompleteRequest, Completion, ModelProvider, OnDelta,
    ProviderInstance,
};
use crate::parts::{
    DeltaFold, FinishReasonV1, PartV1, StreamDeltaV1, TypedRequestV1, TypedResponseV1,
    UsageSourceV1, UsageV1,
};

/// Its kind, as `ai.yaml`'s `kind:` names it.
pub const KIND: &str = "fixture";

/// One answer: its text, the tools it calls (after the text), and how long
/// it waits before each word when it streams (so a test can cancel it
/// mid-way). A plain string is a text answer.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(from = "ReplyRepr")]
pub struct FixtureReply {
    pub text: String,
    pub tool_calls: Vec<FixtureToolCall>,
    pub chunk_delay: Duration,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureToolCall {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ReplyRepr {
    Text(String),
    Full(FullReply),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FullReply {
    #[serde(default)]
    text: String,
    #[serde(default)]
    tool_calls: Vec<FixtureToolCall>,
    #[serde(default)]
    chunk_delay_ms: u64,
}

impl From<ReplyRepr> for FixtureReply {
    fn from(repr: ReplyRepr) -> Self {
        match repr {
            ReplyRepr::Text(text) => text.into(),
            ReplyRepr::Full(full) => Self {
                text: full.text,
                tool_calls: full.tool_calls,
                chunk_delay: Duration::from_millis(full.chunk_delay_ms),
            },
        }
    }
}

impl From<String> for FixtureReply {
    fn from(text: String) -> Self {
        Self {
            text,
            ..Self::default()
        }
    }
}

impl From<&str> for FixtureReply {
    fn from(text: &str) -> Self {
        text.to_string().into()
    }
}

/// Answers with each reply in turn; the last repeats. Usage is unknown.
pub struct FixtureModel {
    replies: Mutex<VecDeque<FixtureReply>>,
    last: Mutex<FixtureReply>,
    asked: Mutex<Vec<String>>,
    asked_typed: Mutex<Vec<TypedRequestV1>>,
    calls: Mutex<u32>,
}

impl FixtureModel {
    pub fn new(replies: Vec<FixtureReply>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            last: Mutex::new(FixtureReply::default()),
            asked: Mutex::new(Vec::new()),
            asked_typed: Mutex::new(Vec::new()),
            calls: Mutex::new(0),
        }
    }

    /// The text prompts it was asked, in order.
    pub fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The typed requests it was asked, in order (a tool round's included).
    pub fn asked_typed(&self) -> Vec<TypedRequestV1> {
        self.asked_typed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn next(&self) -> FixtureReply {
        let next = self
            .replies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front();
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(next) = next {
            *last = next;
        }
        last.clone()
    }

    /// The reply's tool calls as parts, each with an id unique to this
    /// model (`call-<n>`).
    fn tool_calls(&self, reply: &FixtureReply) -> Vec<PartV1> {
        let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
        reply
            .tool_calls
            .iter()
            .map(|call| {
                *calls += 1;
                PartV1::ToolCall {
                    id: format!("call-{calls}"),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                }
            })
            .collect()
    }

    fn typed(&self, req: &TypedRequestV1) -> (FixtureReply, TypedResponseV1) {
        self.asked_typed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(req.clone());
        let reply = self.next();
        let mut parts = Vec::new();
        if !reply.text.is_empty() {
            parts.push(PartV1::Text {
                text: reply.text.clone(),
            });
        }
        parts.extend(self.tool_calls(&reply));
        let finish_reason = if reply.tool_calls.is_empty() {
            FinishReasonV1::Stop
        } else {
            FinishReasonV1::ToolCalls
        };
        let response = TypedResponseV1 {
            schema_version: 1,
            parts,
            finish_reason,
            usage: UsageV1 {
                input_tokens: None,
                output_tokens: None,
                source: UsageSourceV1::Unknown,
            },
        };
        (reply, response)
    }
}

#[async_trait::async_trait]
impl ModelProvider for FixtureModel {
    fn kind(&self) -> &str {
        KIND
    }
    fn title(&self) -> &str {
        "Fixture replies"
    }
    fn default_base_url(&self) -> Option<&str> {
        Some("fixture:")
    }
    async fn complete(
        &self,
        _instance: &ProviderInstance<'_>,
        req: &CompleteRequest<'_>,
    ) -> Result<Completion, AiError> {
        self.asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(req.prompt.to_string());
        Ok(Completion {
            text: self.next().text,
            input_tokens: 0,
            output_tokens: 0,
        })
    }

    /// Any conversation, tools included: the next reply, its tool calls
    /// after its text.
    async fn complete_typed(
        &self,
        _instance: &ProviderInstance<'_>,
        req: &TypedRequestV1,
    ) -> Result<TypedResponseV1, AiError> {
        req.validate_version()?;
        Ok(self.typed(req).1)
    }

    /// The next reply a word at a time, each after its `chunk_delay`, then
    /// its tool calls.
    async fn stream_typed(
        &self,
        instance: &ProviderInstance<'_>,
        req: &TypedRequestV1,
        cancel: &Cancel,
        on_delta: OnDelta<'_>,
    ) -> Result<TypedResponseV1, AiError> {
        req.validate_version()?;
        let (reply, response) = self.typed(req);
        let mut fold = DeltaFold::default();
        let mut send = |delta: StreamDeltaV1, fold: &mut DeltaFold| {
            fold.push(&delta);
            on_delta(delta);
        };
        for word in reply.text.split_inclusive(' ') {
            if let Err(stop) = unless_cancelled(cancel, tokio::time::sleep(reply.chunk_delay)).await
            {
                return Err(stop.error(instance.id, fold.into_partial()));
            }
            send(
                StreamDeltaV1::Text {
                    text: word.to_string(),
                },
                &mut fold,
            );
        }
        for part in &response.parts {
            if let PartV1::ToolCall {
                id,
                name,
                arguments,
            } = part
            {
                send(
                    StreamDeltaV1::ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: arguments.clone(),
                    },
                    &mut fold,
                );
            }
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parts::{MessageRoleV1, MessageV1};
    use serde_json::json;

    fn instance() -> ProviderInstance<'static> {
        ProviderInstance {
            id: "f",
            base_url: None,
            key: None,
        }
    }

    fn ask(text: &str) -> TypedRequestV1 {
        TypedRequestV1 {
            schema_version: 1,
            model: "m".into(),
            messages: vec![MessageV1 {
                role: MessageRoleV1::User,
                parts: vec![PartV1::Text { text: text.into() }],
            }],
            tools: Vec::new(),
        }
    }

    #[tokio::test]
    async fn it_answers_each_reply_in_turn_then_repeats_the_last() {
        let model = FixtureModel::new(vec!["one".into(), "two".into()]);
        let ask = |prompt: &'static str| CompleteRequest {
            model: "m",
            system: None,
            prompt,
            json: false,
        };
        let mut said = Vec::new();
        for prompt in ["a", "b", "c"] {
            said.push(
                model
                    .complete(&instance(), &ask(prompt))
                    .await
                    .unwrap()
                    .text,
            );
        }
        assert_eq!(said, ["one", "two", "two"]);
        assert_eq!(model.asked(), ["a", "b", "c"]);
    }

    /// A reply is a string or `{ text?, toolCalls?, chunkDelayMs? }`.
    #[test]
    fn a_reply_reads_as_a_string_or_its_whole_shape() {
        let replies: Vec<FixtureReply> = serde_json::from_value(json!([
            "plain",
            { "toolCalls": [{ "name": "look", "arguments": { "path": "a" } }], "chunkDelayMs": 5 }
        ]))
        .unwrap();
        assert_eq!(replies[0], "plain".into());
        assert_eq!(replies[1].tool_calls[0].name, "look");
        assert_eq!(replies[1].chunk_delay, Duration::from_millis(5));
        assert!(serde_json::from_value::<FixtureReply>(json!({ "txt": "x" })).is_err());
    }

    /// A typed answer is its text, then its tool calls, each with its own
    /// id; one with calls finishes `toolCalls`.
    #[tokio::test]
    async fn a_typed_answer_carries_its_tool_calls() {
        let model = FixtureModel::new(vec![
            FixtureReply {
                text: "looking".into(),
                tool_calls: vec![FixtureToolCall {
                    name: "look".into(),
                    arguments: json!({ "path": "a" }),
                }],
                ..FixtureReply::default()
            },
            "done".into(),
        ]);
        let first = model.complete_typed(&instance(), &ask("q")).await.unwrap();
        assert_eq!(first.finish_reason, FinishReasonV1::ToolCalls);
        assert_eq!(
            first.parts,
            [
                PartV1::Text {
                    text: "looking".into()
                },
                PartV1::ToolCall {
                    id: "call-1".into(),
                    name: "look".into(),
                    arguments: json!({ "path": "a" })
                }
            ]
        );
        let second = model.complete_typed(&instance(), &ask("r")).await.unwrap();
        assert_eq!(second.finish_reason, FinishReasonV1::Stop);
        assert_eq!(model.asked_typed().len(), 2);
    }

    /// Streamed, a reply comes a word at a time; cancelled mid-way, the
    /// error carries the words that had come.
    #[tokio::test]
    async fn it_streams_word_by_word_until_cancelled() {
        let model = FixtureModel::new(vec![FixtureReply {
            text: "one two three four".into(),
            chunk_delay: Duration::from_millis(150),
            ..FixtureReply::default()
        }]);
        let cancel = Cancel::default();
        let stopper = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(350)).await;
            stopper.cancel();
        });
        let mut words = Vec::new();
        let err = model
            .stream_typed(&instance(), &ask("q"), &cancel, &mut |d| words.push(d))
            .await
            .unwrap_err();
        let AiError::Cancelled { partial, .. } = err else {
            panic!("{err:?}")
        };
        assert_eq!(
            partial.parts,
            [PartV1::Text {
                text: "one two ".into()
            }]
        );
        assert_eq!(words.len(), 2);
    }
}
