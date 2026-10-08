//! Any OpenAI-compatible chat completions API: OpenAI itself, and local
//! servers (Ollama, LM Studio, vLLM, LiteLLM). Its declaration's
//! `config.baseUrl` is the default; without one every instance names its
//! own.

use oxplow_ai::client::{
    bearer, AiError, CompleteRequest, Completion, Http, ModelProvider, ProviderInstance,
};

pub(crate) struct OpenaiCompatible {
    kind: String,
    title: String,
    base_url: Option<String>,
    http: Http,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    #[serde(default)]
    base_url: Option<String>,
}

impl OpenaiCompatible {
    pub(crate) fn new(
        kind: String,
        title: String,
        config: &serde_json::Value,
    ) -> Result<Self, String> {
        let c: Config =
            serde_json::from_value(config.clone()).map_err(|e| format!("config: {e}"))?;
        Ok(Self {
            kind,
            title,
            base_url: c.base_url,
            http: Http::default(),
        })
    }
}

#[async_trait::async_trait]
impl ModelProvider for OpenaiCompatible {
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
        let base = instance.base_url_or(self.base_url.as_deref().unwrap_or_default());
        chat(&self.http, instance, &base, req).await
    }
}

/// One chat completion at `base`: what every OpenAI-compatible API takes.
pub(crate) async fn chat(
    http: &Http,
    instance: &ProviderInstance<'_>,
    base: &str,
    req: &CompleteRequest<'_>,
) -> Result<Completion, AiError> {
    let mut messages = Vec::new();
    if let Some(sys) = req.system.filter(|s| !s.trim().is_empty()) {
        messages.push(serde_json::json!({"role": "system", "content": sys}));
    }
    messages.push(serde_json::json!({"role": "user", "content": req.prompt}));
    let mut body = serde_json::json!({"model": req.model, "messages": messages});
    if req.json {
        body["response_format"] = serde_json::json!({"type": "json_object"});
    }
    let url = format!("{base}/chat/completions");
    let v = http
        .post(instance.id, &url, bearer(instance.key), body)
        .await?;
    let text = v["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| AiError::BadResponse {
            provider: instance.id.to_string(),
            detail: "no choices[0].message.content".into(),
        })?
        .to_string();
    Ok(Completion {
        text,
        input_tokens: v["usage"]["prompt_tokens"].as_i64().unwrap_or(0),
        output_tokens: v["usage"]["completion_tokens"].as_i64().unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_ai::client::{Answer, DecideRequest, Question};
    use oxplow_ai_fake::mock;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn local() -> OpenaiCompatible {
        OpenaiCompatible::new("openai_compatible".into(), "Local".into(), &json!({})).unwrap()
    }

    fn at<'a>(base: &'a str, key: Option<&'a str>) -> ProviderInstance<'a> {
        ProviderInstance {
            id: "p",
            base_url: Some(base),
            key,
        }
    }

    fn ask(json: bool) -> CompleteRequest<'static> {
        CompleteRequest {
            model: "qwen3",
            system: None,
            prompt: "q",
            json,
        }
    }

    #[tokio::test]
    async fn openai_compatible_chat_with_optional_key() {
        let (base, seen) = mock(
            "/chat/completions",
            200,
            json!({"choices": [{"message": {"content": "{\"a\":1}"}}], "usage": {"prompt_tokens": 5, "completion_tokens": 3}}),
        )
        .await;
        let c = local()
            .complete(&at(&base, None), &ask(true))
            .await
            .unwrap();
        assert_eq!(c.text, "{\"a\":1}");
        assert_eq!((c.input_tokens, c.output_tokens), (5, 3));
        let (_, headers, body) = seen.lock().unwrap()[0].clone();
        assert!(
            !headers.contains_key("authorization"),
            "local servers get no auth header"
        );
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[tokio::test]
    async fn decide_falls_back_to_a_json_prompt_on_chat_models() {
        let reply = json!({"answers": {"risky": {"type": "noul", "probability": 0.3}, "area": {"type": "choice", "choice": "ui", "probabilities": {"ui": 0.7, "db": 0.3}}}});
        let (base, seen) = mock(
            "/chat/completions",
            200,
            json!({"choices": [{"message": {"content": reply.to_string()}}], "usage": {"prompt_tokens": 50, "completion_tokens": 20}}),
        )
        .await;
        let questions = BTreeMap::from([
            (
                "risky".to_string(),
                Question::Noul {
                    instructions: "Is this change risky?".into(),
                },
            ),
            (
                "area".to_string(),
                Question::Choice {
                    instructions: "Which area?".into(),
                    options: vec!["ui".into(), "db".into()],
                },
            ),
        ]);
        let d = local()
            .decide(
                &at(&base, None),
                &DecideRequest {
                    model: "qwen3",
                    state: "diff…",
                    questions: &questions,
                },
            )
            .await
            .unwrap();
        assert_eq!(d.answers["risky"], Answer::Noul { probability: 0.3 });
        assert_eq!(d.input_tokens, 50);
        let body = seen.lock().unwrap()[0].2.clone();
        assert!(body["messages"]
            .to_string()
            .contains("Is this change risky?"));
    }

    #[tokio::test]
    async fn http_errors_are_explained() {
        let (base, _) = mock("/chat/completions", 401, json!({"error": "bad key"})).await;
        let err = local()
            .complete(&at(&base, Some("x")), &ask(false))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            AiError::Auth {
                provider: "p".into()
            }
        );
        let (base, _) = mock("/chat/completions", 429, json!({})).await;
        let err = local()
            .complete(&at(&base, Some("x")), &ask(false))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            AiError::RateLimited {
                provider: "p".into()
            }
        );
    }
}
