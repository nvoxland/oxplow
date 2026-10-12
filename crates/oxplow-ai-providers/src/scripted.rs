//! A model provider written as a Starlark script (`.context/ai-providers.md`
//! "Scripted providers"): the script says what to send and what came back;
//! the host makes the one HTTP call.
//!
//! The script defines two pure functions:
//!
//! - `request(x)` — `x` is `{ op, model, system, prompt, json }` for
//!   `complete` or `{ op, model, state, questions }` for `decide` — and
//!   returns `{ path, headers?, body }`. `path` is relative to the
//!   instance's base URL (its configured `baseUrl`, else the declaration's),
//!   so a call never leaves that host. `{{key}}` in a header or the path is
//!   the instance's key, spliced in by the host after `request` returns —
//!   the script never sees it; a header that needs a key the instance
//!   doesn't have is left out. For `decide`, `None` means "ask it as a
//!   chat" ([`decide_via_chat`]).
//! - `response(x)` — `x` is `{ op, model, body }` (a 2xx reply's JSON) —
//!   and returns `{ text, usage? }` for `complete`, `{ answers, usage? }`
//!   for `decide` (oxplow's answer shapes, a noul's as `probability`), or
//!   `{ error }` when the reply isn't usable. `usage` is `{ input, output }`.
//!
//! A reply the host maps itself: 401/403 is `Auth`, 429 `RateLimited`,
//! anything else not 2xx `Http`. Its declaration's `config` is
//! `{ baseUrl?, ops?, models? }`: `ops` lists what it answers natively,
//! `complete` (the default), `completeTyped`, `streamTyped` and `decide`;
//! without `decide` a typed question is asked as a chat.
//!
//! One that streams (`streamTyped`) defines a third function, `delta(x)`:
//! `request` gets `{ op: streamTyped, model, request }` and asks the
//! vendor for a stream; the host reads the reply's server-sent events and
//! hands each batch that arrived together to `delta` as `{ op, model,
//! events: [{ event, data }], state }`, which returns `{ deltas, state }`
//! (`deltas` are `StreamDeltaV1`s; `state` is the script's own fold,
//! carried to the next batch); at the end `response` gets `{ op, model,
//! state }` and returns the whole V1 reply. Cancelling drops the
//! connection; what the deltas had said is the partial reply.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use oxplow_ai::capabilities::{CapabilitySnapshotV1, ModelCatalogV1};
use oxplow_ai::client::{
    complete_typed_via_text, decide_via_chat, parse_answers, stream_whole, unless_cancelled,
    AiError, Cancel, CompleteRequest, Completion, DecideRequest, Decision, Http, ModelProvider,
    OnDelta, ProviderInstance, Question,
};
use oxplow_ai::parts::{DeltaFold, StreamDeltaV1, TypedRequestV1, TypedResponseV1};
use serde_json::{json, Value};

/// How long one of the script's functions may run.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(5);

/// The functions a provider script defines.
pub const FUNCTIONS: &[&str] = &["request", "response"];

/// What one that streams (`ops: [streamTyped]`) defines besides.
pub const STREAM_FUNCTIONS: &[&str] = &["request", "response", "delta"];

/// Whether the script may run now: a person approved it as it is
/// (`ProgramKind::AiProvider`), or why not. Asked before every call, so an
/// approval takes effect at once and an edit stops it.
pub type Gate = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    ops: Option<Vec<String>>,
    #[serde(default)]
    models: Option<Vec<CapabilitySnapshotV1>>,
}

/// A provider whose calls a script shapes.
pub struct Scripted {
    kind: String,
    title: String,
    base_url: Option<String>,
    complete: bool,
    complete_typed: bool,
    stream_typed: bool,
    decide: bool,
    models: ModelCatalogV1,
    script: Arc<str>,
    gate: Gate,
    http: Http,
}

/// The provider `script` is, registered as `id` (named `title`) and
/// configured with `config`; an error for a config that doesn't hold or a
/// script that doesn't define [`FUNCTIONS`].
pub fn scripted(
    id: &str,
    title: &str,
    entry: &str,
    script: &str,
    config: &Value,
    gate: Gate,
) -> Result<Arc<dyn ModelProvider>, String> {
    let c: Config = serde_json::from_value(config.clone()).map_err(|e| format!("config: {e}"))?;
    let models = ModelCatalogV1 {
        schema_version: 1,
        catalog_known: c.models.is_some(),
        models: c.models.unwrap_or_default(),
    };
    models
        .validate()
        .map_err(|e| format!("config models: {e}"))?;
    let ops = c.ops.unwrap_or_else(|| vec!["complete".into()]);
    if let Some(other) = ops
        .iter()
        .find(|o| !["complete", "decide", "completeTyped", "streamTyped"].contains(&o.as_str()))
    {
        return Err(format!(
            "config: `ops` lists `{other}`; it answers `complete`, `completeTyped`, \
             `streamTyped` and `decide`"
        ));
    }
    let stream_typed = ops.iter().any(|o| o == "streamTyped");
    let functions = if stream_typed {
        STREAM_FUNCTIONS
    } else {
        FUNCTIONS
    };
    oxplow_script::runtime::check_starlark_defines(entry, script, functions)
        .map_err(|e| format!("`{entry}` {e}"))?;
    Ok(Arc::new(Scripted {
        kind: id.to_string(),
        title: title.to_string(),
        base_url: c.base_url,
        complete: ops.iter().any(|o| o == "complete"),
        complete_typed: ops.iter().any(|o| o == "completeTyped"),
        stream_typed,
        decide: ops.iter().any(|o| o == "decide"),
        models,
        script: script.into(),
        gate,
        http: Http::default(),
    }))
}

impl Scripted {
    /// Run the script's `func` on `input` in a script worker — once a
    /// person approved it.
    async fn call(
        &self,
        provider: &str,
        func: &'static str,
        input: Value,
    ) -> Result<Value, AiError> {
        (self.gate)().map_err(|message| AiError::Unapproved {
            provider: provider.to_string(),
            message,
        })?;
        let script = self.script.clone();
        let budget = oxplow_domain::run::Budget::with_timeout(SCRIPT_TIMEOUT);
        let ran = tokio::task::spawn_blocking(move || {
            use oxplow_script::workers::{shared, ScriptRun};
            shared()
                .run(&budget, ScriptRun::starlark_fn(&script, func, input), None)
                .map(|ran| ran.value)
        })
        .await;
        let bad = |detail: String| AiError::BadResponse {
            provider: provider.to_string(),
            detail,
        };
        ran.map_err(|e| bad(format!("its `{func}` panicked: {e}")))?
            .map_err(|e| bad(format!("its `{func}`: {e}")))
    }

    /// `request(x)`, sent: the reply's JSON, or `None` when the script
    /// declined (a `decide` to ask as a chat).
    async fn exchange(
        &self,
        instance: &ProviderInstance<'_>,
        x: Value,
    ) -> Result<Option<Value>, AiError> {
        let Some(call) = self.prepare(instance, x).await? else {
            return Ok(None);
        };
        let reply = self
            .http
            .post_with(instance.id, &call.url, call.headers, call.body)
            .await;
        match reply {
            Ok(mut value) => {
                if let Some(key) = instance.key.filter(|key| !key.is_empty()) {
                    redact_key(&mut value, key);
                }
                Ok(Some(value))
            }
            Err(error) => Err(redacted(error, instance.key)),
        }
    }

    /// `request(x)`, checked and ready to send: under the base URL, the key
    /// spliced in, the gate asked; `None` when the script declined.
    async fn prepare(
        &self,
        instance: &ProviderInstance<'_>,
        x: Value,
    ) -> Result<Option<Call>, AiError> {
        let id = instance.id;
        let bad = |detail: String| AiError::BadResponse {
            provider: id.to_string(),
            detail,
        };
        let req = self.call(id, "request", x).await?;
        if req.is_null() {
            return Ok(None);
        }
        let path = req["path"]
            .as_str()
            .ok_or_else(|| bad("its `request` gave no `path`".into()))?;
        if !path.starts_with('/') || path.contains("://") {
            return Err(bad(format!(
                "its `request` path `{path}` isn't a path under the base URL"
            )));
        }
        let path = with_key(path, instance.key)
            .ok_or_else(|| bad("its path needs a key, and this provider has none".into()))?;
        let mut headers: Vec<(String, String)> = Vec::new();
        if let Some(map) = req["headers"].as_object() {
            for (name, value) in map {
                if [
                    "host",
                    "connection",
                    "proxy-authorization",
                    "proxy-connection",
                ]
                .contains(&name.to_ascii_lowercase().as_str())
                {
                    return Err(bad(
                        "its request cannot override routing or proxy headers".into()
                    ));
                }
                let value = value
                    .as_str()
                    .ok_or_else(|| bad(format!("its header `{name}` isn't a string")))?;
                // A header that needs a key the instance doesn't have is
                // left out: a local server takes none.
                if let Some(value) = with_key(value, instance.key) {
                    headers.push((name.clone(), value));
                }
            }
        }
        let base = instance.base_url_or(self.base_url.as_deref().unwrap_or_default());
        if base.is_empty() {
            return Err(bad("it has no base URL: set one on the provider".into()));
        }
        let url = format!("{base}{path}");
        let body = req.get("body").cloned().unwrap_or(Value::Null);
        (self.gate)().map_err(|message| AiError::Unapproved {
            provider: id.to_string(),
            message,
        })?;
        Ok(Some(Call { url, headers, body }))
    }

    /// `request` sent as a stream, each batch of events folded by `delta`
    /// (the script's own `state` carried from one batch to the next) into
    /// the deltas handed to `on_delta`, and the whole reply `response`
    /// makes of the last `state`.
    async fn stream(
        &self,
        instance: &ProviderInstance<'_>,
        req: &TypedRequestV1,
        cancel: &Cancel,
        on_delta: OnDelta<'_>,
    ) -> Result<TypedResponseV1, AiError> {
        let id = instance.id;
        let bad = |detail: String| AiError::BadResponse {
            provider: id.to_string(),
            detail,
        };
        let x = json!({ "op": "streamTyped", "model": req.model, "request": req });
        let call = self
            .prepare(instance, x)
            .await?
            .ok_or_else(|| bad("its request declined a streamed completion".into()))?;
        let mut fold = DeltaFold::default();
        let mut stream = unless_cancelled(
            cancel,
            self.http
                .post_stream(id, &call.url, call.headers, call.body),
        )
        .await
        .map_err(|stop| stop.error(id, TypedResponseV1::partial(Vec::new())))?
        .map_err(|e| redacted(e, instance.key))?;
        let mut state = Value::Null;
        loop {
            let events = match stream.next_batch_until(cancel).await {
                Err(stop) => return Err(stop.error(id, fold.into_partial())),
                Ok(batch) => match batch.map_err(|e| redacted(e, instance.key))? {
                    Some(events) => events,
                    None => break,
                },
            };
            let mut events = serde_json::to_value(events).expect("events serialize");
            if let Some(key) = instance.key.filter(|key| !key.is_empty()) {
                redact_key(&mut events, key);
            }
            let out = self
                .call(
                    id,
                    "delta",
                    json!({ "op": "streamTyped", "model": req.model, "events": events,
                            "state": state }),
                )
                .await?;
            if let Some(e) = out.get("error") {
                return Err(bad(e.as_str().unwrap_or("its stream isn't usable").into()));
            }
            state = out.get("state").cloned().unwrap_or(Value::Null);
            let deltas: Vec<StreamDeltaV1> =
                serde_json::from_value(out.get("deltas").cloned().unwrap_or(json!([])))
                    .map_err(|e| bad(format!("its `delta` gave deltas that are not V1: {e}")))?;
            for delta in deltas {
                fold.push(&delta);
                on_delta(delta);
            }
            if cancel.is_cancelled() {
                return Err(AiError::Cancelled {
                    provider: id.to_string(),
                    partial: Box::new(fold.into_partial()),
                });
            }
        }
        let response = self
            .call(
                id,
                "response",
                json!({ "op": "streamTyped", "model": req.model, "state": state }),
            )
            .await?;
        typed_response(id, response)
    }
}

/// A request ready to send.
struct Call {
    url: String,
    headers: Vec<(String, String)>,
    body: Value,
}

/// `error` with the key taken out of an HTTP error's body.
fn redacted(error: AiError, key: Option<&str>) -> AiError {
    match (error, key.filter(|key| !key.is_empty())) {
        (
            AiError::Http {
                provider,
                status,
                body,
            },
            Some(key),
        ) => AiError::Http {
            provider,
            status,
            body: body.replace(key, "[redacted]"),
        },
        (error, _) => error,
    }
}

/// A script's typed reply: its `{ error }`, or a V1 response.
fn typed_response(provider: &str, response: Value) -> Result<TypedResponseV1, AiError> {
    if let Some(e) = response.get("error") {
        return Err(AiError::BadResponse {
            provider: provider.into(),
            detail: e.as_str().unwrap_or("its reply isn't usable").to_string(),
        });
    }
    serde_json::from_value(response).map_err(|e| AiError::BadResponse {
        provider: provider.into(),
        detail: format!("its typed response is not V1: {e}"),
    })
}

/// The host never hands an echoed credential back to a mapping script.
fn redact_key(value: &mut Value, key: &str) {
    match value {
        Value::String(text) => *text = text.replace(key, "[redacted]"),
        Value::Array(values) => values.iter_mut().for_each(|value| redact_key(value, key)),
        Value::Object(map) => {
            *map = std::mem::take(map)
                .into_iter()
                .map(|(name, mut value)| {
                    redact_key(&mut value, key);
                    (name.replace(key, "[redacted]"), value)
                })
                .collect();
        }
        Value::Number(number) if number.to_string().contains(key) => *value = Value::Null,
        _ => (),
    }
}

/// `text` with `{{key}}` taken from `key`; `None` when it names the key
/// and there is none.
fn with_key(text: &str, key: Option<&str>) -> Option<String> {
    oxplow_domain::template::splice(text, |p| match (p.key().as_str(), key) {
        ("key", Some(k)) => Ok(k.to_string()),
        ("key", None) => Err(()),
        (other, _) => Ok(format!("{{{{{other}}}}}")),
    })
    .ok()
}

fn usage(v: &Value) -> (i64, i64) {
    (
        v["usage"]["input"].as_i64().unwrap_or(0),
        v["usage"]["output"].as_i64().unwrap_or(0),
    )
}

#[async_trait::async_trait]
impl ModelProvider for Scripted {
    fn kind(&self) -> &str {
        &self.kind
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn default_base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    async fn discover_models(
        &self,
        _instance: &ProviderInstance<'_>,
    ) -> Result<ModelCatalogV1, AiError> {
        Ok(self.models.clone())
    }

    async fn discover(
        &self,
        _instance: &ProviderInstance<'_>,
        model: &str,
    ) -> Result<CapabilitySnapshotV1, AiError> {
        Ok(self
            .models
            .models
            .iter()
            .find(|entry| entry.model.as_deref() == Some(model))
            .cloned()
            .unwrap_or_else(|| CapabilitySnapshotV1::unknown(model)))
    }

    async fn complete_typed(
        &self,
        instance: &ProviderInstance<'_>,
        req: &TypedRequestV1,
    ) -> Result<TypedResponseV1, AiError> {
        req.validate_version()?;
        if !self.complete_typed {
            return complete_typed_via_text(self, instance, req).await;
        }
        let x = json!({ "op": "completeTyped", "model": req.model, "request": req });
        let body = self
            .exchange(instance, x)
            .await?
            .ok_or_else(|| AiError::BadResponse {
                provider: instance.id.into(),
                detail: "its request declined a typed completion".into(),
            })?;
        let response = self
            .call(
                instance.id,
                "response",
                json!({
                    "op": "completeTyped", "model": req.model, "body": body
                }),
            )
            .await?;
        typed_response(instance.id, response)
    }

    async fn stream_typed(
        &self,
        instance: &ProviderInstance<'_>,
        req: &TypedRequestV1,
        cancel: &Cancel,
        on_delta: OnDelta<'_>,
    ) -> Result<TypedResponseV1, AiError> {
        req.validate_version()?;
        if !self.stream_typed {
            return stream_whole(self, instance, req, cancel, on_delta).await;
        }
        self.stream(instance, req, cancel, on_delta).await
    }

    async fn complete(
        &self,
        instance: &ProviderInstance<'_>,
        req: &CompleteRequest<'_>,
    ) -> Result<Completion, AiError> {
        let id = instance.id;
        let bad = |detail: String| AiError::BadResponse {
            provider: id.to_string(),
            detail,
        };
        if !self.complete {
            return Err(bad(format!(
                "{} only answers typed questions; use it for the decide role",
                self.title
            )));
        }
        let x = json!({ "op": "complete", "model": req.model, "system": req.system,
                        "prompt": req.prompt, "json": req.json });
        let body = self
            .exchange(instance, x)
            .await?
            .ok_or_else(|| bad("its `request` declined a completion".into()))?;
        let out = self
            .call(
                id,
                "response",
                json!({ "op": "complete", "model": req.model, "body": body }),
            )
            .await?;
        if let Some(e) = out.get("error") {
            return Err(bad(e
                .as_str()
                .unwrap_or("its reply isn't usable")
                .to_string()));
        }
        let text = out["text"]
            .as_str()
            .ok_or_else(|| bad("its `response` gave no `text`".into()))?
            .to_string();
        let (input_tokens, output_tokens) = usage(&out);
        Ok(Completion {
            text,
            input_tokens,
            output_tokens,
        })
    }

    async fn decide(
        &self,
        instance: &ProviderInstance<'_>,
        req: &DecideRequest<'_>,
    ) -> Result<Decision, AiError> {
        if !self.decide {
            return decide_via_chat(self, instance, req).await;
        }
        let id = instance.id;
        let bad = |detail: String| AiError::BadResponse {
            provider: id.to_string(),
            detail,
        };
        let x = json!({ "op": "decide", "model": req.model, "state": req.state,
                        "questions": req.questions });
        let Some(body) = self.exchange(instance, x).await? else {
            return decide_via_chat(self, instance, req).await;
        };
        let out = self
            .call(
                id,
                "response",
                json!({ "op": "decide", "model": req.model, "body": body }),
            )
            .await?;
        if let Some(e) = out.get("error") {
            return Err(bad(e
                .as_str()
                .unwrap_or("its reply isn't usable")
                .to_string()));
        }
        let answers = parse_answers(id, &out["answers"], req.questions, "probability")?;
        let (input_tokens, output_tokens) = usage(&out);
        Ok(Decision {
            answers,
            input_tokens,
            output_tokens,
        })
    }

    /// One small call to check the key, URL and model: a completion, or —
    /// for one that only decides — a yes/no question.
    async fn test(&self, instance: &ProviderInstance<'_>, model: &str) -> Result<String, AiError> {
        if self.complete_typed && !self.complete && !self.decide {
            let request = TypedRequestV1 {
                schema_version: 1,
                model: model.into(),
                tools: Vec::new(),
                messages: vec![oxplow_ai::parts::MessageV1 {
                    role: oxplow_ai::parts::MessageRoleV1::User,
                    parts: vec![oxplow_ai::parts::PartV1::Text {
                        text: "Reply with the single word OK.".into(),
                    }],
                }],
            };
            let response = self.complete_typed(instance, &request).await?;
            let text: String = response
                .parts
                .iter()
                .filter_map(|part| match part {
                    oxplow_ai::parts::PartV1::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            if text.is_empty() {
                return Err(AiError::BadResponse {
                    provider: instance.id.into(),
                    detail: "the connection check returned no text".into(),
                });
            }
            return Ok(text.trim().chars().take(200).collect());
        }
        if self.complete {
            let c = self
                .complete(
                    instance,
                    &CompleteRequest {
                        model,
                        system: None,
                        prompt: "Reply with the single word OK.",
                        json: false,
                    },
                )
                .await?;
            return Ok(c.text.trim().chars().take(200).collect());
        }
        let questions = BTreeMap::from([(
            "ok".to_string(),
            Question::Noul {
                instructions: "Is this a connection test?".into(),
            },
        )]);
        let d = self
            .decide(
                instance,
                &DecideRequest {
                    model,
                    state: "A connection test.",
                    questions: &questions,
                },
            )
            .await?;
        Ok(match d.answers.get("ok") {
            Some(oxplow_ai::client::Answer::Noul { probability }) => {
                format!("Answered (yes: {:.0}%)", probability * 100.0)
            }
            _ => "Answered".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_ai::client::Answer;
    use oxplow_ai_fake::mock;

    const CHAT: &str = r#"
def request(x):
    if x["op"] == "decide":
        return None
    return {
        "path": "/chat",
        "headers": {"authorization": "Bearer {{key}}"},
        "body": {"model": x["model"], "prompt": x["prompt"]},
    }

def response(x):
    b = x["body"]
    if "out" not in b:
        return {"error": "no out"}
    return {"text": b["out"], "usage": {"input": b["in_t"], "output": b["out_t"]}}
"#;

    fn open() -> Gate {
        Arc::new(|| Ok(()))
    }

    fn at<'a>(base: &'a str, key: Option<&'a str>) -> ProviderInstance<'a> {
        ProviderInstance {
            id: "p",
            base_url: Some(base),
            key,
        }
    }

    fn ask() -> CompleteRequest<'static> {
        CompleteRequest {
            model: "m",
            system: None,
            prompt: "hello",
            json: false,
        }
    }

    /// The script shapes the call and reads the reply; the host sends it
    /// to the base URL with the key spliced in, which the script never saw.
    #[tokio::test]
    async fn a_script_shapes_the_call_and_the_host_sends_it() {
        let (base, seen) = mock("/chat", 200, json!({ "out": "hi", "in_t": 3, "out_t": 1 })).await;
        let p = scripted("x", "X", "p.star", CHAT, &json!({}), open()).unwrap();
        let c = p.complete(&at(&base, Some("k1")), &ask()).await.unwrap();
        assert_eq!(
            c,
            Completion {
                text: "hi".into(),
                input_tokens: 3,
                output_tokens: 1
            }
        );
        let (_, headers, body) = seen.lock().unwrap()[0].clone();
        assert_eq!(headers["authorization"], "Bearer k1");
        assert_eq!(body, json!({ "model": "m", "prompt": "hello" }));
        // No key: the header that needs one is left out.
        p.complete(&at(&base, None), &ask()).await.unwrap();
        assert!(!seen.lock().unwrap()[1].1.contains_key("authorization"));
    }

    /// A status is the host's to read; an unusable body is the script's.
    #[tokio::test]
    async fn errors_are_the_hosts_by_status_and_the_scripts_by_body() {
        let p = scripted("x", "X", "p.star", CHAT, &json!({}), open()).unwrap();
        let (base, _) = mock("/chat", 401, json!({})).await;
        assert!(matches!(
            p.complete(&at(&base, Some("k")), &ask()).await,
            Err(AiError::Auth { .. })
        ));
        let (base, _) = mock("/chat", 200, json!({ "nope": 1 })).await;
        let err = p.complete(&at(&base, Some("k")), &ask()).await.unwrap_err();
        assert!(err.to_string().contains("no out"), "{err}");
    }

    /// Without `decide` in its ops a typed question is asked as a chat;
    /// with it, the script's answers are read in oxplow's shape.
    #[tokio::test]
    async fn decide_is_native_or_a_chat() {
        let questions = BTreeMap::from([(
            "risky".to_string(),
            Question::Noul {
                instructions: "Risky?".into(),
            },
        )]);
        let req = DecideRequest {
            model: "m",
            state: "a diff",
            questions: &questions,
        };
        let chat_reply = json!({ "answers": { "risky": { "type": "noul", "probability": 0.7 } } });
        let (base, _) = mock(
            "/chat",
            200,
            json!({ "out": chat_reply.to_string(), "in_t": 1, "out_t": 1 }),
        )
        .await;
        let p = scripted("x", "X", "p.star", CHAT, &json!({}), open()).unwrap();
        let d = p.decide(&at(&base, None), &req).await.unwrap();
        assert_eq!(d.answers["risky"], Answer::Noul { probability: 0.7 });

        let native = r#"
def request(x):
    return {"path": "/decide", "body": {"state": x["state"], "questions": x["questions"]}}

def response(x):
    return {"answers": {"risky": {"type": "noul", "probability": x["body"]["p"]}}}
"#;
        let (base, seen) = mock("/decide", 200, json!({ "p": 0.2 })).await;
        let p = scripted(
            "x",
            "X",
            "p.star",
            native,
            &json!({ "ops": ["decide"] }),
            open(),
        )
        .unwrap();
        let d = p.decide(&at(&base, None), &req).await.unwrap();
        assert_eq!(d.answers["risky"], Answer::Noul { probability: 0.2 });
        assert_eq!(
            seen.lock().unwrap()[0].2["questions"]["risky"]["type"],
            "noul"
        );
        assert!(
            p.complete(&at(&base, None), &ask()).await.is_err(),
            "it only decides"
        );
    }

    /// A script runs only once a person approved it: until then a call
    /// says where to approve it, and nothing is sent.
    #[tokio::test]
    async fn an_unapproved_script_sends_nothing() {
        let (base, seen) = mock("/chat", 200, json!({ "out": "hi", "in_t": 1, "out_t": 1 })).await;
        let shut: Gate = Arc::new(|| Err("needs approving".into()));
        let p = scripted("x", "X", "p.star", CHAT, &json!({}), shut).unwrap();
        let err = p.complete(&at(&base, Some("k")), &ask()).await.unwrap_err();
        assert!(matches!(err, AiError::Unapproved { .. }), "{err}");
        assert!(
            err.to_string().contains("Settings → Data → Programs"),
            "{err}"
        );
        assert!(seen.lock().unwrap().is_empty());
    }

    /// A path never leaves the base URL's host, and a script must define
    /// both functions.
    #[tokio::test]
    async fn a_script_stays_under_its_base_url() {
        let away = r#"
def request(x):
    return {"path": "https://elsewhere.test/x", "body": {}}

def response(x):
    return {"text": ""}
"#;
        let (base, seen) = mock("/x", 200, json!({})).await;
        let p = scripted("x", "X", "p.star", away, &json!({}), open()).unwrap();
        let err = p.complete(&at(&base, None), &ask()).await.unwrap_err();
        assert!(
            err.to_string().contains("isn't a path under the base URL"),
            "{err}"
        );
        assert!(seen.lock().unwrap().is_empty());
        let err = scripted(
            "x",
            "X",
            "p.star",
            "def request(x):\n    return None\n",
            &json!({}),
            open(),
        )
        .err()
        .unwrap();
        assert!(err.contains("`response`"), "{err}");
        let err = scripted(
            "x",
            "X",
            "p.star",
            CHAT,
            &json!({ "ops": ["embed"] }),
            open(),
        )
        .err()
        .unwrap();
        assert!(err.contains("`embed`"), "{err}");
    }
    const ANTHROPIC: &str =
        include_str!("../../../extensions/oxplow-foundation/providers/anthropic.star");
    const OPENAI: &str =
        include_str!("../../../extensions/oxplow-foundation/providers/openai_compatible.star");

    /// A script that answers `completeTyped` with a fixed V1 reply, so the
    /// host path (not a vendor mapping) is what is under test.
    const TYPED: &str = r#"
def request(x):
    return {"path": "/typed", "body": x["request"]}

def response(x):
    return x["body"]
"#;

    fn typed_ops() -> Value {
        json!({ "ops": ["completeTyped"] })
    }

    fn user_text(text: &str) -> TypedRequestV1 {
        use oxplow_ai::parts::{MessageRoleV1, MessageV1, PartV1};
        TypedRequestV1 {
            schema_version: 1,
            model: "m".into(),
            tools: Vec::new(),
            messages: vec![MessageV1 {
                role: MessageRoleV1::User,
                parts: vec![PartV1::Text { text: text.into() }],
            }],
        }
    }

    fn snapshot(model: &str, tools: &str) -> Value {
        json!({ "schemaVersion": 1, "model": model, "modelKnown": true,
                "text": "supported", "image": "unknown", "toolCalling": tools,
                "streaming": "unknown",
                "contextTokens": { "value": 1000, "unit": "tokens" }, "outputTokens": null })
    }

    /// Declared `config.models` are the whole discovery answer: no script
    /// run, no key, no request. A model the config doesn't list, and a
    /// provider with no `models`, stay unknown.
    #[tokio::test]
    async fn discovery_reads_declared_metadata_and_nothing_else() {
        let (base, seen) = mock("/chat", 200, json!({})).await;
        let ran = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = ran.clone();
        let counting: Gate = Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });
        let config =
            json!({ "models": [snapshot("a", "supported"), snapshot("b", "unsupported")] });
        let p = scripted("x", "X", "p.star", CHAT, &config, counting).unwrap();
        let instance = at(&base, None);

        let a = p.discover(&instance, "a").await.unwrap();
        assert!(a.model_known);
        assert_eq!(
            a.tool_calling,
            oxplow_ai::capabilities::CapabilitySupport::Supported
        );
        assert_eq!(a.context_tokens.as_ref().map(|l| l.value), Some(1000));
        // A limit the declaration leaves out is unknown, not zero.
        assert_eq!(a.output_tokens, None);
        let b = p.discover(&instance, "b").await.unwrap();
        assert_eq!(
            b.tool_calling,
            oxplow_ai::capabilities::CapabilitySupport::Unsupported
        );
        assert_eq!(
            p.discover(&instance, "c").await.unwrap(),
            CapabilitySnapshotV1::unknown("c")
        );

        let catalog = p.discover_models(&instance).await.unwrap();
        assert!(catalog.catalog_known);
        assert_eq!(catalog.models.len(), 2);

        let bare = scripted("y", "Y", "p.star", CHAT, &json!({}), open()).unwrap();
        assert_eq!(
            bare.discover_models(&instance).await.unwrap(),
            ModelCatalogV1::unknown()
        );
        let empty = scripted("z", "Z", "p.star", CHAT, &json!({ "models": [] }), open()).unwrap();
        let known_empty = empty.discover_models(&instance).await.unwrap();
        assert!(known_empty.catalog_known && known_empty.models.is_empty());

        assert!(seen.lock().unwrap().is_empty(), "no network call");
        assert_eq!(
            ran.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "no script ran"
        );
    }

    #[test]
    fn declared_models_must_be_resolved_and_unique() {
        let dup = json!({ "models": [snapshot("a", "supported"), snapshot("a", "supported")] });
        let err = scripted("x", "X", "p.star", CHAT, &dup, open())
            .err()
            .unwrap();
        assert!(err.contains("config models"), "{err}");
        let mut unresolved = snapshot("a", "supported");
        unresolved["modelKnown"] = json!(false);
        let err = scripted(
            "x",
            "X",
            "p.star",
            CHAT,
            &json!({ "models": [unresolved] }),
            open(),
        )
        .err()
        .unwrap();
        assert!(err.contains("config models"), "{err}");
        let mut stale = snapshot("a", "supported");
        stale["schemaVersion"] = json!(9);
        let err = scripted(
            "x",
            "X",
            "p.star",
            CHAT,
            &json!({ "models": [stale] }),
            open(),
        )
        .err()
        .unwrap();
        assert!(err.contains("config"), "{err}");
    }

    /// A `completeTyped` script returns the V1 shape; ordered parts, finish
    /// category and usage with its provenance reach the caller untouched.
    #[tokio::test]
    async fn a_typed_script_returns_ordered_parts_finish_and_usage() {
        let reply = json!({ "schemaVersion": 1, "finishReason": "toolCalls",
            "parts": [
                { "type": "text", "text": "looking" },
                { "type": "toolCall", "id": "c1", "name": "grep", "arguments": { "q": "x" } },
                { "type": "text", "text": "done" } ],
            "usage": { "inputTokens": 0, "outputTokens": null, "source": "estimated" } });
        let (base, seen) = mock("/typed", 200, reply).await;
        let p = scripted("x", "X", "p.star", TYPED, &typed_ops(), open()).unwrap();
        let r = p
            .complete_typed(&at(&base, None), &user_text("hi"))
            .await
            .unwrap();
        use oxplow_ai::parts::{FinishReasonV1, PartV1, UsageSourceV1};
        assert_eq!(r.finish_reason, FinishReasonV1::ToolCalls);
        assert!(matches!(r.parts.as_slice(),
            [PartV1::Text { text: a }, PartV1::ToolCall { id, .. }, PartV1::Text { text: b }]
            if a == "looking" && id == "c1" && b == "done"));
        assert_eq!(r.usage.source, UsageSourceV1::Estimated);
        assert_eq!(r.usage.input_tokens, Some(0), "known zero stays zero");
        assert_eq!(r.usage.output_tokens, None);
        assert_eq!(
            seen.lock().unwrap()[0].2["messages"][0]["parts"][0]["text"],
            "hi"
        );
    }

    /// A reply that isn't V1 (a wrong version, an unknown part kind) is a
    /// clear error, not a silently trimmed response.
    #[tokio::test]
    async fn a_typed_reply_that_is_not_v1_is_refused() {
        let p = scripted("x", "X", "p.star", TYPED, &typed_ops(), open()).unwrap();
        for bad in [
            json!({ "schemaVersion": 2, "parts": [], "finishReason": "stop",
                    "usage": { "inputTokens": null, "outputTokens": null, "source": "unknown" } }),
            json!({ "schemaVersion": 1, "parts": [{ "type": "audio" }], "finishReason": "stop",
                    "usage": { "inputTokens": null, "outputTokens": null, "source": "unknown" } }),
        ] {
            let (base, _) = mock("/typed", 200, bad).await;
            let err = p
                .complete_typed(&at(&base, None), &user_text("hi"))
                .await
                .unwrap_err();
            assert!(err.to_string().contains("not V1"), "{err}");
        }
    }

    /// Without `completeTyped` the text adapter runs: text works, an image
    /// is refused before anything is sent.
    #[tokio::test]
    async fn a_text_only_script_refuses_an_image_part() {
        use oxplow_ai::parts::{MessageRoleV1, MessageV1, PartV1};
        let (base, seen) = mock("/chat", 200, json!({ "out": "ok", "in_t": 2, "out_t": 1 })).await;
        let p = scripted("x", "X", "p.star", CHAT, &json!({}), open()).unwrap();
        let r = p
            .complete_typed(&at(&base, None), &user_text("hi"))
            .await
            .unwrap();
        assert_eq!(r.parts, vec![PartV1::Text { text: "ok".into() }]);
        let mut with_image = user_text("hi");
        with_image.messages.push(MessageV1 {
            role: MessageRoleV1::User,
            parts: vec![PartV1::Image {
                asset_ref: "a".into(),
                media_type: "image/png".into(),
            }],
        });
        let before = seen.lock().unwrap().len();
        let err = p
            .complete_typed(&at(&base, None), &with_image)
            .await
            .unwrap_err();
        assert!(matches!(err, AiError::UnsupportedFeature { .. }), "{err}");
        assert_eq!(seen.lock().unwrap().len(), before);
    }

    fn shipped_typed(script: &str, base_url: &str) -> Arc<dyn ModelProvider> {
        scripted(
            "x",
            "X",
            "p.star",
            script,
            &json!({ "baseUrl": base_url, "ops": ["complete", "completeTyped"] }),
            open(),
        )
        .unwrap()
    }

    /// The shipped Anthropic mapping reads content blocks in order and
    /// counts cache reads and writes as input.
    #[tokio::test]
    async fn the_anthropic_mapping_reads_ordered_blocks_and_cached_input() {
        let body = json!({ "stop_reason": "tool_use",
            "content": [
                { "type": "text", "text": "one" },
                { "type": "tool_use", "id": "t1", "name": "grep", "input": { "q": "x" } },
                { "type": "text", "text": "two" } ],
            "usage": { "input_tokens": 10, "cache_creation_input_tokens": 5,
                       "cache_read_input_tokens": 100, "output_tokens": 7 } });
        let (base, seen) = mock("/v1/messages", 200, body).await;
        let p = shipped_typed(ANTHROPIC, &base);
        let mut req = user_text("go");
        req.tools.push(oxplow_ai::parts::ToolDefinitionV1 {
            name: "grep".into(),
            description: Some("search".into()),
            parameters: json!({ "type": "object" }),
        });
        let r = p
            .complete_typed(&at(&base, Some("sk-test-key")), &req)
            .await
            .unwrap();
        use oxplow_ai::parts::{FinishReasonV1, PartV1, UsageSourceV1};
        assert!(matches!(r.parts.as_slice(),
            [PartV1::Text { text: a }, PartV1::ToolCall { id, name, arguments }, PartV1::Text { text: b }]
            if a == "one" && id == "t1" && name == "grep" && arguments["q"] == "x" && b == "two"));
        assert_eq!(r.finish_reason, FinishReasonV1::ToolCalls);
        assert_eq!(r.usage.input_tokens, Some(115));
        assert_eq!(r.usage.output_tokens, Some(7));
        assert_eq!(r.usage.source, UsageSourceV1::Observed);
        let (_, headers, sent) = seen.lock().unwrap()[0].clone();
        assert_eq!(headers["x-api-key"], "sk-test-key");
        assert_eq!(
            sent["tools"][0]["input_schema"],
            json!({ "type": "object" })
        );
        assert_eq!(sent["messages"][0]["content"][0]["text"], "go");
    }

    /// Usage the provider omits stays unknown rather than becoming zero.
    #[tokio::test]
    async fn the_anthropic_mapping_leaves_missing_usage_unknown() {
        let body =
            json!({ "stop_reason": "max_tokens", "content": [{ "type": "text", "text": "t" }] });
        let (base, _) = mock("/v1/messages", 200, body).await;
        let r = shipped_typed(ANTHROPIC, &base)
            .complete_typed(&at(&base, None), &user_text("go"))
            .await
            .unwrap();
        use oxplow_ai::parts::{FinishReasonV1, UsageSourceV1};
        assert_eq!(r.finish_reason, FinishReasonV1::Length);
        assert_eq!((r.usage.input_tokens, r.usage.output_tokens), (None, None));
        assert_eq!(r.usage.source, UsageSourceV1::Unknown);
    }

    /// The shipped OpenAI mapping puts the text before the tool calls the
    /// message carries, with the finish category and prompt/completion usage.
    #[tokio::test]
    async fn the_openai_mapping_reads_text_then_calls_and_usage() {
        let body = json!({ "choices": [{ "finish_reason": "tool_calls", "message": {
                "role": "assistant", "content": "checking",
                "tool_calls": [
                    { "id": "c1", "type": "function", "function": { "name": "a", "arguments": "{\"n\":1}" } },
                    { "id": "c2", "type": "function", "function": { "name": "b", "arguments": "{}" } } ] } }],
            "usage": { "prompt_tokens": 20, "completion_tokens": 9 } });
        let (base, seen) = mock("/chat/completions", 200, body).await;
        let r = shipped_typed(OPENAI, &base)
            .complete_typed(&at(&base, Some("sk-test-key")), &user_text("go"))
            .await
            .unwrap();
        use oxplow_ai::parts::{FinishReasonV1, PartV1, UsageSourceV1};
        assert!(matches!(r.parts.as_slice(),
            [PartV1::Text { text }, PartV1::ToolCall { id: i1, arguments, .. }, PartV1::ToolCall { id: i2, .. }]
            if text == "checking" && i1 == "c1" && arguments["n"] == 1 && i2 == "c2"));
        assert_eq!(r.finish_reason, FinishReasonV1::ToolCalls);
        assert_eq!(
            (r.usage.input_tokens, r.usage.output_tokens),
            (Some(20), Some(9))
        );
        assert_eq!(r.usage.source, UsageSourceV1::Observed);
        assert_eq!(
            seen.lock().unwrap()[0].1["authorization"],
            "Bearer sk-test-key"
        );
    }

    /// A reply part the mapping cannot represent is an error, not a drop.
    #[tokio::test]
    async fn the_shipped_mappings_refuse_parts_they_cannot_represent() {
        let (base, _) = mock(
            "/v1/messages",
            200,
            json!({ "content": [{ "type": "thinking", "thinking": "hm" }] }),
        )
        .await;
        let err = shipped_typed(ANTHROPIC, &base)
            .complete_typed(&at(&base, None), &user_text("go"))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("not V1") || err.to_string().contains("unsupported"),
            "{err}"
        );
    }
    /// A script's own `{error}` for a typed call reads as it does for text:
    /// its message, not a complaint about the V1 shape.
    #[tokio::test]
    async fn a_typed_script_error_is_reported_as_the_scripts_message() {
        let (base, _) = mock("/typed", 200, json!({ "error": "no assistant message" })).await;
        let p = scripted("x", "X", "p.star", TYPED, &typed_ops(), open()).unwrap();
        let err = p
            .complete_typed(&at(&base, None), &user_text("hi"))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            AiError::BadResponse {
                provider: "p".into(),
                detail: "no assistant message".into()
            }
        );
        let (base, _) = mock("/v1/messages", 200, json!({ "content": "nope" })).await;
        let err = shipped_typed(ANTHROPIC, &base)
            .complete_typed(&at(&base, None), &user_text("hi"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no assistant content"), "{err}");
    }

    /// A streaming script that folds `{"t": text}` and `{"call": {...}}`
    /// events, so the host's half is what's under test.
    const STREAM: &str = r#"
def request(x):
    return {"path": "/stream", "headers": {"authorization": "Bearer {{key}}"},
            "body": {"model": x["model"], "stream": True}}

def delta(x):
    s = x["state"] or {"text": "", "calls": []}
    text = s["text"]
    calls = list(s["calls"])
    out = []
    for e in x["events"]:
        if e["data"] == "[DONE]":
            continue
        d = json.decode(e["data"])
        if "t" in d:
            text += d["t"]
            out.append({"type": "text", "text": d["t"]})
        if "call" in d:
            c = {"type": "toolCall", "id": d["call"]["id"], "name": d["call"]["name"],
                 "arguments": d["call"]["arguments"]}
            calls.append(c)
            out.append(c)
    return {"deltas": out, "state": {"text": text, "calls": calls}}

def response(x):
    s = x["state"]
    parts = ([{"type": "text", "text": s["text"]}] if s["text"] else []) + s["calls"]
    return {"schemaVersion": 1, "parts": parts,
            "finishReason": "toolCalls" if s["calls"] else "stop",
            "usage": {"inputTokens": None, "outputTokens": None, "source": "unknown"}}
"#;

    fn stream_ops() -> Value {
        json!({ "ops": ["streamTyped"] })
    }

    fn sse_data(data: Value) -> String {
        format!("data: {data}\n\n")
    }

    /// The host reads the events, `delta` turns them into deltas as they
    /// come, and `response` makes the whole reply of its fold. The key is
    /// spliced into the request and taken out of what came back.
    #[tokio::test]
    async fn a_streaming_script_folds_events_into_deltas_and_a_reply() {
        let (base, seen, _) = oxplow_ai_fake::mock_sse(
            "/stream",
            vec![
                sse_data(json!({ "t": "hel" })),
                sse_data(json!({ "t": "lo k1" })),
                sse_data(
                    json!({ "call": { "id": "c1", "name": "look", "arguments": { "p": 1 } } }),
                ),
                "data: [DONE]\n\n".into(),
            ],
            Duration::from_millis(10),
            Duration::ZERO,
        )
        .await;
        let p = scripted("x", "X", "p.star", STREAM, &stream_ops(), open()).unwrap();
        let mut deltas = Vec::new();
        let reply = p
            .stream_typed(
                &at(&base, Some("k1")),
                &user_text("hi"),
                &Cancel::default(),
                &mut |d| deltas.push(d),
            )
            .await
            .unwrap();
        let text: String = deltas
            .iter()
            .filter_map(|d| match d {
                StreamDeltaV1::Text { text } => Some(text.as_str()),
                StreamDeltaV1::ToolCall { .. } => None,
            })
            .collect();
        assert_eq!(text, "hello [redacted]");
        assert!(
            matches!(deltas.last(), Some(StreamDeltaV1::ToolCall { name, .. }) if name == "look")
        );
        assert_eq!(
            reply.finish_reason,
            oxplow_ai::parts::FinishReasonV1::ToolCalls
        );
        assert_eq!(reply.parts.len(), 2);
        let (_, headers, body) = seen.lock().unwrap()[0].clone();
        assert_eq!(headers["authorization"], "Bearer k1");
        assert_eq!(body["stream"], true);
    }

    /// Cancelled mid-stream, the call stops at once with what the deltas
    /// had said, and the connection is let go.
    #[tokio::test]
    async fn a_cancelled_streaming_script_reports_what_had_come() {
        let chunks = (0..40)
            .map(|i| sse_data(json!({ "t": format!("{i} ") })))
            .collect();
        let (base, _, written) = oxplow_ai_fake::mock_sse(
            "/stream",
            chunks,
            Duration::from_millis(100),
            Duration::from_secs(30),
        )
        .await;
        let p = scripted("x", "X", "p.star", STREAM, &stream_ops(), open()).unwrap();
        let cancel = Cancel::default();
        let stopper = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(450)).await;
            stopper.cancel();
        });
        let err = p
            .stream_typed(&at(&base, None), &user_text("hi"), &cancel, &mut |_| {})
            .await
            .unwrap_err();
        let AiError::Cancelled { partial, .. } = err else {
            panic!("{err:?}")
        };
        let oxplow_ai::parts::PartV1::Text { text } = &partial.parts[0] else {
            panic!("{partial:?}")
        };
        assert!(text.starts_with("0 1 "), "{text}");
        tokio::time::sleep(Duration::from_millis(400)).await;
        let at_end = written.load(std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(written.load(std::sync::atomic::Ordering::SeqCst), at_end);
        assert!(at_end < 40);
    }

    /// A script that says it streams must define `delta`; one that doesn't
    /// stream answers a stream with its whole reply at once.
    #[tokio::test]
    async fn streaming_needs_delta_and_otherwise_comes_whole() {
        let err = scripted("x", "X", "p.star", TYPED, &stream_ops(), open())
            .err()
            .unwrap();
        assert!(err.contains("`delta`"), "{err}");
        let reply = json!({ "schemaVersion": 1, "parts": [{ "type": "text", "text": "all" }],
            "finishReason": "stop",
            "usage": { "inputTokens": null, "outputTokens": null, "source": "unknown" } });
        let (base, _) = mock("/typed", 200, reply).await;
        let p = scripted("x", "X", "p.star", TYPED, &typed_ops(), open()).unwrap();
        let mut deltas = Vec::new();
        p.stream_typed(
            &at(&base, None),
            &user_text("hi"),
            &Cancel::default(),
            &mut |d| deltas.push(d),
        )
        .await
        .unwrap();
        assert_eq!(deltas, [StreamDeltaV1::Text { text: "all".into() }]);
    }
}
