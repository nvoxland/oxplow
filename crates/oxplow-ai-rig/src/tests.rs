//! Each kind against a local stand-in for its API (`oxplow_ai_fake`): the
//! request shape on the wire (system, tools, history, keys), the reply
//! read back, streaming in order, cancel and errors.

use std::collections::BTreeMap;
use std::time::Duration;

use oxplow_ai::capabilities::CapabilitySupport;
use oxplow_ai::client::{
    AiError, Answer, Cancel, CompleteRequest, DecideRequest, ModelProvider, ProviderInstance,
    Question,
};
use oxplow_ai::parts::{
    FinishReasonV1, MessageRoleV1, MessageV1, PartV1, StreamDeltaV1, ToolDefinitionV1,
    TypedRequestV1, TypedResponseV1, UsageSourceV1,
};
use oxplow_ai_fake::{mock, mock_sse};
use serde_json::{json, Value};

use crate::{completion_model, provider, KINDS};

fn shipped(kind: &str, base: Option<&str>) -> std::sync::Arc<dyn ModelProvider> {
    provider(kind, kind, kind, base).expect("a kind")
}

fn at<'a>(base: &'a str, key: Option<&'a str>) -> ProviderInstance<'a> {
    ProviderInstance {
        id: "p",
        base_url: Some(base),
        key,
    }
}

fn ask(system: Option<&'static str>, json: bool) -> CompleteRequest<'static> {
    CompleteRequest {
        model: "m",
        system,
        prompt: "hello",
        json,
    }
}

/// A conversation with a tool round in it: the user asks, the model
/// called `grep`, its result came back.
fn tool_round() -> TypedRequestV1 {
    TypedRequestV1 {
        schema_version: 1,
        model: "m".into(),
        tools: vec![ToolDefinitionV1 {
            name: "grep".into(),
            description: Some("search".into()),
            parameters: json!({ "type": "object", "properties": { "q": { "type": "string" } } }),
        }],
        messages: vec![
            MessageV1 {
                role: MessageRoleV1::System,
                parts: vec![PartV1::Text {
                    text: "be brief".into(),
                }],
            },
            MessageV1 {
                role: MessageRoleV1::User,
                parts: vec![PartV1::Text {
                    text: "find x".into(),
                }],
            },
            MessageV1 {
                role: MessageRoleV1::Assistant,
                parts: vec![PartV1::ToolCall {
                    id: "c0".into(),
                    name: "grep".into(),
                    arguments: json!({ "q": "y" }),
                }],
            },
            MessageV1 {
                role: MessageRoleV1::Tool,
                parts: vec![PartV1::ToolResult {
                    call_id: "c0".into(),
                    result: json!(["hit"]),
                }],
            },
        ],
    }
}

fn sse(event: Option<&str>, data: Value) -> String {
    match event {
        Some(event) => format!("event: {event}\ndata: {data}\n\n"),
        None => format!("data: {data}\n\n"),
    }
}

/// A recorded Messages API stream: text, a tool call whose input arrives
/// in fragments, more text, usage with cache counts.
fn anthropic_events() -> Vec<String> {
    let ev = |data: Value| {
        let kind = data["type"].as_str().unwrap().to_string();
        sse(Some(&kind), data)
    };
    vec![
        ev(
            json!({ "type": "message_start", "message": { "id": "msg_1", "type": "message", "role": "assistant",
            "model": "m", "content": [], "stop_reason": null, "stop_sequence": null,
            "usage": { "input_tokens": 10, "cache_creation_input_tokens": 5,
                       "cache_read_input_tokens": 100, "output_tokens": 1 } } }),
        ),
        ev(json!({ "type": "content_block_start", "index": 0,
            "content_block": { "type": "text", "text": "" } })),
        sse(Some("ping"), json!({ "type": "ping" })),
        ev(json!({ "type": "content_block_delta", "index": 0,
            "delta": { "type": "text_delta", "text": "on" } })),
        ev(json!({ "type": "content_block_delta", "index": 0,
            "delta": { "type": "text_delta", "text": "e" } })),
        ev(json!({ "type": "content_block_stop", "index": 0 })),
        ev(json!({ "type": "content_block_start", "index": 1,
            "content_block": { "type": "tool_use", "id": "t1", "name": "grep", "input": {} } })),
        ev(json!({ "type": "content_block_delta", "index": 1,
            "delta": { "type": "input_json_delta", "partial_json": "{\"q\": " } })),
        ev(json!({ "type": "content_block_delta", "index": 1,
            "delta": { "type": "input_json_delta", "partial_json": "\"x\"}" } })),
        ev(json!({ "type": "content_block_stop", "index": 1 })),
        ev(json!({ "type": "content_block_start", "index": 2,
            "content_block": { "type": "text", "text": "" } })),
        ev(json!({ "type": "content_block_delta", "index": 2,
            "delta": { "type": "text_delta", "text": "two" } })),
        ev(json!({ "type": "content_block_stop", "index": 2 })),
        ev(json!({ "type": "message_delta",
            "delta": { "stop_reason": "tool_use", "stop_sequence": null },
            "usage": { "output_tokens": 7 } })),
        ev(json!({ "type": "message_stop" })),
    ]
}

/// A recorded chat-completions stream: text, a tool call whose arguments
/// arrive in fragments, a second call, the finish reason, the usage chunk
/// and `[DONE]`.
fn openai_chunks() -> Vec<String> {
    let chunk = |delta: Value, finish: Value| {
        sse(
            None,
            json!({ "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m",
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }] }),
        )
    };
    vec![
        chunk(json!({ "role": "assistant", "content": "" }), Value::Null),
        chunk(json!({ "content": "check" }), Value::Null),
        chunk(json!({ "content": "ing" }), Value::Null),
        chunk(
            json!({ "tool_calls": [{ "index": 0, "id": "c1", "type": "function",
                "function": { "name": "a", "arguments": "" } }] }),
            Value::Null,
        ),
        chunk(
            json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "{\"n\":" } }] }),
            Value::Null,
        ),
        chunk(
            json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "1}" } }] }),
            Value::Null,
        ),
        chunk(
            json!({ "tool_calls": [{ "index": 1, "id": "c2", "type": "function",
                "function": { "name": "b", "arguments": "{}" } }] }),
            Value::Null,
        ),
        chunk(json!({}), json!("tool_calls")),
        sse(
            None,
            json!({ "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m", "choices": [],
            "usage": { "prompt_tokens": 20, "completion_tokens": 9, "total_tokens": 29 } }),
        ),
        "data: [DONE]\n\n".into(),
    ]
}

/// What `stream_typed` of `kind` over `chunks` gives, the deltas it
/// handed over, and the request body.
async fn streamed(
    kind: &str,
    path: &'static str,
    chunks: Vec<String>,
) -> (Result<TypedResponseV1, AiError>, Vec<StreamDeltaV1>, Value) {
    let (base, seen, _) = mock_sse(path, chunks, Duration::ZERO, Duration::ZERO).await;
    let mut deltas = Vec::new();
    let out = shipped(kind, None)
        .stream_typed(
            &at(&base, Some("k1")),
            &tool_round(),
            &Cancel::default(),
            &mut |d| deltas.push(d),
        )
        .await;
    let body = seen.lock().unwrap()[0].2.clone();
    (out, deltas, body)
}

#[tokio::test]
async fn anthropic_speaks_messages() {
    let (base, seen) = mock(
        "/v1/messages",
        200,
        json!({ "id": "msg", "type": "message", "role": "assistant", "model": "claude-x",
                "content": [{"type": "text", "text": "hi"}], "stop_reason": "end_turn",
                "usage": {"input_tokens": 7, "output_tokens": 2}}),
    )
    .await;
    let c = shipped("anthropic", Some("https://api.anthropic.com"))
        .complete(&at(&base, Some("k1")), &ask(Some("be brief"), false))
        .await
        .unwrap();
    assert_eq!(
        (c.text.as_str(), c.input_tokens, c.output_tokens),
        ("hi", 7, 2)
    );
    let (_, headers, body) = seen.lock().unwrap()[0].clone();
    assert_eq!(headers["x-api-key"], "k1");
    assert!(headers.contains_key("anthropic-version"));
    assert!(!headers.contains_key("authorization"));
    assert_eq!(body["model"], "m");
    assert!(body["system"].to_string().contains("be brief"), "{body}");
    assert!(body["messages"][0]["content"].to_string().contains("hello"));
    assert!(body["max_tokens"].as_u64().is_some_and(|n| n > 0));
}

/// A typed call carries its system text, history and tools; a reply's
/// text and tool calls come back in order with the stop reason, and the
/// input count holds the cached input.
#[tokio::test]
async fn anthropic_typed_carries_the_conversation_and_tools() {
    let (base, seen) = mock(
        "/v1/messages",
        200,
        json!({ "id": "msg", "type": "message", "role": "assistant", "model": "m",
            "content": [
                {"type": "text", "text": "first"},
                {"type": "tool_use", "id": "t1", "name": "grep", "input": {"q": "x"}},
                {"type": "text", "text": "last"}],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "cache_read_input_tokens": 100,
                      "cache_creation_input_tokens": 5, "output_tokens": 6}}),
    )
    .await;
    let out = shipped("anthropic", None)
        .complete_typed(&at(&base, Some("k1")), &tool_round())
        .await
        .unwrap();
    assert_eq!(
        out.parts,
        vec![
            PartV1::Text {
                text: "first".into()
            },
            PartV1::ToolCall {
                id: "t1".into(),
                name: "grep".into(),
                arguments: json!({"q": "x"})
            },
            PartV1::Text {
                text: "last".into()
            },
        ]
    );
    assert_eq!(out.finish_reason, FinishReasonV1::ToolCalls);
    assert_eq!(out.usage.input_tokens, Some(115));
    assert_eq!(out.usage.output_tokens, Some(6));
    assert_eq!(out.usage.source, UsageSourceV1::Observed);

    let body = seen.lock().unwrap()[0].2.clone();
    assert!(body["system"].to_string().contains("be brief"));
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3, "{body}");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"][0]["type"], "tool_use");
    assert_eq!(messages[1]["content"][0]["id"], "c0");
    assert_eq!(messages[1]["content"][0]["input"], json!({"q": "y"}));
    assert_eq!(messages[2]["role"], "user");
    assert_eq!(messages[2]["content"][0]["type"], "tool_result");
    assert_eq!(messages[2]["content"][0]["tool_use_id"], "c0");
    assert!(messages[2]["content"][0].to_string().contains("hit"));
    assert_eq!(body["tools"][0]["name"], "grep");
    assert_eq!(body["tools"][0]["description"], "search");
    assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
}

/// Streamed, text arrives as it comes and a tool call once its input is
/// whole, in order; the reply is what a whole call would give.
#[tokio::test]
async fn anthropic_streams_in_order() {
    let (out, deltas, body) = streamed("anthropic", "/v1/messages", anthropic_events()).await;
    let out = out.unwrap();
    assert_eq!(body["stream"], true);
    assert_eq!(
        deltas,
        vec![
            StreamDeltaV1::Text { text: "on".into() },
            StreamDeltaV1::Text { text: "e".into() },
            StreamDeltaV1::ToolCall {
                id: "t1".into(),
                name: "grep".into(),
                arguments: json!({"q": "x"})
            },
            StreamDeltaV1::Text { text: "two".into() },
        ]
    );
    assert_eq!(StreamDeltaV1::of(&out.parts).len(), 3);
    assert!(matches!(&out.parts[0], PartV1::Text { text } if text == "one"));
    assert_eq!(out.finish_reason, FinishReasonV1::ToolCalls);
    assert_eq!(out.usage.input_tokens, Some(115));
    assert_eq!(out.usage.output_tokens, Some(7));
}

/// A stream that ends before the message is whole is an error, never a
/// partial reply.
#[tokio::test]
async fn a_truncated_stream_is_an_error() {
    let mut events = anthropic_events();
    events.truncate(5);
    let (out, _, _) = streamed("anthropic", "/v1/messages", events).await;
    assert!(matches!(out, Err(AiError::BadResponse { .. })), "{out:?}");
}

/// An OpenAI-compatible server: a local one takes no key and gets no auth
/// header and has no default URL; JSON mode asks for an object; a typed
/// question is asked as a chat; a refused key and a rate limit read as
/// such.
#[tokio::test]
async fn openai_compatible_speaks_chat() {
    let (base, seen) = mock(
        "/chat/completions",
        200,
        json!({"id": "c", "object": "chat.completion", "created": 1, "model": "m",
               "choices": [{"index": 0, "message": {"role": "assistant", "content": "{\"a\":1}"}, "finish_reason": "stop"}],
               "usage": {"prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8}}),
    )
    .await;
    let local = shipped("openai_compatible", None);
    assert_eq!(local.default_base_url(), None);
    let c = local
        .complete(&at(&base, None), &ask(None, true))
        .await
        .unwrap();
    assert_eq!(
        (c.text.as_str(), c.input_tokens, c.output_tokens),
        ("{\"a\":1}", 5, 3)
    );
    let (_, headers, body) = seen.lock().unwrap()[0].clone();
    assert!(
        !headers.contains_key("authorization"),
        "a local server gets no auth header"
    );
    assert_eq!(body["response_format"]["type"], "json_object");
    assert!(body["messages"].to_string().contains("hello"));

    let reply = json!({"answers": {"risky": {"type": "noul", "probability": 0.3}}});
    let (base, seen) = mock(
        "/chat/completions",
        200,
        json!({"id": "c", "object": "chat.completion", "created": 1, "model": "m",
               "choices": [{"index": 0, "message": {"role": "assistant", "content": reply.to_string()}, "finish_reason": "stop"}],
               "usage": {"prompt_tokens": 50, "completion_tokens": 20, "total_tokens": 70}}),
    )
    .await;
    let qs = BTreeMap::from([(
        "risky".to_string(),
        Question::Noul {
            instructions: "Is this change risky?".into(),
        },
    )]);
    let d = local
        .decide(
            &at(&base, Some("sk")),
            &DecideRequest {
                model: "qwen3",
                state: "diff…",
                questions: &qs,
            },
        )
        .await
        .unwrap();
    assert_eq!(d.answers["risky"], Answer::Noul { probability: 0.3 });
    assert_eq!(d.input_tokens, 50);
    let (_, headers, body) = seen.lock().unwrap()[0].clone();
    assert_eq!(headers["authorization"], "Bearer sk");
    assert!(body["messages"]
        .to_string()
        .contains("Is this change risky?"));

    let (base, _) = mock("/chat/completions", 401, json!({"error": "bad key"})).await;
    assert!(matches!(
        local
            .complete(&at(&base, Some("x")), &ask(None, false))
            .await,
        Err(AiError::Auth { .. })
    ));
    let (base, _) = mock("/chat/completions", 429, json!({})).await;
    assert!(matches!(
        local
            .complete(&at(&base, Some("x")), &ask(None, false))
            .await,
        Err(AiError::RateLimited { .. })
    ));
    let err = local
        .complete(
            &ProviderInstance {
                id: "p",
                base_url: None,
                key: None,
            },
            &ask(None, false),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AiError::BadRequest { .. }), "{err}");
}

/// Chat Completions carries the history (a call, then its result as a
/// `tool` message) and the tools; its stream folds argument fragments
/// into whole calls, in order, with the usage chunk.
#[tokio::test]
async fn openai_compatible_typed_and_streamed() {
    let (out, deltas, body) =
        streamed("openai_compatible", "/chat/completions", openai_chunks()).await;
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(messages[2]["tool_calls"][0]["id"], "c0");
    assert_eq!(messages[2]["tool_calls"][0]["function"]["name"], "grep");
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["tool_call_id"], "c0");
    assert_eq!(body["tools"][0]["function"]["name"], "grep");
    assert_eq!(body["stream"], true);

    let out = out.unwrap();
    assert_eq!(
        deltas,
        vec![
            StreamDeltaV1::Text {
                text: "check".into()
            },
            StreamDeltaV1::Text { text: "ing".into() },
            StreamDeltaV1::ToolCall {
                id: "c1".into(),
                name: "a".into(),
                arguments: json!({"n": 1})
            },
            StreamDeltaV1::ToolCall {
                id: "c2".into(),
                name: "b".into(),
                arguments: json!({})
            },
        ]
    );
    assert_eq!(out.parts.len(), 3);
    assert_eq!(out.finish_reason, FinishReasonV1::ToolCalls);
    assert_eq!(
        (out.usage.input_tokens, out.usage.output_tokens),
        (Some(20), Some(9))
    );
}

/// Cancelled mid-stream, the call lets go of the connection and reports
/// the text that had come.
#[tokio::test]
async fn a_cancelled_stream_reports_what_had_come() {
    let (base, _, written) = mock_sse(
        "/v1/messages",
        anthropic_events(),
        Duration::from_millis(100),
        Duration::from_secs(30),
    )
    .await;
    let cancel = Cancel::default();
    let mut deltas = Vec::new();
    let asked = cancel.clone();
    let mut on = |d: StreamDeltaV1| {
        deltas.push(d);
        asked.cancel();
    };
    let err = shipped("anthropic", None)
        .stream_typed(&at(&base, Some("k")), &tool_round(), &cancel, &mut on)
        .await
        .unwrap_err();
    let AiError::Cancelled { partial, .. } = err else {
        panic!("{err:?}");
    };
    assert_eq!(partial.parts, vec![PartV1::Text { text: "on".into() }]);
    let sent = written.load(std::sync::atomic::Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        written.load(std::sync::atomic::Ordering::SeqCst) <= sent + 1,
        "the server stops writing once the client is gone"
    );
}

/// OpenRouter chats like OpenAI; a Jev model answers typed questions
/// natively at `/systemone`.
#[tokio::test]
async fn openrouter_decides_natively_for_jev() {
    let (base, seen) = mock(
        "/systemone",
        200,
        json!({"answers": {"area": {"type": "choice", "choice": "db", "probabilities": {"ui": 0.1, "db": 0.9}}},
               "usage": {"input_tokens": 3, "output_tokens": 0}}),
    )
    .await;
    let qs = BTreeMap::from([(
        "area".to_string(),
        Question::Choice {
            instructions: "Which area?".into(),
            options: vec!["ui".into(), "db".into()],
        },
    )]);
    let d = shipped("openrouter", Some("https://openrouter.ai/api/v1"))
        .decide(
            &at(&base, Some("or")),
            &DecideRequest {
                model: "typesafe/jev",
                state: "diff…",
                questions: &qs,
            },
        )
        .await
        .unwrap();
    assert!(matches!(&d.answers["area"], Answer::Choice { choice, .. } if choice == "db"));
    let (_, headers, body) = seen.lock().unwrap()[0].clone();
    assert_eq!(headers["authorization"], "Bearer or");
    assert_eq!(body["questions"]["area"]["criteria"]["db"], Value::Null);
}

/// The official OpenAI kind speaks Rig's flagship route for it, the
/// Responses API.
#[tokio::test]
async fn openai_speaks_responses() {
    let (base, seen) = mock(
        "/responses",
        200,
        json!({"id": "resp_1", "object": "response", "created_at": 1, "status": "completed",
               "model": "m", "output": [{"type": "message", "id": "msg_1", "status": "completed",
               "role": "assistant", "content": [{"type": "output_text", "text": "hi", "annotations": []}]}],
               "usage": {"input_tokens": 4, "output_tokens": 1, "total_tokens": 5}}),
    )
    .await;
    let c = shipped("openai", Some("https://api.openai.com/v1"))
        .complete(&at(&base, Some("sk")), &ask(Some("be brief"), false))
        .await
        .unwrap();
    assert_eq!((c.text.as_str(), c.input_tokens), ("hi", 4));
    let (_, headers, body) = seen.lock().unwrap()[0].clone();
    assert_eq!(headers["authorization"], "Bearer sk");
    assert!(body.to_string().contains("be brief"));
}

/// Discovery reads Rig's built-in catalog: no call, no key. A hosted
/// kind lists its models; a local one's are unknown; a model the catalog
/// doesn't list is unknown across the board.
#[tokio::test]
async fn discovery_reads_the_catalog_without_a_call() {
    let none = ProviderInstance {
        id: "p",
        base_url: Some("http://127.0.0.1:9"),
        key: None,
    };
    let anthropic = shipped("anthropic", None);
    let catalog = anthropic.discover_models(&none).await.unwrap();
    assert!(catalog.catalog_known);
    assert!(catalog.validate().is_ok());
    let haiku = catalog
        .models
        .iter()
        .find(|m| m.model.as_deref() == Some("claude-haiku-4-5"))
        .expect("Haiku 4.5 is listed");
    assert_eq!(haiku.tool_calling, CapabilitySupport::Supported);
    assert_eq!(haiku.image, CapabilitySupport::Supported);
    assert!(haiku.context_tokens.is_some());
    let snap = anthropic.discover(&none, "claude-haiku-4-5").await.unwrap();
    assert!(snap.model_known);
    assert_eq!(
        anthropic.discover(&none, "no-such-model").await.unwrap(),
        oxplow_ai::capabilities::CapabilitySnapshotV1::unknown("no-such-model")
    );
    for local in ["ollama", "openai_compatible", "llamacpp"] {
        let catalog = shipped(local, None).discover_models(&none).await.unwrap();
        assert!(!catalog.catalog_known, "{local}");
    }
}

/// Every kind but TypeSafe yields a Rig model handle from an instance.
#[test]
fn every_rig_kind_builds_a_model_handle() {
    let instance = ProviderInstance {
        id: "p",
        base_url: Some("http://127.0.0.1:9/v1"),
        key: Some("k"),
    };
    for kind in KINDS {
        let built = completion_model(kind.name, None, &instance, "some-model");
        if kind.name == "typesafe" {
            assert!(built.is_err());
        } else {
            let model = built.unwrap_or_else(|e| panic!("{}: {e}", kind.name));
            assert_eq!(model.id(), Some("some-model"), "{}", kind.name);
        }
    }
    assert!(completion_model("nope", None, &instance, "m").is_err());
}

/// An unknown kind names the known ones; each hosted kind's Rig default
/// URL is known.
#[test]
fn kinds_are_named_and_known() {
    let err = provider("nope", "x", "X", None).err().unwrap().to_string();
    assert!(
        err.contains("anthropic") && err.contains("typesafe"),
        "{err}"
    );
    let anthropic = crate::kind("anthropic").unwrap();
    assert_eq!(
        anthropic.rig_base_url().as_deref(),
        Some("https://api.anthropic.com")
    );
    assert_eq!(
        crate::kind("openai_compatible").unwrap().rig_base_url(),
        None
    );
    assert!(crate::kind("openrouter").unwrap().decides_natively);
    assert!(!crate::kind("openai").unwrap().decides_natively);
}
