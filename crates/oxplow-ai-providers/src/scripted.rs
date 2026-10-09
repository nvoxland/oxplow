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
//! `{ baseUrl?, ops? }`: `ops` lists what it answers natively, `complete`
//! (the default) and `decide`; without `decide` a typed question is asked
//! as a chat.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use oxplow_ai::client::{
    decide_via_chat, parse_answers, AiError, CompleteRequest, Completion, DecideRequest, Decision,
    Http, ModelProvider, ProviderInstance, Question,
};
use serde_json::{json, Value};

/// How long one of the script's functions may run.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(5);

/// The functions a provider script defines.
pub const FUNCTIONS: &[&str] = &["request", "response"];

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
}

/// A provider whose calls a script shapes.
pub struct Scripted {
    kind: String,
    title: String,
    base_url: Option<String>,
    complete: bool,
    decide: bool,
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
    let ops = c.ops.unwrap_or_else(|| vec!["complete".into()]);
    if let Some(other) = ops
        .iter()
        .find(|o| !["complete", "decide"].contains(&o.as_str()))
    {
        return Err(format!(
            "config: `ops` lists `{other}`; it answers `complete` and `decide`"
        ));
    }
    oxplow_script::runtime::check_starlark_defines(entry, script, FUNCTIONS)
        .map_err(|e| format!("`{entry}` {e}"))?;
    Ok(Arc::new(Scripted {
        kind: id.to_string(),
        title: title.to_string(),
        base_url: c.base_url,
        complete: ops.iter().any(|o| o == "complete"),
        decide: ops.iter().any(|o| o == "decide"),
        script: script.into(),
        gate,
        http: Http::default(),
    }))
}

impl Scripted {
    /// Run the script's `func` on `input`, sandboxed — once a person
    /// approved it.
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
        let budget = oxplow_script::SandboxBudget::with_timeout(SCRIPT_TIMEOUT);
        let ran = tokio::task::spawn_blocking(move || {
            oxplow_script::runtime::run_sandboxed(&budget, move || {
                oxplow_script::runtime::run_starlark_fn(&script, func, &input)
            })
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
        self.http.post_with(id, &url, headers, body).await.map(Some)
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
}
