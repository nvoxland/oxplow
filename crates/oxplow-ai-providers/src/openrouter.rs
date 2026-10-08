//! OpenRouter: one key for many models over OpenAI-compatible chat; a Jev
//! model answers typed questions natively (`/systemone`).

use oxplow_ai::client::{
    decide_via_chat, AiError, CompleteRequest, Completion, DecideRequest, Decision, Http,
    ModelProvider, ProviderInstance,
};

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";

pub(crate) struct Openrouter {
    kind: String,
    title: String,
    http: Http,
}

impl Openrouter {
    pub(crate) fn new(kind: String, title: String) -> Self {
        Self {
            kind,
            title,
            http: Http::default(),
        }
    }
}

#[async_trait::async_trait]
impl ModelProvider for Openrouter {
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
        let base = instance.base_url_or(DEFAULT_BASE_URL);
        crate::openai_compatible::chat(&self.http, instance, &base, req).await
    }

    async fn decide(
        &self,
        instance: &ProviderInstance<'_>,
        req: &DecideRequest<'_>,
    ) -> Result<Decision, AiError> {
        if req.model.contains("jev") {
            let url = format!("{}/systemone", instance.base_url_or(DEFAULT_BASE_URL));
            return crate::typesafe::systemone(&self.http, instance, &url, req).await;
        }
        decide_via_chat(self, instance, req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_ai::client::{Answer, Question};
    use oxplow_ai_fake::mock;
    use serde_json::json;
    use std::collections::BTreeMap;

    /// A Jev model answers on OpenRouter's `/systemone`.
    #[tokio::test]
    async fn a_jev_model_decides_natively() {
        let (base, seen) = mock(
            "/systemone",
            200,
            json!({"answers": {"risky": {"type": "noul", "noul": 0.6}}}),
        )
        .await;
        let questions = BTreeMap::from([(
            "risky".to_string(),
            Question::Noul {
                instructions: "Risky?".into(),
            },
        )]);
        let d = Openrouter::new("openrouter".into(), "openrouter".into())
            .decide(
                &ProviderInstance {
                    id: "or",
                    base_url: Some(&base),
                    key: Some("k"),
                },
                &DecideRequest {
                    model: "typesafe/jev-latest",
                    state: "s",
                    questions: &questions,
                },
            )
            .await
            .unwrap();
        assert_eq!(d.answers["risky"], Answer::Noul { probability: 0.6 });
        assert_eq!(seen.lock().unwrap()[0].2["model"], "typesafe/jev-latest");
    }
}
