//! TypeSafe: Jev's typed decisions (`/v1/systemone`). It only answers
//! typed questions; it has no chat.

use oxplow_ai::client::{
    bearer, parse_answers, AiError, Answer, CompleteRequest, Completion, DecideRequest, Decision,
    Http, ModelProvider, ProviderInstance, Question,
};

const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";

pub(crate) struct Typesafe {
    kind: String,
    title: String,
    http: Http,
}

impl Typesafe {
    pub(crate) fn new(kind: String, title: String) -> Self {
        Self {
            kind,
            title,
            http: Http::default(),
        }
    }
}

#[async_trait::async_trait]
impl ModelProvider for Typesafe {
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
        _: &CompleteRequest<'_>,
    ) -> Result<Completion, AiError> {
        Err(AiError::BadResponse {
            provider: instance.id.to_string(),
            detail: "TypeSafe (Jev) only answers typed questions; use it for the decide role"
                .into(),
        })
    }

    async fn decide(
        &self,
        instance: &ProviderInstance<'_>,
        req: &DecideRequest<'_>,
    ) -> Result<Decision, AiError> {
        let url = format!("{}/v1/systemone", instance.base_url_or(DEFAULT_BASE_URL));
        systemone(&self.http, instance, &url, req).await
    }

    /// It can't complete, so the check asks it one yes/no question.
    async fn test(&self, instance: &ProviderInstance<'_>, model: &str) -> Result<String, AiError> {
        let questions = std::collections::BTreeMap::from([(
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
            Some(Answer::Noul { probability }) => {
                format!("Answered (yes: {:.0}%)", probability * 100.0)
            }
            _ => "Answered".to_string(),
        })
    }
}

/// Jev's typed-decision call at `url` (TypeSafe's, or OpenRouter's).
pub(crate) async fn systemone(
    http: &Http,
    instance: &ProviderInstance<'_>,
    url: &str,
    req: &DecideRequest<'_>,
) -> Result<Decision, AiError> {
    let qs: serde_json::Map<String, serde_json::Value> = req
        .questions
        .iter()
        .map(|(name, q)| {
            let v = match q {
                Question::Noul { instructions } => {
                    serde_json::json!({"type": "noul", "instructions": instructions})
                }
                Question::Choice {
                    instructions,
                    options,
                } => serde_json::json!({
                    "type": "choice",
                    "instructions": instructions,
                    "criteria": options.iter().map(|o| (o.clone(), serde_json::Value::Null)).collect::<serde_json::Map<_, _>>(),
                }),
                Question::Score {
                    instructions,
                    levels,
                } => serde_json::json!({
                    "type": "score", "instructions": instructions, "criteria": levels,
                }),
            };
            (name.clone(), v)
        })
        .collect();
    let body = serde_json::json!({"model": req.model, "state": req.state, "questions": qs});
    let v = http
        .post(instance.id, url, bearer(instance.key), body)
        .await?;
    let answers = parse_answers(instance.id, &v["answers"], req.questions, "noul")?;
    Ok(Decision {
        answers,
        input_tokens: v["usage"]["input_tokens"].as_i64().unwrap_or(0),
        output_tokens: v["usage"]["output_tokens"].as_i64().unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_ai_fake::mock;
    use serde_json::{json, Value};
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn jev_systemone_natively() {
        let (base, seen) = mock(
            "/v1/systemone",
            200,
            json!({
                "model": "jev-1.13.0",
                "answers": {
                    "risky": {"type": "noul", "noul": 0.8},
                    "area": {"type": "choice", "choice": "db", "probabilities": {"ui": 0.1, "db": 0.9}, "confidence": 0.8}
                },
                "usage": {"input_tokens": 1000000, "output_tokens": 0}
            }),
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
        let d = Typesafe::new("typesafe".into(), "typesafe".into())
            .decide(
                &ProviderInstance {
                    id: "p",
                    base_url: Some(&base),
                    key: Some("tk"),
                },
                &DecideRequest {
                    model: "jev-latest",
                    state: "diff…",
                    questions: &questions,
                },
            )
            .await
            .unwrap();
        assert_eq!(d.answers["risky"], Answer::Noul { probability: 0.8 });
        assert!(matches!(&d.answers["area"], Answer::Choice { choice, .. } if choice == "db"));
        let (_, headers, body) = seen.lock().unwrap()[0].clone();
        assert_eq!(headers["authorization"], "Bearer tk");
        assert_eq!(body["state"], "diff…");
        assert_eq!(body["questions"]["risky"]["type"], "noul");
        assert_eq!(body["questions"]["area"]["criteria"]["db"], Value::Null);
    }

    /// Its connection check is a typed question, since it can't chat.
    #[tokio::test]
    async fn its_check_asks_a_question() {
        let (base, _) = mock(
            "/v1/systemone",
            200,
            json!({"answers": {"ok": {"type": "noul", "noul": 0.9}}}),
        )
        .await;
        let reply = Typesafe::new("typesafe".into(), "typesafe".into())
            .test(
                &ProviderInstance {
                    id: "p",
                    base_url: Some(&base),
                    key: None,
                },
                "jev",
            )
            .await
            .unwrap();
        assert_eq!(reply, "Answered (yes: 90%)");
    }
}
