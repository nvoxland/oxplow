//! Anthropic's Messages API.

use oxplow_ai::client::{
    AiError, CompleteRequest, Completion, Http, ModelProvider, ProviderInstance,
};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

pub(crate) struct Anthropic {
    kind: String,
    title: String,
    http: Http,
}

impl Anthropic {
    pub(crate) fn new(kind: String, title: String) -> Self {
        Self {
            kind,
            title,
            http: Http::default(),
        }
    }
}

#[async_trait::async_trait]
impl ModelProvider for Anthropic {
    fn kind(&self) -> &str {
        &self.kind
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn default_base_url(&self) -> Option<&str> {
        Some(DEFAULT_BASE_URL)
    }

    async fn complete(
        &self,
        instance: &ProviderInstance<'_>,
        req: &CompleteRequest<'_>,
    ) -> Result<Completion, AiError> {
        let mut system = req.system.unwrap_or_default().to_string();
        if req.json {
            system.push_str("\n\nReply with a single JSON object and nothing else.");
        }
        let mut body = serde_json::json!({
            "model": req.model,
            "max_tokens": 4096,
            "messages": [{"role": "user", "content": req.prompt}],
        });
        if !system.trim().is_empty() {
            body["system"] = serde_json::Value::String(system.trim().to_string());
        }
        let mut headers = vec![("anthropic-version", "2023-06-01".to_string())];
        if let Some(k) = instance.key {
            headers.push(("x-api-key", k.to_string()));
        }
        let url = format!("{}/v1/messages", instance.base_url_or(DEFAULT_BASE_URL));
        let v = self.http.post(instance.id, &url, headers, body).await?;
        let text = v["content"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .ok_or_else(|| AiError::BadResponse {
                provider: instance.id.to_string(),
                detail: "no content".into(),
            })?;
        Ok(Completion {
            text,
            input_tokens: v["usage"]["input_tokens"].as_i64().unwrap_or(0),
            output_tokens: v["usage"]["output_tokens"].as_i64().unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_ai_fake::mock;
    use serde_json::json;

    #[tokio::test]
    async fn anthropic_messages() {
        let (base, seen) = mock(
            "/v1/messages",
            200,
            json!({"content": [{"type": "text", "text": "hi"}], "usage": {"input_tokens": 7, "output_tokens": 2}}),
        )
        .await;
        let c = Anthropic::new("anthropic".into(), "anthropic".into())
            .complete(
                &ProviderInstance {
                    id: "p",
                    base_url: Some(&base),
                    key: Some("k1"),
                },
                &CompleteRequest {
                    model: "claude-x",
                    system: Some("be brief"),
                    prompt: "hello",
                    json: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            c,
            Completion {
                text: "hi".into(),
                input_tokens: 7,
                output_tokens: 2,
            }
        );
        let (_, headers, body) = seen.lock().unwrap()[0].clone();
        assert_eq!(headers["x-api-key"], "k1");
        assert!(headers.contains_key("anthropic-version"));
        assert_eq!(body["model"], "claude-x");
        assert_eq!(body["system"], "be brief");
        assert_eq!(body["messages"][0]["content"], "hello");
    }
}
